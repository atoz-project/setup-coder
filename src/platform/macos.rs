//! macOS 薄接缝:PATH 注入追加 shell rc(zsh 默认登录 shell,bash 读 .bash_profile)。

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::prefix::{PathInjection, Prefix};

pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    // macOS 默认 zsh(登录 shell 读 .zshrc);bash 登录 shell 读 .bash_profile
    super::ensure_path_via_shell_rc(bin_dir, &[".zshrc", ".bash_profile"])
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

/// 确保 git 可用:系统已有 → 跳过;否则触发 `xcode-select --install` 弹窗并轮询等待。
/// 手工验证说明(无 CLT 的干净 macOS):删掉/重命名 /Library/Developer/CommandLineTools
/// 需管理员,故本分支以单测覆盖等待逻辑,真机验证走「已有 git 跳过」分支。
pub fn ensure_git(_prefix: &Prefix) -> Result<super::GitOutcome, Box<dyn Error>> {
    if super::git_on_path_works() {
        return Ok(super::GitOutcome::Skipped);
    }

    // 触发系统安装弹窗(已在安装中/已装过时该命令退出非零,属正常,均继续轮询)
    match Command::new("xcode-select").arg("--install").status() {
        Ok(_) => println!("{}", super::clt_prompt_message()),
        Err(e) => {
            return Err(format!("无法触发 xcode-select --install:{e}。请手工执行该命令后重跑 install").into())
        }
    }
    println!(
        "等待安装完成(每 {} 秒检查一次,最长 {} 分钟)…",
        super::GIT_POLL_INTERVAL.as_secs(),
        super::GIT_WAIT_TIMEOUT.as_secs() / 60
    );
    if !super::wait_until(
        super::git_on_path_works,
        super::GIT_WAIT_TIMEOUT,
        super::GIT_POLL_INTERVAL,
    ) {
        return Err(super::clt_wait_timeout_error().into());
    }
    Ok(super::GitOutcome::Installed)
}
