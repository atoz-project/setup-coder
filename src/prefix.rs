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

/// state.json 格式版本号。v2:Node 来源模型(工单 #17),与 v1 不兼容
pub const STATE_VERSION: u32 = 2;

/// 私有前缀 `~/.setup-coder/` 的路径集合。
///
/// 布局(详见 ARCHITECTURE.md):
/// `bin/` 唯一进 PATH;`npm/` npm prefix;`git/` 仅 Windows;
/// `cache/` 下载缓存;`state.json` 安装清单。
/// 注:布局中没有 `node/`——Node 永不落私有前缀(ADR-0003,工单 #22):达标裸 Node
/// 直接复用,否则经用户级 nvm/fnm(缺失时新装 fnm)装入用户级目录。
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

/// state.json:安装清单(v2)。记录一切对前缀外的改动,uninstall 按它回滚。
///
/// v2 与 v1 不兼容:`node` 记录必须有 `source` 三值枚举,清单里没有
/// prefix 来源(Node 永不落前缀,见 ADR-0003)。旧版 v1 清单不做迁移,
/// `State::load` 直接报错并给出中文重装指引(快速失败,绝不静默误解析)。
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

/// Node 来源(v2 三值,serde snake_case):不存在 prefix 值——Node 永不落前缀
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeSourceKind {
    /// 用户机器上 PATH 里的裸 Node(不属任何版本管理器),复用
    UserBare,
    /// 经用户已有的 nvm 安装/复用的 Node
    UserNvm,
    /// 经 fnm 安装/复用的 Node(含 setup-coder 代装 fnm 的情形;
    /// 机器上最多一个 fnm,是否代装由 fnm 钩子的 rc 注入记录区分)
    UserFnm,
}

