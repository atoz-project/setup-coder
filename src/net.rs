//! 下载 + Mirror 容错链:依次尝试多个镜像源,首个成功者落盘。
//!
//! HTTP 客户端为 ureq + rustls(纯 Rust,无 openssl/系统 C 依赖——
//! CI 的 mac-x64 是交叉编译,含 C 依赖会炸)。检测到代理环境变量则尊重
//! (`ureq::Proxy::try_from_env`:ALL_PROXY / HTTPS_PROXY / HTTP_PROXY,
//! 并自动处理 NO_PROXY),不要求用户有代理(零代理假设)。
//!
//! 各产物的 Mirror URL 常量也集中在本文(容错链定义见 ARCHITECTURE.md)。

use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

/// 单文件下载体积上限(Node 发行包约 50 MB,留足余量)
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Mirror URL 常量(每个产物一条容错链;fnm 主源华为云,其余 npmmirror 主源)
// ---------------------------------------------------------------------------

/// MinGit(Windows 便携版 git)版本与 tag。升级 = 改这两行并重测。
/// 核实来源:npmmirror git-for-windows 镜像,2026-08 时最新稳定为 v2.55.0.windows.1;
/// `.windows.1` 发行的 zip 文件名不带第四位(MinGit-<ver>-64-bit.zip)。
#[cfg(any(windows, test))]
pub const MINGIT_VERSION: &str = "2.55.0";
#[cfg(any(windows, test))]
pub const MINGIT_TAG: &str = "v2.55.0.windows.1";

/// MinGit zip 文件名
#[cfg(any(windows, test))]
pub fn mingit_archive_name() -> String {
    format!("MinGit-{MINGIT_VERSION}-64-bit.zip")
}

/// MinGit 下载 URL 容错链(npmmirror 主源 + CDN + 华为云兜底,均为国内可达 Mirror;
/// 已交叉核验三源 200/302 可达,zip 根目录即 cmd/git.exe 无顶层包裹目录)
#[cfg(any(windows, test))]
pub fn mingit_urls() -> Vec<String> {
    let file = mingit_archive_name();
    vec![
        format!("https://registry.npmmirror.com/-/binary/git-for-windows/{MINGIT_TAG}/{file}"),
        format!("https://cdn.npmmirror.com/binaries/git-for-windows/{MINGIT_TAG}/{file}"),
        format!("https://mirrors.huaweicloud.com/git-for-windows/{MINGIT_TAG}/{file}"),
    ]
}

/// fnm(Node 版本管理器)版本与 tag。升级 = 改这两行并重测。
/// 核实来源:GitHub Schniz/fnm releases/latest,2026-09 时最新稳定为 v1.39.0。
#[allow(dead_code)] // FNM_TAG 已用于 URL 链;VERSION 供执行层展示/自检,未接线(工单 #19+)
pub const FNM_VERSION: &str = "1.39.0";
pub const FNM_TAG: &str = "v1.39.0";

/// fnm 发行资产文件名:`fnm-<asset>.zip`(`asset` 由 platform::fnm_asset_suffix_for 决定:
/// macos 为 universal 二进制的 `macos`,linux x64 为 `linux`,linux arm64 为 `arm64`,windows 为 `windows`)
pub fn fnm_archive_name(asset: &str) -> String {
    format!("fnm-{asset}.zip")
}

/// fnm 下载 URL 容错链(华为云主源 + gh-proxy 加速 + GitHub Release 直连兜底)。
///
/// 与 Node/MinGit 链不同:npmmirror 并不镜像 fnm(`/-/binary/fnm/` 返回 NOT_FOUND,
/// 已于 2026-09 实测),故主源改用华为云;gh-proxy.com 为 GitHub Release 加速(国内可达),
/// GitHub 直连兜底(302 → release-assets.githubusercontent.com,ureq 默认跟随 10 次重定向)。
/// zip 内容单一:unix 为 `fnm`,Windows 为 `fnm.exe`,无顶层包裹目录。
pub fn fnm_urls(asset: &str) -> Vec<String> {
    let file = fnm_archive_name(asset);
    let github = format!("https://github.com/Schniz/fnm/releases/download/{FNM_TAG}/{file}");
    vec![
        format!("https://mirrors.huaweicloud.com/fnm/{FNM_TAG}/{file}"),
        format!("https://gh-proxy.com/{github}"),
        github,
    ]
}

/// omp(Oh My Pi)版本与 tag。升级 = 改这两行并重测。
/// 核实来源:GitHub can1357/oh-my-pi releases/latest,2026-09-11 时为 v18.1.17。
pub const OMP_VERSION: &str = "18.1.17";
pub const OMP_TAG: &str = "v18.1.17";

