//! Windows 薄接缝:PATH 注入写 HKCU 用户 PATH,shim 为 .cmd,Node 为 zip。

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::net;
use crate::prefix::{PathInjection, Prefix};

/// 把 bin_dir 追加进 HKCU 用户 PATH(幂等,保留原有 REG_EXPAND_SZ 类型)。
pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu.create_subkey("Environment")?;
    let dir = bin_dir.to_string_lossy().into_owned();

    // 现有值可能不存在;保留 REG_EXPAND_SZ(用户 PATH 常含 %VAR%)
    let raw = env.get_raw_value("Path").ok();
    let existing = match &raw {
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let Some(merged) = super::windows_path_merge(&existing, &dir) else {
        return Ok(Vec::new()); // 已在 PATH,重跑无副作用
    };

    set_user_path(&env, &merged, raw.as_ref())?;
    // 注:不广播 WM_SETTINGCHANGE(需额外 crate);新开的终端自然生效。
    Ok(vec![PathInjection::WindowsUserPath {
        dir: bin_dir.to_path_buf(),
    }])
}

/// doctor:PATH 持久化体检——读 HKCU\Environment\Path 原始值(不展开 %VAR%),
/// 与 install 幂等判断同一归一化(windows_path_contains)。只读,不改动注册表。
pub fn path_persisted(bin_dir: &Path) -> io::Result<bool> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env = match hkcu.open_subkey("Environment") {
        Ok(key) => key,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let raw = match env.get_raw_value("Path") {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(super::windows_path_contains(
        &raw.to_string(),
        &bin_dir.to_string_lossy(),
    ))
}

/// 写 HKCU 用户 Path:保留原有 REG_EXPAND_SZ 类型(ensure_path / rollback_injection 共用)。
/// REG_EXPAND_SZ 无现成构造函数:手工编码为带 NUL 结尾的 UTF-16LE。
fn set_user_path(
    env: &winreg::RegKey,
    merged: &str,
    raw: Option<&winreg::RegValue>,
) -> io::Result<()> {
    use winreg::enums::*;
    if raw.is_some_and(|v| v.vtype == REG_EXPAND_SZ) {
        let bytes: Vec<u8> = merged
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        let value = winreg::RegValue {
            vtype: REG_EXPAND_SZ,
            bytes: bytes.into(),
        };
        env.set_raw_value("Path", &value)?;
    } else {
        env.set_value("Path", &merged)?;
    }
    Ok(())
}

/// 按安装清单记录精确回滚一条 PATH 注入(保留原有 REG_EXPAND_SZ 类型)。
pub fn rollback_injection(injection: &PathInjection) -> io::Result<bool> {
    use winreg::enums::*;
    use winreg::RegKey;

    match injection {
        PathInjection::WindowsUserPath { dir } => {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let (env, _) = hkcu.create_subkey("Environment")?;
            let Some(raw) = env.get_raw_value("Path").ok() else {
                return Ok(false);
            };
            let existing = raw.to_string();
            let dir = dir.to_string_lossy();
            let Some(merged) = super::windows_path_remove(&existing, &dir) else {
                return Ok(false); // 已回滚过,幂等无副作用
            };
            set_user_path(&env, &merged, Some(&raw))?;
            Ok(true)
        }
        // fnm 钩子与 unix rc 行同为精确行注入,回滚语义逐字一致(工单 #17/#22)
        PathInjection::FnmHook { file, line } => {
            let existing = match fs::read_to_string(file) {
                Ok(text) => text,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(e) => return Err(e),
            };
            let Some(new) = super::shell_rc_remove(&existing, line) else {
                return Ok(false); // 已回滚过,幂等无副作用
            };
            fs::write(file, new)?;
            Ok(true)
        }
        // unix 注入类型不会出现在本平台的安装清单里
        PathInjection::ShellRc { .. } => Ok(false),
    }
}

/// git 版本:先看系统 PATH(用户自装,doctor 只报告),再退回前缀内 MinGit
pub fn git_version(prefix: &Prefix) -> Option<String> {
    super::version_output_of(Path::new("git"))
        .or_else(|| super::version_output_of(&prefix.git_exe()))
}

pub fn git_missing_hint() -> &'static str {
    "重跑 setup-coder install 自动安装便携版 git(MinGit)"
}

pub fn write_shim(
    bin_dir: &Path,
    node_exe: &Path,
    tool_launcher: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    fs::create_dir_all(bin_dir)?;
    let path = bin_dir.join(super::shim_file_name(bin));
    fs::write(&path, super::shim_content(node_exe, tool_launcher, bin))?;
    Ok(path)
}

pub fn install_self(bin_dir: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(bin_dir)?;
    let current = std::env::current_exe()?;
    let dest = bin_dir.join(super::exe_name("setup-coder"));
    if current == dest {
        return Ok(dest);
    }
    fs::copy(&current, &dest)?;
    Ok(dest)
}

