//! `install` 子命令:选定 Node(决策)→ 装 Tool 进 Private Prefix,零输入完成。
//!
//! 流水线:建前缀骨架 → 探测 Node 事实 → 纯决策 → 按方案执行
//! (达标裸 Node 复用 #19;经已有 nvm/fnm 装下限版本 #20;无 Node 无管理器则新装 fnm
//! 兜底 #22)→ 确保 git(Prerequisite,工单 #3)→ 复制 setup-coder 本体 → npm 装 Tool
//! (注册表)→ 生成 shim(绝对路径 exec 选定 Node,无 PATH 前置,工单 #21)→ 冒烟
//! (`--version`,Installed 定义)→ 注入 PATH → 写 state.json。重跑 = 修复/升级,幂等。

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::node_plan::{self, NodeFacts, NodePlan};
use crate::node_source::{self, NodeSource};
use crate::platform;
use crate::prefix::{NodeSourceKind, NodeState, Prefix, State, ToolState};
use crate::registry::{self, Tool};

/// Tool 安装的 npm registry(npmmirror)
const NPM_REGISTRY: &str = "https://registry.npmmirror.com";

/// 前缀内 npmrc 内容:registry 只写在前缀里,不碰用户 ~/.npmrc(ADR-0002)
const NPMRC_CONTENT: &str = "registry=https://registry.npmmirror.com\n";

pub fn run(tool: Option<String>) {
    if let Err(e) = install(tool.as_deref()) {
        eprintln!("安装失败:{e}");
        std::process::exit(1);
    }
}

fn install(tool: Option<&str>) -> Result<(), Box<dyn Error>> {
    let tools = resolve_tools(tool)?;
    let prefix = Prefix::home()?;
    let mut state = State::load(&prefix)?;

    println!("安装进 Private Prefix:{}", prefix.root().display());
    prefix.create_skeleton()?;

    // 决策点解析出选定 Node:达标裸 Node 复用、经已有 nvm/fnm、或新装 fnm 兜底;
    // 全部指向用户机器上的绝对路径,Node 不落前缀(ADR-0003,无前缀 node/ 目录)
    let node = decide_node(&tools, &mut state)?;
    ensure_git(&prefix)?;
    install_setup_coder_self(&prefix)?;
    write_npmrc(&prefix)?;
    let mut installed = Vec::new();
    for t in tools {
        install_tool(&prefix, &node, t, &mut installed)?;
    }
    // upsert:单装一个 Tool 不得抹掉其他 Tool 的清单记录(幂等 = 修复/升级)
    upsert_tools(&mut state.tools, &installed);

    let injections = platform::ensure_path(&prefix.bin_dir())?;
    for injection in injections {
        state.record_injection(injection);
    }

    state.save(&prefix)?;
    print_summary(&prefix, &installed);
    Ok(())
}

/// 把本次装好的 Tool 记录并入清单:同名覆盖(升级),新名追加。
fn upsert_tools(tools: &mut Vec<ToolState>, installed: &[ToolState]) {
    for new in installed {
        match tools.iter_mut().find(|old| old.name == new.name) {
            Some(old) => *old = new.clone(),
            None => tools.push(new.clone()),
        }
    }
}

/// 解析命令行参数:不带 = 全部注册表 Tool;带 = 指定一个
fn resolve_tools(tool: Option<&str>) -> Result<Vec<&'static Tool>, Box<dyn Error>> {
    match tool {
        None => Ok(registry::all().iter().collect()),
        Some(name) => {
            let t = registry::find(name).ok_or_else(|| {
                let known = registry::all()
                    .iter()
                    .map(|t| t.name)
                    .collect::<Vec<_>>()
                    .join("、");
                format!("未知 Tool「{name}」,目前支持:{known}")
            })?;
            Ok(vec![t])
        }
    }
}

/// Node 决策点(工单 #19/#20/#22):探测机器事实 → 纯决策 → 按方案执行并落账。
///
/// 方案:达标裸 Node 复用(#19)、经已有 nvm/fnm 装下限版本(#20)、
/// 无 Node 无管理器时新装 fnm 兜底(#22)。不下载 Node tarball、不创建前缀 node/
/// 目录;Tool 安装/shim/冒烟全部用选定 Node 的绝对路径。
/// 返回经接缝解析出的选定 Node(= 用户机器上的绝对路径)。
fn decide_node(tools: &[&Tool], state: &mut State) -> Result<NodeSource, Box<dyn Error>> {
    decide_node_with(tools, &platform::detect_node_facts(), state)
}

