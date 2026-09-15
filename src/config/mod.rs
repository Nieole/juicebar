//! `config.toml` 这一头：读取、自举草稿的生成、只填空缺的补全，以及 `primary` 与 `[tray]` 的回写。
//!
//! 几样住在一处是有意的——它们说的是同一份文件的同一套 schema，分开住就会漂。
//! **读**走 `toml` + serde，要的是类型化的模型；**写**走 `toml_edit`，要的是保住用户
//! 写下的注释（见 `docs/adr/0003`）。
//!
//! 结构里只有**当前用得上**的字段。TOML 里其余的键（譬如 `[device.wireless_24g]` 里那个可选的 `address`）
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
//! **五个文件，一个公开面。**上面那句"住在一处"说的是这个模块，不是一个文件：schema、
//! 解析、以及全部公开面的 re-export 在这里，那张写死的实测身份表与"这台设备对上表里
//! 哪一条"的判定在 `known_devices`，草稿的生成与那批用户可见的文案在 `draft`，
//! `toml_edit` 那套格式保留的原地编辑（自动补空块、`primary` 与 `[tray]` 的回写、登记与新建 Device）
//! 在 `edit`，`[tray]` 表的键、取值与认不出时的处置在 `tray`。
//!
//! 拆开的是**内部安排**：加一台设备、改一句草稿文案、改回写机制这三件不相干的事从此
//! 各改一个文件，而调用点看见的仍然是 `config::<名字>` 那一处，一个字都不必改。

mod draft;
mod edit;
mod known_devices;
mod tray;

pub use draft::draft;
pub use edit::{Fill, Note, Pinned, Refreshed, pin_primary, refresh};
pub use edit::{NewDevice, add_device, register_ble, unregister_ble};
pub use edit::{TrayWritten, write_tray};
pub use known_devices::{KNOWN_DEVICES, KnownDevice, UnregisteredHid};
pub use tray::{PrimaryMark, TraySetting, TraySettings, Unrecognised};

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::endpoints::EndpointKind;
use crate::primary::PrimaryRule;
use crate::sources::level::LevelSource;
use crate::sources::{Driver, driver_for};

/// 配置文件的位置：`%APPDATA%\juicebar\config.toml`。日志与状态文件都在它旁边（`crate::state::path_beside_config`）。
///
/// "配置在哪"只能有一个答案：两处各算一遍，迟早会有一处算的是另一个路径，而那种错表现出来是"程序说没有配置，
/// 可我明明写了"。
pub fn default_path() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA").map_err(|_| {
        anyhow!("读不到环境变量 APPDATA，不知道配置文件该在哪（缺省是 %APPDATA%\\juicebar\\config.toml）")
    })?;
    Ok(Path::new(&appdata).join("juicebar").join("config.toml"))
}

/// 首次运行：扫一遍本机，把一份带注释的草稿（[`draft`]）写到 `path`，一个字都不印。托盘启动时没有配置文件就走这里
/// （`crate::shell`），叫人去看一眼的是一条通知（`crate::tray::config`）。
///
/// **只从无到有，绝不覆盖。**用 `create_new` 而不是先 `exists()` 再写：调用方已经查过文件不在了，但那之后到这里之间
/// 它可能被建出来（另一个 juicebar 实例、用户自己），而配置是用户的东西，宁可报一句错也不能把它盖掉。
///
/// [`draft`]: fn@draft
pub fn write_draft(path: &Path) -> Result<()> {
    let draft = draft(&crate::hid::enumerate()?);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("建不了目录 {}", dir.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("写不进配置 {}", path.display()))?;
    std::io::Write::write_all(&mut file, draft.as_bytes())
        .with_context(|| format!("写不进配置 {}", path.display()))?;
    Ok(())
}

/// 一份配置文件的全部内容。
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// 不属于任何单个 Device 的那些选项。`[general]` 整节缺席时全取缺省值。
    #[serde(default)]
    pub general: General,
    /// `[tray]`：托盘图标怎么画、菜单里写什么（ADR-0005 的八项设置，`tray.rs`）。整节缺席时全取缺省值；认不出的取值
    /// 也按缺省值处理，不让整份配置读不动（[`TraySettings::unrecognised`]）。
    #[serde(default)]
    pub tray: TraySettings,
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
///
/// **删掉的键照样静默忽略**，不让整份配置读不动：`show_unknown_ble`（未登记的蓝牙设备的去处是菜单"登记设备"，
/// parking lot Q286）还写在用户真在用的配置里，那份配置照读。
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
    /// 低电量阈值，百分比：电量**低于**它就算低电（`CONTEXT.md`「图标状态」）。缺省 20。
    ///
    /// 单台 `[[device]]` 可以覆盖（[`Device::low_battery`]），所以问"这台低不低电"要走
    /// [`Self::low_battery_for`]，不是直接读这一格：键鼠的电池容量和耗电差很多，同一个 20%
    /// 对两者的紧急程度并不相等。
    #[serde(default = "default_low_battery")]
    pub low_battery: u8,
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

