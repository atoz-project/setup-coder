//! Windows 薄接缝:PATH 注入写 HKCU 用户 PATH,shim 为 .cmd,Node 为 zip。

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

    if let Some(v) = raw {
        if v.vtype == REG_EXPAND_SZ {
            // REG_EXPAND_SZ 无现成构造函数:手工编码为带 NUL 结尾的 UTF-16LE
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
    } else {
        env.set_value("Path", &merged)?;
    }
    // 注:不广播 WM_SETTINGCHANGE(需额外 crate);新开的终端自然生效。
    Ok(vec![PathInjection::WindowsUserPath {
        dir: bin_dir.to_path_buf(),
    }])
}

/// 按安装清单记录精确回滚一条 PATH 注入(保留原有 REG_EXPAND_SZ 类型)。
pub fn rollback_injection(injection: &PathInjection) -> io::Result<bool> {
    use winreg::enums::*;
    use winreg::RegKey;

    let PathInjection::WindowsUserPath { dir } = injection else {
        // unix 注入类型不会出现在本平台的安装清单里
        return Ok(false);
    };
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
    if raw.vtype == REG_EXPAND_SZ {
        // 与 ensure_path 同一编码:带 NUL 结尾的 UTF-16LE
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
    Ok(true)
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
    node_bin_dir: &Path,
    npm_bin_dir: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    fs::create_dir_all(bin_dir)?;
    let path = bin_dir.join(super::shim_file_name(bin));
    fs::write(&path, super::shim_content(node_bin_dir, npm_bin_dir, bin))?;
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

/// Windows:zip 解压,剥掉顶层目录一层。
pub fn extract_node_archive(archive: &Path, dest_dir: &Path) -> io::Result<()> {
    extract_zip(archive, dest_dir, true)
}
