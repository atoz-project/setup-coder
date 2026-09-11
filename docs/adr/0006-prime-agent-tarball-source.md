# 收录 prime-agent:npm tarball 直连 + gh-proxy 容错链,接受 r2.dev 直连风险

取代 ADR-0005 附议中的「prime-agent 暂缓」结论。prime-agent(Prime Intellect,v0.9.4)加入注册表,走新增的 `ToolSource::NpmTarball` 形态:不在任何 npm registry,以 GitHub Releases 的 npm 格式 tarball 分发;主 tarball 经 gh-proxy → GitHub 直连容错链交给 `npm install -g <URL>`,Node 下限 22.8.0(tarball 内 package.json 实测)。

关键风险与证据:tarball 的 3 个兄弟依赖硬编码 `r2.dev`(Cloudflare R2)URL,由 npm 安装时直连,我们无法插入容错链。2026-09-11 实测:三家国内公共 DNS(阿里/腾讯/百度)对该域名解析无污染(与 1.1.1.1 一致);站长工具 20+ 大陆节点(电信/联通/多线)对其 IP ICMP 全通(150-210ms,正常跨太平洋延迟),排除 IP 黑洞;SNI/TLS 级未实测(免费第三方拨测均无大陆可用节点)。研判:大概率不被墙,残留风险是 Cloudflare 免费段的国内 QoS 抖动。安装失败时给出指向 r2.dev 的中文提示,引导重试或配置代理。

同时修复由此暴露的冒烟盲区:`--version` 版本串判定从「只认 stdout」放宽为「stdout 空则取 stderr」——prime-agent 的 `--version` 打在 stderr(install 冒烟与 doctor 体检共用该语义)。

不选 OSS 自托管重打包的理由:光镜像 4 个 tarball 无效(依赖硬编码 r2.dev),必须改写 package.json 重打包并跟踪上游发版;在封锁证据大体排除后,这条流水线的运维成本不划算。若日后 r2.dev 国内实测恶化,重开此决策。

## Considered Options

- OSS 自托管 + 重打包流水线:全链路国内可控,但需改写 deps + 跟踪发版的持续运维;证据不支持现在投入
- 引入 Bun 前置装 omp 的 npm 包:见 ADR-0005(已否,omp 走二进制)
- 维持暂缓:用户的需求明确(收录 prime-agent),证据排除了主要障碍
