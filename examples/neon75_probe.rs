//! `cargo run --example neon75_probe` —— VGN Neon75 专用抓包与协议逆向调试工具。
//!
//! 针对 Neon75 2.4G 协议存疑（就绪位恒为 0、上位机缓冲区残留、缺少抓包验证等问题）提供专用调试命令：
//!   query          向 2.4G dongle 发送 0xF7 查询帧并尝试重试，解析回包就绪状态与电量
//!   diff-watch     持续监控 MI_02 feature 缓冲区（或监听输入），实时捕获并高亮字段跳变（适合配合 VGN Hub 抓包）
//!   send           发送指定或预设命令（如 f7、f6 0a、82），自动追加 CRC 并补零到 64 字节
//!   verify-offline 执行自动化关机/开机验证实验，记录状态演变

use std::io::{self, Write};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};

use juicebar::hid::{self, HidHandle, HidInfo};
use juicebar::sources::vgn_keyboard;

const NEON75_DONGLE_VID: u16 = 0x3151;
const NEON75_DONGLE_PID: u16 = 0x5038;
const NEON75_WIRED_VID: u16 = 0x3151;
const NEON75_WIRED_PID: u16 = 0x502F;

const FEATURE_USAGE_PAGE: u16 = 0xFFFF;
const FEATURE_USAGE: u16 = 0x0002;

const INPUT_USAGE_PAGE: u16 = 0xFFFF;
const INPUT_USAGE: u16 = 0x0001;

#[derive(Parser)]
#[command(name = "neon75_probe", about = "VGN Neon75 专用抓包调试与协议逆向工具")]
struct Cli {
    /// 目标是 2.4G dongle 还是有线本体（默认 dongle）
    #[arg(long)]
    wired: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 发送 0xF7 查询帧，观察是否就绪与读数
    Query {
        /// 最大重试次数
        #[arg(long, default_value_t = 15)]
        retries: u32,
        /// 每次重试等待毫秒数
        #[arg(long, default_value_t = 100)]
        delay_ms: u64,
    },
    /// 持续差分监听：当缓冲区字节变化时即刻打印 diff（配合 VGN Hub 抓包的利器）
    DiffWatch {
        /// 轮询间隔毫秒数（仅在 feature 模式）
        #[arg(long, default_value_t = 100)]
        interval_ms: u64,
        /// 改为监听 MI_01&col05 的纯输入中断报文（32 字节通道）
        #[arg(long)]
        listen_input: bool,
    },
    /// 发送自定义或预设命令并读取回包
    Send {
        /// 预设命令或自定义 hex："f7" / "f6" / "82" 或 "F7 00 00 00 00 00 00"
        command: String,
        /// 发送后连读几次
        #[arg(long, default_value_t = 3)]
        reads: u32,
        /// 每次读取间隔毫秒数
        #[arg(long, default_value_t = 100)]
        interval_ms: u64,
    },
    /// 引导执行离线/在线验证实验（验证就绪位是否真实反映设备在线）
    VerifyOffline,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (vid, pid) = if cli.wired {
        (NEON75_WIRED_VID, NEON75_WIRED_PID)
    } else {
        (NEON75_DONGLE_VID, NEON75_DONGLE_PID)
    };

    match cli.command {
        Commands::Query { retries, delay_ms } => cmd_query(vid, pid, retries, delay_ms),
        Commands::DiffWatch {
            interval_ms,
            listen_input,
        } => cmd_diff_watch(vid, pid, interval_ms, listen_input),
        Commands::Send {
            command,
            reads,
            interval_ms,
        } => cmd_send(vid, pid, &command, reads, interval_ms),
        Commands::VerifyOffline => cmd_verify_offline(vid, pid),
    }
}

