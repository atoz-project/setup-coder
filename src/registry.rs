//! Tool 静态注册表:名称 → npm 包名 → 校验命令 → Node 下限。加新 Tool = 加一行。

use std::cmp::max;

use semver::Version;

/// 一个以 npm 包分发的 AI 编程 CLI(CONTEXT.md:Tool)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// CLI 参数名(`install <tool>` 用的标识)
    pub name: &'static str,
    /// npm 包名(已核实存在于 npmmirror)
    pub package: &'static str,
    /// 该包安装出的可执行文件名,也是 shim 名与校验命令(`<bin> --version`)
    pub bin: &'static str,
    /// engines.node 下限:运行时 Node 版本必须 >= 此值。
    /// 数据源:docs/research/node-version-requirements.md(npm `latest` 元数据,2026-09-05 核实)。
    /// 语义为纯下限,不带上限/范围,故用最小 `Version` 表示而不用 `VersionReq`;
    /// 跨工具比较只做偏序上的 max,`VersionReq` 无此操作。
    pub node_floor: Version,
}

/// v1 三个 Tool
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "codex",
        package: "@openai/codex",
        bin: "codex",
        node_floor: Version::new(16, 0, 0),
    },
    Tool {
        name: "claude-code",
        package: "@anthropic-ai/claude-code",
        bin: "claude",
        node_floor: Version::new(22, 0, 0),
    },
    Tool {
        name: "pi",
        package: "@earendil-works/pi-coding-agent",
        bin: "pi",
        node_floor: Version::new(22, 19, 0),
    },
];

/// 全部 Tool(`install` 不带参数 = 装全部)
pub fn all() -> &'static [Tool] {
    TOOLS
}

/// 按名称(或 bin 名)查找 Tool
pub fn find(name_or_bin: &str) -> Option<&'static Tool> {
    TOOLS
        .iter()
        .find(|t| t.name == name_or_bin || t.bin == name_or_bin)
}

/// 所选工具集的 Node 版本下限:各工具 `engines.node` 下限的最大值。
/// 空集合返回 0.0.0(不限制,任何 Node 均达标);空选择是否合法由调用方决定。
// 注:决策层(工单 #16)尚未接线到 install,接线前豁免死代码告警
#[allow(dead_code)]
pub fn floor_for_tools(tools: &[&Tool]) -> Version {
    tools
        .iter()
        .map(|t| t.node_floor.clone())
        .fold(Version::new(0, 0, 0), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_has_exactly_three_tools() {
        assert_eq!(TOOLS.len(), 3);
    }

    #[test]
    fn names_and_bins_are_unique() {
        for (i, a) in TOOLS.iter().enumerate() {
            for b in &TOOLS[i + 1..] {
                assert_ne!(a.name, b.name, "name 重复");
                assert_ne!(a.bin, b.bin, "bin 重复");
                assert_ne!(a.package, b.package, "package 重复");
            }
        }
    }

    #[test]
    fn find_by_name_and_bin() {
        assert_eq!(find("codex").unwrap().package, "@openai/codex");
        assert_eq!(
            find("claude-code").unwrap().package,
            "@anthropic-ai/claude-code"
        );
        // bin 名也能命中(用户更可能记得 `claude`)
        assert_eq!(find("claude").unwrap().name, "claude-code");
        assert_eq!(
            find("pi").unwrap().package,
            "@earendil-works/pi-coding-agent"
        );
    }

    #[test]
    fn find_unknown_returns_none() {
        assert!(find("cursor").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn floors_match_research_doc() {
        // docs/research/node-version-requirements.md(2026-09-05 核实)是唯一数据源
        assert_eq!(find("codex").unwrap().node_floor, Version::new(16, 0, 0));
        assert_eq!(
            find("claude-code").unwrap().node_floor,
            Version::new(22, 0, 0)
        );
        assert_eq!(find("pi").unwrap().node_floor, Version::new(22, 19, 0));
    }

    #[test]
    fn floor_for_tools_returns_max_floor() {
        let by_name = |name: &str| find(name).unwrap();

        // 全集:pi 的 22.19.0 最严
        let all: Vec<&Tool> = all().iter().collect();
        assert_eq!(floor_for_tools(&all), Version::new(22, 19, 0));

        // 子集与顺序无关
        let codex = by_name("codex");
        let claude = by_name("claude-code");
        assert_eq!(floor_for_tools(&[codex, claude]), Version::new(22, 0, 0));
        assert_eq!(floor_for_tools(&[claude, codex]), Version::new(22, 0, 0));

        // 单工具 = 自身下限
        assert_eq!(floor_for_tools(&[codex]), Version::new(16, 0, 0));
    }

    #[test]
    fn floor_for_empty_set_is_zero() {
        assert_eq!(floor_for_tools(&[]), Version::new(0, 0, 0));
    }
}
