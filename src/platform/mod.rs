//! 平台分叉收敛处:PATH 注入、shim 生成、Node 解压。
//!
//! commands 层写"做什么",这里写"在这个系统上怎么做"。
//! 纯逻辑(字符串拼接/幂等判断)直接放在本文件,全平台可编译、可单测;
//! 涉及文件系统/注册表的薄接缝在 `windows.rs` / `macos.rs` / `linux.rs`。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use std::error::Error;
use std::process::Command;
#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

use crate::prefix::{PathInjection, Prefix};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
use windows as imp;

// ---------------------------------------------------------------------------
// 纯逻辑(全平台可编译,单测覆盖)
// ---------------------------------------------------------------------------

/// Node 发行版目标后缀:`node-vX.Y.Z-<suffix>.<ext>`
pub fn node_dist_suffix_for(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-x64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        ("windows", "x86_64") => Ok("win-x64"),
        _ => Err(format!("暂不支持的平台组合:{os}/{arch}")),
    }
}

/// 当前平台的 Node 发行版目标后缀
pub fn node_dist_suffix() -> Result<&'static str, String> {
    node_dist_suffix_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// Node 发行包扩展名(Windows 为 zip,其余 tar.gz)
pub fn node_archive_ext_for(os: &str) -> &'static str {
    match os {
        "windows" => "zip",
        _ => "tar.gz",
    }
}

pub fn node_archive_ext() -> &'static str {
    node_archive_ext_for(std::env::consts::OS)
}

/// Node 可执行文件相对其解压根目录的子目录(unix `bin/`,Windows 根目录)
pub const fn node_bin_subdir() -> &'static str {
    if cfg!(windows) {
        ""
    } else {
        "bin"
    }
}

/// npm 全局 bin 相对 npm prefix 的子目录(unix `bin/`,Windows 根目录)
pub const fn npm_bin_subdir() -> &'static str {
    if cfg!(windows) {
        ""
    } else {
        "bin"
    }
}

/// 可执行文件名(Windows 加 .exe)
pub fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// shim 文件名(Windows 为 `<bin>.cmd`)
pub fn shim_file_name(bin: &str) -> String {
    if cfg!(windows) {
        format!("{bin}.cmd")
    } else {
        bin.to_string()
    }
}

/// shell rc 追加行内容(幂等判断与 state.json 记录都以这行为准)
#[cfg(any(unix, test))]
pub fn shell_rc_export_line(bin_dir: &Path) -> String {
    format!("export PATH=\"{}:$PATH\"  # setup-coder", bin_dir.display())
}

/// shell rc 精确回滚:删除与安装清单记录逐字匹配的行(trim 比较,防御手抖编辑),
/// 返回删除后的完整新内容;没有匹配行 = 已回滚过,返回 None。不做模糊匹配(工单 #4)。
/// 按行文本处理,与平台无关:unix rc 与 Windows PowerShell profile 共用(工单 #18)。
#[cfg_attr(windows, allow(dead_code))] // Windows 的 fnm 钩子回滚随 uninstall 接线(工单 #19+)
pub fn shell_rc_remove(existing: &str, line: &str) -> Option<String> {
    let target = line.trim();
    let mut removed = false;
    let kept: Vec<&str> = existing
        .lines()
        .filter(|l| {
            if l.trim() == target {
                removed = true;
                false
            } else {
                true
            }
        })
        .collect();
    if !removed {
        return None;
    }
    let mut new = kept.join("\n");
    if !new.is_empty() {
        new.push('\n');
    }
    Some(new)
}

/// shell rc 内容中是否已有该行(trim 比较,与 append/remove 的幂等判断同规则)。
/// doctor 的「PATH 已持久化」体检(工单 #9)与 fnm 钩子幂等(工单 #18)用;平台无关。
pub fn shell_rc_contains(existing: &str, export_line: &str) -> bool {
    existing
        .lines()
        .any(|line| line.trim() == export_line.trim())
}

/// shell rc 幂等追加:内容中已有该行则返回 None,否则返回追加后的完整新内容。
/// 平台无关:unix rc 与 Windows PowerShell profile 共用(fnm 钩子注入,工单 #18)。
pub fn shell_rc_append(existing: &str, export_line: &str) -> Option<String> {
    if shell_rc_contains(existing, export_line) {
        return None;
    }
    let mut new = existing.to_string();
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(export_line);
    new.push('\n');
    Some(new)
}

/// Windows 用户 PATH 条目归一化(幂等比较共用):忽略大小写、首尾空白与尾部 `\`/`/`。
#[cfg(any(windows, test))]
fn normalize_windows_path_entry(s: &str) -> String {
    s.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

/// Windows 用户 PATH 是否已含该目录(与 merge/remove 同一归一化)。
/// install 的幂等判断与 doctor 的「PATH 已持久化」体检(工单 #9)共用。
#[cfg(any(windows, test))]
pub fn windows_path_contains(existing: &str, dir: &str) -> bool {
    let target = normalize_windows_path_entry(dir);
    existing
        .split(';')
        .map(normalize_windows_path_entry)
        .any(|entry| !entry.is_empty() && entry == target)
}

/// Windows 用户 PATH 幂等合并:已含该目录则返回 None,否则返回合并后的完整 PATH。
#[cfg(any(windows, test))]
pub fn windows_path_merge(existing: &str, dir: &str) -> Option<String> {
    if windows_path_contains(existing, dir) {
        return None;
    }
    let mut new = existing.trim_end_matches(';').to_string();
    if !new.is_empty() {
        new.push(';');
    }
    new.push_str(dir.trim());
    Some(new)
}

/// Windows 用户 PATH 精确回滚:移除与安装清单记录匹配的目录(忽略大小写、
/// 忽略首尾 `\`/`/` 与空白,与 merge 同一归一化),返回移除后的完整 PATH;
/// 没有匹配项 = 已回滚过,返回 None。不做模糊匹配(工单 #4)。
#[cfg(any(windows, test))]
pub fn windows_path_remove(existing: &str, dir: &str) -> Option<String> {
    fn normalize(s: &str) -> String {
        s.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase()
    }
    let target = normalize(dir);
    let entries: Vec<&str> = existing.split(';').collect();
    let kept: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|entry| {
            let n = normalize(entry);
            n.is_empty() || n != target
        })
        .collect();
    if kept.len() == entries.len() {
        return None;
    }
    Some(kept.join(";"))
}

/// unix shim 内容:把前缀内 node 加进 PATH,再转交 npm 全局 bin。
fn unix_shim_content(node_bin_dir: &Path, npm_bin_dir: &Path, bin: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # setup-coder shim: {bin}(由 install 生成,重跑覆盖)\n\
         export PATH=\"{}:$PATH\"\n\
         exec \"{}\" \"$@\"\n",
        node_bin_dir.display(),
        npm_bin_dir.join(bin).display(),
    )
}

/// Windows shim 内容(.cmd):同上语义。
fn windows_shim_content(node_bin_dir: &Path, npm_bin_dir: &Path, bin: &str) -> String {
    format!(
        "@echo off\r\n\
         rem setup-coder shim: {bin}(由 install 生成,重跑覆盖)\r\n\
         set \"PATH={};%PATH%\"\r\n\
         \"{}\\{bin}.cmd\" %*\r\n\
         exit /b %errorlevel%\r\n",
        node_bin_dir.display(),
        npm_bin_dir.display(),
    )
}

