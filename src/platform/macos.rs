//! macOS 薄接缝:PATH 注入追加 shell rc(zsh 默认登录 shell,bash 读 .bash_profile)。

use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::prefix::{PathInjection, Prefix};

/// PATH 注入/体检共用的 shell rc 文件清单(macOS 默认 zsh,bash 读 .bash_profile)
const RC_FILES: &[&str] = &[".zshrc", ".bash_profile"];

pub fn ensure_path(bin_dir: &Path) -> io::Result<Vec<PathInjection>> {
    // macOS 默认 zsh(登录 shell 读 .zshrc);bash 登录 shell 读 .bash_profile
    super::ensure_path_via_shell_rc(bin_dir, RC_FILES)
}

/// 探测 Node 来源事实:与 ensure_path 同一份登录 rc 清单(覆盖「已装未 source」)
pub fn detect_node_facts() -> crate::node_plan::NodeFacts {
    super::detect_node_facts_impl(RC_FILES)
}

/// 经已有 nvm 装 Node 并解析 exe 路径(共享 unix 实现:source nvm.sh 后 nvm install/which)
pub fn nvm_install_and_resolve(nvm_dir: &Path, version: &str) -> Result<PathBuf, Box<dyn Error>> {
    super::nvm_install_and_resolve_unix(nvm_dir, version)
}

/// 下载安装 fnm(共享 unix 实现:华为云 → gh-proxy → GitHub 容错链,解出单文件 fnm)
pub fn install_fnm(cache_dir: &Path, dest_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    super::install_fnm_unix(cache_dir, dest_dir)
}

/// 用 fnm 装指定 Node 版本并设为默认(共享 unix 实现)
pub fn fnm_install_and_default(fnm_exe: &Path, version: &str) -> Result<(), Box<dyn Error>> {
    super::fnm_install_and_default_unix(fnm_exe, version)
}

/// 注入 fnm 钩子到登录 rc(幂等,与 ensure_path 同一份 rc 清单;返回 FnmHook 记录)
pub fn inject_fnm_hook(fnm_exe: &Path) -> io::Result<Vec<PathInjection>> {
    super::inject_fnm_hook_via_shell_rc(RC_FILES, fnm_exe)
}

/// doctor:PATH 持久化体检(与 ensure_path 同一份 rc 文件清单)
pub fn path_persisted(bin_dir: &Path) -> io::Result<bool> {
    super::path_persisted_via_shell_rc(bin_dir, RC_FILES)
}

/// git 版本:macOS 只用系统 git(在前缀外,doctor 只报告;与 install 同一检测路径)
pub fn git_version(_prefix: &Prefix) -> Option<String> {
    super::version_output_of(Path::new("git"))
}

pub fn git_missing_hint() -> &'static str {
    "执行 xcode-select --install 安装 Xcode 命令行工具(或重跑 setup-coder install 自动处理)"
}

pub fn write_shim(
    bin_dir: &Path,
    node_exe: &Path,
    tool_launcher: &Path,
    bin: &str,
) -> io::Result<PathBuf> {
    super::write_shim_impl(bin_dir, node_exe, tool_launcher, bin)
}

pub fn install_self(bin_dir: &Path) -> io::Result<PathBuf> {
    super::install_self_impl(bin_dir)
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
            return Err(format!(
                "无法触发 xcode-select --install:{e}。请手工执行该命令后重跑 install"
            )
            .into())
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
