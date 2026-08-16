//! macOS 薄接缝:PATH 注入追加 shell rc(zsh 默认登录 shell,bash 读 .bash_profile)。

use std::io;
use std::path::{Path, PathBuf};

use crate::prefix::PathInjection;

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
