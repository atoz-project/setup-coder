//! Node 来源决策层(工单 #16):纯函数,零 IO,无平台 cfg。
//!
//! 输入是调用方事先探测好的事实(`NodeFacts`),输出是要执行的 Node 方案(`NodePlan`)。
//! 本模块不触碰文件系统、进程、环境变量;决策只看注册表下限,不做 EOL/支持周期检查。
//!
//! 决策优先级(锁定,勿改):达标裸 Node 复用 > nvm > fnm > 新装 fnm。
//! 与 CONTEXT.md「Prerequisite」的长期优先级(复用 nvm > 复用 fnm > 新装 fnm)并存:
//! 本规则在其之前插入「达标裸 Node 直接复用」一档,裸 Node 不达标时两表一致。

use std::path::PathBuf;

use semver::Version;

use crate::registry::{Tool, floor_for_tools};

/// 探测到的机器事实(IO 由调用方完成,本类型只承载结果)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFacts {
    /// PATH 上的裸 Node(不属任何版本管理器):版本 + 可执行文件路径
    pub bare_node: Option<(Version, PathBuf)>,
    /// 已装的 nvm:其当前 Node 版本 + nvm 自身路径
    pub nvm: Option<(Version, PathBuf)>,
    /// 已装的 fnm:其当前 Node 版本 + fnm 自身路径
    pub fnm: Option<(Version, PathBuf)>,
}

/// 决策产物:用哪个 Node,以及必须满足的版本下限。
///
/// 各变体携带的 `Version` 即所选工具集的下限(`floor_for_tools`),由执行层决定
/// 具体装/切到哪个满足下限的版本(本层不挑选具体版本号,不做网络查询)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodePlan {
    /// 裸 Node 已达标,直接复用。携带:node 可执行文件路径、所满足的下限版本
    ReuseBareNode { path: PathBuf, version: Version },
    /// 用已有 nvm 安装/切换到达标 Node。携带:nvm 路径、目标下限版本
    UseNvm { path: PathBuf, version: Version },
    /// 用已有 fnm 安装/切换到达标 Node。携带:fnm 路径、目标下限版本
    UseFnm { path: PathBuf, version: Version },
    /// 无任何可用来源,新装 fnm 再用它装 Node。携带:目标下限版本
    InstallFnm { version: Version },
}

