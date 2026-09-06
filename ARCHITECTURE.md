# ARCHITECTURE

两张图:仓库长什么样,装到用户机器上长什么样。改动这两棵树的形状前先改本文件。

## 仓库布局

参考 ripgrep / uv / volta 等 Rust CLI 惯例:单 crate、按子命令分模块、平台分叉集中一处。

```
/
├── Cargo.toml              # 单 crate(bin),依赖最少化(ADR-0001)
├── src/
│   ├── main.rs             # 入口:clap 解析,分发子命令
│   ├── registry.rs         # Tool 静态注册表:名称 → npm 包名 → 校验命令 → Node 下限(加工具 = 加一行)
│   ├── node_plan.rs        # Node 来源决策层(纯函数,零 IO):facts → NodePlan(工单 #16)
│   ├── node_source.rs      # Node 来源解析接缝:NodePlan/清单 → 选定 Node 的 exe 绝对路径(shim 契约出处,工单 #15/#21)
│   ├── prefix.rs           # 私有前缀布局的唯一真源(路径常量都从这里出)
│   ├── net.rs              # 下载 + 各产物 Mirror 容错链(MinGit/fnm 等,链定义见下「容错链」)
│   ├── commands/
│   │   ├── install.rs
│   │   ├── uninstall.rs
│   │   └── doctor.rs
│   └── platform/           # 平台差异收敛于此(PATH 注入、git 安装、fnm/nvm 探测)
│       ├── windows.rs
│       ├── macos.rs
│       └── linux.rs
├── scripts/
│   ├── install.sh          # one-liner(mac/Ubuntu)
│   ├── install.ps1         # one-liner(Windows;含 PATH 持久化兜底,见规则)
│   └── tests/              # 脚本可测逻辑的最小单测(test-install-sh.sh)+ acceptance-* 验收脚本
├── .github/
│   ├── workflows/
│   │   ├── ci.yml          # push/PR:build matrix 四平台
│   │   └── release.yml     # tag v*:Release + latest.json
│   └── scripts/            # CI 专用脚本(与面向用户的 scripts/ 区分)
│       └── make-latest-json.sh  # 生成 latest.json(产物索引,结构即镜像契约)
├── docs/
│   ├── adr/                # 决策记录
│   ├── specs/              # 规格(ADR 引用的设计文档)
│   ├── research/           # 调研存档(代码引用的数据源)
│   └── agents/             # agent 工作约定
├── README.md               # 用户入口
├── CONTEXT.md              # 术语表(唯一词汇来源)
└── AGENTS.md
```

规则:

- 平台差异只允许出现在 `platform/`;commands 层写"做什么",platform 层写"在这个系统上怎么做"。唯一例外:`node_source.rs` 剥离 Windows canonicalize 产出的 verbatim 路径——cfg 分叉在调用点,平台实现仍收在 `platform::simplify_verbatim_path`。
- 路径不许散落硬编码,一律取自 `prefix.rs`(例外:`scripts/` 的 one-liner 脚本是自举入口,跑起来时 prefix.rs 尚未下载,豁免本条;路径常量以两脚本自身为准,须与下文前缀布局一致)。
- 脚本(`scripts/`)只做"下载二进制并转交",不长逻辑。唯一例外:`install.ps1` 转交成功后把 `bin/` 幂等写入 HKCU 用户 PATH 并广播 WM_SETTINGCHANGE(工单 #9 兜底:二进制写注册表在用户现场可能失败,One-liner 作为入口必须亲自确认 PATH 持久化)。

### 容错链

net.rs 与 one-liner 脚本共用原则:多源依次尝试,首个成功者落盘;全败时报出所有尝试过的源。

- one-liner 下载 setup-coder 自身(`scripts/install.sh|ps1` 头部常量):OSS → Gitee → GitHub 加速前缀 → GitHub 直连;镜像根留空 = 跳过该源。
- MinGit(`net.rs`,仅 Windows):npmmirror → cdn.npmmirror → 华为云。
- fnm(`net.rs`):华为云 → gh-proxy → GitHub 直连(npmmirror 不镜像 fnm,实测 404)。

## 安装前缀布局(用户机器)

参考 rustup(`~/.rustup`)/ volta(`~/.volta`)/ deno(`~/.deno`)的自有家目录模式;单目录自包含,卸载 = 删目录(原 ADR-0002;Node 策略改见 ADR-0003)。Windows 为 `%USERPROFILE%\.setup-coder\`,结构相同。

```
~/.setup-coder/
├── bin/                    # 唯一进 PATH 的目录:setup-coder 本体 + 各 Tool 的 shim
├── npm/                    # npm prefix:unix 实体在 lib/node_modules、启动器在 bin/;Windows 实体在 node_modules、启动器在 npm/ 根
├── .npmrc                  # 前缀内 npm 配置(registry 指向 npmmirror,经 NPM_CONFIG_USERCONFIG 生效)
├── git/                    # 仅 Windows:MinGit 便携版
├── cache/                  # 下载缓存(tarball/zip),可整删,重跑自动补
└── state.json              # 安装清单:已装工具与版本、Node 来源、PATH/rc 注入记录(uninstall/doctor 的依据)
```

注意:布局中**没有 `node/`**。Node 永不落私有前缀——经用户级版本管理器(fnm,或用户已有的 nvm)装入用户级目录,Tool 的 shim 以选定 Node 的绝对路径启动(ADR-0003)。

规则:

- `bin/` 是 Private Prefix 对外的唯一可见面(shim 定义见 CONTEXT.md);PATH 里只出现这一个目录。
- npm 的 registry/prefix 配置通过安装时环境变量作用于本前缀,不写用户的 `~/.npmrc`。
- `state.json` 记录一切对前缀外的改动(shell rc 行、HKCU PATH 条目、fnm 钩子行),uninstall 按它回滚。
- fnm/nvm/裸 Node 的探测(含 Windows 注册表 / unix rc / `command -v` 分叉)收敛在 `platform/` 层,commands 层不出现平台分叉。
