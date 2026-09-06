//! Node 来源解析接缝(工单 #15):所有需要 Node 运行时的调用点统一经此获取
//! 「当前选定的 Node」,不再各自硬编码前缀内路径。
//!
//! 工单 #19 起接缝按 NodePlan 分叉:复用裸 Node → 指向用户机器上的绝对路径;
//! 管理器方案(nvm/fnm)→ 管理器布局下已装版本的 exe。Node 永不落前缀(ADR-0003,
//! 工单 #22):全部四种方案都解析到用户机器上的 Node,不存在前缀保底路径。
//!
//! 契约(工单 #21):JS 入口的 shim 以本接缝给出的选定 Node 绝对路径解释执行,
//! 不再把任何 node 目录前置进 PATH(`platform::shim_content` / `write_shim` 的入参
//! 即 `NodeSource::exe`)。

use std::path::{Path, PathBuf};

use crate::node_plan::NodePlan;
use crate::platform;
use crate::prefix::NodeSourceKind;

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
        let canon = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
        // Windows:canonicalize 产出 \\?\ verbatim 路径,cmd.exe 不认(.cmd shim
        // 冒烟必「找不到路径」,实机 exit 3)——剥回常规盘符路径;unix 原样
        #[cfg(windows)]
        {
            platform::simplify_verbatim_path(&canon)
        }
        #[cfg(unix)]
        {
            canon
        }
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
    ///
    /// 布局:unix 的 exe 位于 `<根>/bin/node`(bin 上级即根);Windows 的 exe 直接
    /// 在解压根(`<根>/node.exe`)——fnm 的 `…/installation/node.exe`、官方 zip、
    /// nvm-windows 版本目录同属此形态,bin_dir 即根。v0.2.0 曾无条件多上一级,
    /// Windows 实机把 installation 的父目录当根,npm-cli 探针必炸(F1 后续)。
    pub fn node_dir(&self) -> &Path {
        node_root_from_bin_dir(self.bin_dir(), cfg!(windows))
    }

    /// 选定 Node 自带的 npm-cli.js 绝对路径(布局因平台而异)
    pub fn npm_cli(&self) -> PathBuf {
        self.node_dir().join(platform::npm_cli_subpath())
    }
}

/// 由 node 所在 bin 目录推导 Node 解压根(纯逻辑,跨平台两种形态可单测):
/// Windows 的 exe 直接在解压根(bin_dir 即根);unix 的 exe 在 `<根>/bin/`
/// (bin 上级为根)。与 `platform::node_bin_subdir_for` 同一 per-OS 约定。
fn node_root_from_bin_dir(bin_dir: &Path, windows: bool) -> &Path {
    if windows {
        bin_dir
    } else {
        bin_dir.parent().expect("node bin 目录必有父目录")
    }
}

/// 管理器来源:指向管理器安装目录下已解析的 node exe(版本已验证为已装达标)。
/// 与复用裸 Node 同走 `from_exe` 实体化(canonicalize):npm_cli 由 exe 逐级上推,
/// 管理器布局若含 symlink(如 nvm-windows 的 junction)也落在真实发行版布局内。
fn managed_node(exe: PathBuf, floor: &semver::Version, kind: NodeSourceKind) -> NodeSource {
    NodeSource::from_exe(exe, format!("v{floor}"), kind)
}

/// 按决策结果解析选定 Node:四种方案都指向用户机器上的绝对路径。
/// 管理器方案(UseNvm/UseFnm/InstallFnm)要求执行层已装好并验证过该版本,
/// 此处只做布局推导;解析不到 = 执行层与布局推导不一致,返回中文错误。
pub fn for_plan(plan: &NodePlan) -> Result<NodeSource, String> {
    Ok(match plan {
        NodePlan::ReuseBareNode { path, .. } => {
            let version = platform::version_output_of(path).unwrap_or_else(|| "unknown".into());
            NodeSource::from_exe(path.clone(), version, NodeSourceKind::UserBare)
        }
        NodePlan::UseNvm { path, version } => managed_node(
            resolve_installed(path, version, platform::ManagerKind::Nvm)?,
            version,
            NodeSourceKind::UserNvm,
        ),
        NodePlan::UseFnm { path, version } => managed_node(
            resolve_installed(path, version, platform::ManagerKind::Fnm)?,
            version,
            NodeSourceKind::UserFnm,
        ),
        // InstallFnm(工单 #22):执行层已把 fnm 装到平台默认数据目录并装好下限版本,
        // 经同一份管理器布局推导解析(落账来源同为 user_fnm)
        NodePlan::InstallFnm { version } => managed_node(
            resolve_installed(&fnm_default_home()?, version, platform::ManagerKind::Fnm)?,
            version,
            NodeSourceKind::UserFnm,
        ),
    })
}