/// 当前平台的 shim 内容(供薄接缝落盘)。
///
/// 契约预告(工单 #21):shim 将改为按绝对路径 exec 选定 Node
/// (取 `node_source::resolve` 的 exe 路径),而非把 `node_bin_dir` 前置进 PATH;
/// 届时本函数签名的 `node_bin_dir` 入参会替换为 node exe 绝对路径。
pub fn shim_content(node_bin_dir: &Path, npm_bin_dir: &Path, bin: &str) -> String {
    if cfg!(windows) {
        windows_shim_content(node_bin_dir, npm_bin_dir, bin)
    } else {
        unix_shim_content(node_bin_dir, npm_bin_dir, bin)
    }
}

/// Node 自带 npm-cli.js 相对 `node/` 的路径(unix 在 `lib/` 下,Windows 在根)
pub fn npm_cli_subpath() -> PathBuf {
    if cfg!(windows) {
        ["node_modules", "npm", "bin", "npm-cli.js"]
            .iter()
            .collect()
    } else {
        ["lib", "node_modules", "npm", "bin", "npm-cli.js"]
            .iter()
            .collect()
    }
}

/// 安装后提示用户如何让 PATH 生效(平台文案分叉收敛于此)
pub fn path_activation_hint() -> &'static str {
    if cfg!(windows) {
        "PATH 已写入用户环境变量;请新开一个终端窗口,然后验证:"
    } else {
        "PATH 已写入 shell 配置;请新开终端(或 source 对应 rc 文件)后验证:"
    }
}

// ---------------------------------------------------------------------------
// uninstall / doctor(工单 #4)——纯逻辑与共享助手
// ---------------------------------------------------------------------------

/// 运行 `<exe> --version`,成功且输出非空则返回去空白后的版本串;否则 None。
pub fn version_output_of(exe: &Path) -> Option<String> {
    let out = Command::new(exe).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// 进程 PATH 环境变量是否包含指定目录(逐条比较;components 归一化尾部斜杠)。
/// 注:Windows 上按逐字比较 —— install 写入的正是这个路径,新开终端后原样出现。
pub fn path_contains_dir(path_var: &std::ffi::OsStr, dir: &Path) -> bool {
    fn normalize(p: &Path) -> PathBuf {
        p.components().collect()
    }
    let target = normalize(dir);
    std::env::split_paths(path_var).any(|p| normalize(&p) == target)
}

/// 删除 `root` 下除 `keep` 文件及其祖先目录外的全部内容(Windows 自身 exe 残留策略用)。
/// file_type 不跟随符号链接:npm 全局 bin 里的 symlink 只删链接本身,不误删目标。
pub fn remove_all_except(root: &Path, keep: &Path) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path == keep {
            continue;
        }
        if entry.file_type()?.is_dir() {
            if keep.starts_with(&path) {
                remove_all_except(&path, keep)?;
                // keep 在其中,目录非空删不掉属预期,忽略错误
                let _ = fs::remove_dir(&path);
            } else {
                fs::remove_dir_all(&path)?;
            }
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// 删除 Private Prefix 目录树。
///
/// 决策(工单 #4,Windows 边角):正在运行的 exe 删不掉自己 —— 策略为「其余全删,
/// 自身留下」:先整体删;失败且当前 exe 在前缀内(仅 Windows 会发生)时,改为删除
/// 除自身 exe 外的全部内容,返回 Some(自身 exe 路径),由 commands 层中文提示手删。
/// unix 可删正在运行的文件,恒走整体删。真机验证归工单 #7;残留逻辑由
/// remove_all_except 的单测覆盖。
pub fn remove_prefix(root: &Path) -> io::Result<Option<PathBuf>> {
    match fs::remove_dir_all(root) {
        Ok(()) => Ok(None),
        Err(e) => {
            let locked_self = if cfg!(windows) {
                std::env::current_exe()
                    .ok()
                    .filter(|exe| exe.starts_with(root))
            } else {
                None
            };
            match locked_self {
                Some(exe) => {
                    remove_all_except(root, &exe)?;
                    Ok(Some(exe))
                }
                None => Err(e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Prerequisite:git(工单 #3)——纯逻辑与共享助手
// ---------------------------------------------------------------------------

/// git 安装结果(commands 层据此打印中文小结)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOutcome {
    /// 系统已有可用 git,未做任何改动
    Skipped,
    /// 本次装好了 git(Windows MinGit / macOS CLT / Ubuntu apt)
    Installed,
}

/// Windows git shim 内容(.cmd):转交前缀内 MinGit 的 cmd/git.exe
#[cfg(any(windows, test))]
pub fn git_shim_content(git_exe: &Path) -> String {
    format!(
        "@echo off\r\n\
         rem setup-coder shim: git(由 install 生成,重跑覆盖)\r\n\
         \"{}\" %*\r\n\
         exit /b %errorlevel%\r\n",
        git_exe.display(),
    )
}

/// 系统 PATH 上的 git 是否可用(`git --version` 通过)。
/// macOS 无 CLT 时 /usr/bin/git 是 stub:本调用可能触发系统安装弹窗,属预期。
pub(super) fn git_on_path_works() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// 指定路径的 git 可执行文件是否可用(Windows 检查前缀内 MinGit)
#[cfg(any(windows, test))]
pub(super) fn git_works_at(exe: &Path) -> bool {
    exe.exists()
        && Command::new(exe)
            .arg("--version")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
}

/// 轮询等待:每 `interval` 调一次 `check`,通过返回 true,`timeout` 内未通过返回 false。
/// 可单测的等待接缝(macOS 等 CLT 安装用)。
#[cfg(any(target_os = "macos", test))]
pub fn wait_until(mut check: impl FnMut() -> bool, timeout: Duration, interval: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if check() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(interval);
    }
}

/// 在 PATH 目录清单中查找可执行文件(Ubuntu 检测 sudo、node 来源探测找 node/fnm 用)。
/// unix 两平台共用;Windows 走 where.exe,不用本函数。
#[cfg(any(unix, test))]
pub fn find_in_path(name: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var).find_map(|dir| {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            Some(candidate)
        } else {
            None
        }
    })
}

#[cfg(any(unix, test))]
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && path
                .metadata()
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        path.is_file()
    }
}

/// Ubuntu:无 sudo 时的中文报错(含手工命令)
#[cfg(any(target_os = "linux", test))]
pub fn no_sudo_error() -> String {
    "未检测到 git,且当前环境没有 sudo,无法自动安装。\
     请用有管理员权限的账户手工执行以下命令后重跑 install:\n  \
     apt-get update && apt-get install -y git"
        .to_string()
}

/// Ubuntu:apt 失败时的中文报错(含手工命令)
#[cfg(any(target_os = "linux", test))]
pub fn apt_install_failed_error() -> String {
    "通过 apt 安装 git 失败。可能是软件源未更新或网络问题,请手工执行以下命令后重跑 install:\n  \
     sudo apt-get update && sudo apt-get install -y git"
        .to_string()
}

/// macOS:等待 CLT 安装的超时与轮询间隔(弹窗后用户手工点击,下载可能较慢)
#[cfg(any(target_os = "macos", test))]
pub const GIT_WAIT_TIMEOUT: Duration = Duration::from_secs(60 * 60);
#[cfg(any(target_os = "macos", test))]
pub const GIT_POLL_INTERVAL: Duration = Duration::from_secs(15);

/// macOS:触发弹窗后的中文提示
#[cfg(any(target_os = "macos", test))]
pub fn clt_prompt_message() -> &'static str {
    "git 需要 Xcode 命令行工具。已触发安装弹窗:请在弹窗中点击「安装」并同意协议;\
     如未看到弹窗,请手工执行 xcode-select --install"
}

/// macOS:等待超时的中文报错(含手工命令)
#[cfg(any(target_os = "macos", test))]
pub fn clt_wait_timeout_error() -> String {
    "等待 Xcode 命令行工具安装超时(仍未检测到 git)。\
     请确认安装弹窗中的流程已完成,或手工执行 xcode-select --install 后重跑 install"
        .to_string()
}

// ---------------------------------------------------------------------------
// 平台薄接缝(文件系统/注册表操作,分派到 imp)
// ---------------------------------------------------------------------------

/// 把 `bin_dir` 注入用户 PATH(幂等),返回实际发生的改动记录。
pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    imp::ensure_path(bin_dir)
}

