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
// Mirror URL 常量(每个产物一条容错链:npmmirror 主源 + CDN + 华为云兜底)
// ---------------------------------------------------------------------------

/// Node 发行包文件名:`node-vX.Y.Z-<suffix>.<ext>`
pub fn node_archive_name(version: &str, suffix: &str, ext: &str) -> String {
    format!("node-{version}-{suffix}.{ext}")
}

/// Node 下载 URL 容错链(npmmirror 主源 + CDN + 华为云兜底)
pub fn node_urls(version: &str, suffix: &str, ext: &str) -> Vec<String> {
    let file = node_archive_name(version, suffix, ext);
    vec![
        format!("https://registry.npmmirror.com/-/binary/node/{version}/{file}"),
        format!("https://cdn.npmmirror.com/binaries/node/{version}/{file}"),
        format!("https://mirrors.huaweicloud.com/nodejs/{version}/{file}"),
    ]
}

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

/// 建 HTTP agent:尊重代理环境变量,大文件下载给足超时(国内慢网)
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30 * 60)))
        .proxy(ureq::Proxy::try_from_env())
        .user_agent(concat!("setup-coder/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
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
    let agent = agent();
    let mut failures = Vec::new();
    for url in urls {
        match download_once(&agent, url, dest) {
            Ok(()) => return Ok(url.clone()),
            Err(e) => {
                let _ = fs::remove_file(dest); // 不留下半截文件
                failures.push(format!("  {url}:{e}"));
            }
        }
    }
    Err(format!("所有 Mirror 均下载失败:\n{}", failures.join("\n")).into())
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
    fn node_archive_name_matches_dist_layout() {
        assert_eq!(
            node_archive_name("v24.19.0", "darwin-arm64", "tar.gz"),
            "node-v24.19.0-darwin-arm64.tar.gz"
        );
        assert_eq!(
            node_archive_name("v24.19.0", "win-x64", "zip"),
            "node-v24.19.0-win-x64.zip"
        );
    }

    #[test]
    fn node_urls_form_a_mirror_chain() {
        let urls = node_urls("v24.19.0", "linux-x64", "tar.gz");
        assert!(urls.len() >= 2, "必须有容错链");
        for u in &urls {
            assert!(u.starts_with("https://"), "只允许 https:{u}");
            assert!(u.contains("/v24.19.0/node-v24.19.0-linux-x64.tar.gz"));
        }
        // 主源必须是 npmmirror(ADR-0002)
        assert!(urls[0].contains("npmmirror.com"));
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
