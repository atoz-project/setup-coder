//! Node 来源解析接缝(工单 #15):所有需要 Node 运行时的调用点统一经此获取
//! 「当前选定的 Node」,不再各自硬编码前缀内路径。
//!
//! 工单 #19 起接缝按 NodePlan 分叉:复用裸 Node → 指向用户机器上的绝对路径;
//! 前缀内新装 LTS → 前缀布局(保底路径,#22 随 fnm 安装接线后随状态迁移移除)。
//!
//! 契约(工单 #21):shim 以本接缝给出的选定 Node 绝对路径直接 exec Tool 入口 JS,
//! 不再把任何 node 目录前置进 PATH(`platform::shim_content` / `write_shim` 的入参
//! 即 `NodeSource::exe`)。

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
    /// node / node.exe 可执行文件的绝对路径(已解析 symlink 的「实体」路径)
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

    /// 解析出 npm-cli.js / shim 能依托的「实体」exe:symlink 解析到真实 Node 本体。
    ///
    /// 关键性(工单 #21):bin_dir()/node_dir()/npm_cli() 全部由 exe 逐级上推目录。
    /// 若 exe 是 PATH 上的 symlink(Homebrew、用户自建软链),直接上推会穿过安装根,
    /// 得到错误前缀(如 `/tmp/…/bin/node` → 根 `/tmp/…`,npm-cli 落在 `/tmp/…/lib/…`)。
    /// canonicalize 跟随 symlink 拿到真实发行版内的 exe(如 nvm 的
    /// `…/versions/node/vX/bin/node`),保证 node_dir/npm_cli 正确。非 symlink 路径原样。
    fn resolved_exe(exe: &Path) -> PathBuf {
        std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf())
    }

    /// 以「实体化」的 exe 构造 NodeSource(复用裸 Node / 从清单恢复共用)
    fn from_exe(exe: PathBuf, version: String, kind: NodeSourceKind) -> NodeSource {
        NodeSource {
            exe: Self::resolved_exe(&exe),
            version,
            kind,
        }
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

/// 管理器来源:指向管理器安装目录下已解析的 node exe(版本已验证为已装达标)。
/// 与复用裸 Node 同走 `from_exe` 实体化(canonicalize):npm_cli 由 exe 逐级上推,
/// 管理器布局若含 symlink(如 nvm-windows 的 junction)也落在真实发行版布局内。
fn managed_node(exe: PathBuf, floor: &semver::Version, kind: NodeSourceKind) -> NodeSource {
    NodeSource::from_exe(exe, format!("v{floor}"), kind)
}

/// 按决策结果解析选定 Node:复用裸 Node / 管理器(nvm/fnm)装好的 Node →
/// 指向用户机器上的绝对路径;其余方案(尚未实现,#22)→ 保底前缀内 Node。
pub fn for_plan(prefix: &Prefix, plan: &NodePlan) -> NodeSource {
    match plan {
        NodePlan::ReuseBareNode { path, .. } => {
            let version = platform::version_output_of(path).unwrap_or_else(|| "unknown".into());
            NodeSource::from_exe(path.clone(), version, NodeSourceKind::UserBare)
        }
        // UseNvm/UseFnm:执行层(decide_node_with)已装好并验证过该版本,这里只做布局推导
        NodePlan::UseNvm { path, version } => managed_node(
            platform::resolve_manager_node(path, &version.to_string(), platform::ManagerKind::Nvm)
                .expect("UseNvm 执行后管理器内必有该版本 Node"),
            version,
            NodeSourceKind::UserNvm,
        ),
        NodePlan::UseFnm { path, version } => managed_node(
            platform::resolve_manager_node(path, &version.to_string(), platform::ManagerKind::Fnm)
                .expect("UseFnm 执行后管理器内必有该版本 Node"),
            version,
            NodeSourceKind::UserFnm,
        ),
        _ => prefix_node(prefix),
    }
}

/// 按 v2 清单记录的 Node 落账解析选定 Node(doctor 等只读命令用)。
/// 清单无 Node 记录或缺 exe 路径时回退保底前缀内 Node。
pub fn from_state(prefix: &Prefix, state: &crate::prefix::State) -> NodeSource {
    match &state.node {
        Some(node) if node.exe.is_some() => NodeSource::from_exe(
            node.exe.clone().expect("已判定 is_some"),
            node.version.clone(),
            node.source,
        ),
        _ => prefix_node(prefix),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;
    use std::fs;

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

    /// 决策 → NodeSource 接线:复用裸 Node 解析 symlink 到「实体」exe,
    /// bin_dir/node_dir/npm_cli 全部相对该实体推导(不穿过安装根)。
    #[test]
    fn reuse_bare_node_points_at_resolved_exe() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let plan = NodePlan::ReuseBareNode {
            path: PathBuf::from("/usr/local/bin/node"),
            version: Version::new(22, 19, 0),
        };
        let node = for_plan(&prefix, &plan);
        // exe = canonicalize(计划路径);若该路径不存在(如本测试在 unix 上造的路径),
        // canonicalize 失败则原样保留。无论哪条,后续派生都自洽。
        let expected_exe = NodeSource::resolved_exe(Path::new("/usr/local/bin/node"));
        assert_eq!(node.exe(), expected_exe);
        assert_eq!(node.kind(), NodeSourceKind::UserBare);
        assert_eq!(node.bin_dir(), expected_exe.parent().unwrap());
        assert_eq!(
            node.npm_cli(),
            node.node_dir().join(platform::npm_cli_subpath())
        );
    }

    /// 决策 → NodeSource 接线(工单 #20):UseNvm/UseFnm 指向管理器目录下
    /// 已装版本的 node exe 绝对路径;kind 分别为 user_nvm / user_fnm。
    /// 用桩 exe(sh 脚本)占位,不依赖真实 nvm/fnm。
    #[cfg(unix)]
    /// symlink 实体化:PATH 上的软链 node(如 Homebrew/自建)解析到真实发行版内的 exe,
    /// 派生的 node_dir/npm_cli 落在真实发行版布局内而非软链所在目录的上级。
    #[cfg(unix)]
    #[test]
    fn reuse_resolves_symlink_to_real_node_root() {
        use std::os::unix::fs::symlink;
        // 造一个假的 Node 发行版布局:<root>/bin/node + <root>/lib/node_modules/npm/bin/npm-cli.js
        let dir = std::env::temp_dir().join(format!("t7-node-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let real_root = dir.join("real-node");
        let real_bin = real_root.join("bin");
        std::fs::create_dir_all(real_root.join("lib/node_modules/npm/bin")).unwrap();
        std::fs::create_dir_all(&real_bin).unwrap();
        std::fs::write(real_bin.join("node"), "fake").unwrap();
        std::fs::write(
            real_root.join("lib/node_modules/npm/bin/npm-cli.js"),
            "fake",
        )
        .unwrap();
        // 在别处建一个软链指向它(模拟 PATH 上的软链 node)
        let link_dir = dir.join("links");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("node");
        symlink(real_bin.join("node"), &link).unwrap();

        let prefix = Prefix::new(dir.join("prefix"));
        let node = for_plan(
            &prefix,
            &NodePlan::ReuseBareNode {
                path: link.clone(),
                version: Version::new(22, 19, 0),
            },
        );
        // exe 解析到真实发行版内的 node;npm_cli 落在真实发行版布局内
        let resolved = NodeSource::resolved_exe(&link);
        assert_eq!(node.exe(), resolved);
        assert!(node.npm_cli().exists(), "npm_cli 应解析到真实存在路径");
        // 期望路径同样经 canonicalize(macOS /var→/private/var),再比较
        let expected =
            NodeSource::resolved_exe(&real_root.join("lib/node_modules/npm/bin/npm-cli.js"));
        assert_eq!(node.npm_cli(), expected);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 选定运行时的 npm-cli.js 解析:prefix / 裸 Node / nvm / fnm 四种布局的
    /// exe 与 node_dir 均为同一 Node 发行版相对布局(unix `<根>/bin/node`),
    /// 故 `lib/node_modules/npm/bin/npm-cli.js` 的相对推导对四者一致成立。
    #[test]
    fn npm_cli_resolves_per_runtime_layout() {
        for (exe, expected_npm_cli) in [
            // 前缀保底 Node
            (
                "/x/.setup-coder/node/bin/node",
                "/x/.setup-coder/node/lib/node_modules/npm/bin/npm-cli.js",
            ),
            // 裸 Node(发行版布局)
            (
                "/usr/local/bin/node",
                "/usr/local/lib/node_modules/npm/bin/npm-cli.js",
            ),
            // nvm:`~/.nvm/versions/node/vX/`
            (
                "/home/u/.nvm/versions/node/v22.19.0/bin/node",
                "/home/u/.nvm/versions/node/v22.19.0/lib/node_modules/npm/bin/npm-cli.js",
            ),
            // fnm:`~/.local/share/fnm/node-versions/vX/installation/`
            (
                "/home/u/.local/share/fnm/node-versions/v22.19.0/installation/bin/node",
                "/home/u/.local/share/fnm/node-versions/v22.19.0/installation/lib/node_modules/npm/bin/npm-cli.js",
            ),
        ] {
            let source = NodeSource {
                exe: PathBuf::from(exe),
                version: "v22.19.0".into(),
                kind: NodeSourceKind::UserBare,
            };
            assert_eq!(source.npm_cli(), Path::new(expected_npm_cli), "exe={exe}");
        }
    }
    #[test]
    fn manager_plans_resolve_to_installed_node() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "setup-coder-test-mgr-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let version = Version::new(22, 19, 0);
        // nvm 布局:<dir>/versions/node/v22.19.0/bin/node;fnm 布局:node-versions/v22.19.0/installation/bin/node
        let nvm_dir = root.join("nvm");
        let fnm_dir = root.join("fnm");
        let nvm_exe = nvm_dir
            .join("versions/node/v22.19.0/bin/node");
        let fnm_exe = fnm_dir.join("node-versions/v22.19.0/installation/bin/node");
        for exe in [&nvm_exe, &fnm_exe] {
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(exe, "#!/bin/sh\necho 'v22.19.0'\n").unwrap();
            fs::set_permissions(exe, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let nvm = for_plan(
            &prefix,
            &NodePlan::UseNvm {
                path: nvm_dir.clone(),
                version: version.clone(),
            },
        );
        assert_eq!(nvm.exe(), NodeSource::resolved_exe(&nvm_exe));
        assert_eq!(nvm.version(), "v22.19.0");
        assert_eq!(nvm.kind(), NodeSourceKind::UserNvm);

        let fnm = for_plan(
            &prefix,
            &NodePlan::UseFnm {
                path: fnm_dir.clone(),
                version,
            },
        );
        assert_eq!(fnm.exe(), NodeSource::resolved_exe(&fnm_exe));
        assert_eq!(fnm.version(), "v22.19.0");
        assert_eq!(fnm.kind(), NodeSourceKind::UserFnm);
        fs::remove_dir_all(&root).unwrap();
    }

    /// InstallFnm(#22)尚未实现 → 保底前缀内 Node
    #[test]
    fn install_fnm_falls_back_to_prefix_node() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let plan = NodePlan::InstallFnm {
            version: Version::new(22, 19, 0),
        };
        assert_eq!(for_plan(&prefix, &plan), prefix_node(&prefix));
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
