//! setup-coder 入口:clap 解析,分发子命令。
//!
//! 面向中国网络环境的小白用户,用一条命令装好各类 AI 编程 CLI 及其前置依赖。

mod commands;
mod net;
mod platform;
mod prefix;
mod registry;

use clap::{CommandFactory, Parser, Subcommand};

/// 中文帮助模板(裸跑与各子命令 `--help` 共用)
const HELP_TEMPLATE: &str =
    "{before-help}{about-with-newline}\n用法: {usage}\n\n{all-args}{after-help}";

#[derive(Parser)]
#[command(
    name = "setup-coder",
    version,
    about = "面向中国网络环境的 AI 编程 CLI 安装器(Win / macOS / Ubuntu)",
    help_template = HELP_TEMPLATE,
    subcommand_help_heading = "子命令",
    next_help_heading = "选项",
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    /// 显示帮助信息
    #[arg(short = 'h', long = "help", action = clap::ArgAction::Help)]
    help: Option<bool>,
    /// 显示版本号
    #[arg(short = 'V', long = "version", action = clap::ArgAction::Version)]
    version: Option<bool>,
    #[command(subcommand)]
    command: Option<Commands>,
}

/// 各子命令共用的中文 help 旗标(替代 clap 内置英文 -h)
#[derive(clap::Args)]
struct CommonArgs {
    /// 显示帮助信息
    #[arg(short = 'h', long = "help", action = clap::ArgAction::Help)]
    help: Option<bool>,
}

/// install 子命令参数
#[derive(clap::Args)]
struct InstallArgs {
    /// 只装指定 Tool(codex / claude-code / pi);不带 = 装全部
    tool: Option<String>,
    #[command(flatten)]
    common: CommonArgs,
}

/// uninstall 子命令参数
#[derive(clap::Args)]
struct UninstallArgs {
    /// 跳过确认提示,直接卸载
    #[arg(long)]
    yes: bool,
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Subcommand)]
enum Commands {
    /// 安装 Tool 及其 Prerequisite(Node.js、git),写入私有前缀
    #[command(help_template = HELP_TEMPLATE, next_help_heading = "选项", disable_help_flag = true)]
    Install(InstallArgs),
    /// 卸载:删除 Private Prefix,并按 state.json 回滚 PATH 等对外改动
    #[command(help_template = HELP_TEMPLATE, next_help_heading = "选项", disable_help_flag = true)]
    Uninstall(UninstallArgs),
    /// 体检:报告 Private Prefix 内各 Tool 与 Prerequisite 的安装状态、PATH 与连通性
    #[command(help_template = HELP_TEMPLATE, next_help_heading = "选项", disable_help_flag = true)]
    Doctor(CommonArgs),
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        // 裸跑 = 打印中文帮助(退出码 0,对小白友好)
        None => {
            Cli::command().print_help().expect("打印帮助信息失败");
            println!();
        }
        Some(Commands::Install(args)) => commands::install::run(args.tool),
        Some(Commands::Uninstall(args)) => commands::uninstall::run(args.yes),
        Some(Commands::Doctor(_)) => commands::doctor::run(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn bare_run_parses_to_no_subcommand() {
        let cli = Cli::try_parse_from(["setup-coder"]).expect("裸跑应能解析");
        assert!(cli.command.is_none());
    }

    #[test]
    fn subcommands_parse() {
        for (args, name) in [
            (["setup-coder", "install"], "install"),
            (["setup-coder", "uninstall"], "uninstall"),
            (["setup-coder", "doctor"], "doctor"),
        ] {
            Cli::try_parse_from(args).unwrap_or_else(|e| panic!("子命令 {name} 应能解析: {e}"));
        }
        Cli::try_parse_from(["setup-coder", "install", "codex"])
            .expect("install 带 Tool 参数应能解析");
    }

    #[test]
    fn uninstall_yes_flag_parses() {
        let cli = Cli::try_parse_from(["setup-coder", "uninstall"]).unwrap();
        let Some(Commands::Uninstall(args)) = cli.command else {
            panic!("应解析为 uninstall")
        };
        assert!(!args.yes, "不带 --yes = 需要确认");

        let cli = Cli::try_parse_from(["setup-coder", "uninstall", "--yes"]).unwrap();
        let Some(Commands::Uninstall(args)) = cli.command else {
            panic!("应解析为 uninstall")
        };
        assert!(args.yes);
    }

    #[test]
    fn install_tool_arg_is_optional() {
        let cli = Cli::try_parse_from(["setup-coder", "install"]).unwrap();
        let Some(Commands::Install(args)) = cli.command else {
            panic!("应解析为 install")
        };
        assert!(args.tool.is_none(), "不带参数 = 装全部");

        let cli = Cli::try_parse_from(["setup-coder", "install", "pi"]).unwrap();
        let Some(Commands::Install(args)) = cli.command else {
            panic!("应解析为 install")
        };
        assert_eq!(args.tool.as_deref(), Some("pi"));
    }

    #[test]
    fn version_flag_prints_version() {
        match Cli::try_parse_from(["setup-coder", "--version"]) {
            Err(e) => assert_eq!(e.kind(), clap::error::ErrorKind::DisplayVersion),
            Ok(_) => panic!("--version 应触发 DisplayVersion 而非解析成功"),
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["setup-coder", "bogus"]).is_err());
    }
}
