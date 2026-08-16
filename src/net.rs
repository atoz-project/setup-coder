//! 下载 + Mirror 容错链:依次尝试多个镜像源,首个成功者落盘。
//!
//! HTTP 客户端为 ureq + rustls(纯 Rust,无 openssl/系统 C 依赖——
//! CI 的 mac-x64 是交叉编译,含 C 依赖会炸)。检测到代理环境变量则尊重
//! (`ureq::Proxy::try_from_env`:ALL_PROXY / HTTPS_PROXY / HTTP_PROXY,
//! 并自动处理 NO_PROXY),不要求用户有代理(零代理假设)。

use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

/// 单文件下载体积上限(Node 发行包约 50 MB,留足余量)
const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

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
}
