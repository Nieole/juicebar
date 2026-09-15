//! 自举与补全都要问的同一件事：这台设备是**实测记录下来的**哪一台。
//!
//! 那张写死的表（`KNOWN_DEVICES`）与三个比对判定住在一处，因为它们说的是同一件事：
//! `recognise` 回答"配置里这条 `[[device]]` 对上表里哪一条"，`is_present` 回答"这条
//! 身份本机此刻枚举得到吗"，`same_scanned_identity` 回答"用户写下的那条和表里的那条
//! 一样吗"。三问都只看身份，不碰草稿的文案，也不碰这份文件该怎么写。
//!
//! **加一台设备只改这一个文件。**

use crate::config::{Config, Device, HidEndpoint};
use crate::endpoints::EndpointKind;
use crate::hid::HidInfo;

/// 一台身份已经实测过的 Device。
///
/// 为什么要有这张表：几套身份之间**没有任何字段能自动缝合**。鼠标的 dongle 是
/// `391D:1A05`，插线时另外冒出来的本体是 `391D:1005`，两者除了 VID 相同以外没有共同的
/// 序列号；而"VID 相同就是同一台设备"在同一家厂商出两件外设时会张冠李戴，写下去的是一份
/// 错配置。所以认设备靠**实测记录下来的整组身份**，不靠推断——表里没有的设备一律不猜，
/// 草稿里留成注释让用户手填。
pub struct KnownDevice {
    /// 稳定的机器可读标识，直接写进草稿的 `id`。
    pub id: &'static str,
    /// 界面上显示的名字。
    pub name: &'static str,
    /// 协议驱动名，见 `src/sources/`。
    pub driver: &'static str,
    /// Wired Endpoint 的身份。
    pub wired: HidEndpoint,
    /// Dongle24G Endpoint 的身份。
    pub dongle_24g: HidEndpoint,
    /// Wired 扫不到时对用户说的那句话。
    ///
    /// 每台设备"怎样才能让它出现"并不一样，这正是它必须逐台写死、不能写成一句通用
    /// 提示的原因：鼠标插上线就行，键盘还得拨机身上的模式开关。
    pub wired_hint: &'static str,
    /// 这条 Wired 身份里有没有**没实测过**的部分。没有就是空串。
    ///
    /// 它会跟着占位一起写进草稿，也会跟在自动补空块那句"补上了 Wired"后面（日志）。
    /// 为什么非要说出来：`is_present` 只核对 VID/PID/usage 四项，`report_id` 是枚举问不出
    /// 来的，所以那一项若是抄来的猜测，补出来的块就带着一个没人验过的值——而报文编号错了的
    /// 帧会被设备静默丢弃，看起来和"设备没反应"一模一样（`docs/protocol.md` 的教训）。
    pub wired_caveat: &'static str,
}

impl KnownDevice {
    /// 这台设备某一种 Endpoint 的身份。
    ///
    /// 两条身份是两个具名字段加一个穷举 `match`，而不是一个按 `PRIORITY` 下标索引的数组
    /// ——后者能省掉这个 `match`，但它假定"每一种 Endpoint 的身份都是 `HidEndpoint`"。
    /// 那一天来了：`Ble` 配的是一个蓝牙地址，不是这个类型。留着 `match` 的回报就在这里
    /// ——编译器把人指到了这一行，而不是在运行时给一个下标越界。
    ///
    /// **返回 `Option` 是因为这张表里确实没有 `Ble` 的身份，而且猜不出来**：同一只鼠标的
    /// BLE 射频是另一颗芯片，连 VID 都和 dongle 不同，几套身份之间没有能缝合的字段。
    /// 让类型说出这件事，好过让每个调用点自己记着"Ble 别问这个"。
    pub(super) fn identity(&self, kind: EndpointKind) -> Option<&HidEndpoint> {
        match kind {
            EndpointKind::Wired => Some(&self.wired),
            EndpointKind::Dongle24G => Some(&self.dongle_24g),
            EndpointKind::Ble => None,
        }
    }

