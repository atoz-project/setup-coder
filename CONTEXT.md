# setup-coder

面向中国网络环境的小白用户,用一条命令装好各类 AI 编程 CLI 及其前置依赖的安装器。Rust 静态单二进制,Win / macOS / Ubuntu 三平台。

## Language

**Tool(工具)**:
一个以 npm 包分发的 AI 编程 CLI(v1:codex、claude code、pi)。
_Avoid_: 软件、应用、agent

**Registry(注册表)**:
代码内的静态表,一行描述一个 Tool:名称 → npm 包名 → 校验命令。加新工具 = 加一行。
_Avoid_: 插件系统、配置中心

**Prerequisite(前置依赖)**:
Tool 运行所需的第三方软件,目前是 Node.js 与 git。由本安装器负责装好。
_Avoid_: 环境、runtime

**Mirror(镜像源)**:
替代被 GFW 阻断/降速的官方源的国内可达下载源(npmmirror、OSS、Gitee 等)。零代理假设:不要求用户有代理,检测到则尊重。
_Avoid_: 代理、翻墙

**Private Prefix(私有前缀)**:
安装器自有目录(`~/.setup-coder/`),Node.js 与所有 Tool 装在其中,不触碰用户已有的 node/npm/全局环境。卸载 = 删除此目录。
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
