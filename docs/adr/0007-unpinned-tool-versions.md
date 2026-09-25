# Tool 版本不固定:omp / prime-agent 安装即最新

omp 与 prime-agent 原在 `net.rs` 以版本常量固定(`OMP_VERSION` / `PRIME_AGENT_VERSION`,升级 = 改常量 + 重测 + 发版)。2026-09-26 起去除:omp 走 GitHub `releases/latest/download/` 直链;prime-agent 的 tarball 资产名内嵌版本号,先经容错链取固定资产 `latest.json` 解析版本,再下同版本 tarball。npm 系三件套(codex / claude-code / pi)本就裸包名装 npm `latest`,至此五 Tool 语义统一:跑 `setup-coder install` 即装到/升级到上游最新。

上游发坏版本的风险由装后 `--version` 冒烟兜住:装坏会炸出明确报错,不会静默成功;npm 系今天本就承担同等敞口。基础设施(fnm / MinGit / Node)维持固定版本——它们是安装手段而非目的,装坏整个流程都跑不了。

被否方案:latest 失败回退固定版本——需保留双套 URL 链与版本常量,复杂度不值;维持固定——每次上游发版都需人工跟进常量并重测,且与「安装即最新」的用户诉求相反。
