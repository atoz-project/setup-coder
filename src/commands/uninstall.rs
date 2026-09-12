//! `uninstall` 子命令:按 state.json 精确回滚 PATH/fnm 钩子注入,再删除 Private Prefix。
//!
//! 不做模糊匹配:只回滚安装清单里逐字记录的改动;系统 git(apt/CLT 所装)
//! 在前缀外、清单无记录,绝不动。用户的 Node 环境(nvm/fnm/裸 Node,含
//! setup-coder 代装的 fnm)一律保留,只按来源打印保留提示与手工移除步骤
//! (ADR-0003,工单 #23)。Windows 自身 exe 残留策略见 platform::remove_prefix
//! 的决策注释(工单 #4,真机验证归工单 #7)。

use std::error::Error;
use std::io::{self, Write};

use crate::platform;
use crate::prefix::{NodeSourceKind, NodeState, PathInjection, Prefix, State};

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
    if let Some(hint) = preserved_hint(&state) {
        println!("保留的 Node 环境:{hint}");
    }
    if rollback_failed > 0 {
        println!("卸载完成,但有 {rollback_failed} 条 PATH 改动回滚失败,请按上面的警告手工核对。");
    } else {
        println!("卸载完成。PATH 改动将在新开终端后生效。");
    }
    Ok(())
}

/// 中文确认提示:列出将发生的改动与将保留的 Node 环境,读 stdin 一行;EOF/非 y 视为取消(安全默认)
fn confirm(prefix: &Prefix, state: &State) -> io::Result<bool> {
    println!("即将卸载 setup-coder:");
    println!(
        "  1. 回滚 PATH 改动({} 条,按安装清单精确回滚)",
        state.path_injections.len()
    );
    println!("  2. 删除 Private Prefix:{}", prefix.root().display());
    match preserved_summary(state) {
        Some(summary) => println!("  3. 保留你的 Node 环境(不删除):{summary}"),
        None => println!("  3. 无 Node 记录(清单里未完成 Node 安装)"),
    }
    println!();
    print!("确认卸载?[y/N] ");
    io::stdout().flush()?;
    let mut reply = String::new();
    io::stdin().read_line(&mut reply)?;
    Ok(confirmed(&reply))
}

// ---------------------------------------------------------------------------
// 保留提示(工单 #23,ADR-0003):uninstall 绝不删用户的 nvm/fnm/Node,
// 按 v2 清单的 node.source 给出「保留了什么 + 手工移除步骤」。
// ---------------------------------------------------------------------------

/// 确认提示里的一行式「保留摘要」(来源 + 版本)。
///
/// 管理器来源带上记录里的 node exe 路径(机器上该 Node 的位置,卸载不会删它)。
fn preserved_summary(state: &State) -> Option<String> {
    let node = state.node.as_ref()?;
    let location = node_location(node);
    Some(match node.source {
        NodeSourceKind::Bare => {
            format!("你机器上原有的 Node({},{location})将被保留", node.version)
        }
        NodeSourceKind::Nvm => {
            format!("你的 nvm 与其 Node({},{location})将被保留", node.version)
        }
        NodeSourceKind::Fnm => {
            format!("fnm 及其 Node({},{location})将被保留", node.version)
        }
    })
}

/// 记录里该 Node 的位置:管理器来源取 exe 路径,裸 Node 在 PATH 上。
fn node_location(node: &NodeState) -> String {
    match &node.exe {
        Some(exe) => format!("{}", exe.display()),
        None => "PATH 上".to_string(),
    }
}

/// 卸载完成后按来源打印的保留提示(中文)。
///
/// `state.node.source` 决定文案;`state.path_injections` 里是否存在 FnmHook
/// 记录区分「fnm 是否由 setup-coder 代装」(代装时 install 一定注入了钩子行)。
/// 部分清单(F3:安装失败在 Node 落账前,node 无记录)但带 FnmHook 记录时,
/// fnm 同样已代装在盘上,提示照常给出。提示只讲「保留了什么 + 手工移除步骤」,
/// 绝不在卸载里替用户删任何 Node 资产。
fn preserved_hint(state: &State) -> Option<String> {
    let Some(node) = state.node.as_ref() else {
        // F3 部分清单(安装失败在 Node 落账前):钩子记录证明 fnm 已代装且在盘上
        if has_fnm_hook(state) {
            let fnm_dir = crate::node_source::fnm_default_home()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|_| "fnm 数据目录".to_string());
            return Some(format!(
                "setup-coder 为你安装了 fnm 与 Node(安装未全部完成;位于 {fnm_dir}),\
                 卸载未删除它们。如需移除:删除 {fnm_dir},并移除 shell rc 中残留的 \
                 fnm 钩子行(卸载已按记录回滚)。"
            ));
        }
        return None;
    };
    let location = node_location(node);
    Some(match node.source {
        NodeSourceKind::Bare => {
            format!("你机器上原有的 Node({},{location})未受影响。", node.version)
        }
        NodeSourceKind::Nvm => format!(
            "你的 nvm 与其 Node({location})未受影响;如需移除 Node 请自行 `nvm uninstall <版本>`。"
        ),
        NodeSourceKind::Fnm => {
            if has_fnm_hook(state) {
                let fnm_dir = crate::node_source::fnm_default_home()
                    .map(|d| d.display().to_string())
                    .unwrap_or_else(|_| "fnm 数据目录".to_string());
                format!(
                    "setup-coder 为你安装了 fnm 与 Node({location}),卸载未删除它们。\
                     如需移除:删除 {fnm_dir},并移除 shell rc 中残留的 fnm 钩子行(卸载已按记录回滚)。"
                )
            } else {
                format!(
                    "你的 fnm 与其 Node({location})未受影响;如需移除 Node 请自行 `fnm uninstall <版本>`。"
                )
            }
        }
    })
}

