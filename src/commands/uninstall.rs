//! `uninstall` 子命令:按 state.json 精确回滚 PATH 注入,再删除 Private Prefix。
//!
//! 不做模糊匹配:只回滚安装清单里逐字记录的改动;系统 git(apt/CLT 所装)
//! 在前缀外、清单无记录,绝不动。Windows 自身 exe 残留策略见
//! platform::remove_prefix 的决策注释(工单 #4,真机验证归工单 #7)。

use std::error::Error;
use std::io::{self, Write};

use crate::platform;
use crate::prefix::{PathInjection, Prefix, State};

pub fn run(yes: bool) {
    if let Err(e) = uninstall(yes) {
        eprintln!("卸载失败:{e}");
        std::process::exit(1);
    }
}

fn uninstall(yes: bool) -> Result<(), Box<dyn Error>> {
    let prefix = Prefix::home()?;
    // 幂等友好:已无前缀 = 没有可卸载的东西
    if !prefix.root().exists() {
        println!(
            "没有可卸载的东西(Private Prefix 不存在:{})。",
            prefix.root().display()
        );
        return Ok(());
    }
    // 清单损坏 = 无法精确回滚:中止并给出手工下一步,不贸然删前缀
    let state = State::load(&prefix).map_err(|e| {
        format!(
            "{e}\n下一步:可手工删除 {},并自行清理 shell 配置中 # setup-coder 标记的 PATH 行",
            prefix.root().display()
        )
    })?;

    if !yes && !confirm(&prefix, &state)? {
        println!("已取消卸载。");
        return Ok(());
    }

    // 1. 按 state.json 逐条精确回滚 PATH 注入(先回滚,清单随后随前缀一起删)
    let mut rollback_failed = 0;
    for injection in &state.path_injections {
        match platform::rollback_injection(injection) {
            Ok(true) => println!("已回滚 PATH 改动:{}", describe_injection(injection)),
            Ok(false) => println!("PATH 改动已不存在,跳过:{}", describe_injection(injection)),
            // 单条回滚失败不中断:继续其余回滚与前缀删除,结尾统一提示
            Err(e) => {
                rollback_failed += 1;
                eprintln!(
                    "警告:回滚 PATH 改动失败({}):{e},请稍后手工核对",
                    describe_injection(injection)
                );
            }
        }
    }

    // 2. 删除前缀(Windows 自身 exe 残留策略见 platform::remove_prefix 注释)
    match platform::remove_prefix(prefix.root())? {
        None => println!("已删除 Private Prefix:{}", prefix.root().display()),
        Some(leftover) => {
            println!("已删除 Private Prefix 其余内容:{}", prefix.root().display());
            println!(
                "注意:正在运行的卸载程序无法删除自己,请手工删除残留文件:{}",
                leftover.display()
            );
        }
    }

    println!();
    if rollback_failed > 0 {
        println!("卸载完成,但有 {rollback_failed} 条 PATH 改动回滚失败,请按上面的警告手工核对。");
    } else {
        println!("卸载完成。PATH 改动将在新开终端后生效。");
    }
    Ok(())
}

/// 中文确认提示:列出将发生的改动,读 stdin 一行;EOF/非 y 视为取消(安全默认)
fn confirm(prefix: &Prefix, state: &State) -> io::Result<bool> {
    println!("即将卸载 setup-coder:");
    println!(
        "  1. 回滚 PATH 改动({} 条,按安装清单精确回滚)",
        state.path_injections.len()
    );
    println!("  2. 删除 Private Prefix:{}", prefix.root().display());
    println!();
    print!("确认卸载?[y/N] ");
    io::stdout().flush()?;
    let mut reply = String::new();
    io::stdin().read_line(&mut reply)?;
    Ok(confirmed(&reply))
}

/// 确认输入判定:只认 y / yes(大小写不敏感),其余一律视为取消
fn confirmed(reply: &str) -> bool {
    matches!(reply.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// 一条 PATH 注入的中文描述(用于回滚日志)
fn describe_injection(injection: &PathInjection) -> String {
    match injection {
        PathInjection::ShellRc { file, .. } => format!("{} 中的 PATH 行", file.display()),
        PathInjection::WindowsUserPath { dir } => {
            format!("用户环境变量 Path 中的 {}", dir.display())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn confirmed_only_accepts_y_or_yes() {
        assert!(confirmed("y"));
        assert!(confirmed("Y\n"));
        assert!(confirmed(" yes "));
        assert!(confirmed("YES"));
        // 默认否:空、EOF 空串、任意其他输入都视为取消(安全默认)
        assert!(!confirmed(""));
        assert!(!confirmed("\n"));
        assert!(!confirmed("n"));
        assert!(!confirmed("yep"));
    }

    #[test]
    fn describe_injection_covers_both_kinds() {
        let rc = describe_injection(&PathInjection::ShellRc {
            file: PathBuf::from("/home/u/.zshrc"),
            line: "export PATH=...".into(),
        });
        assert!(rc.contains(".zshrc"));
        let win = describe_injection(&PathInjection::WindowsUserPath {
            dir: PathBuf::from(r"C:\Users\u\.setup-coder\bin"),
        });
        assert!(win.contains("Path"));
    }
}
