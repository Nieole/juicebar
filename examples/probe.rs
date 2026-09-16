//! `cargo run --example probe` —— 向指定 collection 发一帧原始数据，打印回包。
//!
//! 接新设备时的主力工具。**只发已知含义的读命令**：协议是从上位机里逆出来的，
//! 命令表里混着 `EnterUsbUpdateMode`、`ClearSetting` 这类会改设备状态的命令，
//! 盲试撞上去的后果不可逆。见 `docs/protocol.md` 第 1 节。
//!
//! 与 `scan`、`caps` 一样是住在 `examples/` 里的诊断工具，不进发布构建，只调用库的公开面。

mod common;

use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use clap::Parser;

use common::parse_hex_u16;
use juicebar::hid;
use juicebar::sources::{vgn_keyboard, vgn_mouse};

/// 向指定 collection 发一帧原始数据并打印回包。
///
/// 只发已知含义的读命令——命令表里混着会改设备状态的命令，盲试不可逆。
#[derive(Parser)]
struct Args {
    #[arg(long, value_parser = parse_hex_u16)]
    vid: u16,
    #[arg(long, value_parser = parse_hex_u16)]
    pid: u16,
    /// 缩小到某个 usage page，如 ff02、ffff
    #[arg(long, value_parser = parse_hex_u16)]
    usage_page: Option<u16>,
    /// 缩小到某个 usage，如 0002
    #[arg(long, value_parser = parse_hex_u16)]
    usage: Option<u16>,
    /// Report ID，不带编号报文填 0
    #[arg(long, default_value_t = 0)]
    report_id: u8,
    /// 要发的字节，如 "04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 ef" 或 "f7"。`--listen` 或 `--no-write` 时不需要
    bytes: Option<String>,
    /// 按 VGN 鼠标那套算法覆写帧末字节的校验和（16 字节帧，0x55 - sum - report_id）
    #[arg(long)]
    vgn_crc: bool,
    /// 按 VGN 键盘（Neon75）那套算法追加校验和并补零到 64 字节（255 - sum）
    #[arg(long, visible_alias = "kbd-crc")]
    vgn_keyboard_crc: bool,
    /// 等回包的毫秒数；`--listen` 时是整段监听时长
    #[arg(long, default_value_t = 1000)]
    timeout: u32,
    /// 连读几份回包（设备可能先回无关报文；feature 模式下为轮询读取次数）
    #[arg(long, default_value_t = 1)]
    reads: u32,
    /// 连续读取时每次之间的等待毫秒数（仅在 feature 报文或多轮读生效）
    #[arg(long, default_value_t = 100)]
    interval: u64,
    /// 走 feature 报文而不是 output 报文。VGN 键盘的 vendor 通道只有 feature。
    #[arg(long)]
    feature: bool,
    /// 只 GetFeature、不 SetFeature。用来判断回读内容到底受不受所发命令影响。
    #[arg(long)]
    no_write: bool,
    /// 持续差分监控：循环读取并比对，当且仅当缓冲区字节发生变化时打印 diff
    #[arg(long)]
    watch: bool,
    /// 解析回包已知字段的含义（支持 "neon75" 或 "mouse"）
    #[arg(long)]
    parse: Option<String>,

