//! `juicebar scan` —— 把本机所有可能藏着电量的通道摊出来。
//!
//! 这个子命令有两个用处：接新设备时用它找 vendor collection，以及首次运行时
//! 用它的结果生成配置草稿。纯只读，不向任何设备发一个字节。

use anyhow::Result;
use std::collections::BTreeMap;

use crate::bluetooth;
use crate::hid;

/// 从接口路径里抠出 `mi_01&col05` 这一段，配置文件里认 collection 时用得上。
fn interface_suffix(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    let mut parts: Vec<&str> = Vec::new();
    for seg in lower.split('#').nth(1).unwrap_or("").split('&') {
        if seg.starts_with("mi_") || seg.starts_with("col") {
            parts.push(seg);
        }
    }
    parts.join("&")
}

/// HID usage page 的通俗名。只覆盖会在键鼠上出现的那几个。
fn usage_page_name(page: u16, usage: u16) -> &'static str {
    match (page, usage) {
        (0x01, 0x02) => "鼠标",
        (0x01, 0x06) => "键盘",
        (0x01, 0x80) => "系统控制",
        (0x0C, 0x01) => "消费控制",
        (0x01, _) => "通用桌面",
        (0x84, _) | (0x85, _) => "电源/电池",
        (p, _) if p >= 0xFF00 => "★ 厂商自定义",
        _ => "",
    }
}

pub fn run(all: bool) -> Result<()> {
    println!("== HID collections ==\n");

    let mut devices = hid::enumerate()?;
    devices.sort_by_key(|d| (d.vid, d.pid, d.usage_page, d.usage));

    // 按 VID/PID 归组：一个物理接收器会暴露一串 collection，分开看没有意义。
    let mut groups: BTreeMap<(u16, u16), Vec<&hid::HidInfo>> = BTreeMap::new();
    for d in &devices {
        groups.entry((d.vid, d.pid)).or_default().push(d);
    }

    for ((vid, pid), items) in &groups {
        let vendor_count = items.iter().filter(|d| d.is_vendor_defined()).count();
        if !all && vendor_count == 0 {
            continue;
        }
        let name = items
            .iter()
            .map(|d| d.product.as_str())
            .chain(items.iter().map(|d| d.manufacturer.as_str()))
            .find(|s| !s.is_empty())
            .unwrap_or("(无产品名)");
        // 固件版本要显示出来：协议是按特定版本逆的，换了固件对不上时这是第一个线索。
        let rev = items.first().map(|d| d.version).unwrap_or(0);
        println!("VID_{vid:04X}&PID_{pid:04X}  REV_{rev:04X}  {name}");

        for d in items {
            let tag = usage_page_name(d.usage_page, d.usage);
            println!(
                "    {:<14} UP:{:04X} U:{:04X}  in:{:<3} out:{:<3} feat:{:<3} {}",
                if interface_suffix(&d.path).is_empty() {
                    "(单接口)".to_string()
                } else {
                    interface_suffix(&d.path)
                },
                d.usage_page,
                d.usage,
                d.input_len,
                d.output_len,
                d.feature_len,
                tag
            );
        }
        println!();
    }

    if !all {
        let hidden = groups
            .values()
            .filter(|v| v.iter().all(|d| !d.is_vendor_defined()))
            .count();
        if hidden > 0 {
            println!("（另有 {hidden} 个设备不含厂商自定义通道，已折叠。加 --all 全部列出。）\n");
        }
    }

    println!("== 蓝牙（BLE）电量 ==\n");
    let ble = bluetooth::enumerate()?;
    if ble.is_empty() {
        println!("没有找到带电量属性的 BLE 设备。\n");
    }
    for d in &ble {
        let level = d.level.map(|l| format!("{l}%")).unwrap_or("—".into());
        println!(
            "{:<32} {:<5} {:<12} {}",
            d.friendly_name,
            level,
            d.age_text(),
            if d.connected { "在线" } else { "离线" }
        );
        println!("    MAC {}", d.address);
    }

    println!();
    println!("提示：");
    println!("  · 私有协议通道一定落在 ★ 厂商自定义那几行，out 列必须非 0 才能发命令。");
    println!("  · 蓝牙读数是 Windows 的缓存，「多久前」那一列才是它的真实可信度。");
    println!("  · 用 `juicebar probe` 向某条 collection 发帧看回包。");

    Ok(())
}