/// 清单里是否有 fnm 钩子注入记录(即「fnm 由 setup-coder 代装」的判别依据)。
fn has_fnm_hook(state: &State) -> bool {
    state
        .path_injections
        .iter()
        .any(|i| matches!(i, PathInjection::FnmHook { .. }))
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
        PathInjection::FnmHook { file, .. } => {
            format!("{} 中的 fnm 钩子行(setup-coder 注入)", file.display())
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
    fn describe_injection_covers_all_kinds() {
        let rc = describe_injection(&PathInjection::ShellRc {
            file: PathBuf::from("/home/u/.zshrc"),
            line: "export PATH=...".into(),
        });
        assert!(rc.contains(".zshrc"));
        let win = describe_injection(&PathInjection::WindowsUserPath {
            dir: PathBuf::from(r"C:\Users\u\.setup-coder\bin"),
        });
        assert!(win.contains("Path"));
        // fnm 钩子注入:描述须指向 rc 文件并标明是 fnm 钩子
        let hook = describe_injection(&PathInjection::FnmHook {
            file: PathBuf::from("/home/u/.zshrc"),
            line: "eval \"$(fnm env --use-on-cd)\"".into(),
        });
        assert!(hook.contains(".zshrc"));
        assert!(hook.contains("fnm"));
    }

    /// 工单 #23:按来源给出保留提示;FnmHook 记录区分「setup-coder 代装 fnm」
    /// 与「用户自有 fnm」。提示必须指明保留内容 + 手工移除步骤。
    fn state_with(
        source: NodeSourceKind,
        exe: Option<&str>,
        injections: Vec<PathInjection>,
    ) -> State {
        State {
            version: crate::prefix::STATE_VERSION,
            node: Some(NodeState {
                source,
                version: "v24.19.0".into(),
                exe: exe.map(PathBuf::from),
            }),
            tools: Vec::new(),
            path_injections: injections,
        }
    }

    #[test]
    fn preserved_hint_user_bare() {
        let s = state_with(NodeSourceKind::Bare, None, Vec::new());
        let hint = preserved_hint(&s).unwrap();
        assert!(hint.contains("未受影响"), "bare: {hint}");
        assert!(hint.contains("v24.19.0"), "bare 带版本: {hint}");
        // 裸 Node 无 exe 记录 → 位置落在 PATH 上
        assert!(hint.contains("PATH"), "bare 位置: {hint}");
    }

    #[test]
    fn preserved_hint_user_nvm() {
        let s = state_with(
            NodeSourceKind::Nvm,
            Some("/home/u/.nvm/versions/node/v24.19.0/bin/node"),
            Vec::new(),
        );
        let hint = preserved_hint(&s).unwrap();
        assert!(hint.contains("nvm"), "nvm: {hint}");
        assert!(hint.contains("nvm uninstall"), "nvm 手工移除步骤: {hint}");
        assert!(hint.contains(".nvm"), "nvm 带 exe 路径: {hint}");
        assert!(hint.contains("未受影响"), "nvm: {hint}");
    }

    #[test]
    fn preserved_hint_user_fnm_setup_coder_installed() {
        // 清单里有 FnmHook 记录 → fnm 由 setup-coder 代装
        let s = state_with(
            NodeSourceKind::Fnm,
            Some("/home/u/.local/share/fnm/node-versions/v24.19.0/installation/bin/node"),
            vec![PathInjection::FnmHook {
                file: PathBuf::from("/home/u/.zshrc"),
                line: "eval \"$(fnm env --use-on-cd)\"  # setup-coder fnm".into(),
            }],
        );
        let hint = preserved_hint(&s).unwrap();
        assert!(hint.contains("setup-coder 为你安装了 fnm"), "代装措辞: {hint}");
        assert!(hint.contains("fnm"), "代装给出 fnm: {hint}");
        assert!(hint.contains("删除"), "代装给出移除步骤: {hint}");
        // 代装情形必须提示「卸载未删除它们」
        assert!(hint.contains("未删除"), "代装说明未删: {hint}");
    }

    #[test]
    fn preserved_hint_user_fnm_user_owned() {
        // 清单里无 FnmHook 记录 → fnm 是用户自己的,提示保持原样、给 fnm uninstall
        let s = state_with(
            NodeSourceKind::Fnm,
            Some("/home/u/.local/share/fnm/node-versions/v24.19.0/installation/bin/node"),
            Vec::new(),
        );
        let hint = preserved_hint(&s).unwrap();
        assert!(hint.contains("未受影响"), "自有 fnm 保留: {hint}");
        assert!(hint.contains("fnm uninstall"), "自有 fnm 手工移除步骤: {hint}");
        // 自有 fnm 不得出现「setup-coder 为你安装」的措辞
        assert!(!hint.contains("为你安装"), "自有 fnm 不得误标代装: {hint}");
    }

    #[test]
    fn has_fnm_hook_detection() {
        let with = state_with(
            NodeSourceKind::Fnm,
            Some("/x"),
            vec![PathInjection::FnmHook {
                file: PathBuf::from("/home/u/.zshrc"),
                line: "l".into(),
            }],
        );
        assert!(has_fnm_hook(&with));
        // ShellRc 注入不算 fnm 钩子
        let without = state_with(
            NodeSourceKind::Fnm,
            Some("/x"),
            vec![PathInjection::ShellRc {
                file: PathBuf::from("/home/u/.zshrc"),
                line: "export PATH=...".into(),
            }],
        );
        assert!(!has_fnm_hook(&without));
    }

    #[test]
    fn preserved_hint_none_when_no_node_record() {
        let s = State::default();
        assert!(preserved_hint(&s).is_none());
        assert!(preserved_summary(&s).is_none());
    }

    /// F3 部分清单(node 未落账、仅 FnmHook 记录):保留提示仍按「代装 fnm」给出
    /// (fnm 已在盘上,uninstall 按设计保留);v0.2.0 此时直接 None。
    #[test]
    fn preserved_hint_partial_manifest_setup_coder_fnm() {
        let mut s = State::default();
        s.record_injection(PathInjection::FnmHook {
            file: PathBuf::from("/home/u/.zshrc"),
            line: "l".into(),
        });
        let hint = preserved_hint(&s).expect("部分清单(仅钩子记录)也应给代装 fnm 提示");
        assert!(hint.contains("setup-coder 为你安装了 fnm"), "代装措辞:{hint}");
        assert!(hint.contains("删除"), "给出移除步骤:{hint}");
        // 无钩子记录的部分清单 → 仍 None(与空清单一致)
        assert!(preserved_hint(&State::default()).is_none());
    }

    /// F3 回归:部分清单(node 未落账、仅 FnmHook 记录——v0.2.0 Windows 实机的
    /// 失败形态)下 uninstall 仍按记录精确回滚钩子行、删前缀,用户 rc 其余内容
    /// 不受影响。
    #[cfg(unix)]
    #[test]
    fn uninstall_rolls_back_hooks_from_partial_manifest() {
        let root = std::env::temp_dir().join(format!(
            "setup-coder-test-un-partial-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _home = crate::test_util::ScopedHome::set(&root);
        let prefix = Prefix::home().unwrap();
        std::fs::create_dir_all(prefix.root()).unwrap();

        // 部分清单:fnm 已代装、钩子已注入 rc、Node 未落账(F1 失败形态)
        let hook_line = platform::fnm_hook_line(std::path::Path::new("/x/fnm/fnm"));
        let rc = root.join(".zshrc");
        std::fs::write(&rc, format!("# user rc\n{hook_line}\n")).unwrap();
        let mut state = State::default();
        state.record_injection(PathInjection::FnmHook {
            file: rc.clone(),
            line: hook_line.clone(),
        });
        state.save(&prefix).unwrap();

        uninstall(true).unwrap();

        let after = std::fs::read_to_string(&rc).unwrap();
        assert!(!after.contains("env --use-on-cd"), "钩子行应被精确回滚:{after}");
        assert!(after.contains("# user rc"), "用户原内容保留:{after}");
        assert!(!prefix.root().exists(), "前缀已删");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preserved_summary_names_source_and_version() {
        let bare = state_with(NodeSourceKind::Bare, None, Vec::new());
        let s = preserved_summary(&bare).unwrap();
        assert!(s.contains("Node"), "summary: {s}");
        assert!(s.contains("v24.19.0"), "summary 带版本: {s}");
        let fnm = state_with(NodeSourceKind::Fnm, Some("/fnm/x"), Vec::new());
        assert!(preserved_summary(&fnm).unwrap().contains("fnm"));
    }
}
