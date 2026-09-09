//! `config.toml` 的读取。
//!
//! 结构里只有**当前用得上**的字段。TOML 里其余的键（`level_source`、
//! `[general]` 里还没人读的那几项 …）会被静默忽略，等要用它们的那一步再加进来——这样
//! 用户的配置不必随实现进度反复改写，样例配置也可以先于代码写全。
//!
//! 一处有意的例外：`poll_interval_bluetooth`。三个轮询间隔是**一组**，而另两个是两条 HID
//! 陈旧阈值的来源（3 倍，见 `crate::staleness`），只解析其中两个会让下一个读这个结构的人
//! 以为漏了一个。它本身不参与陈旧判定——`Ble` 读的是 Windows 缓存，轮询得多勤也不会让缓存
//! 里的数字变新——真正读它的是常驻轮询那一步。
//!
//! 字段名沿用 `CONTEXT.md` 的词：配置里的 `wireless_24g` 块在代码里叫
//! `dongle_24g`，因为它描述的正是 Dongle24G 这条 Endpoint。

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// 一份配置文件的全部内容。
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// 不属于任何单个 Device 的那些选项。`[general]` 整节缺席时全取缺省值。
    #[serde(default)]
    pub general: General,
    /// 登记在册的 Device，顺序即配置里的书写顺序。
    #[serde(rename = "device", default)]
    pub devices: Vec<Device>,
}

/// `[general]` 里**当前用得上**的那几项。
///
/// **缺省值逐字段给，不靠 `#[derive(Default)]`。**那个派生会把每个间隔归零，而 3 × 0
/// 秒的陈旧阈值意味着每一份读数在取到的同一刻就已经陈旧——一份没写 `[general]` 的配置
/// 会因此让整个工具把所有读数都标成不可信。缺省的那几个数与 `config.example.toml` 的
/// 注释是同一份。
#[derive(Debug, Clone, Deserialize)]
pub struct General {
    /// 列表里是否显示未在 `[[device]]` 里登记的 BLE 设备。
    ///
    /// 说的是本机扫得到、而配置里没有对应 `[[device]]` 的那些。缺省是关的：一台机器
    /// 上的 BLE 设备多数跟键鼠无关（耳机、手机、手环），默认全列出来只会把两行有用的
    /// 埋掉。它是一条**独立的输出维度**，不是某个 Device 那一行上的事。
    #[serde(default)]
    pub show_unknown_ble: bool,
    /// Wired 的轮询间隔，秒。走线取数，设备还在外部供电，不耗它的电，可以勤一点。
    ///
    /// 它同时**决定 Wired 的陈旧阈值**（3 倍，见 `crate::staleness`）：改了间隔阈值
    /// 自动跟着走，不会出现"改了间隔忘了改阈值"的不一致——所以 HID 那两级不另设阈值
    /// 配置项。
    #[serde(default = "default_poll_interval_wired")]
    pub poll_interval_wired: u64,
    /// Dongle24G 的轮询间隔，秒。要往设备发无线包，耗设备的电，省着来。
    ///
    /// 与 `poll_interval_wired` 同理，它也是 Dongle24G 那一级陈旧阈值的来源。
    #[serde(default = "default_poll_interval_24g")]
    pub poll_interval_24g: u64,
    /// Ble 的轮询间隔，秒。纯本地属性读取，几乎免费。
    ///
    /// **它不参与陈旧判定**：那一级读的是 Windows 攒的缓存，读得多勤也不会让缓存里的
    /// 数字变新——它的新鲜度是 `stale_after` / `very_stale_after` 的事。
    #[serde(default = "default_poll_interval_bluetooth")]
    pub poll_interval_bluetooth: u64,
    /// 陈旧阈值，秒：取得时刻距今超过这么久，读数就被标注为陈旧。**只对 Ble 生效。**
    #[serde(default = "default_stale_after")]
    pub stale_after: u64,
    /// 陈旧得不该再显示数字的阈值，秒：超过它就只说这个数是哪一天的，不显示百分比。
    ///
    /// **只对 Ble 生效**；票 08 另拿它当持久化读数的丢弃期限。
    #[serde(default = "default_very_stale_after")]
    pub very_stale_after: u64,
}

fn default_poll_interval_wired() -> u64 {
    30
}

fn default_poll_interval_24g() -> u64 {
    60
}

fn default_poll_interval_bluetooth() -> u64 {
    10
}

fn default_stale_after() -> u64 {
    3_600
}

fn default_very_stale_after() -> u64 {
    86_400
}

impl Default for General {
    /// 整节 `[general]` 缺席时的取值。走的是逐字段 `#[serde(default = …)]` 用的同一批
    /// 函数——两处各写一份缺省是这个结构最容易出的错，共用一份就出不了。
    fn default() -> Self {
        Self {
            show_unknown_ble: false,
            poll_interval_wired: default_poll_interval_wired(),
            poll_interval_24g: default_poll_interval_24g(),
            poll_interval_bluetooth: default_poll_interval_bluetooth(),
            stale_after: default_stale_after(),
            very_stale_after: default_very_stale_after(),
        }
    }
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
    /// 存成字符串而不是枚举：配置里可以出现本次编译还没实现的驱动名，那该是取数时
    /// 那一行报"尚未实现"，而不是让整份配置读不动。认得哪些名字见
    /// `sources::driver_for`。
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
    /// 经蓝牙的 Endpoint。**不是 `HidEndpoint`**：它读的是 Windows 攒的设备属性，
    /// 既没有 collection 可认，也不经过 `driver` 那一维。
    pub bluetooth: Option<BluetoothEndpoint>,
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

/// 一条 Ble Endpoint 在本机的定位方式：一个蓝牙地址，别无其它。
///
/// 认设备只能靠 MAC。BLE 的 Device Information Service 只缓存了芯片型号，而同一
/// GUID 下的 VID/PID 在容器节点上实测全是空（见 `bluetooth.rs`）——MAC 是唯一可靠的
/// 实例标识。
#[derive(Debug, Clone, Deserialize)]
pub struct BluetoothEndpoint {
    /// 设备的蓝牙地址。大小写与 `:` / `-` 分隔随便写，比对时一律归一化。
    pub address: String,
}

impl BluetoothEndpoint {
    /// 配置里写的地址和 Windows 报的地址是不是同一台设备。
    ///
    /// 大小写和分隔符都容忍。这不是宽松：MAC 的写法太多（`E4:52:43:00:72:A9` /
    /// `e4-52-43-00-72-a9` / `e452430072a9` 都是同一台），而写法对不上的症状是
    /// **蓝牙那一级永远不在场**，它和"设备没配对"长得一模一样——没有任何东西会指向
    /// 那个多写的冒号。
    pub fn matches(&self, address: &str) -> bool {
        normalized_address(&self.address) == normalized_address(address)
    }
}

/// 去掉分隔符、统一成小写，好让两种写法的同一个 MAC 比得出相等。
fn normalized_address(address: &str) -> String {
    address
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
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
