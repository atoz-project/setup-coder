//! Node 来源解析接缝(工单 #15):所有需要 Node 运行时的调用点统一经此获取
//! 「当前选定的 Node」,不再各自硬编码前缀内路径。
//!
//! 工单 #19 起接缝按 NodePlan 分叉:复用裸 Node → 指向用户机器上的绝对路径;
//! 前缀内新装 LTS → 前缀布局(保底路径,#22 随 fnm 安装接线后随状态迁移移除)。
//!
//! 契约预告(工单 #21):shim 生成将改为按绝对路径 exec 选定 Node,
//! 而非把 node 目录前置进 PATH;届时 `platform::shim_content` / `write_shim`
//! 的入参会从 node_bin_dir 变为本接缝给出的 exe 绝对路径。

use std::path::{Path, PathBuf};

use crate::node_plan::NodePlan;
use crate::platform;
use crate::prefix::{NodeSourceKind, Prefix};

/// Node LTS(Krypton)。升级 = 改这一行并重测。
/// 核实来源:npmmirror node 镜像 index.json,2026-08 时为最新 LTS。
pub const NODE_VERSION: &str = "v24.19.0";

/// 解析接缝的产出:当前选定的 Node。
///
/// 本接缝也承担 v2 清单的来源标签(`crate::prefix::NodeSourceKind`,三值,
/// 不存在 prefix 值——Node 永不落前缀,见 ADR-0003)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSource {
    /// node / node.exe 可执行文件的绝对路径
    exe: PathBuf,
    /// Node 版本(如 `v24.19.0`)
    version: String,
    /// 来源标签(v2 清单落账用)
    kind: NodeSourceKind,
}

impl NodeSource {
    /// node / node.exe 可执行文件的绝对路径
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Node 版本
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 来源标签(v2 三值;doctor 按来源报告「复用用户裸 Node」等)
    pub fn kind(&self) -> NodeSourceKind {
        self.kind
    }

    /// node 可执行文件所在目录(用于把选定 Node 前置进子进程 PATH)
    pub fn bin_dir(&self) -> &Path {
        // exe = <bin_dir>/node[.exe],父目录即 bin 目录
        self.exe.parent().expect("node exe 必有父目录")
    }

    /// Node 解压根目录(npm-cli.js 相对它定位)。
    /// 前缀内 Node 与裸 Node 同为 Node 发行版布局,规则一致
    /// (exe 位于 `<根>/bin/node`,Windows 位于 `<根>/node.exe`)。
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

/// 保底来源:前缀内 Node + 固定 LTS 版本(下载路径用;#22 随 fnm 安装接线后移除)
fn prefix_node(prefix: &Prefix) -> NodeSource {
    NodeSource {
        exe: prefix.node_exe(),
        version: NODE_VERSION.to_string(),
        // 前缀内 Node 不落 v2 清单(清单里不存在 prefix 来源),仅占位
        kind: NodeSourceKind::UserBare,
    }
}

/// 按决策结果解析选定 Node:复用裸 Node → 指向用户机器上的绝对路径;
/// 其余方案(尚未实现,#20/#22)→ 保底前缀内 Node。
pub fn for_plan(prefix: &Prefix, plan: &NodePlan) -> NodeSource {
    match plan {
        NodePlan::ReuseBareNode { path, .. } => {
            let version = platform::version_output_of(path).unwrap_or_else(|| "unknown".into());
            NodeSource {
                exe: path.clone(),
                version,
                kind: NodeSourceKind::UserBare,
            }
        }
        _ => prefix_node(prefix),
    }
}

/// 按 v2 清单记录的 Node 落账解析选定 Node(doctor 等只读命令用)。
/// 清单无 Node 记录或缺 exe 路径时回退保底前缀内 Node。
pub fn from_state(prefix: &Prefix, state: &crate::prefix::State) -> NodeSource {
    match &state.node {
        Some(node) if node.exe.is_some() => NodeSource {
            exe: node.exe.clone().expect("已判定 is_some"),
            version: node.version.clone(),
            kind: node.source,
        },
        _ => prefix_node(prefix),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    #[test]
    fn prefix_fallback_resolves_prefix_node_with_fixed_lts() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let node = prefix_node(&prefix);
        assert_eq!(node.exe(), prefix.node_exe());
        assert_eq!(node.version(), NODE_VERSION);
    }

    #[test]
    fn derived_paths_match_prefix_layout() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let node = prefix_node(&prefix);
        assert_eq!(node.bin_dir(), prefix.node_bin_dir());
        assert_eq!(node.node_dir(), prefix.node_dir());
        assert_eq!(
            node.npm_cli(),
            prefix.node_dir().join(platform::npm_cli_subpath())
        );
    }

    /// 决策 → NodeSource 接线:复用裸 Node 指向用户机器上的绝对路径
    #[test]
    fn reuse_bare_node_points_at_user_exe() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let plan = NodePlan::ReuseBareNode {
            path: PathBuf::from("/usr/local/bin/node"),
            version: Version::new(22, 19, 0),
        };
        let node = for_plan(&prefix, &plan);
        assert_eq!(node.exe(), Path::new("/usr/local/bin/node"));
        assert_eq!(node.kind(), NodeSourceKind::UserBare);
        // bin/解压根/npm-cli 按 Node 发行版布局推导(本机该路径不存在 → version unknown)
        assert_eq!(node.bin_dir(), Path::new("/usr/local/bin"));
        assert_eq!(node.node_dir(), Path::new("/usr/local"));
        assert_eq!(
            node.npm_cli(),
            Path::new("/usr/local").join(platform::npm_cli_subpath())
        );
    }

    #[test]
    fn non_reuse_plans_fall_back_to_prefix_node() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        for plan in [
            NodePlan::UseNvm {
                path: PathBuf::from("/home/u/.nvm"),
                version: Version::new(22, 19, 0),
            },
            NodePlan::UseFnm {
                path: PathBuf::from("/home/u/.local/share/fnm"),
                version: Version::new(22, 19, 0),
            },
            NodePlan::InstallFnm {
                version: Version::new(22, 19, 0),
            },
        ] {
            assert_eq!(
                for_plan(&prefix, &plan),
                prefix_node(&prefix),
                "{plan:?} 尚未实现,应保底前缀内 Node"
            );
        }
    }

    #[test]
    fn from_state_prefers_recorded_reused_node() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let mut state = crate::prefix::State::default();
        // 无记录 → 保底
        assert_eq!(from_state(&prefix, &state), prefix_node(&prefix));
        // 有记录(user-bare + exe)→ 按落账解析
        state.node = Some(crate::prefix::NodeState {
            source: NodeSourceKind::UserBare,
            version: "v24.19.0".into(),
            exe: Some(PathBuf::from("/opt/node/bin/node")),
        });
        let node = from_state(&prefix, &state);
        assert_eq!(node.exe(), Path::new("/opt/node/bin/node"));
        assert_eq!(node.version(), "v24.19.0");
        assert_eq!(node.kind(), NodeSourceKind::UserBare);
    }
}
