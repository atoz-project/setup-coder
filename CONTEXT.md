# setup-coder

面向中国网络环境的小白用户,用一条命令装好各类 AI 编程 CLI 及其前置依赖的安装器。Rust 静态单二进制,Win / macOS / Ubuntu 三平台。

## Language

**Tool(工具)**:
一个 AI 编程 CLI(v1:codex、claude code、pi、omp)。分发形态两种:npm 包(codex / claude code / pi,经 npmmirror)与预编译二进制资产(omp,经 GitHub Releases 容错链——其 npm 包硬依赖 Bun 运行时,见 ADR-0005)。
_Avoid_: 软件、应用、agent

**Registry(注册表)**:
代码内的静态表,一行描述一个 Tool:名称 → 分发来源(npm 包名 / 二进制资产)→ 校验命令。加新工具 = 加一行。
_Avoid_: 插件系统、配置中心

**Prerequisite(前置依赖)**:
Tool 运行所需的第三方软件,目前是 Node.js 与 git。原则是优先复用机器上已有的,只在缺失或版本不达标时才新装;Node.js 的复用/安装优先级为 复用已有 nvm > 复用已有 fnm > 新装 fnm(三平台统一默认)。
_Avoid_: 环境、runtime

**Mirror(镜像源)**:
替代被 GFW 阻断/降速的官方源的国内可达下载源(npmmirror、OSS、Gitee 等)。零代理假设:不要求用户有代理,检测到则尊重。
_Avoid_: 代理、翻墙

**Private Prefix(私有前缀)**:
安装器自有目录(`~/.setup-coder/`),装 Tool、Shim、cache 与 state.json;Node 与 npm 永不落此目录,也不进 PATH 劫持用户。卸载 = 删除此目录并按 state.json 回滚 rc 注入。
_Avoid_: 全局安装、系统环境

**Shim**:
暴露在 PATH 上的 Tool 启动入口,是 Private Prefix 对外的唯一可见面。
_Avoid_: 软链接(实现细节)

**Installed(安装成功)**:
Tool 能启动并报出版本号(冒烟 = `--version` 通过)。明确**不含**"模型端点可用"——网络接入是用户自己的事(v1 范围决策)。
_Avoid_: 可用、能对话

**Doctor(体检)**:
只读诊断命令:报告各 Tool 与 Prerequisite 的安装状态、PATH 与镜像连通性,不改动任何东西。
_Avoid_: 诊断、自检、检查

**One-liner(一行命令)**:
用户的自举入口:Win `irm … | iex`,mac/Ubuntu `curl … | sh`,从国内可达源下载静态二进制并自动执行 `install`——小白全程零输入,flags 仅供老手。
_Avoid_: 安装包、setup.exe