/// Node 安装记录:来源 + 解析出的版本;管理器来源另记 node exe 绝对路径
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeState {
    /// Node 来源(必填;缺失即非 v2 清单,加载直接报错)
    pub source: NodeSourceKind,
    /// 解析出的 Node 版本(如 `v24.19.0`)
    pub version: String,
    /// 管理器来源(nvm/fnm)下选定的 node 可执行文件绝对路径
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<PathBuf>,
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
    /// 向 rc/profile 文件注入了一行 fnm 钩子(`fnm env` / `fnm completions`,
    /// 供新开终端识别 fnm 管理的 Node)。与 ShellRc 同为精确行记录,
    /// 回滚语义逐字一致;独立成类以便 uninstall 提示与 doctor 区分两类注入
    FnmHook { file: PathBuf, line: String },
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
                    format!(
                        "state.json({})来自不兼容的旧版本或已损坏:{e}\n\
                         下一步:删除 {} 后重跑 setup-coder install 重建安装清单",
                        path.display(),
                        prefix.root().display()
                    ),
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
    fn npm_bin_dir_follows_platform_layout() {
        let p = Prefix::new(PathBuf::from("/x/.setup-coder"));
        if cfg!(windows) {
            assert_eq!(p.npm_bin_dir(), Path::new("/x/.setup-coder/npm"));
        } else {
            assert_eq!(p.npm_bin_dir(), Path::new("/x/.setup-coder/npm/bin"));
        }
    }

    #[test]
    fn state_roundtrip() {
        let p = temp_prefix("state-roundtrip");
        fs::create_dir_all(p.root()).unwrap();
        let mut s = State {
            node: Some(NodeState {
                source: NodeSourceKind::UserBare,
                version: "v24.19.0".into(),
                exe: None,
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
    fn node_source_variants_roundtrip() {
        let p = temp_prefix("node-source-roundtrip");
        fs::create_dir_all(p.root()).unwrap();

        // 三种来源各自序列化为 snake_case 标签并完整往返
        let cases = [
            (
                NodeSourceKind::UserBare,
                "\"user_bare\"",
                NodeState {
                    source: NodeSourceKind::UserBare,
                    version: "v24.19.0".into(),
                    exe: None,
                },
            ),
            (
                NodeSourceKind::UserNvm,
                "\"user_nvm\"",
                NodeState {
                    source: NodeSourceKind::UserNvm,
                    version: "v22.19.0".into(),
                    exe: Some(PathBuf::from(
                        "/home/u/.nvm/versions/node/v22.19.0/bin/node",
                    )),
                },
            ),
            (
                NodeSourceKind::UserFnm,
                "\"user_fnm\"",
                NodeState {
                    source: NodeSourceKind::UserFnm,
                    version: "v22.19.0".into(),
                    exe: Some(PathBuf::from(
                        "/home/u/.local/share/fnm/node-versions/v22.19.0/installation/bin/node",
                    )),
                },
            ),
        ];
        for (kind, tag, node) in cases {
            assert_eq!(serde_json::to_string(&kind).unwrap(), tag);
            let s = State {
                node: Some(node.clone()),
                ..State::default()
            };
            s.save(&p).unwrap();
            let text = fs::read_to_string(p.state_path()).unwrap();
            assert!(text.contains(tag), "落盘文本应含来源标签 {tag}:{text}");
            assert_eq!(State::load(&p).unwrap().node, Some(node));
        }
        // 清单里不存在 prefix 来源值:伪造旧标签必须解析失败
        assert!(serde_json::from_str::<NodeSourceKind>("\"prefix\"").is_err());

        fs::remove_dir_all(p.root()).unwrap();
    }

    #[test]
    fn fnm_hook_record_roundtrip_and_exact_line_rollback() {
        let p = temp_prefix("fnm-hook");
        fs::create_dir_all(p.root()).unwrap();

        let rc = p.root().join("zshrc");
        let hook = "eval \"$(fnm env --use-on-cd)\"  # setup-coder fnm";
        let mut s = State::default();
        s.record_injection(PathInjection::FnmHook {
            file: rc.clone(),
            line: hook.into(),
        });
        // 去重:同一条 fnm 钩子不重复记
        s.record_injection(PathInjection::FnmHook {
            file: rc.clone(),
            line: hook.into(),
        });
        assert_eq!(s.path_injections.len(), 1);

        // 落盘带 kind=fnm_hook 标签并完整往返
        s.save(&p).unwrap();
        let text = fs::read_to_string(p.state_path()).unwrap();
        assert!(
            text.contains("\"fnm_hook\""),
            "落盘文本应含 fnm_hook:{text}"
        );
        assert_eq!(State::load(&p).unwrap(), s);

        // 精确行回滚:只删记录行,其余内容保留
        fs::write(&rc, format!("export FOO=1\n{hook}\nexport BAR=2\n")).unwrap();
        assert!(platform::rollback_injection(&s.path_injections[0]).unwrap());
        assert_eq!(
            fs::read_to_string(&rc).unwrap(),
            "export FOO=1\nexport BAR=2\n"
        );
        // 已删除 → 幂等 Ok(false);行从不存在 → 同样 Ok(false),绝不模糊匹配
        assert!(!platform::rollback_injection(&s.path_injections[0]).unwrap());
        fs::write(&rc, "export FOO=1\n").unwrap();
        assert!(!platform::rollback_injection(&s.path_injections[0]).unwrap());

        fs::remove_dir_all(p.root()).unwrap();
    }

    #[test]
    fn v1_shaped_manifest_fails_fast_with_reinstall_hint() {
        let p = temp_prefix("v1-manifest");
        fs::create_dir_all(p.root()).unwrap();

        // v1 形态:node 只有 {version},无 source 字段
        fs::write(
            p.state_path(),
            "{\n  \"version\": 1,\n  \"node\": {\n    \"version\": \"v24.19.0\"\n  },\n  \"tools\": [],\n  \"path_injections\": []\n}\n",
        )
        .unwrap();
        let err = State::load(&p).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let msg = err.to_string();
        // 清晰中文指引:告知不兼容 + 给出删除前缀/重跑 install 的下一步
        assert!(msg.contains("不兼容"), "应提示版本不兼容:{msg}");
        assert!(msg.contains("setup-coder install"), "应给出重跑指引:{msg}");

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
