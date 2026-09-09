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

use crate::bluetooth::{self, BleBattery};
use crate::clock::Timestamp;
use crate::config::{BluetoothEndpoint, Device, HidEndpoint};
use crate::hid::{self, HidInfo};
use crate::sources::hid_transport::{FeatureReportTransport, OutputReportTransport};
use crate::sources::{Reading, ReportKind, Transport, driver_for};

/// 一个 Device 上一条能取到读数的通路的种类。
///
/// `Ble` **不经过协议驱动**（走 `bluetooth.rs` 的 Windows 属性读取，不是一次 HID
/// 往返），所以取数那一步对这个枚举 `match` 到底、不写通配分支——加第四种时编译器会把
/// 人指到那一行去，而不是让新来的悄悄走上别人那条路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointKind {
    /// Device 通过 USB 线直连时出现的 Endpoint。
    Wired,
    /// 经 2.4G 接收器的 Endpoint。
    Dongle24G,
    /// 经蓝牙的 Endpoint。数据取自 Windows 缓存而非当场问设备，因此天然可能陈旧。
    Ble,
}

impl EndpointKind {
    /// 取数的优先级，从高到低。依次尝试，遇第一个成功即停。
    ///
    /// `Wired` 排在前面不是因为它更快，而是因为**鼠标插线时 Dongle24G 会超时**：
    /// 设备不再经 2.4G 传数据，此刻能读到数的只有 Wired。不读有线，充电中的鼠标
    /// 就没有任何数据源，只能显示离线——而那是明确的误报（见 `docs/adr/0001`）。
    ///
    /// `Ble` 排在最后是因为它读的是缓存：当场问得出来的时候，一个可能是几个月前的
    /// 数字不该抢在前面（`CONTEXT.md` 说它「天然可能陈旧」）。反过来，两条 HID 都
    /// 不可用时（设备关机、收进抽屉、接收器拔了）它是唯一还答得出话的一级。
    ///
    /// 它同时是**唯一的全变体清单**：要遍历所有种类的地方（列出配置过哪几条、
    /// 筛出哪几条在场）都走这里，不各自维护一份名单。
    ///
    /// 它管的只是**名单**，不是每一种的行为：怎么配、怎么算在场、怎么取一次数、来源那一段
    /// 怎么印，都是逐种类不同的事，各自写成对这个枚举穷举的 `match`（都不写通配分支，
    /// 于是加一种时编译器会把人一个个指过去）。加一种 Endpoint 要改的是这一行**加上**那几处
    /// 编译器指出来的地方。
    pub const PRIORITY: [Self; 3] = [Self::Wired, Self::Dongle24G, Self::Ble];

    /// 这个 Device 上这条 **HID** Endpoint 的配置块，没配就是 `None`。
    ///
    /// 名字里的 `hid` 是认真的：**对 `Ble` 永远是 `None`**，它配的是 `[device.bluetooth]`
    /// 里一个蓝牙地址，根本不是这个类型。所以这个方法答不了"配没配"——那件事问
    /// [`Self::is_configured_in`]。
    pub fn hid_config_in(self, device: &Device) -> Option<&HidEndpoint> {
        match self {
            Self::Wired => device.wired.as_ref(),
            Self::Dongle24G => device.dongle_24g.as_ref(),
            Self::Ble => None,
        }
    }

    /// 这个 Device 配了这条 Endpoint 吗。
    ///
    /// 与 [`Self::hid_config_in`] 分开，是因为那个方法对 `Ble` 恒为 `None`：拿它当"配没配"
    /// 的判据，会把蓝牙从"这个 Device 配了哪几条"的名单里漏掉——而那份名单正是一条都
    /// 不在场时印给用户看的东西，漏一条就等于让用户去找一个根本不在名单上的原因。
    pub fn is_configured_in(self, device: &Device) -> bool {
        match self {
            Self::Wired | Self::Dongle24G => self.hid_config_in(device).is_some(),
            Self::Ble => device.bluetooth.is_some(),
        }
    }

