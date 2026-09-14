//! 自举：把一次 HID 枚举变成一份配置草稿。
//!
//! 扫到什么写什么，猜不动的留成注释、并写明补全办法。生成逻辑和那批**用户可见的文案
//! 常量**一起住在这里：改一句提示只动这一个文件，不必在 schema 和 `toml_edit` 之间翻页。
//!
//! `endpoint_block` 也住在这里，而 `edit` 那一侧的插入复用它——草稿写下的块与
//! 自动补空块补出来的块因此逐字一致。

use std::collections::BTreeMap;

use crate::config::HidEndpoint;
use crate::config::known_devices::{KNOWN_DEVICES, is_present};
use crate::endpoints::EndpointKind;
use crate::hid::HidInfo;

/// 一条 Endpoint 的块名那一行，缩进两格。
///
/// 单独抽出来是因为它有两个写处：[`endpoint_block`] 那几行真块，以及身份表里没有身份时
/// 那个只剩块名的占位。手抄第二遍就是给"块该长什么样只有一处写法"开一个口子——而那条
/// 规矩正是 [`blank_endpoint_block`] 的文档立下来的。
fn block_header(kind: EndpointKind) -> String {
    format!("  [device.{}]\n", kind.config_key())
}

/// 一条 Endpoint 的配置块，缩进两格跟在它的 `[[device]]` 下面。
pub(super) fn endpoint_block(kind: EndpointKind, endpoint: &HidEndpoint) -> String {
    format!(
        "{}  \
         vid = 0x{:04X}\n  \
         pid = 0x{:04X}\n  \
         usage_page = 0x{:04X}\n  \
         usage = 0x{:04X}\n  \
         report_id = {}\n",
        block_header(kind),
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
            .filter_map(|kind| known.identity(*kind))
            .any(|identity| is_present(identity, collections))
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
        // **三条 Endpoint 都从这一个循环出来**，包括身份表里根本没有身份的 Ble：它以前是一段
        // 硬写的常量，措辞和排布与另两条各说各话。
        for kind in EndpointKind::PRIORITY {
            let identity = known.identity(kind);
            let present = identity.filter(|identity| is_present(identity, collections));
            out.push('\n');
            // 有身份、而且此刻在场，才写成真块；其余一律是注释掉的占位（Ble 恒走后者）。
            if let Some(identity) = present {
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
    }
    if !wrote_a_device {
        out.push_str(NOTHING_RECOGNISED);
    }
    out.push_str(&unrecognised_section(collections));
    out
}

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
# 插上之后程序会自动补上：该插的插好（键盘还要把机身模式开关拨到有线档），托盘开着时
# 会再扫一遍、把此刻在场而配置里空着的 Endpoint 块填进来，并弹一条通知说补了哪台的哪条；
# 托盘启动时也扫一遍。**只填空着的，你写过的值和注释一概不动**（见 docs/adr/0003）。
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

# 低电量阈值（百分比）：电量低于它就算低电，托盘图标换成低电的样子。可被单个 [[device]] 覆盖。
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
/// **这里让用户删掉文件、重新启动，而不是"插上之后程序会自动补上"**，因为后者在这里是假话：
/// [`refresh`] 只往已有的 `[[device]]` 里补空着的块，它不新增 Device（那条边界见
/// `.scratch/parking-lot.md` 的 Q49；托盘里新增 Device 只能由用户在菜单里点，resident-tray 票 12）。
/// 而这份文件此刻没有任何 Device，删掉它不损失任何东西——里面只有用户还没动过的默认值。
///
/// **光删掉不够，要重新启动**：托盘运行中配置文件没了，当作读不了、沿用上一份，不重新生成草稿（parking lot
/// Q253）；草稿只在启动时没有配置文件才写。这句话怎么改的记在 parking lot Q272。
///
/// [`refresh`]: crate::config::refresh
const NOTHING_RECOGNISED: &str = "\
\n# 本机一台认得出来的设备都没扫到，所以这份草稿里没有任何 [[device]]，它还派不上用场。
#
# 可能是设备或接收器没插，也可能它根本不在程序认得的那张表里（表在
# src/config/known_devices.rs 的 KNOWN_DEVICES）。跑 `juicebar scan` 看本机到底有哪些
# HID collection。
#
# 插好之后**把这个文件删掉，再重新启动 juicebar**（右键托盘图标点\"退出\"，再打开它），程序会
# 重新扫一遍、生成一份带 Device 的新草稿。（光插上不够：插上之后程序只往已有的 [[device]]
# 里补空着的块，不会替你新增一台设备。这个文件现在没有你动过的东西，删掉不损失什么。）
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
            EndpointKind::PRIORITY
                .into_iter()
                .filter_map(move |kind| known.identity(kind))
                .map(|identity| (identity.vid, identity.pid))
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

/// 一条补不上的 Endpoint 在草稿里要说的那几句话。
///
/// 四样各自具名而不是一个位置元组：每条 `match` 臂都要把它们从头写一遍，元组读到第三个
/// 逗号才知道那句是 `remedy`。
struct Unfillable {
    /// 开头那句里"为什么补不上"的那半句。
    why: String,
    /// 下面那几行是什么、可不可信。
    provenance: String,
    /// 弄好之后它怎么才能变成一个真块。
    remedy: String,
    /// 注释掉之前的那几行 TOML。**逐种类自己交**，不在外面按"有没有身份"再分一次岔：
    /// 那会是第二个沿同一根轴的开关，而两个开关咬合靠的是一条谁都没写下来的不变式
    /// ——将来某个既有身份、又有键要人自己抄的变体会被静默丢掉一半。
    block: String,
}

/// 补不上的那条 Endpoint 在草稿里的样子：一段说明，加上逐行注释掉的那几行 TOML。
///
/// **三条 Endpoint 走的是同一套**——同样的开头、同样的"怎么让它出现"、同样的 `  # ` 缩进。
/// 逐种类不同的东西全由下面那个穷举 `match` 交出来；身份表里没有身份的那一种走的也是
/// 这里，不再是另写一段常量。
///
/// **但正文必然不同，那不是不一致**：两条 HID 的占位里是一组实测记下来的
/// VID/PID/usage，而 Ble 没有这样一组身份可写——BLE 射频是另一颗芯片，与 dongle 之间
/// 没有能缝合的字段（`KnownDevice::identity` 对它恒为 `None` 就是这句话的类型写法）。
/// 给它凑一组看着像真的身份，正是这一片代码存在的目的所要拒绝的那件事。
///
/// 为什么非要写这一段而不是干脆省掉：一条 Endpoint 悄悄缺席，用户既不知道自己少了什么，
/// 也不知道该做什么才能补上——而缺的偏偏常常是 Wired，也就是设备插着线充电、最需要看见
/// 电量的那一刻。
fn commented_placeholder(
    kind: EndpointKind,
    identity: Option<&HidEndpoint>,
    hint: &str,
    caveat: &str,
) -> String {
    let key = kind.config_key();
    // **这个 `match` 不写通配分支，是本函数存在的一半理由**：加第四种 Endpoint 时编译器
    // 会把人指到这里，草稿于是不会静默少说一条。以前 Ble 那一段是个硬写的常量、根本不
    // 经过这里，那正是这道守门缺的那一角。
    let Unfillable {
        why,
        provenance,
        remedy,
        block,
    } = match kind {
        // 两条 HID 的身份实测记在表里，补不上只是因为**此刻扫不到**——插上之后
        // 托盘就能把它补成真块（自动补空块，`crate::tray::config`）。
        EndpointKind::Wired | EndpointKind::Dongle24G => Unfillable {
            why: format!("{kind} 现在扫不到"),
            // 下面那组身份可不可信，要么说"实测记下来的"，要么把没实测的那一项点出来——
            // 两句话不能同时出现，否则用户读到的是自相矛盾。
            provenance: if caveat.is_empty() {
                "下面那组身份是实测记下来的。".to_string()
            } else {
                caveat.to_string()
            },
            remedy: format!(
                "插上之后程序会自动补上：托盘开着时它一出现、或者托盘启动时它在场，程序就把真正的\n\
                 [device.{key}] 写进来，并弹一条通知（只填空着的块，你写过的值和注释一概不动）。\n\
                 到那时这几行就只是历史记录了，留着或删掉都行。"
            ),
            // 正文是 [`endpoint_block`] 的输出，用户把 `#` 去掉得到的正是程序自己会写的
            // 那几行，两者之间不会漂。身份表对这两种恒有身份；真落到 `None`，块里退成一个
            // 光块名，而不是无声无息地少掉一整段。
            block: identity.map_or_else(|| block_header(kind), |id| endpoint_block(kind, id)),
        },
        // Ble 不是"这次扫不到"，是**程序永远不该猜**：身份表里没有 BLE 身份，因为 BLE
        // 射频是另一颗芯片，与 dongle 之间没有能缝合的字段。
        //
        // 这里也**不给草稿加一次 BLE 扫描**：扫出来的地址没有能归属到某一台 Device 的
        // 字段，只能整份列出来，而那正是 `juicebar scan` 已经在印的东西。
        EndpointKind::Ble => Unfillable {
            why: format!("{kind} 要一个蓝牙地址，而程序猜不出来"),
            provenance: "身份表里没有 BLE 身份，也猜不出来：BLE 射频是另一颗芯片，\n\
                         连 VID 都和 dongle 不同，几套身份之间没有能自动缝合的字段。"
                .to_string(),
            remedy: "`juicebar config-refresh` 也补不上它：跑 `juicebar scan`，看「蓝牙（BLE）\n\
                     电量」那一段，把地址抄到下面来。没有蓝牙、或者不想用它，这几行删掉即可。"
                .to_string(),
            // 它配的不是一组 VID/PID，是一个地址，所以块里只有块名和那一个等人抄的键。
            block: format!(
                "{}  address = \"把 scan 里那串 12 位十六进制抄过来\"\n",
                block_header(kind)
            ),
        },
    };

    let mut out = format!(
        "  # {why}，所以下面这一块是**注释**，不是配置。\n\
         \x20 # 怎么让它出现：{hint}\n"
    );
    for line in provenance.lines().chain(remedy.lines()) {
        out.push_str(&format!("  # {line}\n"));
    }
    out.push_str(&comment_out(&block, "  # "));
    out
}
