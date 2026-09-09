//! `config.toml` 的读取。
//!
//! 结构里只有**当前用得上**的字段。TOML 里其余的键（`[general]`、`level_source`、
//! `[device.bluetooth]` …）会被静默忽略，等要用它们的那一步再加进来——这样用户的
//! 配置不必随实现进度反复改写，样例配置也可以先于代码写全。
//!
//! 字段名沿用 `CONTEXT.md` 的词：配置里的 `wireless_24g` 块在代码里叫
//! `dongle_24g`，因为它描述的正是 Dongle24G 这条 Endpoint。

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// 一份配置文件的全部内容。
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// 登记在册的 Device，顺序即配置里的书写顺序。
    #[serde(rename = "device", default)]
    pub devices: Vec<Device>,
}

/// 一个物理外设。它的几条 Endpoint 合在这一条下面。
#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    /// 稳定的机器可读标识，用于在配置里互相引用。
    pub id: String,
    /// 人类可读的名字，就是界面上显示的那一个。
    pub name: String,
    /// 用哪个协议驱动，见 `src/sources/`。只作用于 HID Endpoint。
    ///
    /// 存成字符串而不是枚举：配置里可以出现本次编译还没实现的驱动名
    /// （样例配置就写着 `vgn_keyboard`），那该是取数时报一行"尚未实现"，
    /// 而不是让整份配置读不动。
    pub driver: String,
    /// Device 通过 USB 线直连时出现的 Endpoint。
    ///
    /// 它有**独立于 Dongle24G 的另一组 VID/PID**，插线时作为一个额外的设备被枚举
    /// 出来，而不是替换掉 Dongle24G（见 `docs/adr/0001`）。所以这一块在自举时通常
    /// 缺席——只有插着线才扫得到。
    pub wired: Option<HidEndpoint>,
    /// 经 2.4G 接收器的 Endpoint。接收器自己有一组 VID/PID，与 Device 本体不同。
    #[serde(rename = "wireless_24g")]
    pub dongle_24g: Option<HidEndpoint>,
}

/// 一条 HID Endpoint 在本机的定位方式。
///
/// 认 collection 用 usage page / usage 而不是 `MI_xx` 路径——后者在不同机器上可能不一样。
#[derive(Debug, Clone, Deserialize)]
pub struct HidEndpoint {
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// 报文的 Report ID，不带编号的报文填 0。
    pub report_id: u8,
}

impl Config {
    /// 解析一段 TOML 文本。
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("配置解析失败")
    }

    /// 从磁盘读一份配置。
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读不到配置 {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("配置 {} 有问题", path.display()))
    }
}