    /// 这条 Endpoint 在 TOML 里的块名，即 `[device.<这个>]`。
    ///
    /// 与 [`Self::hid_config_in`] / [`Self::is_configured_in`] 是同一份知识的两面——一个
    /// 按名字取出来，一个把名字写回去（自举草稿和 `config-refresh` 要写），所以住在一起。
    /// `Display` 印的是 `CONTEXT.md` 的词（`Dongle24G`），给人看；这里印的是配置里的键
    /// （`wireless_24g`），给文件看，两者不可混用。
    ///
    /// `Ble` 也有块名，尽管 `config-refresh` 从不写它（地址猜不出来）：块名是这条
    /// Endpoint 的固有属性，不因为谁写不写它而改变。
    pub fn config_key(self) -> &'static str {
        match self {
            Self::Wired => "wired",
            Self::Dongle24G => "wireless_24g",
            Self::Ble => "bluetooth",
        }
    }

    /// 块名反查种类，认不出就是 `None`。[`Self::config_key`] 的另一面。
    ///
    /// 与它住在一起，理由与"按名字取出来 / 把名字写回去"那两个同一条：这三个字符串只该有
    /// 一处出处。而这一面**不自己写一个 `match`**，是从 [`Self::PRIORITY`] 上比 `config_key`
    /// 比出来的——那样正查改一个字、反查跟着改，两面之间没有能漂开的缝。
    ///
    /// 谁需要它：状态文件（`crate::state`）把种类按块名写进磁盘，读回来要认回去。配置那一侧
    /// 用不上它——TOML 的块名在那边是由 serde 按字段名认的。
    pub fn from_config_key(key: &str) -> Option<Self> {
        Self::PRIORITY
            .into_iter()
            .find(|kind| kind.config_key() == key)
    }
}

impl std::fmt::Display for EndpointKind {
    /// 用 `CONTEXT.md` 的词，原样。这几个字会出现在 `status` 的每一行上。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Wired => "Wired",
            Self::Dongle24G => "Dongle24G",
            Self::Ble => "Ble",
        })
    }
}

/// 一份 [`Reading`]，连同它来自哪条 Endpoint。
///
/// `CONTEXT.md` 说的 Reading 是"电量、充电状态、电压、取得时刻，以及来自哪条
/// Endpoint"——它是分层拼起来的：[`Reading`] 是驱动从一帧里能解析出来的那部分，
/// **来源 Endpoint 与取得时刻都由这一步补上**（取得时刻要一个"当下"，来自 Clock 接缝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointReading {
    /// 这份读数是从哪条 Endpoint 上取到的。
    pub endpoint: EndpointKind,
    /// 驱动解析出来的那部分。
    pub reading: Reading,
    /// Windows 那份缓存的更新时刻距今多少秒。
    ///
    /// **只有 `Ble` 会有值。**两条 HID 是当场往返，根本没有缓存这回事，恒为 `None`；
    /// `Ble` 这一侧的 `None` 是另一个意思——`bluetooth.rs` 的原话：这台设备根本没有
    /// 更新时间戳。哪一种由 `endpoint` 区分。
    ///
    /// 它是 Windows 交出来的**原始数字**，陈旧判定不直接读它——判定读的是
    /// [`Self::taken_at`]，那个数由这一个换算出来。留着它是因为它比换算结果更原始：
    /// "Windows 说这份缓存 10 天前更新过"是来源那一段要印的话。
    pub cache_age_secs: Option<u64>,
    /// 这份读数**所描述的那一刻**——`CONTEXT.md` 说的"取得时刻"。
    ///
    /// 注意它不是"我们跑这次查询的那一刻"。两条 HID 是当场往返，两者重合；`Ble` 读的是
    /// Windows 攒的缓存，那个百分比描述的是 `cache_age_secs` 秒**之前**的设备，所以这里
    /// 记的是往回退过的时刻。按查询时刻记，`Ble` 就永远陈旧不了，而 `CONTEXT.md` 偏偏说
    /// 「只有 Ble 的 Reading 会陈旧到有实际影响」——那句定义会当场落空。
    ///
    /// `None` 是"说不出这个数是什么时候的"，目前只有一个来源：那台 BLE 设备根本没有更新
    /// 时间戳（`bluetooth.rs` 的原话）。**它不是"新鲜"的同义词**——陈旧判定把这一种判成
    /// 陈旧，因为说它新鲜就是替设备编一个它没给的时间戳。
    ///
    /// 陈旧判定在 `crate::staleness`。票 08（持久化）要把这个时刻写盘再读回来
    /// （`Timestamp::as_unix_secs` / `from_unix_secs` 就是为此留的），票 09（Primary 选择）
    /// 要拿它筛"只有新鲜且可信的参与比较"。
    pub taken_at: Option<Timestamp>,
}

impl EndpointReading {
    /// 一次 HID 往返读到的读数。当场问出来的，没有缓存年龄可言，取得时刻就是 `now`。
    pub fn from_hid(endpoint: EndpointKind, reading: Reading, now: Timestamp) -> Self {
        Self {
            endpoint,
            reading,
            cache_age_secs: None,
            taken_at: Some(now),
        }
    }

