//! `juicebar probe` —— 向指定 collection 发一帧原始数据，打印回包。
//!
//! 接新设备时的主力工具。**只发已知含义的读命令**：协议是从上位机里逆出来的，
//! 命令表里混着 `EnterUsbUpdateMode`、`ClearSetting` 这类会改设备状态的命令，
//! 盲试撞上去的后果不可逆。见 `docs/protocol.md` 第 1 节。

use anyhow::{Result, anyhow, bail};

use crate::hid;

/// 十六进制字符串转字节。允许空格和 `0x` 前缀：`04 00 ef` / `0x04,0x00`。
pub fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let cleaned: String = s
        .replace("0x", " ")
        .replace(',', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("");
    if cleaned.len() % 2 != 0 {
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

/// VGN 鼠标那套校验：`crc = 0x55 - sum(帧[0..15])`，再减 Report ID 落到帧[15]。
/// 全程 u8 回绕，原始 JS 依赖 Uint8Array 赋值的截断。
pub fn vgn_mouse_crc(frame: &mut [u8], report_id: u8) -> Result<()> {
    if frame.len() != 16 {
        bail!("VGN 鼠标帧必须是 16 字节，当前 {}", frame.len());
    }
    let sum = frame[..15].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    frame[15] = 0x55u8.wrapping_sub(sum).wrapping_sub(report_id);
    Ok(())
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
pub fn run(
    vid: u16,
    pid: u16,
    usage_page: Option<u16>,
    usage: Option<u16>,
    report_id: u8,
    bytes: &str,
    vgn_crc: bool,
    timeout_ms: u32,
    reads: u32,
    feature: bool,
    no_write: bool,
) -> Result<()> {
    let candidates: Vec<hid::HidInfo> = hid::enumerate()?
        .into_iter()
        .filter(|d| d.vid == vid && d.pid == pid)
        .filter(|d| usage_page.is_none_or(|u| d.usage_page == u))
        .filter(|d| usage.is_none_or(|u| d.usage == u))
        // 能发命令的通道：output 报文或 feature 报文，至少得有一样。
        // VGN 键盘走的就是后者——它的 vendor collection 输入输出都是 0，只有 feature。
        .filter(|d| if feature { d.feature_len > 0 } else { d.output_len > 0 })
        .collect();

    let target = match candidates.len() {
        0 => bail!(
            "没有匹配且能发{}报文的 collection。先跑 `juicebar scan` 看看有哪些，             注意 out（或 feat）列为 0 的通道发不了命令。",
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

    let mut frame = parse_hex(bytes)?;
    if vgn_crc {
        vgn_mouse_crc(&mut frame, report_id)?;
    }

    println!(
        "目标  UP:{:04X} U:{:04X}  in:{} out:{} feat:{}",
        target.usage_page, target.usage, target.input_len, target.output_len, target.feature_len
    );
    println!(
        "方式  {}",
        if feature { "feature 报文" } else { "output 报文 + input 回包" }
    );
    if no_write {
        println!("发送  （跳过，--no-write）");
    } else {
        println!("发送  [id {report_id:02X}] {}", hex_dump(&frame));
    }

    if feature {
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