/// 决策/执行纯接线:facts 由参数注入(IO 在 detect_node_facts),可单测。
fn decide_node_with(
    tools: &[&Tool],
    facts: &NodeFacts,
    state: &mut State,
) -> Result<NodeSource, Box<dyn Error>> {
    match node_plan::decide(facts, tools) {
        NodePlan::ReuseBareNode {
            ref path,
            version: floor,
        } => {
            println!("复用已有 Node.js:{}(所需下限 v{floor})", path.display());
            let node = node_source::for_plan(&NodePlan::ReuseBareNode {
                path: path.clone(),
                version: floor,
            })?;
            // 落账(state v2):复用来源 = user_bare + 解析出的版本 + exe 绝对路径
            state.node = Some(NodeState {
                source: NodeSourceKind::UserBare,
                version: node.version().to_string(),
                exe: Some(node.exe().to_path_buf()),
            });
            Ok(node)
        }
        NodePlan::UseNvm {
            ref path,
            version: floor,
        } => use_nvm(path, &floor, state),
        NodePlan::UseFnm {
            ref path,
            version: floor,
        } => use_fnm(path, &floor, state),
        NodePlan::InstallFnm { version: floor } => install_fnm_branch(&floor, state),
    }
}

/// 经已有 nvm 装/复用下限版本 Node(工单 #20)。
///
/// 幂等:管理器内已有该版本则直接复用;否则 `nvm install <floor>`(nvm 是 shell
/// 函数,platform 层 source nvm.sh 执行)。绝不改用户的 nvm default alias(不劫持)。
/// fnm 若同时存在,完全不触碰。落账 user_nvm + 解析出的 exe 绝对路径。
fn use_nvm(
    nvm_dir: &Path,
    floor: &semver::Version,
    state: &mut State,
) -> Result<NodeSource, Box<dyn Error>> {
    let spec = floor.to_string();
    if let Some(exe) = platform::resolve_manager_node(nvm_dir, &spec, platform::ManagerKind::Nvm) {
        println!("经 nvm 复用已装 Node.js v{spec}:{}", exe.display());
    } else {
        println!(
            "经 nvm({})安装 Node.js v{spec}(所需下限)…",
            nvm_dir.display()
        );
        let exe = platform::nvm_install_and_resolve(nvm_dir, &spec)?;
        // 自检:解析出的 node 必须真实存在且版本正确(防 nvm which 异常输出)
        if platform::version_output_of(&exe).as_deref() != Some(format!("v{spec}").as_str()) {
            return Err(format!(
                "经 nvm 安装 Node.js v{spec} 后自检失败:{} 不可用或版本不符",
                exe.display()
            )
            .into());
        }
        println!("已用 nvm 安装 Node.js v{spec}:{}", exe.display());
    }
    let node = node_source::for_plan(&NodePlan::UseNvm {
        path: nvm_dir.to_path_buf(),
        version: floor.clone(),
    })?;
    state.node = Some(NodeState {
        source: NodeSourceKind::UserNvm,
        version: node.version().to_string(),
        exe: Some(node.exe().to_path_buf()),
    });
    Ok(node)
}

/// 经已有 fnm 装/复用下限版本 Node(工单 #20)。
///
/// 幂等:管理器内已有该版本则直接复用,不再重跑 fnm(用户自有 fnm 的默认/钩子
/// 归用户自己管,我们不改)。否则 `fnm install <floor>` + `fnm default <floor>`
/// (首次经 fnm 装时设为默认,使该 Node 在用户终端可解析)。
/// 落账 user_fnm + 解析出的 exe 绝对路径。
fn use_fnm(
    fnm_path: &Path,
    floor: &semver::Version,
    state: &mut State,
) -> Result<NodeSource, Box<dyn Error>> {
    let spec = floor.to_string();
    if let Some(exe) = platform::resolve_manager_node(fnm_path, &spec, platform::ManagerKind::Fnm) {
        println!("经 fnm 复用已装 Node.js v{spec}:{}", exe.display());
    } else {
        println!(
            "经 fnm({})安装 Node.js v{spec}(所需下限)…",
            fnm_path.display()
        );
        platform::fnm_install_and_default(&platform::fnm_exe_path(fnm_path), &spec)?;
        println!("已用 fnm 安装 Node.js v{spec} 并设为默认");
    }
    let node = node_source::for_plan(&NodePlan::UseFnm {
        path: fnm_path.to_path_buf(),
        version: floor.clone(),
    })?;
    state.node = Some(NodeState {
        source: NodeSourceKind::UserFnm,
        version: node.version().to_string(),
        exe: Some(node.exe().to_path_buf()),
    });
    Ok(node)
}

