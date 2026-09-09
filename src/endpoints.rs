//! 枚举接缝：回答"这个 Device 当前有哪些 Endpoint 在场"。
//!
//! 逻辑很薄，单独抽出来是为了让"拔线 → `Wired` 从枚举里消失 → 同周期降级到
//! `Dongle24G`"这条路径可测——那是实测确认过、也最容易写错的行为，而在真机上
//! 拔一次线才复现一次。
//!
//! 接缝只回答**在场**，不回答**先试谁**：优先级是 [`EndpointKind::PRIORITY`]，
//! 住在取数那一步。这样一份假枚举没法靠调换顺序改变取数的先后，用例断言的就真是
//! 优先级本身。

use anyhow::{Result, anyhow};

use crate::config::{Device, HidEndpoint};
use crate::hid::{self, HidInfo};
use crate::sources::hid_transport::{FeatureReportTransport, OutputReportTransport};
use crate::sources::{Reading, ReportKind, Transport, driver_for};

/// 一个 Device 上一条能取到读数的通路的种类。
///
/// `Ble` 是票 05 的活。它**不经过协议驱动**（走 `bluetooth.rs` 的 Windows 属性读取，
/// 不是一次 HID 往返），所以取数那一步对这个枚举 `match` 到底、不写通配分支——
/// 加进第三种时编译器会把人指到那一行去，而不是让 Ble 悄悄走上 HID 那条路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    /// Device 通过 USB 线直连时出现的 Endpoint。
    Wired,
    /// 经 2.4G 接收器的 Endpoint。
    Dongle24G,
}

impl EndpointKind {
    /// 取数的优先级，从高到低。依次尝试，遇第一个成功即停。
    ///
    /// `Wired` 排在前面不是因为它更快，而是因为**鼠标插线时 Dongle24G 会超时**：
    /// 设备不再经 2.4G 传数据，此刻能读到数的只有 Wired。不读有线，充电中的鼠标
    /// 就没有任何数据源，只能显示离线——而那是明确的误报（见 `docs/adr/0001`）。
    ///
    /// 它同时是**唯一的全变体清单**：要遍历所有种类的地方（列出配置过哪几条、
    /// 筛出哪几条在场）都走这里，于是加一种 Endpoint 只改这一行。
    pub const PRIORITY: [Self; 2] = [Self::Wired, Self::Dongle24G];

    /// 这个 Device 上这条 Endpoint 的配置块，没配就是 `None`。
    ///
    /// 返回 `HidEndpoint` 是因为现有两种都是 HID。票 05 的 `Ble` 配的是
    /// `[device.bluetooth]` 里一个蓝牙地址，不是这个类型——它要另一条路。
    pub fn config_in(self, device: &Device) -> Option<&HidEndpoint> {
        match self {
            Self::Wired => device.wired.as_ref(),
            Self::Dongle24G => device.dongle_24g.as_ref(),
        }
    }
}

impl std::fmt::Display for EndpointKind {
    /// 用 `CONTEXT.md` 的词，原样。这几个字会出现在 `status` 的每一行上。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Wired => "Wired",
            Self::Dongle24G => "Dongle24G",
        })
    }
}

/// 一份 [`Reading`]，连同它来自哪条 Endpoint。
///
/// `CONTEXT.md` 说的 Reading 是"电量、充电状态、电压、取得时刻，以及来自哪条
/// Endpoint"——它是分层拼起来的：[`Reading`] 是驱动从一帧里能解析出来的那部分，
/// **来源 Endpoint 由这一步补上**，取得时刻由 Clock 接缝那一步（票 06）补上，
/// 也补在这里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointReading {
    /// 这份读数是从哪条 Endpoint 上取到的。
    pub endpoint: EndpointKind,
    /// 驱动解析出来的那部分。
    pub reading: Reading,
}

/// 枚举接缝。
pub trait Endpoints {
    /// 这个 Device 当前在场的 Endpoint。
    ///
    /// 返回的是一个**集合**，顺序不表达任何东西。"在场"指本机此刻真的找得到它：
    /// 配置里没有这一块、或者配了但设备没插，都算不在场。
    fn present(&self, device: &Device) -> Vec<EndpointKind>;

    /// 打开一条在场的 Endpoint，拿到往返用的 Transport。
    ///
    /// 失败即"这条 Endpoint 这一次用不上"，与读不到是同一件事，调用方接着试下一条。
    /// 最常见的失败是厂商上位机正占着这条通路（票 07 会把它变成"暂停"而不是失败）。
    ///
    /// 现在只造 output+input 那一种 Transport。键盘的 vendor collection 只有 feature
    /// 报文、超时也不是 3000 ms，接它的时候这里要按协议家族分支——那是驱动那一维的事，
    /// 接缝把位置留在这里。
    fn open_transport(&self, device: &Device, endpoint: EndpointKind)
    -> Result<Box<dyn Transport>>;
}