/// doctor 体检:PATH 持久化载体(Windows:HKCU 用户 Path;unix:shell rc 文件)
/// 是否已含 bin_dir。不依赖当前会话环境变量——这是工单 #9 的核心检查项。
/// 只读;读取出错返回 Err 由 doctor 报告。
pub fn path_persisted(bin_dir: &Path) -> io::Result<bool> {
    imp::path_persisted(bin_dir)
}

/// doctor 输出用:本平台 PATH 持久化载体的中文描述(平台文案分叉收敛于此)
pub fn path_persistence_location() -> &'static str {
    if cfg!(windows) {
        "用户环境变量 Path(注册表 HKCU\\Environment)"
    } else {
        "shell rc 文件(# setup-coder 标记行)"
    }
}

/// 生成 shim 到 `bin_dir`(重跑覆盖)。
/// 契约预告(工单 #21):同 `shim_content`,`node_bin_dir` 入参将替换为选定 Node 的 exe 绝对路径。
pub fn write_shim(
    bin_dir: &Path,
    node_bin_dir: &Path,
    npm_bin_dir: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    imp::write_shim(bin_dir, node_bin_dir, npm_bin_dir, bin)
}

/// 把 setup-coder 本体复制进 `bin_dir`(与 PATH 注入配合,uninstall 才能整体回收)。
pub fn install_self(bin_dir: &Path) -> io::Result<PathBuf> {
    imp::install_self(bin_dir)
}

/// Prerequisite:确保 git 可用(工单 #3)。
/// 已有 git → Skipped;否则按平台装(Windows MinGit / macOS CLT / Ubuntu apt)。
pub fn ensure_git(prefix: &Prefix) -> Result<GitOutcome, Box<dyn Error>> {
    imp::ensure_git(prefix)
}

/// 解压 Node 发行包到 `dest_dir`(剥掉顶层 `node-vX-…/` 一层)。
pub fn extract_node_archive(archive: &Path, dest_dir: &Path) -> io::Result<()> {
    imp::extract_node_archive(archive, dest_dir)
}

/// 按 state.json 记录精确回滚一条 PATH 注入(不做模糊匹配)。
/// Ok(true) = 实际回滚了;Ok(false) = 对应内容已不存在(幂等,无需处理)。
/// unix 两平台实现相同,直接收在此处;Windows 走注册表,分派 imp。
#[cfg(unix)]
pub fn rollback_injection(injection: &PathInjection) -> io::Result<bool> {
    match injection {
        PathInjection::ShellRc { file, line } => rollback_shell_rc(file, line),
        // fnm 钩子与 PATH 行同为 rc 精确行注入,回滚语义逐字一致(工单 #17)
        PathInjection::FnmHook { file, line } => rollback_shell_rc(file, line),
        // Windows 注入类型不会出现在本平台的安装清单里
        PathInjection::WindowsUserPath { .. } => Ok(false),
    }
}

/// Windows:按 state.json 记录精确回滚一条 PATH 注入(HKCU 用户 PATH)
#[cfg(windows)]
pub fn rollback_injection(injection: &PathInjection) -> io::Result<bool> {
    imp::rollback_injection(injection)
}

/// git 版本串:unix 只看系统 PATH(系统 git 在前缀外,doctor 只报告);
/// Windows 先看系统 PATH 再看前缀内 MinGit。
pub fn git_version(prefix: &Prefix) -> Option<String> {
    imp::git_version(prefix)
}

/// doctor 体检时 git 不可用的指引文案(平台文案分叉收敛于此)
pub fn git_missing_hint() -> &'static str {
    imp::git_missing_hint()
}
// ---------------------------------------------------------------------------
// Prerequisite:node 来源探测 + fnm 执行原语(工单 #18)
//
// 探测产出 NodeFacts(裸 Node / nvm / fnm 各自的有无、版本、路径),供决策层
// (node_plan::decide)消费;执行原语(装 fnm / fnm 装 Node / 注入 shell 钩子)
// 落实施工。本工单只落地接缝,尚未接线到 install(工单 #19+),故死代码豁免。
// 平台分叉(Windows 注册表 / unix rc / where.exe)收敛在 imp,命令层不见 cfg。
// ---------------------------------------------------------------------------

/// 探测机器上的 Node 来源事实,供 node_plan::decide 决策(分派 imp)。
///
/// 必须能识别「已装但未在当前 shell 生效」的安装(读安装痕迹与版本,而非仅看
/// 当前 PATH):nvm 走默认目录 + rc 行,fnm 走数据目录 + 可执行文件。
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn detect_node_facts() -> crate::node_plan::NodeFacts {
    imp::detect_node_facts()
}

/// 下载并安装 fnm 到 `dest_dir`,返回 fnm 可执行文件路径(分派 imp)。
/// 经 net::fnm_urls 镜像容错链下载;幂等:目标已装且可用则直接复用。
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn install_fnm(cache_dir: &Path, dest_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    imp::install_fnm(cache_dir, dest_dir)
}

/// 用给定 fnm 装指定 Node 版本并设为默认:`fnm install <version>` + `fnm default <version>`。
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn fnm_install_and_default(fnm_exe: &Path, version: &str) -> Result<(), Box<dyn Error>> {
    imp::fnm_install_and_default(fnm_exe, version)
}

/// 把 fnm 的 shell 钩子幂等注入用户 shell 配置文件(unix:各登录 rc;Windows:PowerShell
/// profile)。重跑不产生重复行。unix 返回实际改动的注入记录(供 state.json 精确回滚)。
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn inject_fnm_hook() -> io::Result<Vec<PathInjection>> {
    imp::inject_fnm_hook()
}

/// 当前平台的 fnm 发行资产后缀(fnm-<suffix>.zip)
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn fnm_asset_suffix() -> Result<&'static str, String> {
    fnm_asset_suffix_for(std::env::consts::OS, std::env::consts::ARCH)
}

// ---------------------------------------------------------------------------
// node 探测纯逻辑(全平台可编译,单测覆盖)
// ---------------------------------------------------------------------------