    /// 这条 Endpoint 现在不在场时，要用户去做的那件事。
    pub(super) fn absent_hint(&self, kind: EndpointKind) -> &'static str {
        match kind {
            EndpointKind::Wired => self.wired_hint,
            // Dongle24G 不在场只有一种原因，不必逐台写。
            EndpointKind::Dongle24G => "把 2.4G 接收器插上。",
            // Ble 不是"插上就出现"的：它要先在系统蓝牙里配对，地址还得人在托盘菜单里点一下登记。
            EndpointKind::Ble => {
                "先在系统蓝牙设置里把它配对上，再右键托盘图标，在\"登记设备\"里把它登记到这台。"
            }
        }
    }

    /// 这条 Endpoint 的身份里有没有没实测过的部分，没有就是空串。
    pub(super) fn caveat(&self, kind: EndpointKind) -> &'static str {
        match kind {
            EndpointKind::Wired => self.wired_caveat,
            // 两台设备的 Dongle24G 都是实测抓的——协议本来就是在那条路上跑通的。
            EndpointKind::Dongle24G => "",
            // Ble 的身份这张表里没有，也就没有"实测过没有"可言。
            EndpointKind::Ble => "",
        }
    }
}

/// 已经实测过身份的设备。加一台设备就在这里加一条。
pub const KNOWN_DEVICES: &[KnownDevice] = &[
    KnownDevice {
        id: "dragonfly3",
        name: "Dragonfly 3 Master+",
        driver: "vgn_mouse",
        wired: HidEndpoint {
            vid: 0x391D,
            pid: 0x1005,
            usage_page: 0xFF02,
            usage: 0x0002,
            report_id: 8,
        },
        dongle_24g: HidEndpoint {
            vid: 0x391D,
            pid: 0x1A05,
            usage_page: 0xFF02,
            usage: 0x0002,
            report_id: 8,
        },
        wired_hint: "插上 USB 线，它会作为一个额外的设备冒出来，dongle 照旧在场，两者并存。",
        // 这一整组身份都是实测抓到的，没有猜的部分。
        wired_caveat: "",
    },
    // 这一条的 Wired 至今**没有实测过**：键盘要把机身模式开关拨到有线档才枚举得出来，
    // 仅插线不够。VID/PID/usage 取自 docs/protocol.md 记下的观察，`report_id` 是照它的
    // Dongle24G 抄的（那一条实测就是 0）。写下去不会造成错配置：草稿里它是注释掉的占位，
    // 而自动补空块只在本机真的枚举得到这组 VID/PID/usage 时才把它填成真块。
    KnownDevice {
        id: "neon75",
        name: "VGN Neon75",
        driver: "vgn_keyboard",
        wired: HidEndpoint {
            vid: 0x3151,
            pid: 0x502F,
            usage_page: 0xFFFF,
            usage: 0x0002,
            report_id: 0,
        },
        dongle_24g: HidEndpoint {
            vid: 0x3151,
            pid: 0x5038,
            usage_page: 0xFFFF,
            usage: 0x0002,
            report_id: 0,
        },
        wired_hint: "把机身上的模式开关拨到有线档，再插线。开关还在 2.4G 档时线只负责充电。",
        // 分两行写：它要跟着占位一起进草稿，一行放不下。
        wired_caveat: "下面那组身份**没有实测过**：report_id = 0 是照它的 Dongle24G 抄的。\n\
读不到数就先怀疑这一项。",
    },
];

/// 本机此刻是否枚举得到这条 Endpoint。
///
/// 这里**不要求**这条通路能发输出报文，而 `HidEndpoints::find_output_collection` 要求。
/// 两处问的不是同一个问题：那边要发命令取数，这边只是往配置里写下一条身份。键盘的 vendor
/// collection 实测是 `in:0 out:0 feat:65`，按输出报文筛会把它筛掉——于是那条最需要
/// 自动补空块的 Endpoint 反而永远补不上。
pub(super) fn is_present(endpoint: &HidEndpoint, collections: &[HidInfo]) -> bool {
    collections.iter().any(|c| {
        c.vid == endpoint.vid
            && c.pid == endpoint.pid
            && c.usage_page == endpoint.usage_page
            && c.usage == endpoint.usage
    })
}