/// 新装 fnm 兜底(工单 #22):无 Node 且无版本管理器的零输入路径。
///
/// 步骤:经镜像链下载 fnm 到平台默认数据目录(unix `~/.local/share/fnm`,与探测
/// `detect_node_facts` 同一定义——重跑探测即命中 UseFnm 分支,幂等成立)→
/// `fnm install <floor>` + `fnm default <floor>` → 幂等注入 fnm shell 钩子(记为
/// FnmHook 注入,uninstall 按它精确回滚)→ 落账。
///
/// 落账模型(ADR-0003):虽由 setup-coder 代装,来源仍记 user_fnm(没有也不该有
/// prefix 来源值——Node 永不落前缀);「是否由 setup-coder 代装 fnm」由 state.json
/// 里的 FnmHook rc 注入记录区分,不靠来源枚举。
fn install_fnm_branch(
    floor: &semver::Version,
    state: &mut State,
) -> Result<NodeSource, Box<dyn Error>> {
    let home = std::env::home_dir()
        .ok_or_else(|| io_error("无法确定用户家目录(HOME 未设置),无法安装 fnm"))?;
    let fnm_dir = platform::fnm_default_dir_impl(&home);
    let spec = floor.to_string();

    // 1. 装 fnm 本体(幂等:已能跑则复用;镜像容错链下载)
    let fnm_exe = platform::install_fnm(&prefix_cache_dir()?, &fnm_dir)?;
    println!(
        "已安装 fnm {}:{}",
        fnm_version_of(&fnm_exe),
        fnm_exe.display()
    );

    // 2. 经 fnm 装下限 Node 并设为默认(新装 fnm 必无该版本,直接装)
    println!("经 fnm 安装 Node.js v{spec}(所需下限)…");
    platform::fnm_install_and_default(&fnm_exe, &spec)?;
    println!("已用 fnm 安装 Node.js v{spec} 并设为默认");

    // 3. 幂等注入 fnm shell 钩子(FnmHook 记录,供精确回滚);重跑不重复注入
    for injection in platform::inject_fnm_hook()? {
        if let crate::prefix::PathInjection::FnmHook { file, line } = &injection {
            println!("已向 {} 注入 fnm 钩子:{line}", file.display());
        }
        state.record_injection(injection);
    }
    println!("新开终端即可获得 node/npm(fnm 钩子生效)");

    // 4. 解析 + 落账:与 UseFnm 同一条 resolve_manager_node → from_exe(实体化)路径
    let node = node_source::for_plan(&NodePlan::InstallFnm {
        version: floor.clone(),
    })?;
    state.node = Some(NodeState {
        source: NodeSourceKind::UserFnm,
        version: node.version().to_string(),
        exe: Some(node.exe().to_path_buf()),
    });
    Ok(node)
}

/// 新装 fnm 展示用的版本串(`fnm --version`;解析不到则显示「未知版本」)
fn fnm_version_of(fnm_exe: &Path) -> String {
    platform::version_output_of(fnm_exe)
        .and_then(|out| platform::parse_fnm_version(&out))
        .map(|v| format!("v{v}"))
        .unwrap_or_else(|| "(版本未知)".to_string())
}

/// 前缀下载缓存目录(经 Prefix::home;fnm zip 缓存进前缀,可整删)
fn prefix_cache_dir() -> Result<PathBuf, Box<dyn Error>> {
    Ok(Prefix::home()?.cache_dir())
}

fn io_error(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, msg.to_string())
}

/// Prerequisite:确保 git 可用(工单 #3;平台做法收敛在 platform/)
fn ensure_git(prefix: &Prefix) -> Result<(), Box<dyn Error>> {
    match platform::ensure_git(prefix)? {
        platform::GitOutcome::Skipped => println!("git 已可用,跳过安装"),
        platform::GitOutcome::Installed => println!("git 安装完成"),
    }
    Ok(())
}

/// 把 setup-coder 本体复制进前缀 bin/(布局约定:bin/ 含本体 + shim)
fn install_setup_coder_self(prefix: &Prefix) -> Result<(), Box<dyn Error>> {
    let dest = platform::install_self(&prefix.bin_dir())?;
    println!("setup-coder 本体:{}", dest.display());
    Ok(())
}

