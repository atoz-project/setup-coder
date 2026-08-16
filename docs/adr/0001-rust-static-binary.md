# 用 Rust 静态单二进制实现安装器,而非 shell 脚本

安装器跑在三个平台上、面对小白用户的各种残缺环境(无 node、无 git、PowerShell 执行策略各异),shell/PowerShell 双脚本维护成本高且脆弱。选 Rust:静态编译(musl / 静态 CRT)、单文件、零运行时依赖、跨平台交叉编译免费拿 Linux。代价:自举需要先下载二进制(由 one-liner 脚本解决),二进制未签名(Windows SmartScreen 提示,v1 接受)。

## Considered Options

- bash + PowerShell 双脚本:零下载自举,但两套逻辑、无类型、错误处理弱,鲁棒性差
- Go:同样静态单二进制,可行;选 Rust 属团队偏好,二进制更小
