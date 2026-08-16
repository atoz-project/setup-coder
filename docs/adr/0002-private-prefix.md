# Node.js 与所有工具装进私有前缀,不碰用户全局环境

Node.js(npmmirror 的 LTS)与全部 Tool 安装进 `~/.setup-coder/`,npm registry 配置(npmmirror)也只写在该前缀内;PATH 上只暴露 Tool 的 shim。用户已有的 node/npm/.npmrc 一概不读不写。理由:永不与既有环境冲突、重跑幂等、卸载 = 删一个目录;代价:用户不会因此获得"系统 node",另有需要须自装。

## Considered Options

- 全局装 node(官方安装器/包管理器):对用户"更有用",但会与既有版本冲突、污染全局 .npmrc、卸载困难,支持成本高