/// omp 预编译二进制下载 URL 容错链(gh-proxy 加速 + GitHub Release 直连兜底)。
///
/// npmmirror 不镜像 oh-my-pi 二进制(`/-/binary/oh-my-pi/` 实测 NOT_FOUND,2026-09-11),
/// 华为云同理无此项目,故与 fnm 链同形。资产为单文件免运行时二进制
/// (bun build --compile 产物,文件名如 omp-linux-x64,由 platform::omp_asset_name 定)。
pub fn omp_urls(asset: &str) -> Vec<String> {
    let github =
        format!("https://github.com/can1357/oh-my-pi/releases/download/{OMP_TAG}/{asset}");
    vec![format!("https://gh-proxy.com/{github}"), github]
}

/// 建 HTTP agent:尊重代理环境变量;超时由调用方定(大文件下载给足,体检探测要短)
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .proxy(ureq::Proxy::try_from_env())
        .user_agent(concat!("setup-coder/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// doctor 连通性体检端点:与下载容错链同源的 Mirror 根(只发轻量 HEAD,不下载大包)
pub fn connectivity_endpoints() -> Vec<(&'static str, &'static str)> {
    vec![
        ("npmmirror registry", "https://registry.npmmirror.com"),
        ("npmmirror CDN", "https://cdn.npmmirror.com"),
        ("华为云镜像", "https://mirrors.huaweicloud.com"),
    ]
}

/// 轻量连通性探测:HEAD 请求 + 短超时;有 HTTP 响应(无论状态码)即算连通,
/// 只有网络层错误才算不通。返回 HTTP 状态码或错误描述。
pub fn head_status(url: &str) -> Result<u16, String> {
    let agent = agent(Duration::from_secs(10));
    match agent.head(url).call() {
        Ok(resp) => Ok(resp.status().as_u16()),
        Err(ureq::Error::StatusCode(code)) => Ok(code),
        Err(e) => Err(e.to_string()),
    }
}

fn download_once(agent: &ureq::Agent, url: &str, dest: &Path) -> Result<(), Box<dyn Error>> {
    let resp = agent.get(url).call()?;
    if resp.status() != ureq::http::StatusCode::OK {
        return Err(format!("HTTP {}", resp.status()).into());
    }
    let mut body = resp.into_body();
    let mut reader = body.with_config().limit(MAX_DOWNLOAD_BYTES).reader();
    let mut file = fs::File::create(dest)?;
    io::copy(&mut reader, &mut file)?;
    Ok(())
}

/// Mirror 容错链:依次尝试 `urls`,首个成功者写入 `dest` 并返回命中的 URL;
/// 全部失败则汇总各源错误后报错。
pub fn download_first(urls: &[String], dest: &Path) -> Result<String, Box<dyn Error>> {
    // 大文件下载给足超时(国内慢网)
    let agent = agent(Duration::from_secs(30 * 60));
    let mut failures = Vec::new();
    for url in urls {
        match download_once(&agent, url, dest) {
            Ok(()) if looks_like_payload(dest) => return Ok(url.clone()),
            Ok(()) => {
                // HTTP 200 但内容不是合法产物(华为云对缺失资产返回 200 + HTML 错误页)
                let _ = fs::remove_file(dest);
                failures.push(format!("  {url}:HTTP 200 但内容不是可识别产物(镜像错误页)"));
            }
            Err(e) => {
                let _ = fs::remove_file(dest); // 不留下半截文件
                failures.push(format!("  {url}:{e}"));
            }
        }
    }
    Err(format!("所有 Mirror 均下载失败:\n{}", failures.join("\n")).into())
}

/// 产物魔数嗅探:本仓库下载的产物为 zip(`PK\x03\x04`,fnm/MinGit)、预编译可执行文件
/// (ELF `\x7fELF` = omp linux;`MZ` = omp windows;Mach-O = omp macOS)之一。
/// 华为云等镜像会对不存在的资产返回 HTTP 200 + HTML 错误页(实测 fnm-macos.zip),
/// 仅靠状态码无法识别,导致解压/执行才炸;下载后以首字节魔数快速判定,不符合即
/// 当作该源失败,容错链继续换下一镜像。
fn looks_like_payload(dest: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let n = fs::File::open(dest)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    if n < 2 {
        return false;
    }
    head.starts_with(b"PK\x03\x04")            // zip
        || head.starts_with(b"\x7fELF")        // ELF(linux)
        || head.starts_with(b"MZ")             // PE(windows)
        || head.starts_with(b"\xcf\xfa\xed\xfe") // Mach-O 64 LE(macOS)
        || head.starts_with(b"\xfe\xed\xfa\xcf") // Mach-O 64 BE
        || head.starts_with(b"\xca\xfe\xba\xbe") // Mach-O fat/universal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_mirrors_failing_reports_every_url() {
        let dir = std::env::temp_dir().join(format!("setup-coder-test-net-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("out.bin");
        // 不可达端口,立即失败
        let urls = vec![
            "http://127.0.0.1:1/a".to_string(),
            "http://127.0.0.1:1/b".to_string(),
        ];
        let err = download_first(&urls, &dest).unwrap_err().to_string();
        assert!(err.contains("/a"), "应列出第一个源:{err}");
        assert!(err.contains("/b"), "应列出第二个源:{err}");
        assert!(!dest.exists(), "失败不得留下半截文件");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn connectivity_endpoints_are_mirror_roots_over_https() {
        let endpoints = connectivity_endpoints();
        assert!(endpoints.len() >= 2, "至少覆盖 npmmirror 与一个下载源");
        for (name, url) in &endpoints {
            assert!(url.starts_with("https://"), "只允许 https:{url}");
            assert!(!name.is_empty());
        }
        // 与下载容错链同源:主源必须是 npmmirror
        assert!(endpoints[0].1.contains("npmmirror.com"));
    }

    #[test]
    fn head_status_reports_network_error_without_panic() {
        // 不可达端口,立即失败;应返回 Err 而非 panic
        assert!(head_status("http://127.0.0.1:1/").is_err());
    }

    #[test]
    fn fnm_archive_name_matches_release_layout() {
        assert_eq!(fnm_archive_name("macos"), "fnm-macos.zip");
        assert_eq!(fnm_archive_name("windows"), "fnm-windows.zip");
    }

    #[test]
    fn fnm_urls_form_a_mirror_chain() {
        let urls = fnm_urls("macos");
        assert!(urls.len() >= 3, "必须有容错链");
        for u in &urls {
            assert!(u.starts_with("https://"), "只允许 https:{u}");
            assert!(u.contains(FNM_TAG), "URL 应含 tag:{u}");
            assert!(
                u.ends_with(&fnm_archive_name("macos")),
                "URL 应含文件名:{u}"
            );
        }
        // npmmirror 不镜像 fnm(实测 404),故主源为华为云
        assert!(
            urls[0].contains("huaweicloud.com"),
            "主源应为华为云:{}",
            urls[0]
        );
        assert!(
            urls.iter().all(|u| !u.contains("npmmirror.com")),
            "fnm 链不应含 npmmirror(其不镜像 fnm)"
        );
        // 末位为 GitHub Release 直连兜底
        assert!(
            urls.last().unwrap().contains("github.com"),
            "兜底应为 GitHub 直连"
        );
    }

    #[test]
    fn omp_urls_form_a_mirror_chain() {
        let urls = omp_urls("omp-linux-x64");
        assert!(urls.len() >= 2, "必须有容错链");
        for u in &urls {
            assert!(u.starts_with("https://"), "只允许 https:{u}");
            assert!(u.contains(OMP_TAG), "URL 应含 tag:{u}");
            assert!(u.ends_with("omp-linux-x64"), "URL 应含文件名:{u}");
        }
        // npmmirror/华为云均不镜像 oh-my-pi 二进制(实测 NOT_FOUND),主源为 gh-proxy
        assert!(urls[0].contains("gh-proxy.com"), "主源应为 gh-proxy");
        assert!(
            urls.last().unwrap().starts_with("https://github.com/"),
            "兜底应为 GitHub 直连"
        );
    }

    #[test]
    fn payload_magic_recognizes_zip_and_executables() {
        let dir = std::env::temp_dir().join(format!("setup-coder-magic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let check = |bytes: &[u8]| {
            let p = dir.join("p");
            fs::write(&p, bytes).unwrap();
            looks_like_payload(&p)
        };
        assert!(check(b"PK\x03\x04aaaa"), "zip");
        assert!(check(b"\x7fELF\x02\x01"), "ELF(omp linux)");
        assert!(check(b"MZ\x90\x00\x03"), "PE(omp windows)");
        assert!(check(b"\xcf\xfa\xed\xfe\x07"), "Mach-O(omp macOS)");
        assert!(!check(b"<html>404</html>"), "镜像错误页应判负");
        assert!(!check(b""), "空文件应判负");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mingit_archive_name_matches_dist_layout() {
        // `.windows.1` 发行的 zip 文件名不带第四位(已在 npmmirror 镜像核实)
        assert_eq!(mingit_archive_name(), "MinGit-2.55.0-64-bit.zip");
        assert!(MINGIT_TAG.contains(MINGIT_VERSION));
    }

    #[test]
    fn mingit_urls_form_a_mirror_chain() {
        let urls = mingit_urls();
        assert!(urls.len() >= 3, "必须有容错链");
        for u in &urls {
            assert!(u.starts_with("https://"), "只允许 https:{u}");
            assert!(u.contains(MINGIT_TAG), "URL 应含 tag:{u}");
            assert!(u.ends_with(&mingit_archive_name()), "URL 应含文件名:{u}");
        }
        // 主源必须是 npmmirror(国内可达 Mirror)
        assert!(urls[0].contains("npmmirror.com"));
    }
}
