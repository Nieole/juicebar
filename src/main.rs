//! juicebar 的入口。实现都在库那一半（见 `src/lib.rs`），这里只分流：
//!
//! - **不带参数就是托盘**（`juicebar::shell`）。双击程序就是这一支。
//! - 过渡期还认 `status` 与 `config-refresh` 两条子命令（票 14 收掉），它们照旧往终端印字。
//!
//! 做协议逆向用的诊断工具（`scan`、`caps`、`probe`）不在这里，在 `examples/` 下，不进发布构建。
//!
//! **它是窗口程序，不是控制台程序**（`windows_subsystem = "windows"`）：双击托盘不该闪出一个黑框。代价落在
//! 那两条子命令上——窗口程序没有自己的控制台，所以它们先借启动它的那个终端（`AttachConsole`）：字印得
//! 出来，但终端不等它结束（字落在提示符之后），也拿不到它的退出码。从没提权的终端里跑会先弹 UAC，提权后
//! 的那个进程借不借得到原来的终端，没有验过。取舍记在 parking lot Q160。

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use juicebar::cli;
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

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
    if std::env::args_os().len() <= 1 {
        if let Err(e) = juicebar::shell::run() {
            juicebar::shell::report_fatal(&e);
        }
        return Ok(());
    }
    // SAFETY: 借父进程的控制台；没有（从资源管理器里带参数启动）就失败，字随之无处可印，无妨。
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    match Cli::parse().command {
        Command::Status { config } => cli::status::run(config),
        Command::ConfigRefresh { config } => cli::config_refresh::run(config),
    }
}
