//! `config.toml` 这一头：读取、自举草稿的生成、以及只填空缺的补全。
//!
//! 三样住在一处是有意的——它们说的是同一份文件的同一套 schema，分开住就会漂。
//! **读**走 `toml` + serde，要的是类型化的模型；**写**走 `toml_edit`，要的是保住用户
//! 写下的注释（见 `docs/adr/0003`）。
//!
//! 结构里只有**当前用得上**的字段。TOML 里其余的键（`[general]`、`level_source`、
//! `[device.bluetooth]` …）会被静默忽略，等要用它们的那一步再加进来——这样用户的
//! 配置不必随实现进度反复改写，样例配置也可以先于代码写全。**草稿照样把它们写出来**，
//! 因为草稿是给人读的：一个还没实现的开关，用户也该知道它存在。
//!
//! 字段名沿用 `CONTEXT.md` 的词：配置里的 `wireless_24g` 块在代码里叫
//! `dongle_24g`，因为它描述的正是 Dongle24G 这条 Endpoint。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::endpoints::EndpointKind;
use crate::hid::HidInfo;

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

// ---------------------------------------------------------------
// 自举：把一次 HID 枚举变成一份配置草稿
// ---------------------------------------------------------------

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
    /// 它会跟着占位一起写进草稿，也会跟在 `config-refresh` 那句"补上了 Wired"后面。
    /// 为什么非要说出来：`is_present` 只核对 VID/PID/usage 四项，`report_id` 是枚举问不出
    /// 来的，所以那一项若是抄来的猜测，补出来的块就带着一个没人验过的值——而报文编号错了的
    /// 帧会被设备静默丢弃，看起来和"设备没反应"一模一样（`docs/protocol.md` 的教训）。
    pub wired_caveat: &'static str,
}

impl KnownDevice {
    /// 这台设备某一种 Endpoint 的身份。
    ///
    /// 两条身份是两个具名字段加一个穷举 `match`，而不是一个按 `PRIORITY` 下标索引的数组
    /// ——后者能省掉这个 `match`，但它假定"每一种 Endpoint 的身份都是 `HidEndpoint`"，
    /// 而票 05 的 `Ble` 配的是一个蓝牙地址，不是这个类型（`endpoints.rs` 的
    /// `config_in` 已经点明了这件事）。留着 `match`，加上第三种的那一天编译器会把人指到
    /// 这里，逼他面对"Ble 的身份根本不是同一个形状"；换成数组，那一天只会得到一个下标越界。
    fn identity(&self, kind: EndpointKind) -> &HidEndpoint {
        match kind {
            EndpointKind::Wired => &self.wired,
            EndpointKind::Dongle24G => &self.dongle_24g,
        }
    }

