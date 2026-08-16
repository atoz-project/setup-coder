//! Linux(Ubuntu)薄接缝:PATH 注入追加 shell rc(bash 默认登录 shell,zsh 次之)。

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::prefix::{PathInjection, Prefix};

pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    // Ubuntu 默认 bash(读 .bashrc);zsh 用户读 .zshrc,一并处理
    super::ensure_path_via_shell_rc(bin_dir, &[".bashrc", ".zshrc"])
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