fn locate_collection(vid: u16, pid: u16, up: u16, u: u16) -> Result<HidInfo> {
    let list = hid::enumerate()?;
    let matched = list
        .into_iter()
        .find(|d| d.vid == vid && d.pid == pid && d.usage_page == up && d.usage == u);
    matched.ok_or_else(|| {
        anyhow!(
            "未找到目标设备 (VID: 0x{vid:04X}, PID: 0x{pid:04X}, UP: 0x{up:04X}, U: 0x{u:04X})。请确保键盘/接收器已插入！"
        )
    })
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

fn now_str() -> String {
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

fn explain_neon75_frame(buf: &[u8]) -> String {
    // buf[0] 为 report id (0)
    // buf[1] 为就绪标志
    // buf[2] 为电量
    // buf[10] 为充电标志
    if buf.len() < 11 {
        return "报文过短，无法解析".into();
    }
    let header = buf[1];
    let level = buf[2];
    let sleep = buf[4];
    let charging = buf[10];

    if level == 0 && buf[4] == 0 && buf[5] == 0 {
        return "未就绪 (全零初始帧)".into();
    }
    let status_text = if sleep == 1 {
        "键盘休眠(sleep=1)"
    } else {
        "在线活跃(sleep=0)"
    };
    let charge_text = if charging != 0 { "充电中" } else { "未充电" };
    format!("{status_text} | 固件电量: {level}% | 状态位: 0x{header:02X} | 充电: {charge_text}")
}

fn print_buffer_diff(old: &[u8], new: &[u8]) {
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
    println!("{} 缓冲区检测到变化 ({} 处):", now_str(), changes.len());
    for (idx, o, n) in changes {
        let o_str = o
            .map(|b| format!("0x{b:02X} ({b})"))
            .unwrap_or_else(|| "--".into());
        let n_str = n
            .map(|b| format!("0x{b:02X} ({b})"))
            .unwrap_or_else(|| "--".into());
        let annotation = match idx {
            1 => " ←【首字节标志】(0x00/0x01)",
            2 => " ←【电量百分比】★",
            4 => " ←【休眠状态位】(0=活跃, 1=休眠)",
            10 => " ←【充电状态位】",
            _ => "",
        };
        println!("  下标 {idx:2} (0x{idx:02X}): {o_str} -> {n_str}{annotation}");
    }
}

fn resolve_command(input: &str) -> Result<Vec<u8>> {
    let s = input.trim().to_ascii_lowercase();
    let raw = match s.as_str() {
        "f7" | "dongle-data" => vec![0xF7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        "f6" | "init" => vec![0xF6, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00],
        "82" | "battery" => vec![0x82, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => {
            let cleaned: String = input
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
                .collect::<Result<Vec<u8>>>()?
        }
    };

    let mut padded = raw;
    if padded.len() < 7 {
        padded.resize(7, 0);
    }
    let frame = vgn_keyboard::build_frame(&padded)?;
    Ok(frame.to_vec())
}

fn cmd_query(vid: u16, pid: u16, retries: u32, delay_ms: u64) -> Result<()> {
    let target = locate_collection(vid, pid, FEATURE_USAGE_PAGE, FEATURE_USAGE)?;
    let handle = HidHandle::open_sync(&target)?;
    let request = vgn_keyboard::build_frame(&[0xF7, 0, 0, 0, 0, 0, 0])?;

    println!("==================================================");
    println!("Neon75 2.4G 连通性与电量读取测试");
    println!("目标设备: VID 0x{vid:04X} PID 0x{pid:04X} (MI_02 Feature 65B)");
    println!("请求帧:   [00] {}", hex_dump(&request));
    println!("==================================================");

    let mut got_valid = false;
    for attempt in 1..=retries {
        if attempt > 1 && delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(delay_ms));
        }

        handle.set_feature(0, &request)?;
        let response = handle.get_feature(0)?;

        let explanation = explain_neon75_frame(&response);
        let header = response.get(1).copied().unwrap_or(0xFF);
        let level = response.get(2).copied().unwrap_or(0);
        let valid = header <= 1 && level > 0 && level <= 100;

        println!(
            "{:>2}/{retries} | {} | {}",
            attempt,
            explanation,
            hex_dump(&response[..12])
        );

        if valid {
            println!("\n[√] 成功！成功读取到 Neon75 电量: {level}%！");
            got_valid = true;
            break;
        }
    }

    if !got_valid {
        println!("\n[!] 连发 {retries} 次后仍未读取到有效电量。");
        println!("可能原因：");
        println!("  1. 键盘处于休眠或已关机；");
        println!("  2. 键盘处于蓝牙档或有线档，2.4G 射频未激活。");
    }

    Ok(())
}

fn cmd_diff_watch(vid: u16, pid: u16, interval_ms: u64, listen_input: bool) -> Result<()> {
    if listen_input {
        let target = locate_collection(vid, pid, INPUT_USAGE_PAGE, INPUT_USAGE)?;
        let handle = HidHandle::open(&target)?;
        println!("==================================================");
        println!("正在监听纯输入中断通道 (MI_01&Col05 32B)...");
        println!("请操作键盘按键、打开 VGN Hub 或切换模式，观察有无报文到达。");
        println!("按 Ctrl+C 退出。");
        println!("==================================================");
        let mut last_buf = Vec::new();
        loop {
            if let Ok(buf) = handle.read_report(200) {
                if buf != last_buf {
                    print_buffer_diff(&last_buf, &buf);
                    println!("  完整报文: {}", hex_dump(&buf));
                    last_buf = buf;
                }
            }
        }
    } else {
        let target = locate_collection(vid, pid, FEATURE_USAGE_PAGE, FEATURE_USAGE)?;
        let handle = HidHandle::open_sync(&target)?;
        println!("==================================================");
        println!("正在以 {interval_ms}ms 间隔持续监控 Feature 缓冲区变化...");
        println!("★ 此时可在后台打开 VGN Hub、点击刷新、或开关键盘，实时捕获字节变化！");
        println!("按 Ctrl+C 退出。");
        println!("==================================================");

        let mut last_buf = handle.get_feature(0)?;
        println!("{} 初始缓冲区状态:", now_str());
        println!("  {}", hex_dump(&last_buf));
        println!("  解读: {}", explain_neon75_frame(&last_buf));

        loop {
            std::thread::sleep(Duration::from_millis(interval_ms));
            let buf = handle.get_feature(0)?;
            if buf != last_buf {
                print_buffer_diff(&last_buf, &buf);
                println!("  当前完整帧: {}", hex_dump(&buf));
                println!("  解读: {}", explain_neon75_frame(&buf));
                last_buf = buf;
            }
        }
    }
}

fn cmd_send(vid: u16, pid: u16, cmd_str: &str, reads: u32, interval_ms: u64) -> Result<()> {
    let frame = resolve_command(cmd_str)?;
    let target = locate_collection(vid, pid, FEATURE_USAGE_PAGE, FEATURE_USAGE)?;
    let handle = HidHandle::open_sync(&target)?;

    println!("目标设备: VID 0x{vid:04X} PID 0x{pid:04X}");
    println!("发送帧:   [00] {}", hex_dump(&frame));

    handle.set_feature(0, &frame)?;

    for i in 1..=reads.max(1) {
        if i > 1 && interval_ms > 0 {
            std::thread::sleep(Duration::from_millis(interval_ms));
        }
        let buf = handle.get_feature(0)?;
        println!(
            "回读 {:>2} | {} | {}",
            i,
            explain_neon75_frame(&buf),
            hex_dump(&buf)
        );
    }
    Ok(())
}

fn cmd_verify_offline(vid: u16, pid: u16) -> Result<()> {
    let target = locate_collection(vid, pid, FEATURE_USAGE_PAGE, FEATURE_USAGE)?;
    let handle = HidHandle::open_sync(&target)?;

    println!("==================================================");
    println!("Neon75 2.4G 在线/离线断电实验助手");
    println!("目的：验证就绪位与电量是否受键盘物理开机/关机影响。");
    println!("==================================================\n");

    println!("【第一阶段：基线采样】");
    println!("请确认键盘处于【2.4G 档】且处于【开机】状态。");
    print!("确认无误后请按回车键开始采样...");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;

    let base_req = vgn_keyboard::build_frame(&[0xF7, 0, 0, 0, 0, 0, 0])?;
    handle.set_feature(0, &base_req)?;
    let base_resp = handle.get_feature(0)?;
    println!("开机基线: {}", hex_dump(&base_resp[..12]));
    println!("基线解读: {}\n", explain_neon75_frame(&base_resp));

    println!("【第二阶段：关机观测】");
    println!("现在，请将键盘机身开关拨至【OFF（关机）】或切换至【有线/蓝牙档】。");
    println!("程序将进行 30 秒连续监测，观察缓冲区是否有变化...\n");

    let start = Instant::now();
    let mut last = base_resp;
    let mut changes_count = 0;
    while start.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(500));
        let buf = handle.get_feature(0)?;
        if buf != last {
            changes_count += 1;
            print_buffer_diff(&last, &buf);
            last = buf;
        }
    }

    if changes_count == 0 {
        println!("关机监测结束：30 秒内缓冲区【完全无变化】。");
    } else {
        println!("关机监测结束：监测到 {changes_count} 次变化。");
    }

    println!("\n【第三阶段：开机恢复】");
    println!("现在，请重新将键盘拨回【2.4G 档并开机】。");
    print!("开机后请按回车键开始监测恢复过程...");
    io::stdout().flush()?;
    line.clear();
    io::stdin().read_line(&mut line)?;

    println!("正在尝试发送 F7 请求并观察就绪状态恢复...");
    for i in 1..=15 {
        std::thread::sleep(Duration::from_millis(200));
        handle.set_feature(0, &base_req)?;
        let buf = handle.get_feature(0)?;
        let explanation = explain_neon75_frame(&buf);
        println!("{:>2} | {} | {}", i, explanation, hex_dump(&buf[..12]));
        if buf.get(1).copied().unwrap_or(0) == 1 {
            println!("\n[√] 成功捕获到开机后的就绪状态恢复！");
            return Ok(());
        }
    }

    println!("\n实验小结：");
    println!("就绪位未翻转。这表明接收器固件在无上位机特定初始化时不会自动完成空中握手。");
    println!("建议：保持 diff-watch 开启，运行一次 VGN Hub 读取电量，捕获其初始化触发序列。");
    Ok(())
}
