//! Node 来源解析接缝(工单 #15):所有需要 Node 运行时的调用点统一经此获取
//! 「当前选定的 Node」,不再各自硬编码前缀内路径。
//!
//! 本阶段接缝仍始终返回前缀内 Node + 固定 LTS 版本(行为与此前完全一致);
//! 后续工单(Prerequisite 复用已有 nvm/fnm,见 CONTEXT.md)将在此插拔其他来源。
//!
//! 契约预告(工单 #21):shim 生成将改为按绝对路径 exec 选定 Node,
//! 而非把 node 目录前置进 PATH;届时 `platform::shim_content` / `write_shim`
//! 的入参会从 node_bin_dir 变为本接缝给出的 exe 绝对路径。

use std::path::{Path, PathBuf};

use crate::platform;
use crate::prefix::Prefix;

/// Node LTS(Krypton)。升级 = 改这一行并重测。
/// 核实来源:npmmirror node 镜像 index.json,2026-08 时为最新 LTS。
pub const NODE_VERSION: &str = "v24.19.0";

/// Node 来源标签:选定 Node 从哪来(本阶段恒为前缀内)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSourceKind {
    /// 前缀内新装的 Node LTS(当前唯一来源)
    Prefix,
}

/// 解析接缝的产出:当前选定的 Node。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSource {
    /// node / node.exe 可执行文件的绝对路径
    exe: PathBuf,
    /// Node 版本(如 `v24.19.0`)
    version: &'static str,
    /// 来源标签
    kind: NodeSourceKind,
}

impl NodeSource {
    /// node / node.exe 可执行文件的绝对路径
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Node 版本
    pub fn version(&self) -> &str {
        self.version
    }

    /// 来源标签
    // 本阶段尚无消费方(后续工单按来源分支,如 doctor 报告「复用 nvm」);接缝契约的一部分,死代码豁免
    #[allow(dead_code)]
    pub fn kind(&self) -> NodeSourceKind {
        self.kind
    }

    /// node 可执行文件所在目录(用于把选定 Node 前置进子进程 PATH)
    pub fn bin_dir(&self) -> &Path {
        // exe = <bin_dir>/node[.exe],父目录即 bin 目录
        self.exe.parent().expect("node exe 必有父目录")
    }

    /// Node 解压根目录(npm-cli.js 相对它定位)
    pub fn node_dir(&self) -> &Path {
        // 布局:unix 为 node/bin/node、Windows 为 node/node.exe
        // (见 platform::node_bin_subdir),故 bin_dir 的上级即解压根
        self.bin_dir().parent().expect("node bin 目录必有父目录")
    }

    /// 选定 Node 自带的 npm-cli.js 绝对路径(布局因平台而异)
    pub fn npm_cli(&self) -> PathBuf {
        self.node_dir().join(platform::npm_cli_subpath())
    }
}

/// 唯一的 Node 解析接缝:返回当前选定的 Node。
///
/// 当前实现始终返回前缀内 Node + 固定 LTS 版本(NODE_VERSION)。
pub fn resolve(prefix: &Prefix) -> NodeSource {
    NodeSource {
        exe: prefix.node_exe(),
        version: NODE_VERSION,
        kind: NodeSourceKind::Prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_prefix_node_with_fixed_lts() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let node = resolve(&prefix);
        assert_eq!(node.exe(), prefix.node_exe());
        assert_eq!(node.version(), NODE_VERSION);
        assert_eq!(node.kind(), NodeSourceKind::Prefix);
    }

    #[test]
    fn derived_paths_match_prefix_layout() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let node = resolve(&prefix);
        assert_eq!(node.bin_dir(), prefix.node_bin_dir());
        assert_eq!(node.node_dir(), prefix.node_dir());
        assert_eq!(
            node.npm_cli(),
            prefix.node_dir().join(platform::npm_cli_subpath())
        );
    }
}