/// 确保 git 可用:系统已有 → 跳过;否则下载 MinGit 便携版解进前缀 `git/`,
/// 并在 `bin/` 生成 git shim(shim 在前缀内,无需 state.json 记录;PATH 上只有 bin/)。
pub fn ensure_git(prefix: &Prefix) -> Result<super::GitOutcome, Box<dyn Error>> {
    if super::git_on_path_works() {
        return Ok(super::GitOutcome::Skipped);
    }
    let git_exe = prefix.git_exe();
    if !super::git_works_at(&git_exe) {
        println!("下载 MinGit {} 便携版…", net::MINGIT_VERSION);
        let archive = prefix.cache_dir().join(net::mingit_archive_name());
        let hit = net::download_first(&net::mingit_urls(), &archive)?;
        println!("已从 Mirror 下载:{hit}");

        // 解压到暂存目录,成功后整体替换 git/(避免半残前缀;MinGit zip 根目录即 cmd/,不剥层)
        let staging = prefix.cache_dir().join("git-staging");
        let _ = fs::remove_dir_all(&staging);
        extract_zip(&archive, &staging, false)?;
        let git_dir = prefix.git_dir();
        let _ = fs::remove_dir_all(&git_dir);
        fs::rename(&staging, &git_dir)?;

        // 自检:刚解压的 git 必须能跑
        if !super::git_works_at(&git_exe) {
            return Err("MinGit 解压后自检失败:`git --version` 未通过".into());
        }
    }

    // shim 进 bin/(与 Tool 同一处理:bin/ 是 Private Prefix 对外的唯一可见面)
    fs::create_dir_all(prefix.bin_dir())?;
    let shim = prefix.bin_dir().join(super::shim_file_name("git"));
    fs::write(&shim, super::git_shim_content(&git_exe))?;
    println!("git shim:{}", shim.display());
    Ok(super::GitOutcome::Installed)
}

/// zip 解压:`strip_top` 为 true 时剥掉顶层目录一层(Node 发行包),false 原样解(MinGit)。
fn extract_zip(archive: &Path, dest_dir: &Path, strip_top: bool) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("zip 损坏:{e}")))?;
    fs::create_dir_all(dest_dir)?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let rel: PathBuf = if strip_top {
            path.components().skip(1).collect()
        } else {
            path
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest_dir.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut w = fs::File::create(&out)?;
            io::copy(&mut entry, &mut w)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// node 来源探测 + fnm 执行原语(工单 #18,Windows 侧)
// ---------------------------------------------------------------------------

/// 探测 Node 来源事实(Windows):裸 Node 走 PATH + `node --version`;
/// nvm 走 NVM_DIR 环境变量 + `%APPDATA%\nvm`(nvm-windows);fnm 走 PATH +
/// `%LOCALAPPDATA%\fnm` + PowerShell profile 钩子痕迹(覆盖「已装未 source」)。

pub fn detect_node_facts() -> crate::node_plan::NodeFacts {
    crate::node_plan::NodeFacts {
        bare_node: detect_bare_node_windows(),
        nvm: detect_nvm_windows(),
        fnm: detect_fnm_windows(),
    }
}

/// fnm 默认数据目录:`%LOCALAPPDATA%\fnm`(fnm 官方安装脚本默认)。
fn fnm_default_dir_windows() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("fnm"))
}

/// 裸 Node:PATH 上的 `node.exe`,`--version` 解析版本。
fn detect_bare_node_windows() -> Option<(semver::Version, PathBuf)> {
    let out = Command::new("where.exe").arg("node").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let first = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if first.is_empty() {
        return None;
    }
    let exe = PathBuf::from(first);
    let version = super::parse_node_version(&super::version_output_of(&exe)?)?;
    Some((version, exe))
}

/// nvm-windows:安装痕迹 = NVM_DIR 环境变量或 `%APPDATA%\nvm` 目录;nvm 自身路径取其目录。
fn detect_nvm_windows() -> Option<(semver::Version, PathBuf)> {
    let dir = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("nvm")))?;
    if !dir.is_dir() {
        return None;
    }
    // nvm-windows 无 alias/default 文件;当前版本读不到则记 0.0.0(决策只看有无 + 路径)
    Some((semver::Version::new(0, 0, 0), dir))
}

/// fnm(Windows):优先 PATH 上的 fnm.exe,否则看默认数据目录或 PowerShell profile 钩子。
fn detect_fnm_windows() -> Option<(semver::Version, PathBuf)> {
    if let Ok(out) = Command::new("where.exe").arg("fnm").output() {
        if out.status.success() {
            if let Some(first) = String::from_utf8_lossy(&out.stdout).lines().next() {
                let exe = PathBuf::from(first.trim());
                if !exe.as_os_str().is_empty() {
                    let version = current_fnm_node_version_windows(&exe)
                        .unwrap_or(semver::Version::new(0, 0, 0));
                    return Some((version, exe));
                }
            }
        }
    }
    let dir = fnm_default_dir_windows()?;
    let profile_hint = powershell_profiles()
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .any(|c| super::fnm_hook_present_powershell(&c));
    if !dir.is_dir() && !profile_hint {
        return None;
    }
    let exe = dir.join(super::exe_name("fnm"));
    let version = current_fnm_node_version_windows(&exe).unwrap_or(semver::Version::new(0, 0, 0));
    Some((version, dir))
}