/// 从 `node --version` 输出解析版本(如 "v24.19.0" / "24.19.0");无法解析返回 None。
pub fn parse_node_version(out: &str) -> Option<semver::Version> {
    parse_semver_like(out.trim())
}

/// 从 `fnm --version` 输出解析版本(如 "fnm 1.39.0" / "1.39.0");无法解析返回 None。
#[allow(dead_code)] // fnm 版本串解析,供 doctor/执行层自检,未接线(工单 #19+)
pub fn parse_fnm_version(out: &str) -> Option<semver::Version> {
    parse_semver_like(out.trim())
}

/// 从 `fnm list` 输出解析「当前默认」的 Node 版本(用于确定 nvm/fnm 管理的当前 Node)。
///
/// fnm list 形如(`* system` 为指向系统裸 Node 的别名,不算 fnm 自己装的版本):
/// ```text
/// * v20.19.0
/// * v22.19.0 default
/// * system
/// ```
/// 策略:优先取带 `default` 标记的行;否则取最后一行非 system 的版本(fnm 装新版本追加在尾)。
fn parse_fnm_list(out: &str) -> Option<semver::Version> {
    let mut marked = None;
    let mut last = None;
    for line in out.lines() {
        let trimmed = line.trim().trim_start_matches('*').trim();
        let first = trimmed.split_whitespace().next()?;
        if first == "system" {
            continue;
        }
        let Some(ver) = parse_semver_like(first) else {
            continue;
        };
        last = Some(ver.clone());
        if trimmed.contains("default") {
            marked = Some(ver);
        }
    }
    marked.or(last)
}

/// 宽松 semver 解析:剥掉前导 `v` 与首个单词标签(如 "fnm"),容忍缺 minor/patch
/// (`22` → `22.0.0`,`22.19` → `22.19.0`)。无法解析返回 None。
fn parse_semver_like(raw: &str) -> Option<semver::Version> {
    let s = raw.trim().trim_start_matches(['v', 'V']).trim();
    // `fnm --version` 输出 "fnm 1.39.0":取最后一个含数字的 token,容忍工具名前缀
    let token = s
        .split_whitespace()
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .last()?;
    let mut it = token.split('.');
    let major: u64 = it.next()?.parse().ok()?;
    let minor: u64 = it.next().unwrap_or("0").parse().ok()?;
    let patch: u64 = it.next().unwrap_or("0").parse().ok()?;
    Some(semver::Version::new(major, minor, patch))
}

/// nvm 默认安装目录:`$NVM_DIR` 未设时为 `<home>/.nvm`(unix;Windows 走 nvm-windows 布局)。
#[cfg_attr(windows, allow(dead_code))] // unix 探测专用;Windows nvm 走 %APPDATA%\nvm(工单 #18)
pub fn nvm_default_dir(home: &Path) -> PathBuf {
    home.join(".nvm")
}

/// nvm 的 shell rc 痕迹:rc 内容中是否 export 了 NVM_DIR(安装脚本的标志行)。
/// 据此识别「已装但未在当前会话 source」的 nvm(不依赖进程 PATH)。
#[cfg_attr(windows, allow(dead_code))] // unix 探测专用;Windows nvm 走 %APPDATA%\nvm(工单 #18)
pub fn nvm_rc_present(rc_content: &str) -> bool {
    rc_content
        .lines()
        .map(str::trim)
        .any(|l| l.contains("NVM_DIR") && (l.starts_with("export ") || l.contains("export NVM_DIR")))
}

/// nvm 当前 Node 版本:`<nvm_dir>/alias/default` 文件内容(安装脚本维护);
/// 返回解析到的版本串(供后续在该 nvm_dir 下定位 `versions/node/vX.Y.Z`)。
#[cfg_attr(windows, allow(dead_code))] // unix 探测专用;Windows nvm 无 alias 文件(工单 #18)
pub fn nvm_default_version(nvm_dir: &Path) -> Option<String> {
    let alias = fs::read_to_string(nvm_dir.join("alias").join("default")).ok()?;
    let v = alias.trim().trim_start_matches('v').to_string();
    if v.is_empty() { None } else { Some(v) }
}

/// fnm 默认安装/数据目录(unix 为 `~/.local/share/fnm`;Windows 由 imp 覆盖为
/// `%LOCALAPPDATA%\fnm`)。nvm/fnm 探测与 install_fnm 的落盘目录共用此定义。
#[cfg(unix)]
pub fn fnm_default_dir(home: &Path) -> PathBuf {
    home.join(".local").join("share").join("fnm")
}

/// fnm 的 unix shell rc 钩子行(写入登录 rc,幂等判断以这行为准)。
/// `--use-on-cd` 是官方安装脚本默认注入的常用旗标。
#[cfg(any(unix, test))]
pub fn fnm_hook_line() -> String {
    "eval \"$(fnm env --use-on-cd)\"  # setup-coder fnm".to_string()
}

/// rc 内容中是否已有 fnm 钩子(任一 fnm env 初始化行;trim 比较)。
/// 与 fnm_hook_line 注入、以及「已装但未 source」探测共用同一判定。
#[cfg(any(unix, test))]
pub fn fnm_hook_present(rc_content: &str) -> bool {
    rc_content
        .lines()
        .map(str::trim)
        .any(|l| l.starts_with("eval") && l.contains("fnm env"))
}

/// fnm 发行资产后缀:`fnm-<suffix>.zip`。
/// 实测 Schniz/fnm v1.39.0:macos 为 x64+arm64 universal 单资产;linux x64 为 `linux`,
/// linux arm64 为 `arm64`;windows 为 `windows`(均单文件 zip,无顶层目录)。
pub fn fnm_asset_suffix_for(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", _) => Ok("macos"), // universal:单资产覆盖 x64 与 arm64
        ("linux", "x86_64") => Ok("linux"),
        ("linux", "aarch64") => Ok("arm64"),
        ("windows", "x86_64") => Ok("windows"),
        _ => Err(format!("暂不支持的平台组合(fnm):{os}/{arch}")),
    }
}

/// fnm 的 PowerShell profile 钩子行(Windows;写入用户 profile,幂等判断以这行为准)。
#[cfg(any(windows, test))]
pub fn fnm_hook_line_powershell() -> String {
    "fnm env --use-on-cd | Out-String | Invoke-Expression  # setup-coder fnm".to_string()
}

/// PowerShell profile 内容中是否已有 fnm 钩子(任一 fnm env 初始化行;trim 比较)。
#[cfg(any(windows, test))]
pub fn fnm_hook_present_powershell(profile_content: &str) -> bool {
    profile_content
        .lines()
        .map(str::trim)
        .any(|l| l.contains("fnm env") && l.contains("Invoke-Expression"))
}

/// Windows 侧 `fnm list` 解析:与 unix 同一规则(默认标记优先,否则最后非 system 版本)。
/// 单列出来仅为平台对称与单测;逻辑与 parse_fnm_list 相同。
#[cfg(any(windows, test))]
pub fn parse_fnm_list_windows(out: &str) -> Option<semver::Version> {
    parse_fnm_list(out)
}

// ---------------------------------------------------------------------------
// unix 共享 node 薄接缝(macos.rs / linux.rs 复用;平台差异只在 rc 文件清单)
// ---------------------------------------------------------------------------

