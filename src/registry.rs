//! Tool 静态注册表:名称 → npm 包名 → 校验命令 → Node 下限。加新 Tool = 加一行。

use std::cmp::max;

use semver::Version;

/// Tool 的分发来源(CONTEXT.md:Tool 的两种形态)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// npm 包(经 npmmirror 安装);运行时需要 Node >= node_floor
    Npm {
        /// npm 包名(已核实存在于 npmmirror)
        package: &'static str,
        /// engines.node 下限:运行时 Node 版本必须 >= 此值。
        /// 数据源:docs/research/node-version-requirements.md(npm `latest` 元数据,2026-09-05 核实)。
        /// 语义为纯下限,不带上限/范围,故用最小 `Version` 表示而不用 `VersionReq`;
        /// 跨工具比较只做偏序上的 max,`VersionReq` 无此操作。
        node_floor: Version,
    },
    /// 预编译二进制资产(GitHub Releases,下载容错链在 net.rs);零运行时依赖
    Binary,
}

/// 一个 AI 编程 CLI(CONTEXT.md:Tool)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// CLI 参数名(`install <tool>` 用的标识)
    pub name: &'static str,
    /// 安装出的可执行文件名,也是 shim/二进制名与校验命令(`<bin> --version`)
    pub bin: &'static str,
    /// 分发来源(决定安装路径:npm install / 二进制下载)
    pub source: ToolSource,
}

impl Tool {
    /// npm 包名;二进制来源返回 None
    pub fn package(&self) -> Option<&'static str> {
        match &self.source {
            ToolSource::Npm { package, .. } => Some(package),
            ToolSource::Binary => None,
        }
    }

    /// Node 版本下限;二进制来源无 Node 依赖,返回 0.0.0(不限制)
    pub fn node_floor(&self) -> Version {
        match &self.source {
            ToolSource::Npm { node_floor, .. } => node_floor.clone(),
            ToolSource::Binary => Version::new(0, 0, 0),
        }
    }
}

/// v1 四个 Tool
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "codex",
        bin: "codex",
        source: ToolSource::Npm {
            package: "@openai/codex",
            node_floor: Version::new(16, 0, 0),
        },
    },
    Tool {
        name: "claude-code",
        bin: "claude",
        source: ToolSource::Npm {
            package: "@anthropic-ai/claude-code",
            node_floor: Version::new(22, 0, 0),
        },
    },
    Tool {
        name: "pi",
        bin: "pi",
        source: ToolSource::Npm {
            package: "@earendil-works/pi-coding-agent",
            node_floor: Version::new(22, 19, 0),
        },
    },
    // omp 在 npmmirror 有包(@oh-my-pi/pi-coding-agent)但硬依赖 Bun 运行时
    // (bun: protocol imports,Node 实测起不来),故走预编译二进制资产(ADR-0005)
    Tool {
        name: "omp",
        bin: "omp",
        source: ToolSource::Binary,
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
pub fn floor_for_tools(tools: &[&Tool]) -> Version {
    tools
        .iter()
        .map(|t| t.node_floor())
        .fold(Version::new(0, 0, 0), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_has_exactly_four_tools() {
        assert_eq!(TOOLS.len(), 4);
    }

    #[test]
    fn names_and_bins_are_unique() {
        for (i, a) in TOOLS.iter().enumerate() {
            for b in &TOOLS[i + 1..] {
                assert_ne!(a.name, b.name, "name 重复");
                assert_ne!(a.bin, b.bin, "bin 重复");
                assert_ne!(a.source, b.source, "source 重复");
            }
        }
    }

    #[test]
    fn find_by_name_and_bin() {
        assert_eq!(find("codex").unwrap().package(), Some("@openai/codex"));
        assert_eq!(
            find("claude-code").unwrap().package(),
            Some("@anthropic-ai/claude-code")
        );
        // bin 名也能命中(用户更可能记得 `claude`)
        assert_eq!(find("claude").unwrap().name, "claude-code");
        assert_eq!(
            find("pi").unwrap().package(),
            Some("@earendil-works/pi-coding-agent")
        );
        // omp:二进制来源,无 npm 包、无 Node 下限
        let omp = find("omp").unwrap();
        assert_eq!(omp.source, ToolSource::Binary);
        assert_eq!(omp.package(), None);
        assert_eq!(omp.node_floor(), Version::new(0, 0, 0));
    }

    #[test]
    fn find_unknown_returns_none() {
        assert!(find("cursor").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn floors_match_research_doc() {
        // docs/research/node-version-requirements.md(2026-09-05 核实)是唯一数据源
        assert_eq!(find("codex").unwrap().node_floor(), Version::new(16, 0, 0));
        assert_eq!(
            find("claude-code").unwrap().node_floor(),
            Version::new(22, 0, 0)
        );
        assert_eq!(find("pi").unwrap().node_floor(), Version::new(22, 19, 0));
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
    fn floor_for_binary_tool_is_zero() {
        // 二进制工具无 Node 依赖,不抬下限
        let omp = find("omp").unwrap();
        assert_eq!(floor_for_tools(&[omp]), Version::new(0, 0, 0));
    }

    #[test]
    fn floor_for_empty_set_is_zero() {
        assert_eq!(floor_for_tools(&[]), Version::new(0, 0, 0));
    }
}