/// fnm 默认安装/数据目录(Windows 平台定义,与 unix 的 `~/.local/share/fnm` 同位):
/// `%LOCALAPPDATA%\fnm`(fnm 官方安装脚本默认)。node_source 的 InstallFnm 解析接缝
/// 与探测、install_fnm 落盘共用此目录。LOCALAPPDATA 缺失时回退 `<home>/.fnm`。
pub fn fnm_default_dir(home: &Path) -> PathBuf {
    fnm_default_dir_windows().unwrap_or_else(|| home.join(".fnm"))
}

/// 读 fnm 当前默认 Node 版本:`fnm list` 解析 default/最新;失败返回 None。
fn current_fnm_node_version_windows(fnm_exe: &Path) -> Option<semver::Version> {
    let out = Command::new(fnm_exe).arg("list").output().ok()?;
    if !out.status.success() {
        return None;
    }
    super::parse_fnm_list_windows(&String::from_utf8_lossy(&out.stdout))
}

/// 下载安装 fnm(Windows):容错链下载 fnm-windows.zip,解出单文件 fnm.exe。幂等复用。
pub fn install_fnm(cache_dir: &Path, dest_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let exe = dest_dir.join(super::exe_name("fnm"));
    if super::version_output_of(&exe).is_some() {
        return Ok(exe); // 已装且可用:幂等复用
    }
    let asset = super::fnm_asset_suffix()?;
    let archive = cache_dir.join(net::fnm_archive_name(asset));
    let hit = net::download_first(&net::fnm_urls(asset), &archive)?;
    println!("已从 Mirror 下载 fnm:{hit}");
    extract_zip(&archive, dest_dir, false)?; // fnm-windows.zip 根目录即 fnm.exe,不剥层
    if super::version_output_of(&exe).is_none() {
        return Err("fnm 解压后自检失败:`fnm --version` 未通过".into());
    }
    Ok(exe)
}

/// 用 fnm 装指定 Node 版本并设为默认(Windows)。
pub fn fnm_install_and_default(fnm_exe: &Path, version: &str) -> Result<(), Box<dyn Error>> {
    run_fnm_windows(fnm_exe, &["install", version])?;
    run_fnm_windows(fnm_exe, &["default", version])
}

/// 经 nvm-windows 安装指定 Node 版本并解析 node.exe 绝对路径(工单 #20)。
///
/// nvm-windows 是 `nvm.exe` 二进制(与 unix 的 shell 函数不同),直接按绝对路径调用。
/// 只装、只解析,不 `nvm use`(不劫持用户当前切换);版本目录布局 NVM_HOME/vX.Y.Z/。
pub fn nvm_install_and_resolve(nvm_dir: &Path, version: &str) -> Result<PathBuf, Box<dyn Error>> {
    let exe = nvm_dir.join(super::exe_name("nvm"));
    let out = Command::new(&exe).args(["install", version]).output()?;
    if !out.status.success() {
        return Err(format!(
            "经 nvm-windows({})安装 Node {} 失败:{}",
            exe.display(),
            version,
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(
        super::resolve_manager_node(nvm_dir, version, super::ManagerKind::Nvm).unwrap_or_else(
            || {
                nvm_dir
                    .join(format!("v{version}"))
                    .join(super::exe_name("node"))
            },
        ),
    )
}

fn run_fnm_windows(fnm_exe: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let out = Command::new(fnm_exe).args(args).output()?;
    if !out.status.success() {
        return Err(format!(
            "`fnm {}` 失败:{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(())
}

/// 注入 fnm 钩子到 PowerShell profile(幂等)。返回实际改动的 FnmHook 记录。
pub fn inject_fnm_hook() -> io::Result<Vec<PathInjection>> {
    let line = super::fnm_hook_line_powershell();
    let mut injections = Vec::new();
    for profile in powershell_profiles() {
        let existing = match fs::read_to_string(&profile) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if let Some(new) = super::shell_rc_append(&existing, &line) {
            if let Some(parent) = profile.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&profile, new)?;
            injections.push(PathInjection::FnmHook {
                file: profile,
                line: line.clone(),
            });
        }
    }
    Ok(injections)
}

/// 用户级 PowerShell profile 路径清单(`$PROFILE` 等价:`~\Documents\PowerShell\` 与
/// 旧版 `~\Documents\WindowsPowerShell\` 的 Microsoft.PowerShell_profile.ps1)。
fn powershell_profiles() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::home_dir() {
        let docs = home.join("Documents");
        out.push(
            docs.join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
        );
        out.push(
            docs.join("WindowsPowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
        );
    }
    out
}
