//! juicebar 的命令行入口。实现都在库那一半（见 `src/lib.rs`），这里只解析参数。
//!
//! 做协议逆向用的诊断工具（`scan`、`caps`、`probe`）不在这里，在 `examples/` 下，不进发布构建。

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use juicebar::cli;

#[derive(Parser)]
#[command(name = "juicebar", version, about = "无线键鼠电量托盘")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 读一次配置里每个 Device 的当前电量，一行一个。
    Status {
        /// 配置文件路径，默认 %APPDATA%\juicebar\config.toml
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// 没有配置就生成一份带注释的草稿；有配置就补上此刻在场、而配置里空着的 Endpoint 块。
    ///
    /// 只填空缺，用户写过的值和注释一概不动（见 docs/adr/0003）。
    ConfigRefresh {
        /// 配置文件路径，默认 %APPDATA%\juicebar\config.toml
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Status { config } => cli::status::run(config),
        Command::ConfigRefresh { config } => cli::config_refresh::run(config),
    }
}