    /// 把本机扫到的一台 BLE 设备读成一份读数。
    ///
    /// 纯函数，不碰 Windows：一台设备怎么变成一份 Reading 因此在不接蓝牙的机器上也
    /// 断言得到，而"本机有哪些 BLE 设备"那一步才是躲不开真系统的（那正是这条接缝
    /// 存在的理由）。假枚举走的也是这个函数，好让它交出的东西与真枚举同形。
    ///
    /// `charging` 与 `voltage_mv` 一律 `None`，理由与键盘那两项同一条（parking lot
    /// Q7/Q8）：Windows 的电量属性里**根本没有**这两项，`None` 是"这条通路说不出来"，
    /// 不是"没在充电"、也不是"0 mV"。填一个值出去，下游就再也分不清两者。
    ///
    /// `now` 是把 Windows 给的"多少秒之前"换成绝对取得时刻用的：它只说了年龄，
    /// 而一份要写盘、要跨重启比较的读数需要的是时刻。
    pub fn from_ble_cache(cached: &BleBattery, now: Timestamp) -> Result<Self> {
        // level 缺席的设备在 `bluetooth::enumerate` 那一步就被滤掉了，走到这里仍要
        // 交代一句：没有电量属性不是"电量为 0"，那是这条通路这一次读不到。
        let reported_level = cached.level.ok_or_else(|| {
            anyhow!(
                "BLE 设备 {} 没有电量属性",
                if cached.friendly_name.is_empty() {
                    cached.address.as_str()
                } else {
                    cached.friendly_name.as_str()
                }
            )
        })?;
        Ok(Self {
            endpoint: EndpointKind::Ble,
            reading: Reading {
                reported_level,
                charging: None,
                voltage_mv: None,
            },
            cache_age_secs: cached.age_secs,
            // 缓存的年龄换成绝对时刻。设备没有更新时间戳时年龄是 `None`，取得时刻就
            // 跟着说不出来——**不拿 `now` 顶上去**：那会让一台没时间戳的设备看着像
            // 当场读的，正是这里最要紧的一句话。
            taken_at: cached.age_secs.map(|age_secs| now.minus_secs(age_secs)),
        })
    }
}

/// 枚举接缝。
pub trait Endpoints {
    /// 这个 Device 当前在场的 Endpoint。
    ///
    /// 返回的是一个**集合**，顺序不表达任何东西。"在场"指本机此刻真的找得到它：
    /// 配置里没有这一块、或者配了但设备没插，都算不在场。
    fn present(&self, device: &Device) -> Vec<EndpointKind>;

    /// 打开一条在场的 **HID** Endpoint，拿到往返用的 Transport。
    ///
    /// 失败即"这条 Endpoint 这一次用不上"，与读不到是同一件事，调用方接着试下一条。
    /// 最常见的失败是厂商上位机正占着这条通路（票 07 会把它变成"暂停"而不是失败）。
    ///
    /// 拿 `Ble` 来问是一次失败：那一级根本没有 Transport 可交（见
    /// [`Self::read_ble`]）。这个方法从来不适用于每一种 Endpoint——它造的东西
    /// （一次字节往返）只有 HID 才有。
    fn open_transport(&self, device: &Device, endpoint: EndpointKind)
    -> Result<Box<dyn Transport>>;

    /// 从 Windows 攒的属性缓存里读一次 `Ble`。
    ///
    /// **它和上面那个方法不对称，因为 Ble 和另两级本来就不对称。**Transport 包的是
    /// "一次到设备的字节往返"，而 Ble 不是往返：没有 report id、没有校验和、没有
    /// [`ReportKind`]、也不经过协议驱动（配置里的 `driver` 只作用于 HID）。硬给它编一个
    /// Transport，就得为一条不发字节的通路编造字节。
    ///
    /// 所以这一级直接交成品：接缝这一侧是唯一知道那份缓存有多旧的地方，而
    /// [`EndpointReading::cache_age_secs`] 只有它填得出来。转换本身是
    /// [`EndpointReading::from_ble_cache`]，纯函数。
    ///
    /// 失败与另两级同义："这条 Endpoint 这一次读不到"——没配 `[device.bluetooth]`、
    /// 本机的 BLE 设备里没有这个地址、或者那台设备没有电量属性。
    ///
    /// `now` 也是这个不对称的一部分：只有这一级需要它。Windows 只说那份缓存"多少秒之前
    /// 更新过"，而交出去的成品要带一个绝对的取得时刻，换算就得有个"当下"。两条 HID 的
    /// 取得时刻由取数那一步盖（它当场就在），所以 `open_transport` 不需要这个参数。
    fn read_ble(&self, device: &Device, now: Timestamp) -> Result<EndpointReading>;
}

