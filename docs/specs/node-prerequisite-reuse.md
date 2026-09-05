## Problem Statement

作为 setup-coder 的用户,我机器上可能已经装了自己的 Node.js / npm,或用 nvm / fnm 管理 Node 版本。当前 setup-coder 把 Node 装进私有前缀,并在 Tool 的 Shim 里把前缀内 node 强行置顶 PATH,导致:

- 我自己的 node/npm 被旁路——Tool 跑在 setup-coder 私备的 Node 上,而非我已有的 Node。
- 我在 Tool 会话里用 npm 装的全局包,落进了 setup-coder 的私有前缀,而非我自己的环境。
- setup-coder 未经许可替我决定"用哪个 Node",这是对用户环境的劫持。

我希望:装 Tool 时优先用我机器上已有的 Node;没有合适的再帮我装;无论如何,不要劫持我的环境。

## Solution

Node.js 作为 Prerequisite,获取策略改为**复用优先、按需安装、绝不劫持**:

1. 探测机器上的 Node 来源(裸 node / nvm / fnm)与版本。
2. 已有版本满足待装 Tool 的最低要求 → 直接复用,Tool 跑在用户已有的 Node 上。
3. 没有或不达标 → 复用用户已有的 nvm 装正确版本;用户没有 nvm 但已有 fnm → 复用 fnm;两者都没有 → 新装 fnm(三平台统一)再装 Node。任何"新装"都注入用户 shell rc 并明确告知。
4. 无论哪种来源,Tool 的 Shim 直接以该 Node 的绝对路径启动,PATH 里不出现任何 setup-coder 的 node 目录;Tool 实体仍装在私有前缀,但运行时是用户认可的 Node。
5. uninstall 只撤销 setup-coder 的交付(删前缀、回滚 `bin/` 的 PATH 注入);用户机器上的 nvm / fnm / Node 一律保留并提示。
6. doctor 报告 Node 来源(裸 Node / nvm / fnm)与达标状态,仍只读。

## User Stories

1. 作为已有合规 Node 的用户,我希望 setup-coder 直接用我的 Node 装 Tool,这样我的环境不被旁路。
2. 作为已有合规 Node 的用户,我希望 Tool 运行时优先解析到我的 node,这样我在 Tool 会话里敲 npm 用的是我自己的 npm。
3. 作为 Node 版本过旧的用户,我希望 setup-coder 帮我装到达标版本,这样 Tool 能跑。
4. 作为已有 nvm 的用户,我希望 setup-coder 用我的 nvm 装达标 Node,这样不会多出一个版本管理器。
5. 作为已有 fnm 但没有 nvm 的用户,我希望 setup-coder 用我的 fnm,这样遵循"用户有的先用"。
6. 作为完全没有 Node 的用户,我希望 setup-coder 帮我装好 fnm 和 Node 并配好 shell,这样我新开终端就能用 node。
7. 作为完全没有 Node 的小白用户,我希望整个安装过程零输入,这样我不需要懂 Node 版本管理。
8. 作为被 setup-coder 新装了 fnm 的用户,我希望它明确告诉我往哪个 rc 文件注入了什么,这样我对环境变更有知情。
9. 作为 Windows 用户,我希望 setup-coder 用 fnm 给我装 Node,这样三平台行为一致。
10. 作为 mac 用户,我希望 setup-coder 复用我已有的 nvm,这样不会重复装 fnm。
11. 作为 Ubuntu 用户,我希望 setup-coder 复用我已有的 nvm,这样不会重复装 fnm。
12. 作为用户,我希望 Tool 的 Shim 不写死 setup-coder 前缀里的 node,这样我的 PATH 干净。
13. 作为用户,我希望 PATH 里只有 setup-coder 的 `bin/` 一个目录,这样劫持面最小。
14. 作为用户,我希望 `setup-coder uninstall` 把我机器上的 nvm/fnm/Node 留着,这样我可能在用的环境不被破坏。
15. 作为用户,我希望 uninstall 明确提示我 fnm/Node 被保留以及如何自行删除,这样我有完整的知情与选择。
16. 作为用户,我希望 `setup-coder doctor` 告诉我 Node 是从哪来的(裸 Node/我的 nvm/我的 fnm)、达不达标,这样我能自己诊断。
17. 作为用户,我希望重跑 install 是幂等的——已有达标 Node 时不重复下载,这样重跑快速且无副作用。
18. 作为用户,我希望 Node 不达标时 setup-coder 装的是 Tool 集合要求的最高版本下限对应的可用版本,这样一次装对。
19. 作为用户,我希望 setup-coder 写的 npm registry 配置仍只作用于它自己装 Tool 的过程,不碰我的 `~/.npmrc`,这样我的 npm 配置不被污染。
20. 作为维护者,我希望"是否达标"的判定只看 Tool 的 `engines.node` 版本下限、不引入 EOL 检查,这样判定规则简单且与用户授权一致。
21. 作为维护者,我希望新增一个 Tool 时仍只改静态注册表一行,这样 Node 策略的改动不破坏 Tool 的可扩展性。
22. 作为维护者,我希望 Node 来源决策是纯逻辑、可脱离真实机器单测,这样核心分支能被完整覆盖。