fn default_low_battery() -> u8 {
    20
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
            poll_interval_wired: default_poll_interval_wired(),
            poll_interval_24g: default_poll_interval_24g(),
            poll_interval_bluetooth: default_poll_interval_bluetooth(),
            low_battery: default_low_battery(),
            stale_after: default_stale_after(),
            very_stale_after: default_very_stale_after(),
            pause_when_vendor_hub_running: default_pause_when_vendor_hub_running(),
            vendor_hub_processes: default_vendor_hub_processes(),
        }
    }
}

impl General {
    /// 这台 Device 的低电量阈值：它自己写了就用它自己的，没写就用 `[general]` 的。
    pub fn low_battery_for(&self, device: &Device) -> u8 {
        device.low_battery.unwrap_or(self.low_battery)
    }
}

/// 一个物理外设。它的几条 Endpoint 合在这一条下面。
#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    /// 稳定的机器可读标识，用于在配置里互相引用。
    pub id: String,
    /// 人类可读的名字，就是界面上显示的那一个。
    pub name: String,
    /// 用哪个协议驱动，见 `src/sources/`。只作用于 HID Endpoint，所以**配了 Wired 或 Dongle24G 才必填**，缺了是配置
    /// 错误（[`Config::parse`] 读的时候就核）；只走蓝牙的 Device 不写它——给它编一个驱动名，等于在配置里说假话
    /// （spec「Device 的 schema」）。取数要驱动时问 [`Self::protocol_driver`]。
    ///
    /// 存成字符串而不是枚举：配置里可以出现本次编译还没实现的驱动名，那该是取数时
    /// 那一行报"尚未实现"，而不是让整份配置读不动。认得哪些名字见
    /// `sources::driver_for`。
    #[serde(default)]
    pub driver: Option<String>,
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
    /// 这台设备自己的低电量阈值，覆盖 `[general]` 的 [`General::low_battery`]。缺席即跟着
    /// `[general]` 走——取值一律经 [`General::low_battery_for`]。
    #[serde(default)]
    pub low_battery: Option<u8>,
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

impl Device {
    /// 这台 Device 的协议驱动（`sources::driver_for`）。没写 `driver`、或者写的名字本次编译还没实现，交回说得出原因的错误
    /// ——取数那一行照写那一句。
    pub fn protocol_driver(&self) -> Result<&'static dyn Driver> {
        let name = self
            .driver
            .as_deref()
            .ok_or_else(|| anyhow!("没写 driver"))?;
        driver_for(name).ok_or_else(|| anyhow!("驱动 {name} 尚未实现"))
    }

    /// 配了 HID Endpoint（Wired、Dongle24G）却没写 `driver`：那两条要靠驱动才读得出电量，读的时候就说，不留到取数时。
    fn check_driver(&self) -> Result<()> {
        let hid: Vec<String> = EndpointKind::PRIORITY
            .into_iter()
            .filter(|kind| kind.hid_config_in(self).is_some())
            .map(|kind| kind.to_string())
            .collect();
        if self.driver.is_some() || hid.is_empty() {
            return Ok(());
        }
        Err(anyhow!(
            "Device \"{}\" 配了 {}，却没写 driver：HID Endpoint 要靠它挑协议驱动（见 src/sources/），只走蓝牙的 Device 才不写",
            self.id,
            hid.join("、")
        ))
    }
}

impl Config {
    /// 解析一段 TOML 文本。
    ///
    /// 类型对得上之外还核一件事：配了 Wired 或 Dongle24G 的 Device 写了 `driver`（[`Device::driver`]）。
    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).context("配置解析失败")?;
        for device in &config.devices {
            device.check_driver().context("配置解析失败")?;
        }
        Ok(config)
    }

    /// 从磁盘读一份配置。
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读不到配置 {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("配置 {} 有问题", path.display()))
    }

    /// 蓝牙地址是 `address` 的那台 Device（写法不同也算，[`BluetoothEndpoint::matches`]）；哪台都没登记着它就是 `None`。
    ///
    /// 菜单"登记设备"列不列一台扫到的蓝牙设备（`crate::tray::register`）与写回拒不拒这一次登记（[`register_ble`]）问的是
    /// 同一件事，只在这里判一次：两处各判一遍，改岔了菜单就会列出一项点了必被拒绝的"登记到"。
    pub fn device_with_bluetooth_address(&self, address: &str) -> Option<&Device> {
        self.devices.iter().find(|device| {
            device
                .bluetooth
                .as_ref()
                .is_some_and(|bluetooth| bluetooth.matches(address))
        })
    }
}
