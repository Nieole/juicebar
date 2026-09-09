//! `config.toml` 这一头：读取、自举草稿的生成、只填空缺的补全，以及 `primary` 的回写。
//!
//! 四样住在一处是有意的——它们说的是同一份文件的同一套 schema，分开住就会漂。
//! **读**走 `toml` + serde，要的是类型化的模型；**写**走 `toml_edit`，要的是保住用户
//! 写下的注释（见 `docs/adr/0003`）。
//!
//! 结构里只有**当前用得上**的字段。TOML 里其余的键（`[general]` 里还没人读的那几项 …）
//! 会被静默忽略，等要用它们的那一步再加进来——这样用户的配置不必随实现进度反复改写，
//! 样例配置也可以先于代码写全。**草稿照样把它们写出来**，因为草稿是给人读的：一个还没
//! 实现的开关，用户也该知道它存在。
//!
//! 一处有意的例外：`poll_interval_bluetooth`。三个轮询间隔是**一组**，而另两个是两条 HID
//! 陈旧阈值的来源（3 倍，见 `crate::staleness`），只解析其中两个会让下一个读这个结构的人
//! 以为漏了一个。它本身不参与陈旧判定——`Ble` 读的是 Windows 缓存，轮询得多勤也不会让缓存
//! 里的数字变新——真正读它的是常驻轮询那一步。
//!
//! 字段名沿用 `CONTEXT.md` 的词：配置里的 `wireless_24g` 块在代码里叫
//! `dongle_24g`，因为它描述的正是 Dongle24G 这条 Endpoint。
//!
//! **四个文件，一个公开面。**上面那句"住在一处"说的是这个模块，不是一个文件：schema、
//! 解析、以及全部公开面的 re-export 在这里，那张写死的实测身份表与"这台设备对上表里
//! 哪一条"的判定在 `known_devices`，草稿的生成与那批用户可见的文案在 `draft`，
//! `toml_edit` 那套格式保留的原地编辑（`config-refresh` 的补全与 `primary` 的回写）
//! 在 `edit`。
//!
//! 拆开的是**内部安排**：加一台设备、改一句草稿文案、改回写机制这三件不相干的事从此
//! 各改一个文件，而调用点看见的仍然是 `config::<名字>` 那一处，一个字都不必改。

mod draft;
mod edit;
mod known_devices;

pub use draft::draft;
pub use edit::{Pinned, Refreshed, pin_primary, refresh};
pub use known_devices::{KNOWN_DEVICES, KnownDevice};

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::primary::PrimaryRule;
use crate::sources::level::LevelSource;

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
    /// 托盘图标当下画哪个 Device，也就是 Primary Device：`"lowest"` 是当前电量最低的
    /// 那个，别的任何字符串都是要钉死的那个 Device 的 id（选的规则在 `crate::primary`）。
    ///
    /// 缺省是 `"lowest"`——托盘总得画一个，而这是 `config.example.toml` 里写着的推荐值。
    /// 那份样例里另外两条也是这一项的一部分，代码这边各有归宿：**参与 `"lowest"` 比较的
    /// 只有新鲜且可信的 Reading**（`crate::primary::select`），以及**这一项程序会回写**
    /// （[`pin_primary`]，格式保留的回写见 `docs/adr/0003`；回写的只该是菜单里的手动
    /// 选择，不是 `"lowest"` 自动选出来的结果，见 parking lot Q41）。
    #[serde(default)]
    pub primary: PrimaryRule,
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
    /// 检测到厂商上位机在运行时，暂停两条 HID Endpoint（`Ble` 照常）。缺省**开着**。
    ///
    /// 这不是防御性设计：键盘 dongle 的 feature 报文是一块保存最近一次应答的共享缓冲区，
    /// 两个程序同时发命令会互相覆盖对方的应答，双方都读到错数据。谁被暂停、为什么只暂停
    /// 两条 HID，见 `crate::vendor_hub`。
    ///
    /// 关着时**连进程都不去枚举**：一次进程枚举不便宜，而开关关着就意味着用户说了"别管
    /// 这件事"。那一步在 `crate::vendor_hub::VendorHub::detect` 里。
    #[serde(default = "default_pause_when_vendor_hub_running")]
    pub pause_when_vendor_hub_running: bool,
    /// 哪些进程算"厂商上位机"，按可执行文件名写。缺省是实测过的那一个。
    ///
    /// 缺省不是一份空名单：空名单会让暂停**永远不触发**，而那和"HUB 没在跑"长得一模一样
    /// ——没有任何东西会指向那个缺省。名字怎么比对（大小写、路径末段）见
    /// `crate::vendor_hub::VendorHub::detect`。
    #[serde(default = "default_vendor_hub_processes")]
    pub vendor_hub_processes: Vec<String>,
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

fn default_pause_when_vendor_hub_running() -> bool {
    true
}

/// 实测过的那个进程名，原样。大小写与用户在任务管理器里看到的一致。
fn default_vendor_hub_processes() -> Vec<String> {
    vec!["VGN VHUB.exe".to_string()]
}

impl Default for General {
    /// 整节 `[general]` 缺席时的取值。走的是逐字段 `#[serde(default = …)]` 用的同一批
    /// 函数——两处各写一份缺省是这个结构最容易出的错，共用一份就出不了。
    fn default() -> Self {
        Self {
            primary: PrimaryRule::default(),
            show_unknown_ble: false,
            poll_interval_wired: default_poll_interval_wired(),
            poll_interval_24g: default_poll_interval_24g(),
            poll_interval_bluetooth: default_poll_interval_bluetooth(),
            stale_after: default_stale_after(),
            very_stale_after: default_very_stale_after(),
            pause_when_vendor_hub_running: default_pause_when_vendor_hub_running(),
            vendor_hub_processes: default_vendor_hub_processes(),
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
    /// 这台设备的电量数值取自哪里，缺省 `"auto"`。
    ///
    /// **每个 Device 一项，没有全局默认**（spec「电量数值」）：想切成 `"reported"` 的
    /// 理由是"某型号固件 level 不准"，那本质上是设备相关的，不该有一个能连带影响其它
    /// 设备的全局值。
    ///
    /// 认不出的值**当场报错**，与上面的 `driver` 相反：那一项留到取数时才报，是因为
    /// 配置里可以出现本次编译还没实现的驱动名；而这一项只有两个合法值，静默按缺省走
    /// 意味着用户以为自己钉住了数值来源、其实没有——而他要钉住它的理由，恰恰是"这台
    /// 设备的另一个来源不可信"。
    #[serde(default)]
    pub level_source: LevelSource,
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