/// 写前缀内 npmrc(指向 npmmirror;经 NPM_CONFIG_USERCONFIG 生效)
fn write_npmrc(prefix: &Prefix) -> Result<(), Box<dyn Error>> {
    fs::write(prefix.npmrc(), NPMRC_CONTENT)?;
    Ok(())
}

/// 给子进程准备的 PATH:选定 Node 的 bin 目录在最前(npm 自身子进程/spawn 依赖 env node;
/// Tool 的 shim 走绝对路径,不依赖它——工单 #21)
fn path_with_node(node: &NodeSource) -> Result<OsString, Box<dyn Error>> {
    let mut paths = vec![node.bin_dir().to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    Ok(std::env::join_paths(paths)?)
}

/// npm 可执行入口:选定 Node + 其自带的 npm-cli.js(布局因平台而异)
fn npm_command(
    prefix: &Prefix,
    node: &NodeSource,
    args: &[&str],
) -> Result<Command, Box<dyn Error>> {
    let npm_cli = node.npm_cli();
    if !npm_cli.exists() {
        return Err(format!(
            "选定 Node 自带 npm 不存在:{}(Node 安装不完整)",
            npm_cli.display()
        )
        .into());
    }
    let mut cmd = Command::new(node.exe());
    cmd.arg(npm_cli)
        .args(args)
        .env("PATH", path_with_node(node)?)
        // registry/prefix/cache 全部限定在前缀内,不碰用户全局(ADR-0002)
        .env("NPM_CONFIG_REGISTRY", NPM_REGISTRY)
        .env("NPM_CONFIG_PREFIX", prefix.npm_dir())
        .env("NPM_CONFIG_USERCONFIG", prefix.npmrc())
        .env("NPM_CONFIG_CACHE", prefix.cache_dir().join("npm"));
    Ok(cmd)
}

/// 装一个 Tool:npm install -g → 生成 shim → 冒烟 --version → 返回清单记录
fn install_tool(
    prefix: &Prefix,
    node: &NodeSource,
    tool: &Tool,
    installed: &mut Vec<ToolState>,
) -> Result<(), Box<dyn Error>> {
    println!("安装 {}({})…", tool.name, tool.package);
    let status = npm_command(prefix, node, &["install", "--global", tool.package])?.status()?;
    if !status.success() {
        return Err(format!("npm 安装 {} 失败(退出码 {:?})", tool.package, status.code()).into());
    }

    // 去劫持契约(工单 #21):shim 以选定 Node 的绝对路径 exec Tool 入口,
    // 不再把 node bin 目录前置进 PATH;重跑覆写旧形态 shim。
    let launcher = platform::tool_launcher(&prefix.npm_bin_dir(), tool.bin)?;
    let shim = platform::write_shim(&prefix.bin_dir(), node.exe(), &launcher, tool.bin)?;

    // 冒烟:Installed = 能启动并报出版本号(CONTEXT.md)
    let version = smoke_version(&shim).map_err(|e| {
        format!(
            "{} 安装后冒烟失败(`{} --version` 未通过):{e}",
            tool.name,
            shim.display()
        )
    })?;
    println!("{} 安装完成:{version}", tool.name);

    installed.push(ToolState {
        name: tool.name.to_string(),
        package: tool.package.to_string(),
        version,
    });
    Ok(())
}

/// 跑 `<shim> --version`,返回版本输出
fn smoke_version(shim: &Path) -> Result<String, Box<dyn Error>> {
    let out = Command::new(shim).arg("--version").output()?;
    if !out.status.success() {
        return Err(format!(
            "退出码 {:?}:{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if version.is_empty() {
        return Err("--version 无输出".into());
    }
    Ok(version)
}

fn print_summary(prefix: &Prefix, installed: &[ToolState]) {
    println!();
    println!("全部安装完成。安装清单:{}", prefix.state_path().display());
    for t in installed {
        println!("  {} = {}", t.name, t.version);
    }
    // 平台相关的 PATH 生效提示出自 platform/(分叉只允许在那里)
    println!("{}", platform::path_activation_hint());
    for t in installed {
        let bin = registry::find(&t.name).map(|t| t.bin).unwrap_or(&t.name);
        println!("  {bin} --version");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn resolve_tools_defaults_to_full_registry() {
        let all = resolve_tools(None).unwrap();
        assert_eq!(all.len(), registry::all().len());
    }

    #[test]
    fn resolve_tools_by_name_and_bin() {
        assert_eq!(
            resolve_tools(Some("codex")).unwrap()[0].package,
            "@openai/codex"
        );
        assert_eq!(
            resolve_tools(Some("claude")).unwrap()[0].name,
            "claude-code"
        );
    }

    #[test]
    fn resolve_tools_unknown_lists_supported_names() {
        let err = resolve_tools(Some("cursor")).unwrap_err().to_string();
        assert!(err.contains("未知 Tool"));
        assert!(err.contains("codex") && err.contains("claude-code") && err.contains("pi"));
    }

    #[test]
    fn upsert_tools_merges_without_wiping_others() {
        let mut tools = vec![
            ToolState {
                name: "codex".into(),
                package: "@openai/codex".into(),
                version: "0.1".into(),
            },
            ToolState {
                name: "pi".into(),
                package: "@earendil-works/pi-coding-agent".into(),
                version: "0.1".into(),
            },
        ];
        // 单装 codex(升级):pi 的记录必须保留
        upsert_tools(
            &mut tools,
            &[ToolState {
                name: "codex".into(),
                package: "@openai/codex".into(),
                version: "0.2".into(),
            }],
        );
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].version, "0.2", "同名应覆盖(升级)");
        assert_eq!(tools[1].name, "pi", "其他 Tool 记录不得被抹掉");
        // 新 Tool 追加
        upsert_tools(
            &mut tools,
            &[ToolState {
                name: "claude-code".into(),
                package: "@anthropic-ai/claude-code".into(),
                version: "2.0".into(),
            }],
        );
        assert_eq!(tools.len(), 3);
    }

    /// PATH 构造:选定 Node 的 bin 目录在最前(复用 Node 时 = 用户机器上的目录)
    #[cfg(unix)]
    #[test]
    fn path_with_node_prepends_resolved_bin_dir() {
        let Some(node_path) = which_node() else {
            return; // 极端无 node 的开发机:跳过
        };
        let node = node_source::for_plan(&NodePlan::ReuseBareNode {
            path: node_path,
            version: semver::Version::new(22, 19, 0),
        })
        .unwrap();
        let path = path_with_node(&node).unwrap();
        assert_eq!(std::env::split_paths(&path).next().unwrap(), node.bin_dir());
    }

    /// npm 子进程环境:registry/prefix/userconfig/cache 全部限定在前缀内(ADR-0002),
    /// 不依赖也不改动用户 ~/.npmrc;复用裸 Node 时同样成立(用 #19 复用路径接线验证)
    #[test]
    fn npm_env_scoped_to_prefix_for_reused_node() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let node = node_source::for_plan(&NodePlan::ReuseBareNode {
            // 本机(开发机)必有 node(构建依赖);绝对路径布局 = Node 发行版
            path: std::env::current_exe()
                .ok()
                .and_then(|_| which_node())
                .expect("开发机 PATH 上应有 node"),
            version: semver::Version::new(22, 19, 0),
        })
        .unwrap();
        let cmd = npm_command(&prefix, &node, &["install", "--global", "@openai/codex"]).unwrap();
        // 入口 = 复用 Node 的绝对 exe + 该 Node 自带的 npm-cli.js
        assert_eq!(cmd.get_program(), node.exe().as_os_str());
        assert_eq!(cmd.get_args().next().unwrap(), node.npm_cli().as_os_str());
        let envs: std::collections::HashMap<_, _> = cmd.get_envs().collect();
        assert_eq!(
            envs.get(std::ffi::OsStr::new("NPM_CONFIG_REGISTRY")),
            Some(&Some(std::ffi::OsStr::new(NPM_REGISTRY)))
        );
        for (key, expected) in [
            ("NPM_CONFIG_PREFIX", prefix.npm_dir()),
            ("NPM_CONFIG_USERCONFIG", prefix.npmrc()),
            ("NPM_CONFIG_CACHE", prefix.cache_dir().join("npm")),
        ] {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(key)),
                Some(&Some(expected.as_os_str())),
                "{key} 应限定在前缀内"
            );
        }
        // PATH 首项 = 复用 Node 的 bin 目录(npm 自身子进程的 env node 依赖它)
        let path = envs
            .get(std::ffi::OsStr::new("PATH"))
            .and_then(|v| v.as_ref())
            .expect("npm 子进程必须带 PATH");
        assert_eq!(std::env::split_paths(path).next().unwrap(), node.bin_dir());
    }

    /// 决策接线:达标裸 Node → 复用,落账 user_bare + 版本 + exe 绝对路径;
    /// 版本恰等于下限同样复用;略低于下限(无其他来源)→ 明确中文错误而非下载。
    #[test]
    fn decide_wiring_reuses_compliant_bare_node() {
        let prefix = Prefix::new(PathBuf::from("/x/.setup-coder"));
        let Some(node_path) = which_node() else {
            return; // 极端无 node 的开发机:跳过(探测 IO 属平台层,#18)
        };
        let floor = registry::floor_for_tools(&registry::all().iter().collect::<Vec<_>>());
        let tools: Vec<&Tool> = registry::all().iter().collect();
        let bare = |v: semver::Version| NodeFacts {
            bare_node: Some((v, node_path.clone())),
            nvm: None,
            fnm: None,
        };

        // 恰等于下限 → 复用
        let mut state = State::default();
        let node = decide_node_with(&tools, &bare(floor.clone()), &mut state).unwrap();
        assert_eq!(node.exe(), node_path);
        assert_eq!(node.kind(), NodeSourceKind::UserBare);
        let recorded = state.node.expect("复用应落账 Node 记录");
        assert_eq!(recorded.source, NodeSourceKind::UserBare);
        assert_eq!(recorded.exe.as_deref(), Some(node_path.as_path()));
        assert!(!recorded.version.is_empty(), "落账版本应为解析出的实际版本");
        // 不落前缀:复用路径只记 user_bare 落账,前缀不存在任何 node 布局路径
        assert!(!prefix.root().exists());

        // 高于下限 → 复用(同上)
        let mut state = State::default();
        decide_node_with(&tools, &bare(semver::Version::new(99, 0, 0)), &mut state).unwrap();
    }

    /// 工单 #20:nvm 存在 + 裸 Node 不达标 → 经 nvm 装/复用下限版本,落账 user_nvm + 解析路径。
    /// 幂等:管理器内已有该版本则复用(不重复安装);二次调用同样复用(状态已落账)。
    /// 用桩 nvm 目录(versions/node/v<floor>/bin/node)注入事实,不需要真实 nvm。
    #[cfg(unix)]
    #[test]
    fn decide_wiring_use_nvm_records_user_nvm_and_reuses() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("setup-coder-test-usenvm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let prefix = Prefix::new(root.join(".setup-coder"));
        let tools: Vec<&Tool> = registry::all().iter().collect();
        let floor = registry::floor_for_tools(&tools);
        // 桩:nvm 目录下已装达标版本(幂等复用分支)
        let nvm_dir = root.join(".nvm");
        let stub_exe = nvm_dir.join(format!("versions/node/v{floor}/bin/node"));
        fs::create_dir_all(stub_exe.parent().unwrap()).unwrap();
        fs::write(&stub_exe, format!("#!/bin/sh\necho 'v{floor}'\n")).unwrap();
        fs::set_permissions(&stub_exe, fs::Permissions::from_mode(0o755)).unwrap();

        let facts = NodeFacts {
            bare_node: Some((
                semver::Version::new(0, 0, 1),
                PathBuf::from("/usr/bin/node"),
            )),
            nvm: Some((semver::Version::new(1, 0, 0), nvm_dir.clone())),
            fnm: None,
        };
        let mut state = State::default();
        let node = decide_node_with(&tools, &facts, &mut state).unwrap();
        // 合并 #21 后选定 exe 经 from_exe canonicalize(macOS /var→/private/var 归一)
        let expected_exe = std::fs::canonicalize(&stub_exe).unwrap_or_else(|_| stub_exe.clone());
        assert_eq!(node.exe(), expected_exe.as_path());
        assert_eq!(node.kind(), NodeSourceKind::UserNvm);
        assert_eq!(node.version(), format!("v{floor}"), "选定版本即工具集下限");
        let recorded = state.node.clone().expect("应落账 user_nvm");
        assert_eq!(recorded.source, NodeSourceKind::UserNvm);
        assert_eq!(recorded.exe.as_deref(), Some(expected_exe.as_path()));
        assert_eq!(recorded.version, format!("v{floor}"));

        // 幂等:二次调用(状态已有记录 + 管理器已有版本)仍复用同一路径
        let node2 = decide_node_with(&tools, &facts, &mut state).unwrap();
        assert_eq!(
            node2.exe(),
            expected_exe.as_path(),
            "重跑不得重装,应复用同一 exe"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    /// 工单 #20:无 nvm、有 fnm + 裸 Node 不达标 → 经 fnm 装/复用下限版本,落账 user_fnm。
    /// 幂等复用分支(管理器内已有该版本)不需真实 fnm 二进制。
    #[cfg(unix)]
    #[test]
    fn decide_wiring_use_fnm_records_user_fnm_and_reuses() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("setup-coder-test-fnm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let prefix = Prefix::new(root.join(".setup-coder"));
        let tools: Vec<&Tool> = registry::all().iter().collect();
        let floor = registry::floor_for_tools(&tools);
        // 桩:fnm 数据目录下已装达标版本(幂等复用分支,不触发 fnm install)
        let fnm_dir = root.join("fnm");
        let stub_exe = fnm_dir.join(format!("node-versions/v{floor}/installation/bin/node"));
        fs::create_dir_all(stub_exe.parent().unwrap()).unwrap();
        fs::write(&stub_exe, format!("#!/bin/sh\necho 'v{floor}'\n")).unwrap();
        fs::set_permissions(&stub_exe, fs::Permissions::from_mode(0o755)).unwrap();

        let facts = NodeFacts {
            bare_node: Some((
                semver::Version::new(0, 0, 1),
                PathBuf::from("/usr/bin/node"),
            )),
            nvm: None,
            fnm: Some((semver::Version::new(1, 0, 0), fnm_dir.clone())),
        };
        let mut state = State::default();
        let node = decide_node_with(&tools, &facts, &mut state).unwrap();
        // 合并 #21 后选定 exe 经 from_exe canonicalize(macOS /var→/private/var 归一)
        let expected_exe = std::fs::canonicalize(&stub_exe).unwrap_or_else(|_| stub_exe.clone());
        assert_eq!(node.exe(), expected_exe.as_path());
        assert_eq!(node.kind(), NodeSourceKind::UserFnm);
        assert_eq!(node.version(), format!("v{floor}"));
        let recorded = state.node.clone().expect("应落账 user_fnm");
        assert_eq!(recorded.source, NodeSourceKind::UserFnm);
        assert_eq!(recorded.exe.as_deref(), Some(expected_exe.as_path()));

        // 幂等:二次调用仍复用同一路径
        let node2 = decide_node_with(&tools, &facts, &mut state).unwrap();
        assert_eq!(node2.exe(), expected_exe.as_path());
        fs::remove_dir_all(&root).unwrap();
    }

    /// 工单 #22:无 node/nvm/fnm → 新装 fnm 兜底。桩出整套 fnm 行为(HOME 指到
    /// 临时根,fnm 数据目录在平台默认位 ~/.local/share/fnm):
    /// - install_fnm 幂等复用:桩 fnm 可执行(`--version` 报版本)→ 不走镜像下载;
    /// - `fnm install/default` = 桩脚本按布局落 node exe(免网络,免真实 Node);
    /// - 钩子注入:注入平台登录 rc 的 FnmHook 行,记入 state;重跑不重复(幂等);
    /// - 落账 user_fnm + 解析出的 exe 绝对路径(与 UseFnm 同一解析路径)。
    #[cfg(unix)]
    #[test]
    fn decide_wiring_install_fnm_end_to_end_with_stub() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "setup-coder-test-installfnm-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // 前缀 cache 走真实 Prefix::home(与 install 一致)→ 隔离 HOME 后落在临时根
        let _home = crate::test_util::ScopedHome::set(&root);
        let tools: Vec<&Tool> = registry::all().iter().collect();
        let floor = registry::floor_for_tools(&tools);

        // 桩 fnm(平台默认数据目录 ~/.local/share/fnm/fnm):
        // `fnm --version` → 报版本(install_fnm 幂等复用命中);
        // `fnm install <v>` → 按 fnm 布局落 node exe(可执行,--version 报该版本);
        // `fnm default <v>` → no-op。
        let fnm_dir = root.join(".local/share/fnm");
        fs::create_dir_all(&fnm_dir).unwrap();
        let fnm_stub = fnm_dir.join("fnm");
        let script = "#!/bin/sh\n\
             FNM_DIR=\"${FNM_DIR:-$(cd \"$(dirname \"$0\")\" && pwd)}\"\n\
             case \"$1\" in\n\
             --version) echo 'fnm 1.39.0' ;;\n\
             install)\n\
             exe=\"$FNM_DIR/node-versions/v$2/installation/bin/node\"\n\
             mkdir -p \"$(dirname \"$exe\")\"\n\
             printf '#!/bin/sh\\necho v%s\\n' \"$2\" > \"$exe\"\n\
             chmod +x \"$exe\" ;;\n\
             esac\n";
        fs::write(&fnm_stub, script).unwrap();
        fs::set_permissions(&fnm_stub, fs::Permissions::from_mode(0o755)).unwrap();

        let facts = NodeFacts {
            bare_node: None,
            nvm: None,
            fnm: None,
        };
        let mut state = State::default();
        let node = decide_node_with(&tools, &facts, &mut state).unwrap();

        // 解析:fnm 布局下的 node exe(canonicalize 后),来源 user_fnm,版本 = 下限
        let stub_node = fnm_dir.join(format!("node-versions/v{floor}/installation/bin/node"));
        let expected_exe = std::fs::canonicalize(&stub_node).unwrap_or_else(|_| stub_node.clone());
        assert_eq!(node.exe(), expected_exe.as_path());
        assert_eq!(node.kind(), NodeSourceKind::UserFnm);
        assert_eq!(node.version(), format!("v{floor}"));
        // 落账:user_fnm + 版本 + exe;Node 不落前缀(前缀根在隔离 HOME 下不存在 node/)
        let recorded = state.node.clone().expect("应落账 user_fnm");
        assert_eq!(recorded.source, NodeSourceKind::UserFnm);
        assert_eq!(recorded.exe.as_deref(), Some(expected_exe.as_path()));
        assert!(!root.join(".setup-coder/node").exists());

        // 钩子注入:平台登录 rc 获得 FnmHook 行(幂等接缝),记入 state 供精确回滚
        let hook_line = platform::fnm_hook_line();
        let hooks: Vec<_> = state
            .path_injections
            .iter()
            .filter(|i| matches!(i, crate::prefix::PathInjection::FnmHook { .. }))
            .collect();
        assert!(!hooks.is_empty(), "应至少注入一个 rc 的 FnmHook 记录");
        for injection in &state.path_injections {
            let crate::prefix::PathInjection::FnmHook { file, line } = injection else {
                panic!("InstallFnm 只记 FnmHook 注入:{injection:?}");
            };
            assert_eq!(line, &hook_line);
            let content = fs::read_to_string(file).unwrap();
            assert_eq!(
                content.matches("fnm env").count(),
                1,
                "{} 应恰有一行钩子",
                file.display()
            );
        }

        // 幂等重跑:不再下载/重装(fnm 已可用),钩子不重复注入、记录不重复落账
        let hook_count = hooks.len();
        let node2 = decide_node_with(&tools, &facts, &mut state).unwrap();
        assert_eq!(node2.exe(), expected_exe.as_path(), "重跑应复用同一 exe");
        assert_eq!(
            state.path_injections.len(),
            hook_count,
            "重跑不得新增注入记录"
        );
        for injection in &state.path_injections {
            let crate::prefix::PathInjection::FnmHook { file, .. } = injection else {
                panic!("InstallFnm 只记 FnmHook 注入:{injection:?}");
            };
            let content = fs::read_to_string(file).unwrap();
            assert_eq!(
                content.matches("fnm env").count(),
                1,
                "{} 重跑不得重复行",
                file.display()
            );
        }

        // FnmHook 记录可精确回滚(uninstall 按记录逐字删行)
        for injection in state.path_injections.clone() {
            assert!(platform::rollback_injection(&injection).unwrap());
        }
        for injection in &state.path_injections {
            let crate::prefix::PathInjection::FnmHook { file, .. } = injection else {
                unreachable!()
            };
            assert!(!fs::read_to_string(file).unwrap().contains("fnm env"));
        }
        fs::remove_dir_all(&root).unwrap();
    }

    /// 测试机 PATH 上的 node 绝对路径(复用决策接线的真实探测对象)
    #[cfg(unix)]
    fn which_node() -> Option<PathBuf> {
        platform::find_in_path("node", &std::env::var_os("PATH").unwrap_or_default())
    }

    #[cfg(windows)]
    fn which_node() -> Option<PathBuf> {
        None // Windows CI 暂无 node 探测助手;该测试在 unix 开发机上覆盖
    }
}
