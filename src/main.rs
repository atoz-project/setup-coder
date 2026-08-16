//! setup-coder 入口:clap 解析,分发子命令。
//!
//! 面向中国网络环境的小白用户,用一条命令装好各类 AI 编程 CLI 及其前置依赖。

mod commands;

use clap::{CommandFactory, Parser, Subcommand};

/// 中文帮助模板(裸跑与各子命令 `--help` 共用)
const HELP_TEMPLATE: &str = "{before-help}{about-with-newline}\n用法: {usage}\n\n{all-args}{after-help}";

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

#[derive(Subcommand)]
enum Commands {
    /// 安装 Tool 及其 Prerequisite(Node.js、git),写入私有前缀
    #[command(help_template = HELP_TEMPLATE, next_help_heading = "选项", disable_help_flag = true)]
    Install(CommonArgs),
    /// 卸载:删除 Private Prefix,并按 state.json 回滚 PATH 等对外改动
    #[command(help_template = HELP_TEMPLATE, next_help_heading = "选项", disable_help_flag = true)]
    Uninstall(CommonArgs),
    /// 体检:检查 Private Prefix 内各 Tool 与 Prerequisite 的安装状态
    #[command(help_template = HELP_TEMPLATE, next_help_heading = "选项", disable_help_flag = true)]
    Doctor(CommonArgs),
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        // 裸跑 = 打印中文帮助(退出码 0,对小白友好)
        None => {
            Cli::command()
                .print_help()
                .expect("打印帮助信息失败");
            println!();
        }
        Some(Commands::Install(_)) => commands::install::run(),
        Some(Commands::Uninstall(_)) => commands::uninstall::run(),
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