/// 两条 Endpoint 身份在**枚举能看见的那几个字段**上是否相同。
///
/// 刻意不比 `report_id`：一次 HID 枚举问不出报文编号，拿它去说"扫描结果与你写的不一致"
/// 是没有依据的。
pub(super) fn same_scanned_identity(a: &HidEndpoint, b: &HidEndpoint) -> bool {
    a.vid == b.vid && a.pid == b.pid && a.usage_page == b.usage_page && a.usage == b.usage
}

/// 认出这台 Device 是 [`KNOWN_DEVICES`] 里的哪一台。
///
/// **先看它已经配好的 Endpoint 身份**：那是证据，能和本机枚举结果逐字段对上。只比
/// VID/PID——usage page / usage 用户可能有理由挑另一条 collection，那不该让人认不出设备。
///
/// 一块都没配时才退到 `id`。这不是假想的情形：用户手写一条 `[[device]]` 骨架、或者删掉
/// 某一块想让程序重新补上，都会落到这里，而"看已配好的身份"这条路那时问不出任何东西。
/// `id` 只能是兜底，因为它是用户随手可改的自由文本，不是证据——而即便认错了也写不出错
/// 配置：填进去的身份必须此刻真的在本机枚举得到。
///
/// **没写 `driver` 的那台不退到 `id`**：它说的是"我只走蓝牙"（[`Device::driver`]），不是表里哪一台 HID 设备，id 恰好
/// 对得上也不是。认成了，补空块就往一台没有驱动的 Device 里补 HID 块，写出一份读不动的配置。
pub(super) fn recognise(device: &Device) -> Option<&'static KnownDevice> {
    let by_identity = KNOWN_DEVICES.iter().find(|known| {
        EndpointKind::PRIORITY.into_iter().any(|kind| {
            // 两边都可能没有：用户没配这一块，或者这一种 Endpoint 不在身份表里（Ble）。
            match (kind.hid_config_in(device), known.identity(kind)) {
                (Some(configured), Some(identity)) => {
                    configured.vid == identity.vid && configured.pid == identity.pid
                }
                _ => false,
            }
        })
    });
    by_identity.or_else(|| {
        device.driver.as_ref()?;
        KNOWN_DEVICES.iter().find(|known| known.id == device.id)
    })
}

/// 身份表认得、本机此刻在场、配置里却没有的一台 HID 设备（[`Config::unregistered_known_devices`]）：菜单"登记设备"里点得到
/// "新建一台 Device"的那一组。
pub struct UnregisteredHid {
    /// 身份表里的那一台。
    pub known: &'static KnownDevice,
    /// 本机此刻在场的那几条 Endpoint，按 [`EndpointKind::PRIORITY`] 排，至少有一条。
    pub present: Vec<EndpointKind>,
}

impl Config {
    /// 身份表认得、本机此刻在场（`collections` 里枚举得到它至少一条 Endpoint 的身份，与自动补空块同一个判法 [`is_present`]）、
    /// 配置里却没有（[`is_in_config`]）的那几台，按身份表的次序。
    pub fn unregistered_known_devices(&self, collections: &[HidInfo]) -> Vec<UnregisteredHid> {
        KNOWN_DEVICES
            .iter()
            .filter(|known| !is_in_config(self, known))
            .filter_map(|known| {
                let present: Vec<EndpointKind> = EndpointKind::PRIORITY
                    .into_iter()
                    .filter(|kind| {
                        known
                            .identity(*kind)
                            .is_some_and(|identity| is_present(identity, collections))
                    })
                    .collect();
                (!present.is_empty()).then_some(UnregisteredHid { known, present })
            })
            .collect()
    }
}

/// 配置里有没有哪一台认得出是这台设备（[`recognise`]）。菜单"登记设备"列不列它（[`Config::unregistered_known_devices`]）与
/// 新建时写不写它（`edit::add_device`）问的是同一件事，只在这里判一次：两处各判一遍，改岔了菜单就会列出一项点了什么都不写的
/// "新建一台 Device"。
pub(super) fn is_in_config(config: &Config, known: &KnownDevice) -> bool {
    config
        .devices
        .iter()
        .any(|device| recognise(device).is_some_and(|recognised| recognised.id == known.id))
}