/// 在执行层已装好指定版本的管理器目录下解析 node exe;解析不到 = 内部不一致,中文报错
fn resolve_installed(
    dir: &Path,
    version: &semver::Version,
    kind: platform::ManagerKind,
) -> Result<PathBuf, String> {
    platform::resolve_manager_node(dir, &version.to_string(), kind)
        .ok_or_else(|| format!("选定管理器({})下未解析到 Node.js v{version}", dir.display()))
}

/// 当前用户家目录下的 fnm 默认数据目录(InstallFnm 的安装目标,与探测一致)。
/// Windows 为 `%APPDATA%\fnm`(fnm 真实默认,Roaming;FNM_DIR 优先),unix 为 `~/.local/share/fnm`。
/// uninstall 的「代装 fnm 手工移除指引」复用同一路径(工单 #23)。
pub(crate) fn fnm_default_home() -> Result<PathBuf, String> {
    let home = std::env::home_dir().ok_or_else(|| "无法确定用户家目录(HOME 未设置)".to_string())?;
    Ok(platform::fnm_default_dir_impl(&home))
}

/// 按 v2 清单记录的 Node 落账解析选定 Node(doctor 等只读命令用)。
/// 清单无 Node 记录或缺 exe 路径(尚未完成安装)时返回 None,由调用方按「未安装」处理。
pub fn from_state(state: &crate::prefix::State) -> Option<NodeSource> {
    let node = state.node.as_ref()?;
    Some(NodeSource::from_exe(
        node.exe.clone()?,
        node.version.clone(),
        node.source,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;
    use std::fs;

    /// 决策 → NodeSource 接线:复用裸 Node 解析 symlink 到「实体」exe,
    /// bin_dir/node_dir/npm_cli 全部相对该实体推导(不穿过安装根)。
    #[test]
    fn reuse_bare_node_points_at_resolved_exe() {
        let plan = NodePlan::ReuseBareNode {
            path: PathBuf::from("/usr/local/bin/node"),
            version: Version::new(22, 19, 0),
        };
        let node = for_plan(&plan).unwrap();
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

        let node = for_plan(&NodePlan::ReuseBareNode {
            path: link.clone(),
            version: Version::new(22, 19, 0),
        })
        .unwrap();
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
    /// 选定运行时的 npm-cli.js 解析:裸 Node / nvm / fnm 布局的 exe 与 node_dir
    /// 均为同一 Node 发行版相对布局(unix `<根>/bin/node`),故
    /// `lib/node_modules/npm/bin/npm-cli.js` 的相对推导对三者一致成立。
    #[test]
    fn npm_cli_resolves_per_runtime_layout() {
        for (exe, expected_npm_cli) in [
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

    /// node_dir/npm_cli(F1 后续,Windows 实机):exe 直接在解压根的布局
    /// (fnm `…/installation/node.exe`、官方 zip 根、nvm-windows 版本目录)下
    /// bin_dir 即解压根——v0.2.0 无条件多上一级,npm-cli 探针指到
    /// `installation` 的父目录,Windows 实机必「选定 Node 自带 npm 不存在」。
    #[test]
    fn node_dir_and_npm_cli_cover_windows_rootless_layouts() {
        use platform::npm_cli_subpath_for;
        // unix 布局不变:<根>/bin → 根;<根>/lib/node_modules/npm/bin/npm-cli.js
        assert_eq!(
            node_root_from_bin_dir(Path::new("/x/node/bin"), false),
            Path::new("/x/node")
        );
        assert_eq!(
            node_root_from_bin_dir(Path::new("/x/node/bin"), false)
                .join(npm_cli_subpath_for(false)),
            Path::new("/x/node/lib/node_modules/npm/bin/npm-cli.js")
        );
        // windows 布局:bin_dir 即根(fnm installation 同形态;实机路径形态)
        let inst = Path::new(r"C:\Users\u\AppData\Roaming\fnm\node-versions\v22.19.0\installation");
        assert_eq!(node_root_from_bin_dir(inst, true), inst);
        assert_eq!(
            node_root_from_bin_dir(inst, true).join(npm_cli_subpath_for(true)),
            inst.join("node_modules").join("npm").join("bin").join("npm-cli.js")
        );
        // 官方 zip / msi 根、nvm-windows 版本目录同规则
        let zip_root = Path::new(r"C:\Program Files\nodejs");
        assert_eq!(node_root_from_bin_dir(zip_root, true), zip_root);
        let nvm_v = Path::new(r"C:\Users\u\AppData\Roaming\nvm\v22.19.0");
        assert_eq!(node_root_from_bin_dir(nvm_v, true), nvm_v);
    }

    #[cfg(unix)]
    #[test]
    fn manager_plans_resolve_to_installed_node() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("setup-coder-test-mgr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let version = Version::new(22, 19, 0);
        // nvm 布局:<dir>/versions/node/v22.19.0/bin/node;fnm 布局:node-versions/v22.19.0/installation/bin/node
        let nvm_dir = root.join("nvm");
        let fnm_dir = root.join("fnm");
        let nvm_exe = nvm_dir.join("versions/node/v22.19.0/bin/node");
        let fnm_exe = fnm_dir.join("node-versions/v22.19.0/installation/bin/node");
        for exe in [&nvm_exe, &fnm_exe] {
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(exe, "#!/bin/sh\necho 'v22.19.0'\n").unwrap();
            fs::set_permissions(exe, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let nvm = for_plan(&NodePlan::UseNvm {
            path: nvm_dir.clone(),
            version: version.clone(),
        })
        .unwrap();
        assert_eq!(nvm.exe(), NodeSource::resolved_exe(&nvm_exe));
        assert_eq!(nvm.version(), "v22.19.0");
        assert_eq!(nvm.kind(), NodeSourceKind::UserNvm);

        let fnm = for_plan(&NodePlan::UseFnm {
            path: fnm_dir.clone(),
            version,
        })
        .unwrap();
        assert_eq!(fnm.exe(), NodeSource::resolved_exe(&fnm_exe));
        assert_eq!(fnm.version(), "v22.19.0");
        assert_eq!(fnm.kind(), NodeSourceKind::UserFnm);
        fs::remove_dir_all(&root).unwrap();
    }

    /// InstallFnm(工单 #22):解析家目录下 fnm 默认数据目录里已装的下限版本,
    /// 来源落账 user_fnm(与 UseFnm 同一份管理器布局推导,经 from_exe 实体化)。
    /// 用桩 fnm 数据目录(HOME 指到临时根)+ 桩 node exe,不依赖真实 fnm。
    #[cfg(unix)]
    #[test]
    fn install_fnm_resolves_fnm_default_dir_node() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "setup-coder-test-installfnm-src-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let version = Version::new(22, 19, 0);
        // 桩:平台默认 fnm 数据目录(unix ~/.local/share/fnm)下已装达标版本
        let stub_exe = root.join(format!(
            ".local/share/fnm/node-versions/v{version}/installation/bin/node"
        ));
        fs::create_dir_all(stub_exe.parent().unwrap()).unwrap();
        fs::write(&stub_exe, format!("#!/bin/sh\necho 'v{version}'\n")).unwrap();
        fs::set_permissions(&stub_exe, fs::Permissions::from_mode(0o755)).unwrap();

        let _home = crate::test_util::ScopedHome::set(&root);
        let node = for_plan(&NodePlan::InstallFnm {
            version: version.clone(),
        })
        .unwrap();
        let expected_exe = NodeSource::resolved_exe(&stub_exe);
        assert_eq!(node.exe(), expected_exe);
        assert_eq!(node.version(), format!("v{version}"));
        assert_eq!(node.kind(), NodeSourceKind::UserFnm);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn from_state_prefers_recorded_reused_node() {
        let mut state = crate::prefix::State::default();
        // 无记录 → 尚未完成安装,None(由调用方按「未安装」处理)
        assert_eq!(from_state(&state), None);
        // 有记录(user-bare + exe)→ 按落账解析
        state.node = Some(crate::prefix::NodeState {
            source: NodeSourceKind::UserBare,
            version: "v24.19.0".into(),
            exe: Some(PathBuf::from("/opt/node/bin/node")),
        });
        let node = from_state(&state).expect("有落账记录应解析出 Node");
        assert_eq!(node.exe(), Path::new("/opt/node/bin/node"));
        assert_eq!(node.version(), "v24.19.0");
        assert_eq!(node.kind(), NodeSourceKind::UserBare);
        // 记录缺 exe 路径(管理器来源缺落账)→ 同样 None
        state.node = Some(crate::prefix::NodeState {
            source: NodeSourceKind::UserFnm,
            version: "v24.19.0".into(),
            exe: None,
        });
        assert_eq!(from_state(&state), None);
    }
}
