//! Tool 静态注册表:名称 → npm 包名 → 校验命令。加新 Tool = 加一行。

/// 一个以 npm 包分发的 AI 编程 CLI(CONTEXT.md:Tool)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    /// CLI 参数名(`install <tool>` 用的标识)
    pub name: &'static str,
    /// npm 包名(已核实存在于 npmmirror)
    pub package: &'static str,
    /// 该包安装出的可执行文件名,也是 shim 名与校验命令(`<bin> --version`)
    pub bin: &'static str,
}

/// v1 三个 Tool
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "codex",
        package: "@openai/codex",
        bin: "codex",
    },
    Tool {
        name: "claude-code",
        package: "@anthropic-ai/claude-code",
        bin: "claude",
    },
    Tool {
        name: "pi",
        package: "@earendil-works/pi-coding-agent",
        bin: "pi",
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
}
