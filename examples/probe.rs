//! `cargo run --example probe` —— 向指定 collection 发一帧原始数据，打印回包。
//!
//! 接新设备时的主力工具。**只发已知含义的读命令**：协议是从上位机里逆出来的，
//! 命令表里混着 `EnterUsbUpdateMode`、`ClearSetting` 这类会改设备状态的命令，
//! 盲试撞上去的后果不可逆。见 `docs/protocol.md` 第 1 节。
//!
//! 与 `scan`、`caps` 一样是住在 `examples/` 里的诊断工具，不进发布构建，只调用库的公开面。

mod common;

use anyhow::{Result, anyhow, bail};
use clap::Parser;

use common::parse_hex_u16;
use juicebar::hid;
use juicebar::sources::vgn_mouse;

/// 向指定 collection 发一帧原始数据并打印回包。
///
/// 只发已知含义的读命令——命令表里混着会改设备状态的命令，盲试不可逆。
#[derive(Parser)]
struct Args {
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
    /// 要发的字节，如 "04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 ef"。`--listen` 时不需要
    bytes: Option<String>,
    /// 按 VGN 鼠标那套算法覆写帧末字节的校验和
    #[arg(long)]
    vgn_crc: bool,
    /// 等回包的毫秒数；`--listen` 时是整段监听时长
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

    /// 只听不发：打开一条**纯输入**通道（`out:0`）收输入报文，一个字节都不发出去。
    ///
    /// 用来查"请求走一条通道、应答从另一条回来"这种异步协议——那种通道用别的模式够不着，因为它们发不出命令。
    #[arg(long)]
    listen: bool,
}

fn main() -> Result<()> {
    let Args {
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
        listen,
    } = Args::parse();
    run(
        vid,
        pid,
        usage_page,
        usage,
        report_id,
        bytes.as_deref(),
        vgn_crc,
        timeout,
        reads,
        feature,
        no_write,
        listen,
    )
}

/// 十六进制字符串转字节。允许空格和 `0x` 前缀：`04 00 ef` / `0x04,0x00`。
fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s
        .replace("0x", " ")
        .replace(',', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("");
    if !cleaned.len().is_multiple_of(2) {
        bail!("十六进制位数是奇数：{cleaned}");
    }
    (0..cleaned.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&cleaned[i..i + 2], 16)
                .map_err(|e| anyhow!("解析 {} 失败: {e}", &cleaned[i..i + 2]))
        })
        .collect()
}

