# ARCHITECTURE

两张图:仓库长什么样,装到用户机器上长什么样。改动这两棵树的形状前先改本文件。

## 仓库布局

参考 ripgrep / uv / volta 等 Rust CLI 惯例:单 crate、按子命令分模块、平台分叉集中一处。

```
/
├── Cargo.toml              # 单 crate(bin),依赖最少化(ADR-0001)
├── src/
│   ├── main.rs             # 入口:clap 解析,分发子命令
│   ├── registry.rs         # Tool 静态注册表:名称 → npm 包名 → 校验命令(加工具 = 加一行)
│   ├── prefix.rs           # 私有前缀布局的唯一真源(路径常量都从这里出)
│   ├── net.rs              # 下载 + Mirror 容错链(OSS → Gitee → GitHub 加速)
│   ├── commands/
│   │   ├── install.rs
│   │   ├── uninstall.rs
│   │   └── doctor.rs
│   └── platform/           # 所有 cfg 分叉收敛于此(PATH 注入、git 安装)
│       ├── windows.rs
│       ├── macos.rs
│       └── linux.rs
├── scripts/
│   ├── install.sh          # one-liner(mac/Ubuntu)
│   └── install.ps1         # one-liner(Windows)
├── .github/
│   ├── workflows/
│   │   ├── ci.yml          # push/PR:build matrix 四平台
│   │   └── release.yml     # tag v*:Release + latest.json
│   └── scripts/            # CI 专用脚本(与面向用户的 scripts/ 区分)
│       └── make-latest-json.sh  # 生成 latest.json(产物索引,结构即镜像契约)
├── docs/
│   ├── adr/                # 决策记录
│   └── agents/             # agent 工作约定
├── CONTEXT.md              # 术语表(唯一词汇来源)
└── AGENTS.md
```

规则:

- 平台差异只允许出现在 `platform/`;commands 层写"做什么",platform 层写"在这个系统上怎么做"。
- 路径不许散落硬编码,一律取自 `prefix.rs`。
- 脚本(`scripts/`)只做"下载二进制并转交",不长逻辑。

## 安装前缀布局(用户机器)

参考 rustup(`~/.rustup`)/ volta(`~/.volta`)/ deno(`~/.deno`)的自有家目录模式;单目录自包含,卸载 = 删目录(ADR-0002)。Windows 为 `%USERPROFILE%\.setup-coder\`,结构相同。

```
~/.setup-coder/
├── bin/                    # 唯一进 PATH 的目录:setup-coder 本体 + 各 Tool 的 shim
├── node/                   # Node LTS 解压目录(node/bin/node)
├── npm/                    # npm prefix:Tool 实体装在 npm/lib/node_modules,bin 在 npm/bin
├── git/                    # 仅 Windows:MinGit 便携版
├── cache/                  # 下载缓存(tarball/zip),可整删,重跑自动补
└── state.json              # 安装清单:已装工具与版本、PATH 注入记录(uninstall/doctor 的依据)
```

规则:

- `bin/` 是 Private Prefix 对外的唯一可见面(shim 定义见 CONTEXT.md);PATH 里只出现这一个目录。
- npm 的 registry/prefix 配置通过安装时环境变量作用于本前缀,不写用户的 `~/.npmrc`。
- `state.json` 记录一切对前缀外的改动(shell rc 行、HKCU PATH 条目),uninstall 按它回滚。