    /// 这条 Endpoint 现在不在场时，要用户去做的那件事。
    fn absent_hint(&self, kind: EndpointKind) -> &'static str {
        match kind {
            EndpointKind::Wired => self.wired_hint,
            // Dongle24G 不在场只有一种原因，不必逐台写。
            EndpointKind::Dongle24G => "把 2.4G 接收器插上。",
        }
    }

    /// 这条 Endpoint 的身份里有没有没实测过的部分，没有就是空串。
    fn caveat(&self, kind: EndpointKind) -> &'static str {
        match kind {
            EndpointKind::Wired => self.wired_caveat,
            // 两台设备的 Dongle24G 都是实测抓的——协议本来就是在那条路上跑通的。
            EndpointKind::Dongle24G => "",
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
    // 而 `config-refresh` 只在本机真的枚举得到这组 VID/PID/usage 时才把它填成真块。
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
/// `config-refresh` 的 Endpoint 反而永远补不上。
fn is_present(endpoint: &HidEndpoint, collections: &[HidInfo]) -> bool {
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
fn same_scanned_identity(a: &HidEndpoint, b: &HidEndpoint) -> bool {
    a.vid == b.vid && a.pid == b.pid && a.usage_page == b.usage_page && a.usage == b.usage
}

/// 一条 Endpoint 的配置块，缩进两格跟在它的 `[[device]]` 下面。
fn endpoint_block(kind: EndpointKind, endpoint: &HidEndpoint) -> String {
    format!(
        "  [device.{}]\n  \
         vid = 0x{:04X}\n  \
         pid = 0x{:04X}\n  \
         usage_page = 0x{:04X}\n  \
         usage = 0x{:04X}\n  \
         report_id = {}\n",
        kind.config_key(),
        endpoint.vid,
        endpoint.pid,
        endpoint.usage_page,
        endpoint.usage,
        endpoint.report_id,
    )
}

/// 首次运行时的配置草稿：扫到什么写什么，猜不动的留成注释并写明补全办法。
///
/// 入参是一次 [`crate::hid::enumerate`] 的结果。它是个纯函数，不自己去碰硬件——
/// "扫到了这些"因此可以在用例里直接摆出来。
pub fn draft(collections: &[HidInfo]) -> String {
    let mut out = String::from(DRAFT_PREAMBLE);
    let mut wrote_a_device = false;
    for known in KNOWN_DEVICES {
        // 一条 Endpoint 都不在场的设备不写进草稿：用户多半根本没有这台设备。
        if !EndpointKind::PRIORITY
            .iter()
            .any(|kind| is_present(known.identity(*kind), collections))
        {
            continue;
        }
        // 那两项的解释只写在第一台设备上，后面的光写值——`config.example.toml` 就是
        // 这么排的。每台都重复一遍，读第二遍的人只会开始跳着看注释。
        let explain = !wrote_a_device;
        wrote_a_device = true;
        out.push_str(&format!(
            "\n[[device]]\nid = \"{}\"\nname = \"{}\"\n",
            known.id, known.name
        ));
        if explain {
            out.push_str("# 用哪个协议驱动，见 src/sources/。只作用于 HID Endpoint。\n");
        }
        out.push_str(&format!("driver = \"{}\"\n", known.driver));
        if explain {
            out.push_str(
                "\n# 电量数值取自哪里。\"auto\"（缺省）在电压高段用固件自报的 Reported Level、\n\
                 # 其余区间用电压查表算出的 Derived Level；\"reported\" 一律用固件自报值。\n\
                 # 这一项是**每 Device** 的，没有全局默认——想切的理由本来就是设备相关的，\n\
                 # 见 docs/adr/0002。\n",
            );
        }
        out.push_str("level_source = \"auto\"\n");
        if explain {
            out.push_str(OPTIONAL_LOW_BATTERY);
        }
        for kind in EndpointKind::PRIORITY {
            let identity = known.identity(kind);
            out.push('\n');
            if is_present(identity, collections) {
                out.push_str(&endpoint_block(kind, identity));
                // `address` 是 Dongle24G 那张表里的一个可选键，所以它只跟在这一块后面
                // （TOML 里空行不结束一张表，取消注释后它落在上面那个块里）。
                if explain && kind == EndpointKind::Dongle24G {
                    out.push_str(OPTIONAL_ADDRESS);
                }
            } else {
                out.push_str(&commented_placeholder(
                    kind,
                    identity,
                    known.absent_hint(kind),
                    known.caveat(kind),
                ));
            }
        }
        out.push_str(BLE_PLACEHOLDER);
    }
    if !wrote_a_device {
        out.push_str(NOTHING_RECOGNISED);
    }
    out.push_str(&unrecognised_section(collections));
    out
}

/// Ble 那条 Endpoint 在草稿里的位置。
///
/// 它和上面两条不一样，是**硬写的一段注释**，不走 [`EndpointKind`]：那个枚举眼下只有
/// `Wired` 和 `Dongle24G` 两个变体，Ble 还没落地。等它进了枚举，这一段就该退掉，改由上面
/// 那个循环连同 `PRIORITY` 一起产出——那时它的"在场"也才真的问得出来。
///
/// 在此之前也不能干脆不写：Ble 是三条 Endpoint 之一，一声不吭地不提它，用户就不知道
/// 自己少了一条。而地址是真的猜不出来——同一只鼠标的 BLE 射频是另一颗芯片，VID 都不同
/// （dongle `0x391D` / BLE `0x3554`），没有任何字段能把两者缝起来。
const BLE_PLACEHOLDER: &str = "
  # 蓝牙那条 Endpoint 要一个 MAC，程序猜不出来：同一只设备的 BLE 射频是另一颗芯片，
  # 连 VID 都和 dongle 不同，几套身份之间没有能自动缝合的字段。跑 `juicebar scan`，
  # 看「蓝牙（BLE）电量」那一段，把地址抄到下面来。
  # 没有蓝牙、或者不想用它，这几行删掉即可。
  # [device.bluetooth]
  # address = \"把 scan 里那串 12 位十六进制抄过来\"
";

/// 草稿开头那段话：这份文件是什么、注释掉的块意味着什么、接下来该做什么。
///
/// `[general]` 写在**所有 `[[device]]` 之前**不是排版偏好：TOML 里顶层键一旦写在某个表
/// 后面就落进那个表里了，顺序颠倒会让这几个开关静默变成某个 Device 的字段。
///
/// 这些键当前多半还没人读（`Config` 只解析用得上的那几个），照样写出来是因为草稿是给
/// 人读的——一个用户看不见的开关等于不存在。长篇解释在 `config.example.toml` 里，
/// 这里每项一两句，多了反而没人读。
const DRAFT_PREAMBLE: &str = "\
# juicebar 配置草稿 —— 首次运行时自动生成的
#
# 程序扫了一遍本机的 HID collection：认得出来的设备直接写成了下面的 [[device]]；
# 认不出来、或者此刻扫不到的，一律写成**注释**留在原处，并附上补全办法。
#
# 所以：注释掉的块 = 需要你看一眼的地方。程序在这里有意不猜——几套身份之间没有任何
# 字段能自动缝合，猜错写下去的是一份错配置，而错配置比没有配置更难查。
#
# 该插的插好之后（键盘还要把机身模式开关拨到有线档）跑 `juicebar config-refresh`，
# 程序会再扫一遍、把当时在场而配置里空着的 Endpoint 块填进来：**只填空着的，你写过的
# 值和注释一概不动**（见 docs/adr/0003）。
#
# 每一项的详细含义见仓库里的 config.example.toml，术语见 CONTEXT.md。

[general]

# 托盘图标画哪个 Device，也就是 Primary Device。\"lowest\" = 当前电量最低的那个（推荐），
# 也可以填某个 Device 的 id 把它钉死。这一项**程序会回写**：在托盘菜单里换 Primary Device
# 时，新选择写回这里，好让配置始终是单一事实来源。
primary = \"lowest\"

# 轮询间隔（秒）。三条 Endpoint 的成本差着数量级，所以分开配：Wired 走线取数、设备还在
# 外部供电，不耗它的电，可以勤一点；Dongle24G 要往设备发无线包，耗设备的电，省着来；
# Ble 是纯本地属性读取，几乎免费。
poll_interval_wired = 30
poll_interval_24g = 60
poll_interval_bluetooth = 10

# 低电量阈值（百分比），图标数字变红。可被单个 [[device]] 覆盖。
low_battery = 20

# 数据陈旧阈值（秒）。**只对 Ble 生效**——它读的是 Windows 缓存，可能过期几个月。
# HID 那两条不用这两项：新鲜度由轮询间隔自动推导（3 倍间隔即视为陈旧）。
stale_after = 3600
very_stale_after = 86400

# 列表里是否显示未在 [[device]] 中登记的 BLE 设备。
show_unknown_ble = false

# 检测到厂商上位机在运行时暂停 HID 轮询。这不是防御性设计：键盘 dongle 的 feature 报文是
# 一块保存最近一次应答的共享缓冲区，两个程序同时发命令会互相覆盖对方的应答，双方都读到
# 错数据。Ble 不参与这个竞争，照常轮询。
pause_when_vendor_hub_running = true
vendor_hub_processes = [\"VGN VHUB.exe\"]

# ---------------------------------------------------------------
# Device。每个物理设备一条，把它的几条 Endpoint 合并在一起。
#
# 取数时按 Wired > Dongle24G > Ble 依次尝试，遇第一个成功即停；靠前的失败会在**同一个
# 周期内**立刻降级，不用等下一轮。
# ---------------------------------------------------------------
";

/// 可选的 per-device 低电阈值，写成注释。
///
/// spec 的「配置」一节把它和下面的 `address` 一起列进本步要有的项。写成注释而不是真值，
/// 是因为它的意义就是"覆盖全局"——真写出来等于替用户做了一个他没要求的覆盖。
const OPTIONAL_LOW_BATTERY: &str = "
# 可选，覆盖 [general] 里的全局值。键鼠的电池容量和耗电差很多，同一个 20% 对两者的
# 实际紧急程度并不相等。
# low_battery = 15
";

/// 可选的无线地址，写成注释。跟在 Dongle24G 那一块后面，因为它是那张表里的一个键。
///
/// 第一行用 `\x20` 起头是有原因的：字符串续行的 `\` 会把下一行的**前导空白一并吃掉**，
/// 直接写两个空格的缩进会在输出里丢掉。
const OPTIONAL_ADDRESS: &str = "\
\x20 # 可选。设备自己的无线地址（鼠标 cmd 3 能问出来，厂商 HUB 也拿它当缓存键）。
  # 只有同时接了两台**同型号**设备、VID/PID 完全相同时才需要写，用来区分谁是谁。
  # 取消注释后它属于上面那张表（TOML 里空行不结束一张表）。
  # address = \"97d435\"
";

/// 一台认得出来的设备都没扫到时补在草稿里的那段话。
///
/// 这一段不能省：一份只有 `[general]` 的文件看起来像"程序坏了"，而实际原因通常是设备
/// 没插或者它不在 [`KNOWN_DEVICES`] 里，两者要用户做的事完全不同。
///
/// **这里让用户删掉文件重来，而不是"插好之后跑 `config-refresh`"**，因为后者是假话：
/// [`refresh`] 只往已有的 `[[device]]` 里补空着的块，它不新增 Device（那条边界见
/// `.scratch/parking-lot.md` 的 Q49）。而这份文件此刻没有任何 Device，删掉它不损失
/// 任何东西——里面只有用户还没动过的默认值。
const NOTHING_RECOGNISED: &str = "\
\n# 本机一台认得出来的设备都没扫到，所以这份草稿里没有任何 [[device]]，它还派不上用场。
#
# 可能是设备或接收器没插，也可能它根本不在程序认得的那张表里（表在 src/config.rs 的
# KNOWN_DEVICES）。跑 `juicebar scan` 看本机到底有哪些 HID collection。
#
# 插好之后**把这个文件删掉**再跑一次 `juicebar config-refresh`，程序会重新扫一遍、生成
# 一份带 Device 的新草稿。（不是跑 config-refresh 就够：它只往已有的 [[device]] 里补空着
# 的块，不会替你新增一台设备。这个文件现在没有你动过的东西，删掉不损失什么。）
";

/// 扫到了、但 [`KNOWN_DEVICES`] 里没有的 vendor collection，写成注释掉的骨架。
///
/// 为什么不猜：一台设备的几套身份之间没有能自动缝合的字段（同一只鼠标的 dongle 是
/// VID `0x391D`、BLE 射频是 `0x3554`，两颗芯片），也没有任何字段说得出这条通路走的是
/// 哪家的私有协议。猜错写下去的是一份错配置，而错配置比没有配置更难查。
///
/// 但**不猜不等于不说**：扫到的东西全列出来，用户于是知道自己有个设备没登记，
/// 也知道该往哪几行里填什么。
fn unrecognised_section(collections: &[HidInfo]) -> String {
    // 已经属于某台认得出来的设备的硬件，整个 (vid, pid) 都不必再提——它可能暴露好几条
    // vendor collection，而那是它自己的事。
    let accounted: Vec<(u16, u16)> = KNOWN_DEVICES
        .iter()
        .flat_map(|known| {
            EndpointKind::PRIORITY.into_iter().map(move |kind| {
                let identity = known.identity(kind);
                (identity.vid, identity.pid)
            })
        })
        .collect();

    // 按 (vid, pid) 归组：一件硬件一个骨架。BTreeMap 是为了输出顺序稳定。
    let mut groups: BTreeMap<(u16, u16), Vec<&HidInfo>> = BTreeMap::new();
    for c in collections {
        // 私有协议通道一定落在厂商自定义页，其余的（普通鼠标、键盘、消费控制…）
        // 不可能藏着电量命令，列出来只是噪音。
        if !c.is_vendor_defined() || accounted.contains(&(c.vid, c.pid)) {
            continue;
        }
        groups.entry((c.vid, c.pid)).or_default().push(c);
    }
    if groups.is_empty() {
        return String::new();
    }

    // 骨架只写一份。绝大多数机器上这张单子里是触摸板、传感器之类本机自带的东西，
    // 每条都配一段十几行的模板，用户第一眼看到的就是一屏与他无关的样板文字。
    let mut out = unrecognised_header();
    for ((vid, pid), items) in &groups {
        let name = items
            .iter()
            .map(|c| c.product.as_str())
            .chain(items.iter().map(|c| c.manufacturer.as_str()))
            .find(|s| !s.is_empty())
            .unwrap_or("(无产品名)");
        let rev = items.first().map(|c| c.version).unwrap_or(0);
        out.push_str(&format!(
            "\n# VID_{vid:04X}&PID_{pid:04X}  REV_{rev:04X}  {name}\n"
        ));
        for c in items {
            out.push_str(&format!(
                "#   UP:{:04X} U:{:04X}  in:{} out:{} feat:{}\n",
                c.usage_page, c.usage, c.input_len, c.output_len, c.feature_len
            ));
        }
    }
    out
}

/// 认不出来那一节的开头：为什么不猜，以及真要用它们的话照什么样子写。
///
/// 里面那个 Endpoint 块**不是手抄的**，是 [`blank_endpoint_block`] 从 [`endpoint_block`] 挖空
/// 得来的——`HidEndpoint` 加一个字段时这份模板跟着变，不会静默过期。
fn unrecognised_header() -> String {
    format!(
        "
# ---------------------------------------------------------------
# 下面这些 vendor collection 扫到了，但程序认不出它们是什么设备。
#
# 认不出来就不猜：几套身份之间没有任何字段能自动缝合，也没有哪个字段说得出这条通路走的
# 是谁家的私有协议，猜错写下去的是一份错配置，而错配置比没有配置更难查。这张单子上多半
# 是本机自带的触摸板、传感器之类，跟电量无关——但万一其中一条真是你的键鼠，照下面这个
# 样子自己加一条 [[device]]，四个 0x____ 从单子上抄：
#
# [[device]]
# id = \"自己起一个稳定的 id\"
# name = \"界面上显示的名字\"
# driver = \"必须手填：src/sources/ 里的驱动名\"
# level_source = \"auto\"
#   # 是接收器就填 wireless_24g，是插线冒出来的本体就填 wired。
{}#
# `juicebar caps` 能问出一条 collection 声明的 Report ID，`juicebar probe` 能试探它认不认
# 已知的读命令。
# ---------------------------------------------------------------
",
        comment_out(&blank_endpoint_block(EndpointKind::Dongle24G), "#   ")
    )
}

/// 一条 Endpoint 块，值全挖成 `0x____` —— 给用户手抄的模板。
///
/// 走 [`endpoint_block`] 而不是另写一遍，是为了让模板的字段清单永远和程序真会写的那几行
/// 一致。`report_id` 留 0：它不是十六进制，也确实是最常见的值。
fn blank_endpoint_block(kind: EndpointKind) -> String {
    let blank = HidEndpoint {
        vid: 0,
        pid: 0,
        usage_page: 0,
        usage: 0,
        report_id: 0,
    };
    endpoint_block(kind, &blank).replace("0x0000", "0x____")
}

/// 把一段 TOML 逐行注释掉，每行前面加上 `prefix`。
///
/// 占位和模板都走它，于是"注释掉的块"和真块之间不会漂：用户把 `#` 去掉，得到的正是程序
/// 自己会写的那几行。原有缩进由 `prefix` 统一接管，所以调用方给的前缀里就含缩进。
fn comment_out(text: &str, prefix: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(prefix);
        out.push_str(line.trim_start());
        out.push('\n');
    }
    out
}

/// 扫不到的那条 Endpoint 在草稿里的样子：一段说明，加上逐行注释掉的真块。
///
/// **占位的内容就是 [`endpoint_block`] 的输出逐行加了 `#`**，所以它和程序自己会写的
/// 那几行之间不会漂：用户把 `#` 去掉，得到的正是真块。
///
/// 为什么非要写这一段而不是干脆省掉：一条 Endpoint 悄悄缺席，用户既不知道自己少了什么，
/// 也不知道该做什么才能补上——而缺的偏偏常常是 Wired，也就是设备插着线充电、最需要看见
/// 电量的那一刻。
fn commented_placeholder(
    kind: EndpointKind,
    endpoint: &HidEndpoint,
    hint: &str,
    caveat: &str,
) -> String {
    let key = kind.config_key();
    let mut out = format!(
        "  # {kind} 现在扫不到，所以下面这一块是**注释**，不是配置。\n\
         \x20 # 怎么让它出现：{hint}\n"
    );
    // 下面那组身份可不可信，要么说"实测记下来的"，要么把没实测的那一项点出来——
    // 两句话不能同时出现，否则用户读到的是自相矛盾。
    let provenance = if caveat.is_empty() {
        "下面那组身份是实测记下来的。"
    } else {
        caveat
    };
    for line in provenance.lines() {
        out.push_str(&format!("  # {line}\n"));
    }
    out.push_str(&format!(
        "  # 弄好之后跑 `juicebar config-refresh`，程序会扫一遍本机、把真正的\n\
         \x20 # [device.{key}] 写进来（只填空着的块，你写过的值和注释一概不动）。到那时这几行\n\
         \x20 # 就只是历史记录了，留着或删掉都行。\n"
    ));
    out.push_str(&comment_out(&endpoint_block(kind, endpoint), "  # "));
    out
}

// ---------------------------------------------------------------
// config-refresh：把当时在场、而配置里空着的 Endpoint 块补上
// ---------------------------------------------------------------

/// 一次 [`refresh`] 的结果。
///
/// 文本、做过的事、没做的事分成三样交出去：调用方要把后两样印在命令行上。ADR-0003 划死的
/// 边界是"只补空缺，扫描结果与用户所写不一致时只提醒"——那句提醒就住在 [`Self::notes`] 里，
/// 它是这条边界唯一的出口。
pub struct Refreshed {
    /// 补全之后的全文。什么都没补时与入参**逐字节相同**。
    pub text: String,
    /// 这一次补上了哪些块，一条一句人话。
    pub filled: Vec<String>,
    /// 没补的地方和为什么，一条一句人话。它不改文件，只解释。
    pub notes: Vec<String>,
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
fn recognise(device: &Device) -> Option<&'static KnownDevice> {
    let by_identity = KNOWN_DEVICES.iter().find(|known| {
        EndpointKind::PRIORITY.into_iter().any(|kind| {
            kind.config_in(device).is_some_and(|configured| {
                let identity = known.identity(kind);
                configured.vid == identity.vid && configured.pid == identity.pid
            })
        })
    });
    by_identity.or_else(|| KNOWN_DEVICES.iter().find(|known| known.id == device.id))
}

/// 把当时在场、而配置里空着的 Endpoint 块补进这份 TOML，**只补空缺**。
///
/// 走 `toml_edit` 而不是 serde 的序列化：后者会把整份文件重写一遍，注释全没了，而这份配置
/// 的价值有一半在注释里（见 `docs/adr/0003`）。原地插入的代价是插进去的块落在该 Device
/// 已有的几块之后，而不是紧挨着草稿留下的那段注释掉的占位——占位于是变成一段历史记录，
/// 草稿里那句"留着或删掉都行"说的就是这件事。
///
/// 与 [`draft`] 一样是纯函数：入参是文本和一次枚举的结果，出参是文本。真正落盘的那一步
/// 在 `cli::config_refresh` 里。
pub fn refresh(text: &str, collections: &[HidInfo]) -> Result<Refreshed> {
    // 读用 serde（要的是"哪一块空着"这个类型化的问题），写用 toml_edit（要的是保住注释）。
    // 同一段文本解析两遍，是这两种需求各自的代价，不是重复劳动。
    let config = Config::parse(text)?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("配置解析失败（toml_edit）")?;
    let mut filled = Vec::new();
    let mut notes = Vec::new();

    for (index, device) in config.devices.iter().enumerate() {
        let recognised = recognise(device);
        let missing: Vec<EndpointKind> = EndpointKind::PRIORITY
            .into_iter()
            .filter(|kind| kind.config_in(device).is_none())
            .collect();

        // ADR-0003 那半句："扫描结果与用户所写不一致时只在命令行提醒"。
        //
        // 不一致指的是**本机此刻在场的身份**和用户写下的那一条对不上——不是"用户写的那条
        // 现在不在场"：鼠标没插线时 Wired 当然不在场，那是常态，每次都印一句就成了噪音，
        // 而噪音会把真正该看的这一句一起淹掉。`report_id` 不参与比较，因为枚举问不出它，
        // 拿它说"扫描结果不一致"是没有依据的。
        if let Some(known) = recognised {
            for kind in EndpointKind::PRIORITY {
                let Some(configured) = kind.config_in(device) else {
                    continue;
                };
                let identity = known.identity(kind);
                if !same_scanned_identity(configured, identity) && is_present(identity, collections)
                {
                    notes.push(format!(
                        "{}：{kind} 你写的是 {}，而本机此刻在场的是 {}（我记下的这台设备的 \
                         {kind} 身份）。没有动它——你写过的值一概不改。",
                        device.id,
                        describe(configured),
                        describe(identity)
                    ));
                }
            }
        }

        if missing.is_empty() {
            continue;
        }
        let Some(known) = recognised else {
            notes.push(format!(
                "{}：认不出这是哪台设备（它已配好的 Endpoint 身份不在程序认得的那张表里），\
                 所以猜不出它缺的 {} 该填什么，得手填。",
                device.id,
                missing
                    .iter()
                    .map(EndpointKind::to_string)
                    .collect::<Vec<_>>()
                    .join("、")
            ));
            continue;
        };

        for kind in missing {
            let identity = known.identity(kind);
            if !is_present(identity, collections) {
                notes.push(format!(
                    "{}：{kind} 现在不在场（本机枚举不到 {}），没有填。{}",
                    device.id,
                    describe(identity),
                    known.absent_hint(kind)
                ));
                continue;
            }
            insert_endpoint_block(&mut doc, index, kind, identity)?;
            // 身份里有没实测过的部分就一并说出来。`is_present` 核对的只有 VID/PID/usage
            // 四项，`report_id` 是枚举问不出来的——那一项若是抄来的猜测，用户有权知道
            // 自己刚拿到的是一个没人验过的值。
            filled.push(format!(
                "{}：补上了 {kind}（{}）。{}",
                device.id,
                describe(identity),
                // 注意事项在草稿里是分行写的（一行放不下），命令行这里要压成一句。
                known.caveat(kind).replace('\n', "")
            ));
        }
    }

    Ok(Refreshed {
        text: doc.to_string(),
        filled,
        notes,
    })
}

/// 一条 Endpoint 身份的人话写法，用在命令行的提醒里。
///
/// 四个字段都印出来：鼠标的有线本体是 `391D:1005`、dongle 是 `391D:1A05`，只差一个字符，
/// 只印一半的话用户根本看不出程序说的是哪一条。
fn describe(endpoint: &HidEndpoint) -> String {
    format!(
        "VID {:04X} PID {:04X} UP {:04X} U {:04X}",
        endpoint.vid, endpoint.pid, endpoint.usage_page, endpoint.usage
    )
}

/// 把一条 Endpoint 的块插进第 `index` 个 `[[device]]` 里。
///
/// 块是**先渲染成文本再解析回来**的：`toml_edit` 保留解析时看到的原始写法，于是
/// `vid = 0x391D` 落到文件里仍然是十六进制，而不是被规范化成 14621。手工构造
/// `Formatted<i64>` 再改 repr 能得到同样的结果，但那要跟 `toml_edit` 的内部表示打交道，
/// 而这里已经有一个"块该长什么样"的唯一写法（[`endpoint_block`]），复用它同时也保证了
/// 补出来的块和草稿里写的块逐字一致。
fn insert_endpoint_block(
    doc: &mut toml_edit::DocumentMut,
    index: usize,
    kind: EndpointKind,
    endpoint: &HidEndpoint,
) -> Result<()> {
    let rendered = endpoint_block(kind, endpoint);
    let snippet = rendered
        .parse::<toml_edit::DocumentMut>()
        .context("渲染出来的 Endpoint 块自己解析不动，这是程序的错，不是配置的错")?;
    let mut block = snippet["device"][kind.config_key()]
        .as_table()
        .ok_or_else(|| anyhow!("渲染出来的 Endpoint 块不是一张表"))?
        .clone();
    // 前面空一行、缩进两格，和这份配置里其余的 Endpoint 块对齐。
    block.decor_mut().set_prefix("\n  ");
    // **这一行不能省。**`toml_edit` 输出时把整份文档的表拉平、按各表解析时记下的
    // "文档内位置"排序；位置为 `None` 的表继承前一张表的位置，于是自然落在它父表的后面。
    // 而上面那张表是从**另一份文档**（那个片段）里克隆出来的，身上带着片段里的位置 1
    // ——搬进主文档就会排到第一个 `[[device]]` 的位置上去。
    //
    // 后果不是排版难看：TOML 里 `[device.wired]` 属于它前面最近的那个 `[[device]]`，
    // 于是键盘的有线身份会挂到鼠标头上，文件照样解析得动，错误直到取数时才以一句
    // "读不到"的面目出现。`tests/config.rs` 里那条
    // `config_refresh_puts_the_block_under_the_right_device` 就是抓这个的。
    block.set_position(None);

    doc["device"]
        .as_array_of_tables_mut()
        .and_then(|tables| tables.get_mut(index))
        .ok_or_else(|| anyhow!("配置里找不到第 {index} 个 [[device]]"))?
        .insert(kind.config_key(), toml_edit::Item::Table(block));
    Ok(())
}
