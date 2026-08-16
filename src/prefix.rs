//! 私有前缀布局的唯一真源:所有路径常量都从这里出(见 ARCHITECTURE.md)。
//!
//! 同时负责 `state.json` 的读写——它是前缀布局的一部分(安装清单,
//! uninstall/doctor 的依据)。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::platform;

/// 前缀根目录名(位于用户家目录下):`~/.setup-coder/`
pub const ROOT_DIR_NAME: &str = ".setup-coder";

/// state.json 格式版本号
pub const STATE_VERSION: u32 = 1;

/// 私有前缀 `~/.setup-coder/` 的路径集合。
///
/// 布局(详见 ARCHITECTURE.md):
/// `bin/` 唯一进 PATH;`node/` Node LTS;`npm/` npm prefix;
/// `git/` 仅 Windows;`cache/` 下载缓存;`state.json` 安装清单。
#[derive(Debug, Clone)]
pub struct Prefix {
    root: PathBuf,
}

impl Prefix {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// 当前用户家目录下的默认前缀(`~/.setup-coder`)
    pub fn home() -> io::Result<Self> {
        let home = std::env::home_dir().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "无法确定用户家目录(HOME 未设置)")
        })?;
        Ok(Self::new(home.join(ROOT_DIR_NAME)))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `bin/`:唯一进 PATH 的目录(setup-coder 本体 + 各 Tool 的 shim)
    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    /// `node/`:Node LTS 解压目录
    pub fn node_dir(&self) -> PathBuf {
        self.root.join("node")
    }

    /// Node 可执行文件所在目录(unix `node/bin`,Windows `node/`)
    pub fn node_bin_dir(&self) -> PathBuf {
        self.node_dir().join(platform::node_bin_subdir())
    }

    /// node / node.exe 本体
    pub fn node_exe(&self) -> PathBuf {
        self.node_bin_dir().join(platform::exe_name("node"))
    }

    /// `npm/`:npm prefix(Tool 实体装在 lib/node_modules,bin 在 bin/)
    pub fn npm_dir(&self) -> PathBuf {
        self.root.join("npm")
    }

    /// npm 全局 bin 目录(unix `npm/bin`,Windows `npm/`)
    pub fn npm_bin_dir(&self) -> PathBuf {
        self.npm_dir().join(platform::npm_bin_subdir())
    }

    /// 前缀内 npmrc(`~/.setup-coder/.npmrc`,指向 npmmirror;不碰用户 ~/.npmrc)
    pub fn npmrc(&self) -> PathBuf {
        self.root.join(".npmrc")
    }

    /// `git/`:仅 Windows 的 MinGit 便携版目录
    // 仅 Windows 的 git 安装流水线使用;unix 构建中保留以固定布局(死代码豁免)
    #[allow(dead_code)]
    pub fn git_dir(&self) -> PathBuf {
        self.root.join("git")
    }

    /// MinGit 可执行文件:`git/cmd/git.exe`(MinGit zip 根目录即 cmd/,见工单 #3 交叉核验)
    // 仅 Windows 使用
    #[allow(dead_code)]
    pub fn git_exe(&self) -> PathBuf {
        self.git_dir().join("cmd").join(platform::exe_name("git"))
    }

    /// `cache/`:下载缓存,可整删,重跑自动补
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// `state.json`:安装清单
    pub fn state_path(&self) -> PathBuf {
        self.root.join("state.json")
    }

    /// 创建前缀目录骨架(bin/cache 等;node/npm 由安装步骤按需建)
    pub fn create_skeleton(&self) -> io::Result<()> {
        fs::create_dir_all(self.bin_dir())?;
        fs::create_dir_all(self.cache_dir())?;
        Ok(())
    }
}

/// state.json:安装清单。记录一切对前缀外的改动,uninstall 按它回滚。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    /// Node.js 安装记录
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeState>,
    /// 已装 Tool 及冒烟版本号
    #[serde(default)]
    pub tools: Vec<ToolState>,
    /// 前缀外改动记录(PATH 注入)
    #[serde(default)]
    pub path_injections: Vec<PathInjection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeState {
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolState {
    pub name: String,
    pub package: String,
    /// `shim --version` 的原始输出(Installed 定义:能启动并报出版本号)
    pub version: String,
}

/// 一条前缀外改动记录
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PathInjection {
    /// 向 shell rc 文件追加了一行 export PATH
    ShellRc { file: PathBuf, line: String },
    /// 向 Windows HKCU 用户 PATH 追加了一个目录
    WindowsUserPath { dir: PathBuf },
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            node: None,
            tools: Vec::new(),
            path_injections: Vec::new(),
        }
    }
}