/// unix 共享探测:裸 Node + nvm(默认目录 + rc 痕迹)+ fnm(数据目录 + 可执行)。
/// 不依赖当前 PATH:nvm/fnm 读安装痕迹(`~/.nvm`、`~/.local/share/fnm`、各登录 rc)。
#[cfg(unix)]
pub(super) fn detect_node_facts_impl(rc_file_names: &[&str]) -> crate::node_plan::NodeFacts {
    let home = std::env::home_dir();
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let rc_contents: Vec<String> = home
        .iter()
        .flat_map(|h| rc_file_names.iter().map(move |n| h.join(n)))
        .filter_map(|f| fs::read_to_string(f).ok())
        .collect();
    crate::node_plan::NodeFacts {
        bare_node: detect_bare_node_unix(&path_var),
        nvm: home.as_deref().and_then(|h| detect_nvm_unix(h, &rc_contents)),
        fnm: home.as_deref().and_then(|h| detect_fnm_unix(h, &rc_contents, &path_var)),
    }
}

/// 裸 Node:在 PATH 上找 node,`--version` 解析版本。找不到 = 无裸 Node。
#[cfg(unix)]
fn detect_bare_node_unix(path_var: &std::ffi::OsStr) -> Option<(semver::Version, PathBuf)> {
    let exe = find_in_path("node", path_var)?;
    let version = parse_node_version(&version_output_of(&exe)?)?;
    Some((version, exe))
}

/// nvm:安装痕迹 = 默认目录存在且 rc 里有 NVM_DIR 行(覆盖「已装未 source」)。
/// 当前 Node 版本取 `<nvm_dir>/alias/default`;nvm 自身路径取其目录。
#[cfg(unix)]
fn detect_nvm_unix(home: &Path, rc_contents: &[String]) -> Option<(semver::Version, PathBuf)> {
    let nvm_dir = nvm_default_dir(home);
    let dir_exists = nvm_dir.is_dir();
    let rc_hint = rc_contents.iter().any(|c| nvm_rc_present(c));
    if !dir_exists && !rc_hint {
        return None;
    }
    let version = nvm_default_version(&nvm_dir)
        .and_then(|v| parse_node_version(&format!("v{v}")))
        .unwrap_or(semver::Version::new(0, 0, 0));
    Some((version, nvm_dir))
}

/// fnm:优先 PATH 上的可执行(实际可用),否则看默认数据目录或 rc 钩子(覆盖
/// 「已装未 source」)。当前默认 Node 版本经 `fnm list` 读取;读不到记 0.0.0。
#[cfg(unix)]
fn detect_fnm_unix(
    home: &Path,
    rc_contents: &[String],
    path_var: &std::ffi::OsStr,
) -> Option<(semver::Version, PathBuf)> {
    if let Some(exe) = find_in_path("fnm", path_var) {
        let version = current_fnm_node_version(&exe).unwrap_or(semver::Version::new(0, 0, 0));
        return Some((version, exe));
    }
    let dir = fnm_default_dir(home);
    let rc_hint = rc_contents.iter().any(|c| fnm_hook_present(c));
    if !dir.is_dir() && !rc_hint {
        return None;
    }
    let exe = dir.join(exe_name("fnm"));
    let version = current_fnm_node_version(&exe).unwrap_or(semver::Version::new(0, 0, 0));
    Some((version, dir))
}

/// 读 fnm 当前默认 Node 版本:`fnm list` 解析 default/最新;失败返回 None。
#[cfg(unix)]
fn current_fnm_node_version(fnm_exe: &Path) -> Option<semver::Version> {
    let out = Command::new(fnm_exe).arg("list").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_fnm_list(&String::from_utf8_lossy(&out.stdout))
}

/// unix 共享:向一组登录 rc 幂等追加 fnm 钩子行。文件不存在则创建。
/// 返回实际发生的注入记录(复用 shell_rc_append 幂等,重跑不重复)。
#[cfg(unix)]
pub(super) fn inject_fnm_hook_via_shell_rc(
    rc_file_names: &[&str],
) -> io::Result<Vec<PathInjection>> {
    let home = std::env::home_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "无法确定用户家目录(HOME 未设置)")
    })?;
    let line = fnm_hook_line();
    let mut injections = Vec::new();
    for name in rc_file_names {
        let file = home.join(name);
        let existing = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if let Some(new) = shell_rc_append(&existing, &line) {
            fs::write(&file, new)?;
            injections.push(PathInjection::ShellRc {
                file,
                line: line.clone(),
            });
        }
    }
    Ok(injections)
}

/// unix 共享:下载 fnm zip 并解出单文件 `fnm` 到 dest_dir,置可执行位。幂等:
/// 目标 fnm 已能跑则直接复用。返回 fnm 可执行文件路径。
#[cfg(unix)]
pub(super) fn install_fnm_unix(
    cache_dir: &Path,
    dest_dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let exe = dest_dir.join(exe_name("fnm"));
    if version_output_of(&exe).is_some() {
        return Ok(exe); // 已装且可用:幂等复用
    }
    let asset = fnm_asset_suffix()?;
    let archive = cache_dir.join(crate::net::fnm_archive_name(asset));
    let hit = crate::net::download_first(&crate::net::fnm_urls(asset), &archive)?;
    println!("已从 Mirror 下载 fnm:{hit}");

    fs::create_dir_all(dest_dir)?;
    let bytes = fs::read(&archive)?;
    let cursor = io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("fnm zip 损坏:{e}"))?;
    let mut entry = zip
        .by_name("fnm")
        .map_err(|e| format!("fnm zip 中无 `fnm` 条目:{e}"))?;
    let mut out = fs::File::create(&exe)?;
    io::copy(&mut entry, &mut out)?;
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755))?;
    if version_output_of(&exe).is_none() {
        return Err("fnm 解压后自检失败:`fnm --version` 未通过".into());
    }
    Ok(exe)
}

/// unix 共享:跑一条 fnm 子命令,按绝对路径调可执行(不依赖 PATH);非零退出带 stderr 报错。
#[cfg(unix)]
pub(super) fn run_fnm(fnm_exe: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
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

/// unix 共享:`fnm install <version>` + `fnm default <version>`。
#[cfg(unix)]
pub(super) fn fnm_install_and_default_unix(
    fnm_exe: &Path,
    version: &str,
) -> Result<(), Box<dyn Error>> {
    run_fnm(fnm_exe, &["install", version])?;
    run_fnm(fnm_exe, &["default", version])
}

// ---------------------------------------------------------------------------
// unix 共享薄接缝(macos.rs / linux.rs 复用;平台差异只在 rc 文件清单)
// ---------------------------------------------------------------------------

/// 向一组 shell rc 文件幂等追加 export PATH 行。文件不存在则创建。
#[cfg(unix)]
pub(super) fn ensure_path_via_shell_rc(
    bin_dir: &Path,
    rc_file_names: &[&str],
) -> io::Result<Vec<PathInjection>> {
    let home = std::env::home_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "无法确定用户家目录(HOME 未设置)")
    })?;
    let export_line = shell_rc_export_line(bin_dir);
    let mut injections = Vec::new();
    for name in rc_file_names {
        let file = home.join(name);
        let existing = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if let Some(new) = shell_rc_append(&existing, &export_line) {
            fs::write(&file, new)?;
            injections.push(PathInjection::ShellRc {
                file,
                line: export_line.clone(),
            });
        }
    }
    Ok(injections)
}

