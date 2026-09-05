//! Linux(Ubuntu)薄接缝:PATH 注入追加 shell rc(bash 默认登录 shell,zsh 次之)。

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::prefix::{PathInjection, Prefix};

/// PATH 注入/体检共用的 shell rc 文件清单(Ubuntu 默认 bash,zsh 次之)
const RC_FILES: &[&str] = &[".bashrc", ".zshrc"];

pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    // Ubuntu 默认 bash(读 .bashrc);zsh 用户读 .zshrc,一并处理
    super::ensure_path_via_shell_rc(bin_dir, RC_FILES)
}

/// 探测 Node 来源事实:与 ensure_path 同一份登录 rc 清单(覆盖「已装未 source」)
pub fn detect_node_facts() -> crate::node_plan::NodeFacts {
    super::detect_node_facts_impl(RC_FILES)
}

/// 经已有 nvm 装 Node 并解析 exe 路径(共享 unix 实现:source nvm.sh 后 nvm install/which)
pub fn nvm_install_and_resolve(
    nvm_dir: &Path,
    version: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    super::nvm_install_and_resolve_unix(nvm_dir, version)
}

/// 下载安装 fnm(共享 unix 实现:华为云 → gh-proxy → GitHub 容错链,解出单文件 fnm)
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn install_fnm(cache_dir: &Path, dest_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    super::install_fnm_unix(cache_dir, dest_dir)
}

/// 用 fnm 装指定 Node 版本并设为默认(共享 unix 实现)
pub fn fnm_install_and_default(fnm_exe: &Path, version: &str) -> Result<(), Box<dyn Error>> {
    super::fnm_install_and_default_unix(fnm_exe, version)
}

/// 注入 fnm 钩子到登录 rc(幂等,与 ensure_path 同一份 rc 清单)
#[allow(dead_code)] // 未接线到 install(工单 #19+)
pub fn inject_fnm_hook() -> io::Result<Vec<PathInjection>> {
    super::inject_fnm_hook_via_shell_rc(RC_FILES)
}

/// doctor:PATH 持久化体检(与 ensure_path 同一份 rc 文件清单)
pub fn path_persisted(bin_dir: &Path) -> io::Result<bool> {
    super::path_persisted_via_shell_rc(bin_dir, RC_FILES)
}

/// git 版本:Ubuntu 只用系统 git(apt 所装,在前缀外,doctor 只报告)
pub fn git_version(_prefix: &Prefix) -> Option<String> {
    super::version_output_of(Path::new("git"))
}

pub fn git_missing_hint() -> &'static str {
    "执行 sudo apt-get update && sudo apt-get install -y git(或重跑 setup-coder install 自动处理)"
}

pub fn write_shim(
    bin_dir: &Path,
    node_bin_dir: &Path,
    npm_bin_dir: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    super::write_shim_impl(bin_dir, node_bin_dir, npm_bin_dir, bin)
}

pub fn install_self(bin_dir: &Path) -> io::Result<PathBuf> {
    super::install_self_impl(bin_dir)
}

pub fn extract_node_archive(archive: &Path, dest_dir: &Path) -> io::Result<()> {
    super::extract_node_archive_impl(archive, dest_dir)
}

/// 确保 git 可用:系统已有 → 跳过;否则 `sudo apt-get install -y git`(交互输密码)。
/// 无 sudo → 中文报错并给手工命令(工单 #3)。
pub fn ensure_git(_prefix: &Prefix) -> Result<super::GitOutcome, Box<dyn Error>> {
    if super::git_on_path_works() {
        return Ok(super::GitOutcome::Skipped);
    }

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    if super::find_in_path("sudo", &path_var).is_none() {
        return Err(super::no_sudo_error().into());
    }

    println!("未检测到 git,将通过 apt 安装(可能需要输入密码)…");
    let status = Command::new("sudo")
        .args(["apt-get", "install", "-y", "git"])
        .status()?;
    if !status.success() {
        return Err(super::apt_install_failed_error().into());
    }
    if !super::git_on_path_works() {
        return Err("apt 报告成功,但 `git --version` 仍未通过,请手工检查 git 安装".into());
    }
    Ok(super::GitOutcome::Installed)
}