/// 落到真机上的枚举：一次 [`hid::enumerate`] 加一次 [`bluetooth::enumerate`] 的快照。
///
/// 快照取一次、整个取数周期共用：一次枚举要打开本机每一条 HID collection，不便宜。
/// 也正因为是快照，"拔线"在一个周期里表现为 `Wired` 从名单上消失——那是**瞬时**的
/// 不在场，而不是一次三秒的超时，所以同周期降级几乎不要钱。
///
/// 名字里不再只有 HID：`Ble` 那一级读的是 Windows 的设备属性，两样都是"本机此刻有
/// 什么"的快照，装在同一个结构里。
pub struct SystemEndpoints {
    collections: Vec<HidInfo>,
    ble: Vec<BleBattery>,
}

impl SystemEndpoints {
    /// 现在枚举一次。
    pub fn enumerate() -> Result<Self> {
        Ok(Self {
            collections: hid::enumerate()?,
            // 蓝牙枚举失败**不该拖累两条 HID**：那时该表现为"Ble 这一级不在场"，
            // 而不是整个 status 一行都印不出来。没有蓝牙硬件的机器是常态。
            ble: bluetooth::enumerate().unwrap_or_default(),
        })
    }

    /// 这一次枚举扫到的全部带电量的 BLE 设备。
    ///
    /// `status` 要用它列出**未登记**的那些（`show_unknown_ble`）。给出快照而不是让它
    /// 自己再扫一遍：同一次输出里的两处说法该来自同一次枚举，否则"未登记"那一段和
    /// 上面每一行可能在讲两个不同时刻的本机。
    pub fn scanned_ble(&self) -> &[BleBattery] {
        &self.ble
    }

    /// 本机扫到的设备里地址对得上的那一台。
    ///
    /// 不看 `connected`：**设备关机、收进抽屉，正是要退到缓存的那一刻**，而那时
    /// Windows 里的值照样在（`bluetooth.rs` 开头记着实测见过 2025 年 3 月的读数）。
    /// 拿在线与否当在场与否，这一级就恰好在最需要它的时候消失。
    fn find_ble(&self, configured: &BluetoothEndpoint) -> Option<&BleBattery> {
        self.ble
            .iter()
            .find(|found| configured.matches(&found.address))
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

impl Endpoints for SystemEndpoints {
    fn present(&self, device: &Device) -> Vec<EndpointKind> {
        EndpointKind::PRIORITY
            .into_iter()
            .filter(|kind| match kind {
                // 报文种类由驱动来答，所以驱动认不出来时一条 HID 也算不上在场——
                // 没有"发得出哪种报文"可言，就没法筛 collection。
                EndpointKind::Wired | EndpointKind::Dongle24G => Self::report_kind_of(device)
                    .zip(kind.hid_config_in(device))
                    .is_some_and(|(report_kind, configured)| {
                        self.find_collection(configured, report_kind).is_some()
                    }),
                // **Ble 不经过驱动**，所以那道筛子不适用于它：`driver` 认不出来
                // （配置里写了本次编译还没实现的驱动名），蓝牙这一级照样在场。
                EndpointKind::Ble => device
                    .bluetooth
                    .as_ref()
                    .and_then(|configured| self.find_ble(configured))
                    .is_some(),
            })
            .collect()
    }

    fn open_transport(
        &self,
        device: &Device,
        endpoint: EndpointKind,
    ) -> Result<Box<dyn Transport>> {
        // Ble 落到这里是调用错了方法而不是配置的问题：那一级没有 Transport 可交，
        // 它走 `read_ble`。`hid_config_in` 对它恒为 None，所以这一句自然就挡住了。
        let configured = endpoint
            .hid_config_in(device)
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

    fn read_ble(&self, device: &Device, now: Timestamp) -> Result<EndpointReading> {
        let configured = device
            .bluetooth
            .as_ref()
            .ok_or_else(|| anyhow!("这个 Device 没有配置 Ble"))?;
        // 这句话里"带电量属性的"不能省：快照来自 `bluetooth::enumerate`，它只收带电量属性
        // 的设备。配对着、但 Windows 手上没有它电量的设备同样落到这一支，而对那台设备说
        // "没配对？"就是把人往错的方向指——它配着对，只是这条通路答不出电量。
        let cached = self.find_ble(configured).ok_or_else(|| {
            anyhow!(
                "本机带电量属性的 BLE 设备里没有地址 {}（没配对？还是这台设备不报电量？\
                 跑 `juicebar scan` 看本机有哪些）",
                configured.address
            )
        })?;
        EndpointReading::from_ble_cache(cached, now)
    }
}
