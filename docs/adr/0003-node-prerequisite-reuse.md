# Node 前置依赖:复用优先于隔离,fnm 三平台统一,rc 注入并告知,卸载保留用户 Node

取代 ADR-0002 中关于 Node.js 的部分(Tool 实体与 shim 仍留私有前缀,不变)。四项决策:

1. **复用优先于隔离**:Node 不再装进私有前缀。探测机器上已有的 Node 来源(裸 node / nvm / fnm),达标即复用;不达标或缺失时,复用已有 nvm > 复用已有 fnm > 新装 fnm。Tool 的 shim 以选定 Node 的绝对路径直接启动,PATH 里不出现任何 setup-coder 的 node 目录。
2. **fnm 作为三平台默认版本管理器**:需要新装时三平台统一装 fnm(单一 Rust 静态二进制,Windows/macOS/Ubuntu 行为一致,启动快、无需 shell 函数包装),不为 Windows 单独设计第二套方案。
3. **rc 注入并告知**:新装 fnm 必须向用户 shell rc/profile 注入 fnm 钩子行并 `fnm default`,否则新开终端找不到 node;注入逐字记录进 state.json(精确行,uninstall 可回滚),并在安装输出中明确告知用户改了哪个文件、加了什么。
4. **uninstall 保留用户 Node 环境**:卸载只删私有前缀、回滚 `bin/` 的 PATH 注入与 fnm 钩子行;用户机器上的 nvm / fnm / Node(含 setup-coder 代装的 fnm)一律保留,并提示用户如何自行删除。

理由:旧模型(ADR-0002)把 Node 装进私有前缀并让 shim 劫持 PATH,旁路了用户已有的 node/npm——用户在 Tool 会话里装的全局包落进 setup-coder 私有前缀而非自己的环境,这是未经许可替用户决定"用哪个 Node"。中国网络环境下的真实用户大多已有自己的 Node(或 nvm/fnm),隔离的确定性收益抵不过劫持用户环境的信任损失。fnm 三平台统一使执行层无平台分叉;rc 注入换取"新开终端即可用 node",以逐字记录 + 告知换可回滚与知情权。代价:不再独占 Node 运行时,版本不达标判定、已装未生效(nvm 装在 rc 但当前会话未 source)等探测复杂度上移;卸载不再是一个目录删到底,须保留并提示用户的 Node 资产。

ADR-0002 的假设为何改变:它假设"用户机器上没有可用的 Node/ npm,或用户不在乎被隔离"。spec(docs/specs/node-prerequisite-reuse.md)的用户故事推翻了这一点——目标用户大量已有 Node 环境且明确拒绝被旁路;隔离从"特性"变成了"劫持"。

## Considered Options

- 维持 ADR-0002 全隔离(Node 进前缀、shim 劫持 PATH):确定性最高、卸载最简,但旁路用户环境、污染用户全局包的落点,信任成本不可接受
- 复用但不装版本管理器(缺失时仍装进前缀):对无 Node 用户无法提供"新开终端可用"的体验,且前缀 node 与用户环境并存仍是双真相
- nvm 作为三平台默认:nvm 无官方 Windows 支持(nvm-windows 是另一个项目),三平台行为无法统一;fnm 单一二进制覆盖三平台
- 静默注入 rc(不告知):安装输出更干净,但用户对环境变更无知情,违背"绝不劫持"原则
