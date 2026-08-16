# setup-coder

面向中国网络环境的小白用户:复制一行命令,零门槛装好 AI 编程 CLI(codex / claude code / pi)及其前置依赖(Node.js、git)。

## 一行命令安装

macOS / Ubuntu(终端):

```sh
curl -fsSL https://ghfast.top/https://github.com/atoz-project/setup-coder/releases/latest/download/install.sh | sh
```

Windows(PowerShell):

```powershell
irm https://ghfast.top/https://github.com/atoz-project/setup-coder/releases/latest/download/install.ps1 | iex
```

全程无需输入。装完**重新打开一个终端窗口**即可使用。

上面的地址连不上时,换用 GitHub 原始文件地址:

```sh
# macOS / Ubuntu
curl -fsSL https://ghfast.top/https://raw.githubusercontent.com/atoz-project/setup-coder/main/scripts/install.sh | sh
```

```powershell
# Windows(PowerShell)
irm https://ghfast.top/https://raw.githubusercontent.com/atoz-project/setup-coder/main/scripts/install.ps1 | iex
```

## 装了什么

- **三个 Tool**:codex、claude code、pi
- **前置依赖**:Node.js、git,由安装器自动装好

所有东西都装在私有前缀 `~/.setup-coder/`(Windows 为 `%USERPROFILE%\.setup-coder\`)里,不触碰你已有的 Node.js 和全局环境。卸载 = `setup-coder uninstall`,删干净并回滚 PATH 改动。

## 命令一览

| 命令 | 说明 |
| --- | --- |
| `setup-coder install` | 安装 Tool 及前置依赖(不带参数装全部;`install codex` 只装指定 Tool) |
| `setup-coder uninstall` | 卸载:删除私有前缀,并回滚 PATH 等对外改动 |
| `setup-coder doctor` | 体检:报告各 Tool 与前置依赖的安装状态 |

## 常见问题

**Windows 弹出「Windows 已保护你的电脑」(SmartScreen)?**
二进制暂未签名,属预期。点「更多信息」→「仍要运行」即可。提示的具体形态正在真机验证中,见 [#7](https://github.com/atoz-project/setup-coder/issues/7)。

**装完就能和 AI 对话了吗?**
不一定。安装成功 = Tool 能启动并报出版本号,**不含**模型端点可用——网络接入(订阅、API key 等)需要自理。

**我已经有代理,会冲突吗?**
不会。已有代理会被尊重,无需额外配置。

## 反馈

遇到问题请到 [GitHub Issues](https://github.com/atoz-project/setup-coder/issues) 反馈。
