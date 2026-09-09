//! `juicebar caps` —— 打印一条 collection 声明的 Report ID。纯本地，不向设备发任何字节。
//!
//! 存在的理由见 `hid::report_ids` 的文档注释：feature 报文的读写都拿缓冲区第 0 字节当
//! 「要哪份 report」的入参，传错编号时驱动可能把缓冲区残留原样交回来，而那看起来和正常
//! 回包毫无区别。接新设备时先跑这个，能省掉一整轮基于错误前提的推理。

use anyhow::{Result, bail};

use crate::hid;

pub fn run(vid: u16, pid: u16, usage_page: Option<u16>, usage: Option<u16>) -> Result<()> {
    let targets: Vec<hid::HidInfo> = hid::enumerate()?
        .into_iter()
        .filter(|d| d.vid == vid && d.pid == pid)
        .filter(|d| usage_page.is_none_or(|u| d.usage_page == u))
        .filter(|d| usage.is_none_or(|u| d.usage == u))
        .collect();

    if targets.is_empty() {
        bail!("没有匹配的 collection，先跑 `juicebar scan` 看看有哪些");
    }

    for t in &targets {
        println!(
            "UP:{:04X} U:{:04X}  in:{} out:{} feat:{}",
            t.usage_page, t.usage, t.input_len, t.output_len, t.feature_len
        );
        match hid::report_ids(t) {
            Ok(ids) => {
                for (name, list, len) in [
                    ("input  ", &ids.input, t.input_len),
                    ("output ", &ids.output, t.output_len),
                    ("feature", &ids.feature, t.feature_len),
                ] {
                    if len == 0 {
                        println!("    {name}  （无此方向的报文）");
                    } else if list.is_empty() {
                        println!("    {name}  报文长 {len}，但描述符里没有任何 caps 项");
                    } else {
                        let shown: Vec<String> =
                            list.iter().map(|i| format!("0x{i:02X}")).collect();
                        let note = if list == &[0u8] {
                            "  ← 不带编号，缓冲区第 0 字节必须填 0"
                        } else {
                            "  ← 带编号，缓冲区第 0 字节必须填其中之一"
                        };
                        println!(
                            "    {name}  报文长 {len}，Report ID: {}{note}",
                            shown.join(" ")
                        );
                    }
                }
            }
            Err(e) => println!("    读取失败: {e}"),
        }
        println!();
    }
    Ok(())
}