impl State {
    /// 读 state.json;不存在 = 空清单(首次安装)
    pub fn load(prefix: &Prefix) -> io::Result<Self> {
        let path = prefix.state_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("state.json 损坏({}):{e}", path.display()),
                )
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// 全量重写 state.json(重跑幂等:每次安装后完整落盘)
    pub fn save(&self, prefix: &Prefix) -> io::Result<()> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(prefix.state_path(), text + "\n")
    }

    /// 记录一条 PATH 注入(去重:同样的改动不重复记)
    pub fn record_injection(&mut self, injection: PathInjection) {
        if !self.path_injections.contains(&injection) {
            self.path_injections.push(injection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_prefix(tag: &str) -> Prefix {
        let dir =
            std::env::temp_dir().join(format!("setup-coder-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Prefix::new(dir)
    }

    #[test]
    fn layout_matches_architecture_doc() {
        let p = Prefix::new(PathBuf::from("/home/u/.setup-coder"));
        assert_eq!(p.bin_dir(), Path::new("/home/u/.setup-coder/bin"));
        assert_eq!(p.node_dir(), Path::new("/home/u/.setup-coder/node"));
        assert_eq!(p.npm_dir(), Path::new("/home/u/.setup-coder/npm"));
        assert_eq!(p.git_dir(), Path::new("/home/u/.setup-coder/git"));
        assert!(p.git_exe().ends_with(if cfg!(windows) {
            "git/cmd/git.exe"
        } else {
            "git/cmd/git"
        }));
        assert_eq!(p.cache_dir(), Path::new("/home/u/.setup-coder/cache"));
        assert_eq!(p.state_path(), Path::new("/home/u/.setup-coder/state.json"));
        assert_eq!(p.npmrc(), Path::new("/home/u/.setup-coder/.npmrc"));
    }

    #[test]
    fn node_and_npm_bin_dirs_follow_platform_layout() {
        let p = Prefix::new(PathBuf::from("/x/.setup-coder"));
        if cfg!(windows) {
            assert_eq!(p.node_bin_dir(), Path::new("/x/.setup-coder/node"));
            assert_eq!(p.npm_bin_dir(), Path::new("/x/.setup-coder/npm"));
            assert!(p.node_exe().ends_with("node.exe"));
        } else {
            assert_eq!(p.node_bin_dir(), Path::new("/x/.setup-coder/node/bin"));
            assert_eq!(p.npm_bin_dir(), Path::new("/x/.setup-coder/npm/bin"));
            assert!(p.node_exe().ends_with("node/bin/node"));
        }
    }

    #[test]
    fn state_roundtrip() {
        let p = temp_prefix("state-roundtrip");
        fs::create_dir_all(p.root()).unwrap();

        // 不存在 = 空清单
        assert_eq!(State::load(&p).unwrap(), State::default());

        let mut s = State {
            node: Some(NodeState {
                version: "v24.19.0".into(),
            }),
            ..State::default()
        };
        s.tools.push(ToolState {
            name: "codex".into(),
            package: "@openai/codex".into(),
            version: "codex-cli 0.147.0".into(),
        });
        s.record_injection(PathInjection::ShellRc {
            file: PathBuf::from("/home/u/.zshrc"),
            line: "export PATH=\"/home/u/.setup-coder/bin:$PATH\"  # setup-coder".into(),
        });
        // 去重:同一条注入不重复记
        s.record_injection(PathInjection::ShellRc {
            file: PathBuf::from("/home/u/.zshrc"),
            line: "export PATH=\"/home/u/.setup-coder/bin:$PATH\"  # setup-coder".into(),
        });
        assert_eq!(s.path_injections.len(), 1);

        s.save(&p).unwrap();
        let loaded = State::load(&p).unwrap();
        assert_eq!(loaded, s);

        fs::remove_dir_all(p.root()).unwrap();
    }

    #[test]
    fn corrupted_state_is_an_error_not_a_panic() {
        let p = temp_prefix("state-corrupt");
        fs::create_dir_all(p.root()).unwrap();
        fs::write(p.state_path(), "{ 不是 json").unwrap();
        let err = State::load(&p).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(p.root()).unwrap();
    }
}