fn hex_dump(buf: &[u8]) -> String {
    buf.iter()
        .enumerate()
        .map(|(i, b)| {
            if i % 8 == 0 && i != 0 {
                format!(" {b:02X}")
            } else {
                format!("{b:02X}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[allow(clippy::too_many_arguments)]
fn run(
    vid: u16,
    pid: u16,
    usage_page: Option<u16>,
    usage: Option<u16>,
    report_id: u8,
    bytes: Option<&str>,
    vgn_crc: bool,
    timeout_ms: u32,
    reads: u32,
    feature: bool,
    no_write: bool,
    listen: bool,
) -> Result<()> {
    if listen && feature {
        bail!("--listen 与 --feature 互斥：feature 报文是主动回读，不存在'听'这回事");
    }
    if listen && no_write {
        bail!("--listen 本来就一个字节都不发，不必再加 --no-write");
    }

    let candidates: Vec<hid::HidInfo> = hid::enumerate()?
        .into_iter()
        .filter(|d| d.vid == vid && d.pid == pid)
        .filter(|d| usage_page.is_none_or(|u| d.usage_page == u))
        .filter(|d| usage.is_none_or(|u| d.usage == u))
        // 能发命令的通道：output 报文或 feature 报文，至少得有一样。
        // VGN 键盘走的就是后者——它的 vendor collection 输入输出都是 0，只有 feature。
        //
        // `--listen` 要的恰好相反：一条**发不出命令**的纯输入通道，所以它只看 input。
        .filter(|d| {
            if listen {
                d.input_len > 0
            } else if feature {
                d.feature_len > 0
            } else {
                d.output_len > 0
            }
        })
        .collect();

    let target = match candidates.len() {
        0 if listen => bail!(
            "没有匹配且收得到输入报文的 collection。先跑 `cargo run --example scan` 看看有哪些，注意 in 列为 0 的通道没有输入报文可听。"
        ),
        0 => bail!(
            "没有匹配且能发{}报文的 collection。先跑 `cargo run --example scan` 看看有哪些，注意 out（或 feat）列为 0 的通道发不了命令。",
            if feature { "feature" } else { "输出" }
        ),
        1 => candidates.into_iter().next().unwrap(),
        _ => {
            eprintln!("匹配到多条 collection，用 --usage-page / --usage 缩小范围：");
            for d in &candidates {
                eprintln!("    UP:{:04X} U:{:04X}  {}", d.usage_page, d.usage, d.path);
            }
            bail!("需要唯一目标");
        }
    };

    let mut frame = match (listen, bytes) {
        (true, _) => Vec::new(),
        (false, Some(b)) => parse_hex(b)?,
        (false, None) => bail!("要发的字节没给。只想听不发就加 --listen"),
    };
    if vgn_crc {
        // 与鼠标驱动共用同一份校验和：两处各存一份，迟早会有一处悄悄改错，
        // 而校验和错了的帧会被设备静默丢弃，看起来和"设备没反应"一模一样。
        vgn_mouse::apply_checksum(&mut frame, report_id)?;
    }

    println!(
        "目标  UP:{:04X} U:{:04X}  in:{} out:{} feat:{}",
        target.usage_page, target.usage, target.input_len, target.output_len, target.feature_len
    );
    println!(
        "方式  {}",
        match (listen, feature) {
            (true, _) => "只听不发（--listen）",
            (_, true) => "feature 报文",
            _ => "output 报文 + input 回包",
        }
    );
    if listen {
        println!("发送  （一个字节都不发）");
    } else if no_write {
        println!("发送  （跳过，--no-write）");
    } else {
        println!("发送  [id {report_id:02X}] {}", hex_dump(&frame));
    }

    if listen {
        // 不发命令，只把这条通道上飘过的输入报文收下来。
        //
        // **等的是别人触发的东西**（多半是厂商上位机去读了一次），所以这里的超时语义与别处
        // 相反：一次读超时是**常态**，不是结束条件。`--timeout` 在这个模式下是**整段监听时长**，
        // 内部用短超时轮着等，收够 `--reads` 份或者时间到才收工。
        //
        // 第一版把超时当成了结束条件，于是 `--timeout 3000 --reads 20` 实际只听了 3 秒就退出，
        // 还打印出"这段时间什么都没来"——一个会骗人的读数。
        const SLICE_MS: u32 = 500;
        let handle = hid::HidHandle::open(&target)?;
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.into());
        let want = reads.max(1);
        println!(
            "监听  {} 秒内最多收 {want} 份（一次读超时不算结束）",
            timeout_ms / 1000
        );
        let mut got = 0u32;
        while got < want && std::time::Instant::now() < deadline {
            if let Ok(buf) = handle.read_report(SLICE_MS) {
                got += 1;
                println!("收到{got} {}", hex_dump(&buf));
            }
        }
        if got == 0 {
            println!("（整段监听里这条通道上什么都没来）");
        }
    } else if feature {
        let handle = hid::HidHandle::open_sync(&target)?;
        if !no_write {
            handle.set_feature(report_id, &frame)?;
        }
        let buf = handle.get_feature(report_id)?;
        println!("回读  {}", hex_dump(&buf));
    } else {
        if no_write {
            bail!("--no-write 只对 --feature 有意义：output 通道不发就没有回包可读");
        }
        let handle = hid::HidHandle::open(&target)?;
        handle.write_report(report_id, &frame)?;
        for i in 0..reads.max(1) {
            match handle.read_report(timeout_ms) {
                Ok(buf) => println!("回包{} {}", i + 1, hex_dump(&buf)),
                Err(e) => {
                    println!("回包{} {e}", i + 1);
                    break;
                }
            }
        }
    }

    println!();
    println!("提示：回包第 0 字节是 Report ID，协议文档里的下标要 +1 才对得上。");
    Ok(())
}