    /// 只听不发：打开一条**纯输入**通道（`out:0`）收输入报文，一个字节都不发出去。
    ///
    /// 用来查"请求走一条通道、应答从另一条回来"这种异步协议——那种通道用别的模式够不着，因为它们发不出命令。
    #[arg(long)]
    listen: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    run(args)
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

fn print_diff(old: &[u8], new: &[u8]) {
    let max_len = old.len().max(new.len());
    let mut changes = Vec::new();
    for i in 0..max_len {
        let o = old.get(i).copied();
        let n = new.get(i).copied();
        if o != n {
            changes.push((i, o, n));
        }
    }
    if changes.is_empty() {
        return;
    }
    let now = chrono_like_now();
    println!("{now} 缓冲区发生变化 ({} 处):", changes.len());
    for (idx, o, n) in changes {
        let o_str = o
            .map(|b| format!("{b:02X} ({b})"))
            .unwrap_or_else(|| "--".into());
        let n_str = n
            .map(|b| format!("{b:02X} ({b})"))
            .unwrap_or_else(|| "--".into());
        let note = match idx {
            1 => " [Neon75 就绪位?]",
            2 => " [Neon75 电量百分比?]",
            10 => " [Neon75 充电标志?]",
            _ => "",
        };
        println!("  下标 {idx:2} (0x{idx:02X}): {o_str} -> {n_str}{note}");
    }
}

fn chrono_like_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let ms = d.subsec_millis();
    let sec_in_day = secs % 86400;
    let hours = (sec_in_day / 3600 + 8) % 24;
    let mins = (sec_in_day % 3600) / 60;
    let s = sec_in_day % 60;
    format!("[{hours:02}:{mins:02}:{s:02}.{ms:03}]")
}