/// 纯决策:给定事实与所选工具,返回 Node 来源方案。
///
/// 规则(锁定,勿改):
/// 1. 裸 Node 存在且版本 >= `floor_for_tools(tools)` → 复用裸 Node
/// 2. 否则有 nvm → 用 nvm
/// 3. 否则有 fnm → 用 fnm
/// 4. 都没有 → 新装 fnm
pub fn decide(facts: &NodeFacts, tools: &[&Tool]) -> NodePlan {
    let floor = floor_for_tools(tools);
    if let Some((version, path)) = &facts.bare_node {
        if version >= &floor {
            return NodePlan::ReuseBareNode {
                path: path.clone(),
                version: floor,
            };
        }
    }
    if let Some((_, path)) = &facts.nvm {
        return NodePlan::UseNvm {
            path: path.clone(),
            version: floor,
        };
    }
    if let Some((_, path)) = &facts.fnm {
        return NodePlan::UseFnm {
            path: path.clone(),
            version: floor,
        };
    }
    NodePlan::InstallFnm { version: floor }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version::new(major, minor, patch)
    }

    fn tool(name: &str) -> &'static Tool {
        crate::registry::find(name).unwrap()
    }

    fn facts(
        bare: Option<(u64, u64, u64)>,
        nvm: Option<(u64, u64, u64)>,
        fnm: Option<(u64, u64, u64)>,
    ) -> NodeFacts {
        NodeFacts {
            bare_node: bare.map(|(a, b, c)| (v(a, b, c), PathBuf::from("/usr/local/bin/node"))),
            nvm: nvm.map(|(a, b, c)| (v(a, b, c), PathBuf::from("/home/u/.nvm"))),
            fnm: fnm.map(|(a, b, c)| (v(a, b, c), PathBuf::from("/home/u/.fnm"))),
        }
    }

    /// 期望(变体, 裸 Node 版本, nvm 版本, fnm 版本, 工具名集, 期望携带版本)
    type Case = (
        &'static str,
        Option<(u64, u64, u64)>,
        Option<(u64, u64, u64)>,
        Option<(u64, u64, u64)>,
        &'static [&'static str],
        (u64, u64, u64),
    );

    /// 期望变体对应的构造器断言在表下逐一展开
    fn cases() -> Vec<Case> {
        vec![
            // 1. 达标裸 Node(全集,22.20.0 >= 22.19.0)→ 复用,即便同时有 nvm/fnm
            (
                "reuse_bare",
                Some((22, 20, 0)),
                Some((22, 20, 0)),
                Some((22, 20, 0)),
                &["codex", "claude-code", "pi"],
                (22, 19, 0),
            ),
            // 2. 裸 Node 不达标(18.x < 22.19.0)且有 nvm → UseNvm
            (
                "use_nvm",
                Some((18, 19, 0)),
                Some((18, 19, 0)),
                Some((18, 19, 0)),
                &["codex", "claude-code", "pi"],
                (22, 19, 0),
            ),
            // 3. 裸 Node 不达标、无 nvm、仅 fnm → UseFnm
            (
                "use_fnm",
                Some((20, 0, 0)),
                None,
                Some((20, 0, 0)),
                &["codex", "claude-code", "pi"],
                (22, 19, 0),
            ),
            // 4. 无任何来源 → InstallFnm
            (
                "install_fnm",
                None,
                None,
                None,
                &["codex", "claude-code", "pi"],
                (22, 19, 0),
            ),
            // 5. 边界:裸 Node 恰等于下限 22.19.0 → 达标,复用
            (
                "reuse_bare",
                Some((22, 19, 0)),
                None,
                None,
                &["pi"],
                (22, 19, 0),
            ),
            // 6. 单工具下限低(仅 codex,>=16)→ 复用全集会拒绝的旧 Node 18
            ("reuse_bare", Some((18, 19, 0)), None, None, &["codex"], (16, 0, 0)),
            // 7. 无裸 Node 但有 nvm → UseNvm(不依赖裸 Node 分支)
            (
                "use_nvm",
                None,
                Some((20, 0, 0)),
                Some((20, 0, 0)),
                &["codex", "claude-code", "pi"],
                (22, 19, 0),
            ),
            // 8. 裸 Node 恰好差一个 patch(22.18.99 < 22.19.0)且无管理器 → InstallFnm
            (
                "install_fnm",
                Some((22, 18, 99)),
                None,
                None,
                &["pi"],
                (22, 19, 0),
            ),
        ]
    }

    #[test]
    fn decision_table_covers_every_branch() {
        for (want, bare, nvm, fnm, tool_names, (fa, fb, fc)) in cases() {
            let tools: Vec<&Tool> = tool_names.iter().map(|n| tool(n)).collect();
            let plan = decide(&facts(bare, nvm, fnm), &tools);
            let floor = v(fa, fb, fc);
            let ctx = format!("want={want} bare={bare:?} nvm={nvm:?} fnm={fnm:?}");
            match (want, &plan) {
                ("reuse_bare", NodePlan::ReuseBareNode { path, version }) => {
                    assert_eq!(path, &PathBuf::from("/usr/local/bin/node"), "{ctx}");
                    assert_eq!(version, &floor, "{ctx}");
                }
                ("use_nvm", NodePlan::UseNvm { path, version }) => {
                    assert_eq!(path, &PathBuf::from("/home/u/.nvm"), "{ctx}");
                    assert_eq!(version, &floor, "{ctx}");
                }
                ("use_fnm", NodePlan::UseFnm { path, version }) => {
                    assert_eq!(path, &PathBuf::from("/home/u/.fnm"), "{ctx}");
                    assert_eq!(version, &floor, "{ctx}");
                }
                ("install_fnm", NodePlan::InstallFnm { version }) => {
                    assert_eq!(version, &floor, "{ctx}");
                }
                _ => panic!("{ctx}: 期望 {want},实际 {plan:?}"),
            }
        }
    }

    #[test]
    fn nvm_wins_over_fnm_when_bare_noncompliant() {
        // 裸 Node 不达标时 nvm 优先于 fnm(CONTEXT.md:Prerequisite 优先级)
        let tools: Vec<&Tool> = crate::registry::all().iter().collect();
        let plan = decide(&facts(Some((20, 0, 0)), Some((20, 0, 0)), Some((20, 0, 0))), &tools);
        assert!(matches!(plan, NodePlan::UseNvm { .. }));
    }
}