/// unix 共享:PATH 持久化体检——任一用户 rc 文件含 export 行即视为已持久化。
/// 与 ensure_path_via_shell_rc 用同一份 rc 文件清单(由各平台薄接缝传入)。
#[cfg(unix)]
pub(super) fn path_persisted_via_shell_rc(
    bin_dir: &Path,
    rc_file_names: &[&str],
) -> io::Result<bool> {
    let home = std::env::home_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "无法确定用户家目录(HOME 未设置)")
    })?;
    let export_line = shell_rc_export_line(bin_dir);
    for name in rc_file_names {
        match fs::read_to_string(home.join(name)) {
            Ok(text) => {
                if shell_rc_contains(&text, &export_line) {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

#[cfg(unix)]
pub(super) fn write_shim_impl(
    bin_dir: &Path,
    node_bin_dir: &Path,
    npm_bin_dir: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(bin_dir)?;
    let path = bin_dir.join(shim_file_name(bin));
    fs::write(&path, shim_content(node_bin_dir, npm_bin_dir, bin))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// 按安装清单记录精确回滚一条 shell rc PATH 注入。文件不存在或无匹配行 = Ok(false)。
#[cfg(unix)]
pub(super) fn rollback_shell_rc(file: &Path, line: &str) -> io::Result<bool> {
    let existing = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let Some(new) = shell_rc_remove(&existing, line) else {
        return Ok(false);
    };
    fs::write(file, new)?;
    Ok(true)
}

#[cfg(unix)]
pub(super) fn install_self_impl(bin_dir: &Path) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(bin_dir)?;
    let current = std::env::current_exe()?;
    let dest = bin_dir.join(exe_name("setup-coder"));
    // 已从前缀内运行(重跑)= 无需自复制
    if current == dest {
        return Ok(dest);
    }
    fs::copy(&current, &dest)?;
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
    Ok(dest)
}

/// unix:tar.gz 解压,剥掉顶层目录一层。
#[cfg(unix)]
pub(super) fn extract_node_archive_impl(archive: &Path, dest_dir: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    fs::create_dir_all(dest_dir)?;
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let stripped: PathBuf = path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        entry.unpack(dest_dir.join(stripped))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_dist_suffix_covers_ci_targets() {
        assert_eq!(
            node_dist_suffix_for("macos", "aarch64").unwrap(),
            "darwin-arm64"
        );
        assert_eq!(
            node_dist_suffix_for("macos", "x86_64").unwrap(),
            "darwin-x64"
        );
        assert_eq!(
            node_dist_suffix_for("linux", "x86_64").unwrap(),
            "linux-x64"
        );
        assert_eq!(
            node_dist_suffix_for("windows", "x86_64").unwrap(),
            "win-x64"
        );
        assert!(node_dist_suffix_for("linux", "aarch64").is_err());
        assert!(node_dist_suffix_for("freebsd", "x86_64").is_err());
    }

    #[test]
    fn node_archive_ext_by_os() {
        assert_eq!(node_archive_ext_for("windows"), "zip");
        assert_eq!(node_archive_ext_for("macos"), "tar.gz");
        assert_eq!(node_archive_ext_for("linux"), "tar.gz");
    }

    #[test]
    fn shell_rc_remove_deletes_only_the_recorded_line() {
        let line = shell_rc_export_line(Path::new("/home/u/.setup-coder/bin"));
        // 精确删除记录行,其他行原样保留
        let rc = format!("export FOO=1\n{line}\nexport BAR=2\n");
        let new = shell_rc_remove(&rc, &line).unwrap();
        assert_eq!(new, "export FOO=1\nexport BAR=2\n");
        // 已是回滚后状态 → None(幂等)
        assert!(shell_rc_remove(&new, &line).is_none());
        // 无该行 → None(不做模糊匹配)
        assert!(shell_rc_remove("export FOO=1\n", &line).is_none());
        // 行首尾有空白也算同一行(与 append 的幂等判断同规则)
        let padded = format!("  {line}  \n");
        assert_eq!(shell_rc_remove(&padded, &line).unwrap(), "");
        // 无结尾换行的文件 → 移除后不引入多余空行
        let no_nl = format!("export FOO=1\n{line}");
        assert_eq!(shell_rc_remove(&no_nl, &line).unwrap(), "export FOO=1\n");
    }

    #[test]
    fn windows_path_contains_shares_merge_normalization() {
        let dir = r"C:\Users\u\.setup-coder\bin";
        // 未含 → false(空串、其他目录、前缀相似目录)
        assert!(!windows_path_contains("", dir));
        assert!(!windows_path_contains(r"C:\Windows", dir));
        assert!(!windows_path_contains(r"C:\Users\u\.setup-coder\bin2", dir));
        // 已含 → true(含大小写/尾斜杠/空白差异,与 merge 同一归一化)
        assert!(windows_path_contains(dir, dir));
        assert!(windows_path_contains(&format!(r"C:\Windows;{dir}"), dir));
        assert!(windows_path_contains(r"c:\users\u\.setup-coder\bin", dir));
        assert!(windows_path_contains(r"C:\Users\u\.setup-coder\bin\", dir));
        assert!(windows_path_contains(
            &format!(r"C:\Windows; {dir} ;C:\Tools"),
            dir
        ));
    }

    #[test]
    fn shell_rc_contains_matches_append_idempotency_rule() {
        let line = shell_rc_export_line(Path::new("/home/u/.setup-coder/bin"));
        assert!(!shell_rc_contains("", &line));
        assert!(!shell_rc_contains("export FOO=1\n", &line));
        assert!(shell_rc_contains(&format!("export FOO=1\n{line}\n"), &line));
        // 行首尾有空白也算已存在(与 append 幂等判断同规则)
        assert!(shell_rc_contains(&format!("  {line}  \n"), &line));
        // 相似但不同的路径不命中
        let other = shell_rc_export_line(Path::new("/home/u/.setup-coder/bin2"));
        assert!(!shell_rc_contains(&format!("{other}\n"), &line));
    }

    #[test]
    fn windows_path_remove_deletes_only_the_recorded_dir() {
        let dir = r"C:\Users\u\.setup-coder\bin";
        let path = format!(r"C:\Windows;{dir};C:\Tools");
        let new = windows_path_remove(&path, dir).unwrap();
        assert_eq!(new, r"C:\Windows;C:\Tools");
        // 已是回滚后状态 → None(幂等)
        assert!(windows_path_remove(&new, dir).is_none());
        // 大小写/尾斜杠差异也命中(与 merge 同一归一化)
        assert!(windows_path_remove(r"c:\users\u\.setup-coder\bin", dir).is_some());
        assert!(windows_path_remove(&path, r"C:\Users\u\.setup-coder\bin\").is_some());
        // 相似但不等的目录不动(不做模糊匹配)
        assert!(windows_path_remove(r"C:\Users\u\.setup-coder\bin2", dir).is_none());
        // 唯一条目被移除 → 空串
        assert_eq!(windows_path_remove(dir, dir).unwrap(), "");
    }

    #[test]
    fn path_contains_dir_matches_exact_entry() {
        let dir = Path::new("/home/u/.setup-coder/bin");
        let path_var = std::ffi::OsString::from(format!("/usr/bin:{}", dir.display()));
        assert!(path_contains_dir(&path_var, dir));
        // 尾部斜杠归一化后仍命中
        let with_slash = std::ffi::OsString::from(format!("{}/", dir.display()));
        assert!(path_contains_dir(&with_slash, dir));
        // 前缀相似但不是同一目录 → 不命中
        let other = std::ffi::OsString::from("/home/u/.setup-coder/bin2");
        assert!(!path_contains_dir(&other, dir));
        assert!(!path_contains_dir(&std::ffi::OsString::from(""), dir));
    }

    #[test]
    fn version_output_of_captures_version_or_none() {
        // 不存在的可执行文件 → None(不 panic)
        assert!(version_output_of(Path::new("/nonexistent/setup-coder-test")).is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir =
                std::env::temp_dir().join(format!("setup-coder-test-ver-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let fake = dir.join("tool");
            fs::write(&fake, "#!/bin/sh\necho '  tool 1.2.3  '\n").unwrap();
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(version_output_of(&fake).unwrap(), "tool 1.2.3");
            // 退出非零 → None
            let bad = dir.join("bad");
            fs::write(&bad, "#!/bin/sh\nexit 1\n").unwrap();
            fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(version_output_of(&bad).is_none());
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn remove_all_except_keeps_only_the_locked_file() {
        let root = std::env::temp_dir().join(format!("setup-coder-test-rm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // 造前缀:bin/setup-coder(假装是正在运行的自身)+ node/ + state.json
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("node")).unwrap();
        let locked = root.join("bin").join(exe_name("setup-coder"));
        fs::write(&locked, "exe").unwrap();
        fs::write(root.join("bin").join("codex"), "shim").unwrap();
        fs::write(root.join("node").join("node"), "node").unwrap();
        fs::write(root.join("state.json"), "{}").unwrap();

        remove_all_except(&root, &locked).unwrap();
        assert!(locked.exists(), "keep 文件必须留下");
        assert!(!root.join("state.json").exists());
        assert!(!root.join("node").exists());
        assert!(!root.join("bin").join("codex").exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn remove_prefix_deletes_everything_on_unix() {
        let root =
            std::env::temp_dir().join(format!("setup-coder-test-rmall-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("state.json"), "{}").unwrap();
        // unix 可删正在运行的文件,无残留
        assert_eq!(remove_prefix(&root).unwrap(), None);
        assert!(!root.exists());
    }

    #[test]
    fn shell_rc_append_is_idempotent() {
        let line = shell_rc_export_line(Path::new("/home/u/.setup-coder/bin"));
        // 空文件 → 追加
        let once = shell_rc_append("", &line).unwrap();
        assert!(once.ends_with(&format!("{line}\n")));
        // 已有该行 → None(重跑不重复追加)
        assert!(shell_rc_append(&once, &line).is_none());
        // 无结尾换行的文件 → 先补换行
        let fixed = shell_rc_append("export FOO=1", &line).unwrap();
        assert!(fixed.starts_with("export FOO=1\n"));
        assert!(fixed.ends_with(&format!("{line}\n")));
        // 行首尾有空白也算已存在(防御手抖编辑)
        assert!(shell_rc_append(&format!("  {line}  \n"), &line).is_none());
    }

    #[test]
    fn windows_path_merge_is_idempotent_and_case_insensitive() {
        let dir = r"C:\Users\u\.setup-coder\bin";
        // 空 PATH → 直接设
        assert_eq!(windows_path_merge("", dir).unwrap(), dir);
        // 追加到已有 PATH
        let merged = windows_path_merge(r"C:\Windows", dir).unwrap();
        assert_eq!(merged, format!(r"C:\Windows;{dir}"));
        // 已有(含大小写/尾斜杠差异)→ None
        assert!(windows_path_merge(&merged, dir).is_none());
        assert!(windows_path_merge(r"c:\users\u\.setup-coder\bin", dir).is_none());
        assert!(windows_path_merge(r"C:\Users\u\.setup-coder\bin\", dir).is_none());
        // 已有 PATH 以分号结尾 → 不产生双分号
        let merged2 = windows_path_merge(r"C:\Windows;", dir).unwrap();
        assert!(!merged2.contains(";;"));
    }

    #[test]
    fn unix_shim_patches_path_and_execs_npm_bin() {
        let s = unix_shim_content(
            Path::new("/x/.setup-coder/node/bin"),
            Path::new("/x/.setup-coder/npm/bin"),
            "codex",
        );
        assert!(s.starts_with("#!/bin/sh"));
        assert!(s.contains("export PATH=\"/x/.setup-coder/node/bin:$PATH\""));
        assert!(s.contains("exec \"/x/.setup-coder/npm/bin/codex\" \"$@\""));
    }

    #[test]
    fn npm_cli_subpath_matches_node_dist_layout() {
        let p = npm_cli_subpath();
        assert!(p.ends_with(Path::new("npm").join("bin").join("npm-cli.js")));
        if cfg!(windows) {
            assert!(p.starts_with("node_modules"));
        } else {
            assert!(p.starts_with("lib"));
        }
    }

    #[test]
    fn windows_shim_patches_path_and_calls_cmd() {
        let s = windows_shim_content(
            Path::new(r"C:\Users\u\.setup-coder\node"),
            Path::new(r"C:\Users\u\.setup-coder\npm"),
            "codex",
        );
        assert!(s.starts_with("@echo off"));
        assert!(s.contains("set \"PATH=C:\\Users\\u\\.setup-coder\\node;%PATH%\""));
        assert!(s.contains(r"C:\Users\u\.setup-coder\npm\codex.cmd"));
    }

    #[test]
    fn git_shim_delegates_to_mingit_exe() {
        let s = git_shim_content(Path::new(r"C:\Users\u\.setup-coder\git\cmd\git.exe"));
        assert!(s.starts_with("@echo off"));
        assert!(s.contains("\"C:\\Users\\u\\.setup-coder\\git\\cmd\\git.exe\" %*"));
        assert!(s.contains("exit /b %errorlevel%"));
    }

    #[test]
    fn wait_until_returns_immediately_when_already_true() {
        assert!(wait_until(
            || true,
            Duration::from_millis(1),
            Duration::from_millis(1)
        ));
    }

    #[test]
    fn wait_until_passes_when_check_flips() {
        let mut calls = 0;
        let ok = wait_until(
            || {
                calls += 1;
                calls >= 3
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
        );
        assert!(ok);
        assert_eq!(calls, 3, "第三次检查通过即停");
    }

    #[test]
    fn wait_until_times_out_when_never_true() {
        let ok = wait_until(
            || false,
            Duration::from_millis(10),
            Duration::from_millis(1),
        );
        assert!(!ok);
    }

    #[test]
    fn git_works_at_checks_a_specific_exe() {
        // 不存在的路径 → false
        assert!(!git_works_at(Path::new("/nonexistent/git")));
        // 造一个假的 git 可执行脚本 → true
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir =
                std::env::temp_dir().join(format!("setup-coder-test-git-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let fake = dir.join("git");
            fs::write(&fake, "#!/bin/sh\necho 'git version 2.55.0'\n").unwrap();
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(git_works_at(&fake));
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_only_matches_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir =
            std::env::temp_dir().join(format!("setup-coder-test-path-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path_var = std::ffi::OsString::from(&dir);

        // 不存在 → None
        assert!(find_in_path("sudo", &path_var).is_none());
        // 存在但不可执行 → None
        fs::write(dir.join("sudo"), "").unwrap();
        assert!(find_in_path("sudo", &path_var).is_none());
        // 加可执行位 → 命中
        fs::set_permissions(dir.join("sudo"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_in_path("sudo", &path_var).unwrap(), dir.join("sudo"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn linux_error_messages_give_manual_command_in_chinese() {
        let no_sudo = no_sudo_error();
        assert!(
            no_sudo.contains("apt-get install -y git"),
            "应给手工命令:{no_sudo}"
        );
        assert!(no_sudo.contains("没有 sudo"));
        let apt_failed = apt_install_failed_error();
        assert!(apt_failed.contains("sudo apt-get update"));
        assert!(apt_failed.contains("sudo apt-get install -y git"));
    }

    #[test]
    fn macos_wait_config_and_messages_are_sane() {
        assert!(GIT_POLL_INTERVAL < GIT_WAIT_TIMEOUT, "轮询间隔必须小于超时");
        assert!(clt_prompt_message().contains("xcode-select --install"));
        assert!(clt_wait_timeout_error().contains("xcode-select --install"));
    }

    // -------------------------------------------------------------------
    // node 来源探测纯逻辑(工单 #18)
    // -------------------------------------------------------------------

    #[test]
    fn parse_node_version_accepts_v_prefixed_and_bare() {
        assert_eq!(
            parse_node_version("v24.19.0").unwrap(),
            semver::Version::new(24, 19, 0)
        );
        assert_eq!(
            parse_node_version("24.19.0").unwrap(),
            semver::Version::new(24, 19, 0)
        );
        assert_eq!(
            parse_node_version("  v22.19.0\n").unwrap(),
            semver::Version::new(22, 19, 0)
        );
        assert!(parse_node_version("").is_none());
        assert!(parse_node_version("not a version").is_none());
    }

    #[test]
    fn parse_fnm_version_strips_tool_name_prefix() {
        assert_eq!(
            parse_fnm_version("fnm 1.39.0").unwrap(),
            semver::Version::new(1, 39, 0)
        );
        assert_eq!(
            parse_fnm_version("1.39.0").unwrap(),
            semver::Version::new(1, 39, 0)
        );
        assert!(parse_fnm_version("fnm").is_none());
    }

    #[test]
    fn parse_fnm_list_prefers_default_marker_then_latest() {
        // 带 default 标记的行优先
        let out = "* v20.19.0\n* v22.19.0 default\n* system\n";
        assert_eq!(
            parse_fnm_list(out).unwrap(),
            semver::Version::new(22, 19, 0)
        );
        // 无 default 标记时取最后一行非 system 版本
        let out = "* v20.19.0\n* v22.19.0\n* system\n";
        assert_eq!(
            parse_fnm_list(out).unwrap(),
            semver::Version::new(22, 19, 0)
        );
        // 只有 system(指向裸 Node)→ fnm 未自装任何版本
        assert!(parse_fnm_list("* system\n").is_none());
        assert!(parse_fnm_list("").is_none());
    }

    #[test]
    fn parse_semver_like_tolerates_missing_minor_patch() {
        assert_eq!(parse_semver_like("22").unwrap(), semver::Version::new(22, 0, 0));
        assert_eq!(
            parse_semver_like("22.19").unwrap(),
            semver::Version::new(22, 19, 0)
        );
        assert!(parse_semver_like("v").is_none());
    }

    #[test]
    fn nvm_default_dir_is_under_home() {
        assert_eq!(
            nvm_default_dir(Path::new("/home/u")),
            Path::new("/home/u/.nvm")
        );
    }

    #[test]
    fn nvm_rc_present_detects_install_trace_not_path() {
        // 安装脚本的标志行(覆盖「已装未在当前会话 source」)
        let rc = "export NVM_DIR=\"$HOME/.nvm\"\n[ -s \"$NVM_DIR/nvm.sh\" ] && . \"$NVM_DIR/nvm.sh\"\n";
        assert!(nvm_rc_present(rc));
        // 无 NVM_DIR → 无痕迹
        assert!(!nvm_rc_present("export PATH=\"$HOME/bin:$PATH\"\n"));
        assert!(!nvm_rc_present(""));
    }

    #[test]
    fn nvm_default_version_reads_alias_file() {
        let dir = std::env::temp_dir().join(format!("setup-coder-test-nvm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("alias")).unwrap();
        fs::write(dir.join("alias").join("default"), "v22.19.0\n").unwrap();
        assert_eq!(nvm_default_version(&dir).as_deref(), Some("22.19.0"));
        // 无 alias/default → None
        let empty = dir.join("empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(nvm_default_version(&empty).is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fnm_hook_line_is_unix_eval_form() {
        let line = fnm_hook_line();
        assert!(line.starts_with("eval"), "应为 eval 钩子:{line}");
        assert!(line.contains("fnm env"), "应初始化 fnm env:{line}");
    }

    #[test]
    fn fnm_hook_present_matches_any_fnm_env_line() {
        assert!(fnm_hook_present(&fnm_hook_line()));
        assert!(fnm_hook_present("eval \"$(fnm env --use-on-cd)\"\n"));
        assert!(fnm_hook_present("  eval \"$(fnm env)\"  \n"));
        assert!(!fnm_hook_present("export PATH=\"$HOME/.fnm:$PATH\"\n"));
        assert!(!fnm_hook_present(""));
    }

    #[test]
    fn inject_fnm_hook_is_idempotent_via_rc_append() {
        // 幂等:重跑同一行不产生重复(复用 shell_rc_append/contains 接缝)
        let line = fnm_hook_line();
        let once = shell_rc_append("# existing\n", &line).unwrap();
        assert!(shell_rc_contains(&once, &line));
        let count = once.matches("fnm env").count();
        assert_eq!(count, 1, "首次注入恰好一行");
        // 重跑 → None(不追加),内容不变
        assert!(shell_rc_append(&once, &line).is_none(), "重跑不得重复注入");
        assert_eq!(once.matches("fnm env").count(), 1);
    }

    #[test]
    fn fnm_asset_suffix_covers_ci_targets() {
        assert_eq!(fnm_asset_suffix_for("macos", "aarch64").unwrap(), "macos");
        assert_eq!(fnm_asset_suffix_for("macos", "x86_64").unwrap(), "macos");
        assert_eq!(fnm_asset_suffix_for("linux", "x86_64").unwrap(), "linux");
        assert_eq!(fnm_asset_suffix_for("linux", "aarch64").unwrap(), "arm64");
        assert_eq!(fnm_asset_suffix_for("windows", "x86_64").unwrap(), "windows");
        assert!(fnm_asset_suffix_for("linux", "riscv64").is_err());
        assert!(fnm_asset_suffix_for("freebsd", "x86_64").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn fnm_default_dir_is_under_home() {
        assert_eq!(
            fnm_default_dir(Path::new("/home/u")),
            Path::new("/home/u/.local/share/fnm")
        );
    }
}
