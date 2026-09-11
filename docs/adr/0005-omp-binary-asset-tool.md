# omp 以预编译二进制资产分发,不经 npm;Registry 模型加 ToolSource

omp(Oh My Pi)加入注册表,走 GitHub Releases 的预编译二进制资产(`bun build --compile` 单文件,容错链 gh-proxy → GitHub 直连),不经 npm 安装。Registry 的 Tool 模型相应从一形态(npm 包)扩为两形态(`ToolSource::Npm` / `ToolSource::Binary`);二进制 Tool 免 Node、免 git、免 npmrc,本体直接落 `bin/`(不生成 shim),纯二进制选择时 install 跳过整个 Node 决策。

理由:omp 的 npm 包(`@oh-my-pi/pi-coding-agent`,npmmirror 有)硬依赖 Bun 运行时——`bun:` protocol imports,Node 24 实测 `ERR_UNSUPPORTED_ESM_URL_SCHEME` 起不来。要经 npm 装 omp 就得先引入 Bun 作为新 Prerequisite(平台层长出与 Node 平行的第二个运行时决策件),而官方二进制零运行时、一条容错链即可下载,与本仓 MinGit/fnm 的下载型产物模式同构。两害相权,二进制资产复杂度更低、与上游主力发行形态对齐。

代价:omp 版本钉死在 `net.rs` 常量(升级 = 改常量 + 重测,同 fnm 既有协议);单平台二进制 130-190 MB,比 npm 包大一个量级;state.json 里其 `package` 字段记为 `binary:can1357/oh-my-pi@<ver>` 展示性串(非 npm 包名)。

附议:prime-agent(Prime Intellect)**暂缓收录**。它不在任何 npm registry,GitHub release 的 npm 格式 tarball 内部依赖指向 r2.dev 的兄弟 tarball——github releases + r2.dev 两个境外源,后者无国内镜像,与零代理假设冲突。除非在 OSS 自托管其 4 个 tarball,否则国内装不通;那是镜像运维决策,不是代码决策。

## Considered Options

- 引入 Bun 前置 + npm 安装 omp:与 npm Tool 形态统一,但平台层要多一个运行时决策件(node_plan/node_source 的 Bun 平行件),收益仅是"形态一致"
- 收 omp-cn(简体中文本地化分支,同 Bun 依赖):同 Bun 问题,且非上游
- 收录 prime-agent(npm tarball URL 安装):r2.dev 依赖链国内无镜像,装不通
