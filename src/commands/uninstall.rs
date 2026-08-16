//! `uninstall` 子命令:删除 Private Prefix 并回滚对外改动。
//!
//! TODO(后续工单):按 state.json 记录回滚 PATH 注入,再删除 ~/.setup-coder/。

pub fn run() {
    eprintln!("uninstall 子命令尚未实现(TODO)");
    std::process::exit(1);
}
