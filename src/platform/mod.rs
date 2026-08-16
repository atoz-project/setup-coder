//! 平台分叉收敛处:PATH 注入、shim 生成、Node 解压。
//!
//! commands 层写"做什么",这里写"在这个系统上怎么做"。
//! 纯逻辑(字符串拼接/幂等判断)直接放在本文件,全平台可编译、可单测;
//! 涉及文件系统/注册表的薄接缝在 `windows.rs` / `macos.rs` / `linux.rs`。

#[cfg(unix)]
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

/// shell rc 幂等追加:内容中已有该行则返回 None,否则返回追加后的完整新内容。
#[cfg(any(unix, test))]
pub fn shell_rc_append(existing: &str, export_line: &str) -> Option<String> {
    if existing
        .lines()
        .any(|line| line.trim() == export_line.trim())
    {
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

/// Windows 用户 PATH 幂等合并:已含该目录(忽略大小写、忽略首尾 `\`/`/` 与空白)
/// 则返回 None,否则返回合并后的完整 PATH。
#[cfg(any(windows, test))]
pub fn windows_path_merge(existing: &str, dir: &str) -> Option<String> {
    fn normalize(s: &str) -> String {
        s.trim().trim_end_matches(['\\', '/']).to_ascii_lowercase()
    }
    let target = normalize(dir);
    if existing
        .split(';')
        .map(normalize)
        .any(|entry| !entry.is_empty() && entry == target)
    {
        return None;
    }
    let mut new = existing.trim_end_matches(';').to_string();
    if !new.is_empty() {
        new.push(';');
    }
    new.push_str(dir.trim());
    Some(new)
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

/// 当前平台的 shim 内容(供薄接缝落盘)
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
pub fn wait_until(
    mut check: impl FnMut() -> bool,
    timeout: Duration,
    interval: Duration,
) -> bool {
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

/// 在 PATH 目录清单中查找可执行文件(Ubuntu 检测 sudo 用)
#[cfg(any(target_os = "linux", all(unix, test)))]
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

#[cfg(any(target_os = "linux", all(unix, test)))]
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

/// 生成 shim 到 `bin_dir`(重跑覆盖)。
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

        let dir = std::env::temp_dir().join(format!("setup-coder-test-path-{}", std::process::id()));
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
        assert!(no_sudo.contains("apt-get install -y git"), "应给手工命令:{no_sudo}");
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
}
