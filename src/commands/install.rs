//! `install` 子命令:装 Node.js 与 Tool 进 Private Prefix,零输入完成。
//!
//! 流水线:建前缀骨架 → 装 Node LTS(npmmirror)→ 确保 git(Prerequisite,工单 #3)→
//! 复制 setup-coder 本体 → npm 装 Tool(注册表)→ 生成 shim → 冒烟(`--version`,Installed 定义)→
//! 注入 PATH → 写 state.json。重跑 = 修复/升级,幂等。

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::net;
use crate::platform;
use crate::prefix::{NodeState, Prefix, State, ToolState};
use crate::registry::{self, Tool};

/// Node LTS(Krypton)。升级 = 改这一行并重测。
/// 核实来源:npmmirror node 镜像 index.json,2026-08 时为最新 LTS。
const NODE_VERSION: &str = "v24.19.0";

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

    ensure_node(&prefix, &mut state)?;
    ensure_git(&prefix)?;
    install_setup_coder_self(&prefix)?;
    write_npmrc(&prefix)?;

    let mut installed = Vec::new();
    for t in tools {
        install_tool(&prefix, t, &mut installed)?;
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

/// Node 发行包文件名:`node-vX.Y.Z-<suffix>.<ext>`
fn node_archive_name(version: &str, suffix: &str, ext: &str) -> String {
    format!("node-{version}-{suffix}.{ext}")
}

/// Node 下载 URL 容错链(npmmirror 主源 + CDN + 华为云兜底)
fn node_urls(version: &str, suffix: &str, ext: &str) -> Vec<String> {
    let file = node_archive_name(version, suffix, ext);
    vec![
        format!("https://registry.npmmirror.com/-/binary/node/{version}/{file}"),
        format!("https://cdn.npmmirror.com/binaries/node/{version}/{file}"),
        format!("https://mirrors.huaweicloud.com/nodejs/{version}/{file}"),
    ]
}

/// 装 Node LTS:已是指定版本则跳过(幂等),否则下载解压替换(修复/升级)
fn ensure_node(prefix: &Prefix, state: &mut State) -> Result<(), Box<dyn Error>> {
    if node_version_matches(prefix)? {
        println!("Node.js {NODE_VERSION} 已就位,跳过下载");
    } else {
        let suffix = platform::node_dist_suffix()?;
        let ext = platform::node_archive_ext();
        println!("下载 Node.js {NODE_VERSION}({suffix})…");
        let archive = prefix
            .cache_dir()
            .join(node_archive_name(NODE_VERSION, suffix, ext));
        let hit = net::download_first(&node_urls(NODE_VERSION, suffix, ext), &archive)?;
        println!("已从 Mirror 下载:{hit}");

        // 解压到暂存目录,成功后整体替换 node/(避免半残前缀)
        let staging = prefix.cache_dir().join("node-staging");
        let _ = fs::remove_dir_all(&staging);
        platform::extract_node_archive(&archive, &staging)?;
        let node_dir = prefix.node_dir();
        let _ = fs::remove_dir_all(&node_dir);
        fs::rename(&staging, &node_dir)?;

        // 冒烟:刚解压的 node 必须能跑且版本对
        if !node_version_matches(prefix)? {
            return Err(format!(
                "Node.js 解压后自检失败:期望 {NODE_VERSION},`node --version` 未通过"
            )
            .into());
        }
        println!("Node.js {NODE_VERSION} 安装完成");
    }
    state.node = Some(NodeState {
        version: NODE_VERSION.to_string(),
    });
    Ok(())
}

/// 前缀内 node 存在且 `--version` 输出等于目标版本
fn node_version_matches(prefix: &Prefix) -> Result<bool, Box<dyn Error>> {
    let node = prefix.node_exe();
    if !node.exists() {
        return Ok(false);
    }
    let Ok(out) = Command::new(&node).arg("--version").output() else {
        return Ok(false); // 跑不起来 = 当作未装,重装修复
    };
    Ok(out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == NODE_VERSION)
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

/// 给子进程准备的 PATH:前缀内 node 在最前(npm/Tool 的 #!/usr/bin/env node 依赖它)
fn path_with_node(prefix: &Prefix) -> Result<OsString, Box<dyn Error>> {
    let mut paths = vec![prefix.node_bin_dir()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    Ok(std::env::join_paths(paths)?)
}

/// npm 可执行入口:前缀内 node + 其自带的 npm-cli.js(布局因平台而异)
fn npm_command(prefix: &Prefix, args: &[&str]) -> Result<Command, Box<dyn Error>> {
    let npm_cli = prefix.node_dir().join(platform::npm_cli_subpath());
    if !npm_cli.exists() {
        return Err(format!("前缀内 npm 不存在:{}(Node 安装不完整)", npm_cli.display()).into());
    }
    let mut cmd = Command::new(prefix.node_exe());
    cmd.arg(npm_cli)
        .args(args)
        .env("PATH", path_with_node(prefix)?)
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
    tool: &Tool,
    installed: &mut Vec<ToolState>,
) -> Result<(), Box<dyn Error>> {
    println!("安装 {}({})…", tool.name, tool.package);
    let status = npm_command(prefix, &["install", "--global", tool.package])?.status()?;
    if !status.success() {
        return Err(format!("npm 安装 {} 失败(退出码 {:?})", tool.package, status.code()).into());
    }

    let shim = platform::write_shim(
        &prefix.bin_dir(),
        &prefix.node_bin_dir(),
        &prefix.npm_bin_dir(),
        tool.bin,
    )?;

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

    #[test]
    fn node_archive_name_matches_dist_layout() {
        assert_eq!(
            node_archive_name("v24.19.0", "darwin-arm64", "tar.gz"),
            "node-v24.19.0-darwin-arm64.tar.gz"
        );
        assert_eq!(
            node_archive_name("v24.19.0", "win-x64", "zip"),
            "node-v24.19.0-win-x64.zip"
        );
    }

    #[test]
    fn node_urls_form_a_mirror_chain() {
        let urls = node_urls("v24.19.0", "linux-x64", "tar.gz");
        assert!(urls.len() >= 2, "必须有容错链");
        for u in &urls {
            assert!(u.starts_with("https://"), "只允许 https:{u}");
            assert!(u.contains("/v24.19.0/node-v24.19.0-linux-x64.tar.gz"));
        }
        // 主源必须是 npmmirror(ADR-0002)
        assert!(urls[0].contains("npmmirror.com"));
    }
}
