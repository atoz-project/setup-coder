# win build 迁 ARC:x86_64-pc-windows-gnu 交叉编译取代 windows-latest MSVC

`ci.yml` 与 `release.yml` 的 win-x64 从 GitHub-hosted `windows-latest`(x86_64-pc-windows-msvc + `+crt-static`)迁到自建 `arc-runner-set-rust`(Linux pod),交叉编译 `x86_64-pc-windows-gnu`;链接器 `gcc-mingw-w64-x86-64` 与 gnu target 预装进 arc-runner-rust 镜像,pod 上零安装。macOS 两个目标留在 GitHub-hosted(macOS 无容器形态,ARC 跑不了);linux-x64 暂留 ubuntu-latest(镜像已备 musl-tools,将来可迁)。

理由:build 矩阵向自有 runner fleet 整合;crate 纯 Rust 无 C 依赖(rustls / miniz_oxide / winreg),gnu 链路零障碍,本地交叉编译实测通过(27s 出 PE32+ exe);ADR-0001 的零运行时依赖由 gnu 默认静态链 mingw 运行时继续满足,`+crt-static` 随 MSVC 一起退场。

代价:release 产物从此是 mingw 构建——AV 误报风险已拍板接受,**不做 VirusTotal 验证**(2026-09-11,用户决策);PDB 调试符号丢失;win 构建工具链从 stable 浮动变为镜像钉版(与 ARC 镜像 rust 版本一致,由 arc-runner-rust 升版协议管)。

## Considered Options

- 维持 windows-latest MSVC:零改动零风险,但 build 矩阵继续分散在两套 runner,GitHub-hosted 依赖不收敛
- x86_64-pc-windows-gnullvm(LLVM/UCRT,rust-lld 自包含、镜像零新增包):Tier 2、先例少;选 Tier 1 的经典 gnu,gnullvm 留作备选
- release workflow 接 VirusTotal 误报门禁(warn-only):被明确否决——不考虑误报问题