## Implementation Decisions

### 领域与文档

- **CONTEXT.md(术语表)已先行更新**:`Prerequisite` 改为"优先复用机器上已有的,只在缺失或不达标时新装;Node 优先级为 复用已有 nvm > 复用已有 fnm > 新装 fnm(三平台统一默认)";`Private Prefix` 改为"装 Tool、Shim、cache 与 state.json;Node 与 npm 永不落此目录,也不进 PATH 劫持用户"。
- **ADR-0002(私有前缀)被本次决策推翻**:在其顶部加 Superseded 标记指向新 ADR,不改正文。
- **新增 ADR-0003**:记录"复用优先于隔离、fnm 三平台统一、rc 注入+告知换 node 可用、uninstall 不碰用户 Node 环境"。该决策难逆(动用户 rc 与全局 node)、无上下文会困惑、有真实权衡,满足立 ADR 的全部三条件。
- **ARCHITECTURE.md**:私有前缀布局图中 `node/` 从私有前缀布局中移除(Node 统一经 fnm 安装到用户级目录,永不落前缀),并补充 fnm/nvm 探测属 `platform/` 层职责。

### 核心:Node 来源的唯一决策点

- 引入唯一的 Node 来源决策抽象(以下称 `ensure_node` 的决策层),三职责分离:
  - **探测**:产出结构化事实——裸 node / nvm / fnm 各自的有无、版本、可执行路径。平台分叉(Windows 注册表 / unix rc / `command -v`)收敛在 platform 层。
  - **决策**:纯函数,入参 = 探测事实 + 待装 Tool 集合,出参 = 一个计划(复用某个 node 的绝对路径,或 新装 fnm)。不含任何 IO,可完整单测。
  - **执行**:把计划落成现实——新装 fnm、`fnm install <版本>`、`fnm default`、rc 注入,平台分叉收敛在 platform 层。
- **达标判定**:仅按待装 Tool 集合的 `engines.node` 取最高版本下限(当前全装 = `>=22.19.0`,见 `docs/research/node-version-requirements.md`),不做 EOL / 官方支持周期检查。版本下限作为静态数据进注册表或与注册表并列,加 Tool 时一并维护。

### Shim 与运行时

- **Shim 不再注入前缀 node 到 PATH**;改为以选定 Node 的绝对路径直接启动 Tool 的真实入口(`exec <node> <tool真实bin>`)。PATH 注入面收缩为仅 `bin/` 一个目录,且其中不含 node/npm。
- **npm 安装 Tool 的子进程**:registry、prefix、cache、userconfig 仍通过 `NPM_CONFIG_*` 环境变量限定在前缀内,不写用户 `~/.npmrc`;但 node 用选定来源的 node,而非强制前缀 node。

