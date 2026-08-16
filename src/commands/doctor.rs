//! `doctor` 子命令:体检 Private Prefix 内的安装状态(只读,不改动任何东西)。
//!
//! 逐项输出 ✓/✗:平台/架构、Node.js、git、注册表各 Tool、PATH 是否生效、
//! Mirror 连通性;每个 ✗ 附「下一步」中文指引。全部通过退出码 0,否则非 0。
//! 未安装(无前缀)时不 panic,报告「尚未安装,先跑 install」。

use crate::net;
use crate::platform;
use crate::prefix::{Prefix, State};
use crate::registry;

pub fn run() {
    std::process::exit(doctor());
}

/// 返回进程退出码:0 = 全部通过,1 = 有待处理项
fn doctor() -> i32 {
    let mut failures = 0;
    println!("setup-coder 体检(只读,不改动任何东西)");
    println!();
    // 平台/架构:信息项,恒 ✓
    println!(
        "✓ 平台:{} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    let prefix = match Prefix::home() {
        Ok(p) => p,
        Err(e) => {
            println!("✗ 无法确定用户家目录:{e}");
            return 1;
        }
    };
    // 未安装状态:友好报告,不 panic
    if !prefix.root().exists() {
        println!(
            "✗ 尚未安装:Private Prefix 不存在({})",
            prefix.root().display()
        );
        println!("  → 下一步:运行 setup-coder install");
        println!();
        println!("体检未通过:1 项待处理。");
        return 1;
    }

    // 安装清单:既是 uninstall 回滚依据,也用于区分 Tool「装过但冒烟失败」与「从未安装」;
    // 清单损坏是明确的待处理项(冒烟仍是安装状态的真据)
    let state = match State::load(&prefix) {
        Ok(s) => Some(s),
        Err(e) => {
            println!("✗ 安装清单 state.json 不可用:{e}");
            println!("  → 下一步:重跑 setup-coder install 重建安装清单");
            failures += 1;
            None
        }
    };

    check(
        &mut failures,
        "Node.js",
        platform::version_output_of(&prefix.node_exe()).map(|v| format!("{v}(前缀内)")),
        "重跑 setup-coder install 修复 Node.js",
    );

    check(
        &mut failures,
        "git",
        platform::git_version(&prefix),
        platform::git_missing_hint(),
    );

    // 注册表各 Tool:Installed = shim 能启动并报出版本号(CONTEXT.md)。
    // 装过(清单有记录)但冒烟未通过 = ✗ 计失败;从未安装 = 信息项,不算待处理
    // (否则只装部分 Tool 的用户永远无法全过,见工单 #4 自审)。
    for tool in registry::all() {
        let shim = prefix.bin_dir().join(platform::shim_file_name(tool.bin));
        if let Some(v) = platform::version_output_of(&shim) {
            println!("✓ Tool {}:{v}", tool.name);
        } else if state
            .as_ref()
            .is_some_and(|s| s.tools.iter().any(|t| t.name == tool.name))
        {
            check(
                &mut failures,
                &format!("Tool {}", tool.name),
                None,
                &format!("运行 setup-coder install {}", tool.name),
            );
        } else {
            println!(
                "− Tool {}:未安装(需要时运行 setup-coder install {})",
                tool.name, tool.name
            );
        }
    }

    // PATH 是否生效:当前进程 PATH 是否含前缀 bin/
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    if platform::path_contains_dir(&path_var, &prefix.bin_dir()) {
        println!("✓ PATH:{} 已生效", prefix.bin_dir().display());
    } else {
        println!("✗ PATH:{} 不在当前 PATH 中", prefix.bin_dir().display());
        println!(
            "  → 下一步:{}",
            platform::path_not_effective_hint(&prefix.bin_dir())
        );
        failures += 1;
    }

    // Mirror 连通性:轻量 HEAD,不下载大包
    for (name, url) in net::connectivity_endpoints() {
        match net::head_status(url) {
            Ok(code) => println!("✓ 连通 {name}:HTTP {code}"),
            Err(e) => {
                println!("✗ 连通 {name}:{e}");
                println!("  → 下一步:确认网络已连接;如经代理上网,请设置 HTTPS_PROXY 后重试");
                failures += 1;
            }
        }
    }

    println!();
    if failures == 0 {
        println!("体检通过:全部正常。");
        0
    } else {
        println!("体检未通过:{failures} 项待处理,按上面的「下一步」逐项处理。");
        1
    }
}

/// 一项体检:`report` 为 Some(版本串) 则 ✓,None 则 ✗ + 下一步指引
fn check(failures: &mut u32, label: &str, report: Option<String>, hint: &str) {
    match report {
        Some(detail) => println!("✓ {label}:{detail}"),
        None => {
            println!("✗ {label}:未检测到或冒烟未通过");
            println!("  → 下一步:{hint}");
            *failures += 1;
        }
    }
}
