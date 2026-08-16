//! `install` 子命令:安装 Tool 及其 Prerequisite。
//!
//! TODO(工单 #2):下载 Node LTS → 解压进 Private Prefix → npm 安装各 Tool →
//! 生成 shim → 注入 PATH → 写 state.json。

pub fn run() {
    eprintln!("install 子命令尚未实现(TODO:见工单 #2)");
    std::process::exit(1);
}