### uninstall 边界

- 删私有前缀、按 state.json 精确回滚 `bin/` 的 PATH 注入。
- 用户机器上的 nvm / fnm / Node 一律不删,输出保留提示与手工删除指引。
- state.json 的形状直接反映新模型(Node 来源 = `user-bare|user-nvm|user-fnm` + 版本 + rc 注入记录),不做旧格式兼容——state.json 是内部清单,非公开契约;setup-coder 代装的 fnm 同样记 `user-fnm`(机器上最多一个 fnm,有才复用、绝不重装),是否代装由 fnm 钩子的 rc 注入记录区分。

### doctor 口径

- 报告 Node 来源(裸 Node / nvm / fnm)、版本、是否达标;仍只读。

### 复用现有机制

- Mirror 容错链、下载缓存、`PathInjection` 精确回滚、platform 薄接缝、`--version` 冒烟(Installed 定义)全部保留并复用。

## Testing Decisions

**好测试的标准**:只测外部可观察行为(决策输出、shim 内容、回滚结果、doctor 报告),不测实现细节;确定性、隔离、可在全量套件中安全运行。

**测试接缝**:只在"Node 来源决策"开一个纯逻辑新接缝,其余复用现有接缝——这是整个改动唯一的决策密集点。

- **Node 来源决策**(新接缝,核心):表驱动单测覆盖全部来源分支——裸 node 达标复用 / 裸 node 不达标且有 nvm 用 nvm / 无 nvm 有 fnm 用 fnm / 两者都无装 fnm / 版本恰好等于下限达标 / 单装某 Tool 取下限较低的版本。
- **达标判定**:复用现有版本探测接缝,测 `>=22.19.0` 边界与"按 Tool 集合取最高下限"。
- **Shim 内容**(现有 `shim_content` 接缝):断言含选定 node 的绝对路径、不含前缀 node 的 PATH 注入。
- **uninstall 回滚**(现有 `PathInjection` / 回滚接缝):断言 `bin/` PATH 注入被精确移除、fnm/Node 保留、保留提示被打印。
- **doctor 报告**(现有 `check` 接缝):断言各来源下 Node 行输出正确的来源标注与达标 ✓/✗。
- **rc 注入幂等**(现有 shell rc 接缝):重跑不重复注入 fnm 行。

**既有可参照测试**:现有 PATH 幂等合并/回滚、shell rc contains、shim 内容、state.json 读写、各命令幂等重跑的纯逻辑单测,全部沿用同一风格与接缝。

## Out of Scope

- Node 的 EOL / 官方支持周期检查(用户明确不要)。
- `~/.npmrc` 或用户全局 npm 配置的任何读写(维持不碰)。
- nvm / fnm / Node 的卸载或清理(用户资产,uninstall 不动)。
- 把 Tool 实体迁出私有前缀——Tool 仍装前缀,仅运行时 Node 来源改变。
- 多版本 Node 并存管理、用户交互式选择版本(超出一行命令零输入定位)。
- Tool 的模型端点可用性(Installed 定义本就不含,v1 范围决策)。

## Further Notes

- 达标下限(当前 `>=22.19.0`)来自 `docs/research/node-version-requirements.md`,随各 Tool 的 npm `latest` 发布可能变化;每次发版或加 Tool 时应重新核对该研究笔记。
- nvm 与 fnm 的探测需处理"已装但未在当前 shell 生效"的情形(如 nvm 装在 rc 里但当前会话未 source)——探测应读安装痕迹与版本,而非仅依赖当前 PATH。
- fnm 在 Windows 的注入面(PowerShell profile / 用户 PATH)与 unix rc 不同,平台分叉收敛在 platform 层,commands 层不出现分叉。
- git 的 Prerequisite 策略(Windows MinGit 进前缀、unix 用系统 git)本次不变;仅 Node 的获取策略调整。
