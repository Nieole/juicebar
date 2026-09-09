//! juicebar 的命令行入口。实现都在库那一半（见 `src/lib.rs`），这里只解析参数。

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

/// 十六进制入参统一用这个解析，写 `391d` 和 `0x391d` 都认。
fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).map_err(|e| format!("{s} 不是十六进制数: {e}"))
}

#[derive(Subcommand)]
enum Command {
    /// 读一次配置里每个 Device 的当前电量，一行一个。
    Status {
        /// 配置文件路径，默认 %APPDATA%\juicebar\config.toml
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// 列出所有 HID collection 和带电量属性的 BLE 设备。纯只读。
    Scan {
        /// 连不含厂商自定义通道的设备也一起列出
        #[arg(long)]
        all: bool,
    },

    /// 打印一条 collection 声明的 Report ID。纯本地，不向设备发任何字节。
    Caps {
        #[arg(long, value_parser = parse_hex_u16)]
        vid: u16,
        #[arg(long, value_parser = parse_hex_u16)]
        pid: u16,
        /// 缩小到某个 usage page，如 ffff
        #[arg(long, value_parser = parse_hex_u16)]
        usage_page: Option<u16>,
        /// 缩小到某个 usage，如 0002
        #[arg(long, value_parser = parse_hex_u16)]
        usage: Option<u16>,
    },

    /// 向指定 collection 发一帧原始数据并打印回包。
    ///
    /// 只发已知含义的读命令——命令表里混着会改设备状态的命令，盲试不可逆。
    Probe {
        #[arg(long, value_parser = parse_hex_u16)]
        vid: u16,
        #[arg(long, value_parser = parse_hex_u16)]
        pid: u16,
        /// 缩小到某个 usage page，如 ff02
        #[arg(long, value_parser = parse_hex_u16)]
        usage_page: Option<u16>,
        /// 缩小到某个 usage，如 0002
        #[arg(long, value_parser = parse_hex_u16)]
        usage: Option<u16>,
        /// Report ID，不带编号报文填 0
        #[arg(long, default_value_t = 0)]
        report_id: u8,
        /// 要发的字节，如 "04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 ef"
        bytes: String,
        /// 按 VGN 鼠标那套算法覆写帧末字节的校验和
        #[arg(long)]
        vgn_crc: bool,
        /// 等回包的毫秒数
        #[arg(long, default_value_t = 1000)]
        timeout: u32,
        /// 连读几份回包（设备可能先回无关报文）
        #[arg(long, default_value_t = 1)]
        reads: u32,
        /// 走 feature 报文而不是 output 报文。VGN 键盘的 vendor 通道只有 feature。
        #[arg(long)]
        feature: bool,
        /// 只 GetFeature、不 SetFeature。用来判断回读内容到底受不受所发命令影响。
        #[arg(long)]
        no_write: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Status { config } => cli::status::run(config),
        Command::Scan { all } => cli::scan::run(all),
        Command::Caps {
            vid,
            pid,
            usage_page,
            usage,
        } => cli::caps::run(vid, pid, usage_page, usage),
        Command::Probe {
            vid,
            pid,
            usage_page,
            usage,
            report_id,
            bytes,
            vgn_crc,
            timeout,
            reads,
            feature,
            no_write,
        } => cli::probe::run(
            vid, pid, usage_page, usage, report_id, &bytes, vgn_crc, timeout, reads, feature,
            no_write,
        ),
    }
}