fn annotate_frame(kind: Option<&str>, buf: &[u8]) -> Option<String> {
    match kind {
        Some("neon75") => {
            if buf.len() >= 11 {
                let ready = buf[1];
                let level = buf[2];
                let charging = buf[10];
                let ready_str = match ready {
                    0 => "未就绪(0)",
                    1 => "已就绪(1)",
                    other => return Some(format!("就绪位异常({other:#02X})")),
                };
                let charge_str = if charging != 0 {
                    "充电中"
                } else {
                    "未充电"
                };
                Some(format!(
                    "Neon75解读: {ready_str}, 电量={level}%, 充电={charge_str}"
                ))
            } else {
                None
            }
        }
        Some("mouse") => {
            if buf.len() >= 10 && buf[1] == 4 {
                let level = buf[6];
                let charging = if buf[7] != 0 { "充电中" } else { "未充电" };
                let voltage = u16::from_be_bytes([buf[8], buf[9]]);
                Some(format!(
                    "鼠标解读: 电量={level}%, 充电={charging}, 电压={voltage}mV"
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn run(args: Args) -> Result<()> {
    let Args {
        vid,
        pid,
        usage_page,
        usage,
        report_id,
        bytes,
        vgn_crc,
        vgn_keyboard_crc,
        timeout: timeout_ms,
        reads,
        interval,
        feature,
        no_write,
        watch,
        parse,
        listen,
    } = args;

    if listen && feature {
        bail!("--listen 与 --feature 互斥：feature 报文是主动回读，不存在'听'这回事");
    }
    if listen && no_write {
        bail!("--listen 本来就一个字节都不发，不必再加 --no-write");
    }
    if vgn_crc && vgn_keyboard_crc {
        bail!("--vgn-crc（鼠标）与 --vgn-keyboard-crc（键盘）互斥");
    }

    let candidates: Vec<hid::HidInfo> = hid::enumerate()?
        .into_iter()
        .filter(|d| d.vid == vid && d.pid == pid)
        .filter(|d| usage_page.is_none_or(|u| d.usage_page == u))
        .filter(|d| usage.is_none_or(|u| d.usage == u))
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

    let mut frame = match (listen || no_write, bytes.as_deref()) {
        (true, _) => Vec::new(),
        (false, Some(b)) => parse_hex(b)?,
        (false, None) => bail!(
            "要发的字节没给。只想听不发就加 --listen，只想纯读 feature 报文加 --no-write"
        ),
    };

    if vgn_crc {
        vgn_mouse::apply_checksum(&mut frame, report_id)?;
    } else if vgn_keyboard_crc {
        if frame.len() < 7 {
            frame.resize(7, 0);
        }
        let full = vgn_keyboard::build_frame(&frame)?;
        frame = full.to_vec();
    }

    println!(
        "目标  UP:{:04X} U:{:04X}  in:{} out:{} feat:{}",
        target.usage_page, target.usage, target.input_len, target.output_len, target.feature_len
    );
    println!(
        "方式  {}",
        match (listen, feature, watch) {
            (true, _, true) => "输入监听 + 持续差分监控（--listen --watch）",
            (true, _, false) => "只听不发（--listen）",
            (_, true, true) => "feature 报文 + 持续差分监控（--feature --watch）",
            (_, true, false) => "feature 报文",
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

    let parse_type = parse.as_deref().or(if vgn_keyboard_crc {
        Some("neon75")
    } else if vgn_crc {
        Some("mouse")
    } else {
        None
    });

    if listen {
        const SLICE_MS: u32 = 500;
        let handle = hid::HidHandle::open(&target)?;
        if watch {
            println!("监控  持续监听输入通道变化（按 Ctrl+C 退出）...");
            let mut last_buf = Vec::new();
            loop {
                if let Ok(buf) = handle.read_report(SLICE_MS) {
                    if buf != last_buf {
                        print_diff(&last_buf, &buf);
                        println!("      当前报文: {}", hex_dump(&buf));
                        last_buf = buf;
                    }
                }
            }
        } else {
            let deadline = Instant::now() + Duration::from_millis(timeout_ms.into());
            let want = reads.max(1);
            println!(
                "监听  {} 秒内最多收 {want} 份（一次读超时不算结束）",
                timeout_ms / 1000
            );
            let mut got = 0u32;
            while got < want && Instant::now() < deadline {
                if let Ok(buf) = handle.read_report(SLICE_MS) {
                    got += 1;
                    println!("收到{got} {}", hex_dump(&buf));
                    if let Some(note) = annotate_frame(parse_type, &buf) {
                        println!("      {note}");
                    }
                }
            }
            if got == 0 {
                println!("（整段监听里这条通道上什么都没来）");
            }
        }
    } else if feature {
        let handle = hid::HidHandle::open_sync(&target)?;
        if !no_write {
            handle.set_feature(report_id, &frame)?;
        }

        if watch {
            println!("监控  以 {interval}ms 间隔持续轮询 feature 缓冲区变化（按 Ctrl+C 退出）...");
            let mut last_buf = handle.get_feature(report_id)?;
            println!("初始  {}", hex_dump(&last_buf));
            if let Some(note) = annotate_frame(parse_type, &last_buf) {
                println!("      {note}");
            }
            loop {
                std::thread::sleep(Duration::from_millis(interval));
                let buf = handle.get_feature(report_id)?;
                if buf != last_buf {
                    print_diff(&last_buf, &buf);
                    println!("      当前完整帧: {}", hex_dump(&buf));
                    if let Some(note) = annotate_frame(parse_type, &buf) {
                        println!("      {note}");
                    }
                    last_buf = buf;
                }
            }
        } else {
            let count = reads.max(1);
            for i in 0..count {
                if i > 0 && interval > 0 {
                    std::thread::sleep(Duration::from_millis(interval));
                }
                let buf = handle.get_feature(report_id)?;
                if count > 1 {
                    println!("回读{} {}", i + 1, hex_dump(&buf));
                } else {
                    println!("回读  {}", hex_dump(&buf));
                }
                if let Some(note) = annotate_frame(parse_type, &buf) {
                    println!("      {note}");
                }
            }
        }
    } else {
        if no_write {
            bail!("--no-write 只对 --feature 有意义：output 通道不发就没有回包可读");
        }
        let handle = hid::HidHandle::open(&target)?;
        handle.write_report(report_id, &frame)?;
        for i in 0..reads.max(1) {
            match handle.read_report(timeout_ms) {
                Ok(buf) => {
                    println!("回包{} {}", i + 1, hex_dump(&buf));
                    if let Some(note) = annotate_frame(parse_type, &buf) {
                        println!("      {note}");
                    }
                }
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