/// 落到真 HID 上的枚举：一次 [`hid::enumerate`] 的快照。
///
/// 快照取一次、整个取数周期共用：一次枚举要打开本机每一条 HID collection，不便宜。
/// 也正因为是快照，"拔线"在一个周期里表现为 `Wired` 从名单上消失——那是**瞬时**的
/// 不在场，而不是一次三秒的超时，所以同周期降级几乎不要钱。
pub struct HidEndpoints {
    collections: Vec<HidInfo>,
}

impl HidEndpoints {
    /// 现在枚举一次。
    pub fn enumerate() -> Result<Self> {
        Ok(Self {
            collections: hid::enumerate()?,
        })
    }

    /// 在本机枚举到的 collection 里找配置指名的那一条，**限于发得出这种报文的**。
    ///
    /// 报文种类这一维躲不掉：键盘的 vendor collection 实测是 `in:0 out:0 feat:65`，
    /// 只认输出报文就会把它筛成"不在场"，而设备好端端插着——那是一句彻底误导人的
    /// "设备没插"。种类由驱动来答（[`ReportKind`]），不进配置。
    fn find_collection(&self, endpoint: &HidEndpoint, kind: ReportKind) -> Option<&HidInfo> {
        self.collections.iter().find(|c| {
            c.vid == endpoint.vid
                && c.pid == endpoint.pid
                && c.usage_page == endpoint.usage_page
                && c.usage == endpoint.usage
                && can_send(c, kind)
        })
    }

    /// 这个 Device 的驱动声明它走哪种报文。
    ///
    /// 认不出驱动名时给 `None`：配置里可以出现本次编译还没实现的驱动名，那该是取数
    /// 那一行写着"尚未实现"，而不是让这个 Device 的每条 Endpoint 都算不在场。
    fn report_kind_of(device: &Device) -> Option<ReportKind> {
        driver_for(&device.driver).map(|driver| driver.report_kind())
    }
}

/// 这条 collection 发得出这种报文吗。
///
/// 长度为 0 的那一侧根本发不出去，认它就是认错通路。
fn can_send(c: &HidInfo, kind: ReportKind) -> bool {
    match kind {
        ReportKind::OutputAndInput { .. } => c.output_len > 0,
        ReportKind::Feature => c.feature_len > 0,
    }
}

impl Endpoints for HidEndpoints {
    fn present(&self, device: &Device) -> Vec<EndpointKind> {
        let Some(report_kind) = Self::report_kind_of(device) else {
            // 驱动都认不出来，就没有"发得出哪种报文"可言，一条也算不上在场。
            return Vec::new();
        };
        EndpointKind::PRIORITY
            .into_iter()
            .filter(|kind| {
                kind.config_in(device)
                    .is_some_and(|endpoint| self.find_collection(endpoint, report_kind).is_some())
            })
            .collect()
    }

    fn open_transport(
        &self,
        device: &Device,
        endpoint: EndpointKind,
    ) -> Result<Box<dyn Transport>> {
        let configured = endpoint
            .config_in(device)
            .ok_or_else(|| anyhow!("这个 Device 没有配置 {endpoint}"))?;
        let report_kind = Self::report_kind_of(device)
            .ok_or_else(|| anyhow!("驱动 {} 尚未实现", device.driver))?;
        let collection = self.find_collection(configured, report_kind).ok_or_else(|| {
            anyhow!(
                "本机没有 VID {:04X} PID {:04X} UP {:04X} U {:04X} 这条发得出 {} 的通路（设备没插？）",
                configured.vid,
                configured.pid,
                configured.usage_page,
                configured.usage,
                match report_kind {
                    ReportKind::OutputAndInput { .. } => "输出报文",
                    ReportKind::Feature => "feature 报文",
                }
            )
        })?;
        // 超时是**协议**的性质，由驱动经 ReportKind 交出来，不在这里写死某一款设备的值。
        Ok(match report_kind {
            ReportKind::OutputAndInput { read_timeout_ms } => Box::new(OutputReportTransport::open(
                collection,
                configured.report_id,
                read_timeout_ms,
            )?) as Box<dyn Transport>,
            ReportKind::Feature => Box::new(FeatureReportTransport::open(
                collection,
                configured.report_id,
            )?),
        })
    }
}
