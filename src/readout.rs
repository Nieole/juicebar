//! 一次取数：这台 Device 这一次拿得出什么读数。
//!
//! 一个完整的故事，从"谁在场"到"这一行的原料"：按 [`EndpointKind::PRIORITY`] 依次试在场
//! 的每一条 Endpoint、按配置里的 `driver` 挑协议驱动、判定这一次让开了哪几条、读不到时退
//! 到状态文件里的上次已知值。**终点是 [`RowReading`]**——再往后是这一轮怎么判（`crate::round`）
//! 与那一行怎么写（菜单设备行与悬停提示，`crate::tray`）。
//!
//! 它住在 crate 根、与 `crate::endpoints` 同级：调用方是托盘的取数线程（`crate::shell`），而取数不归任何
//! 一种呈现。这里**不 re-export** 到别处，调用点一律走这个模块名。

use std::fmt;

use anyhow::Result;

use crate::clock::Timestamp;
use crate::config::{Device, General};
use crate::endpoints::{EndpointKind, EndpointReading, Endpoints};
use crate::sources::{BadFrame, Reading, Transport};
use crate::state::{LastKnown, Provenance};
use crate::vendor_hub::VendorHub;

/// 取一次数：按 [`EndpointKind::PRIORITY`] 依次试在场的每一条 Endpoint，遇第一个
/// 成功即停。
///
/// 靠前的一条不行就**在同一次调用里**接着试下一条，不等下一个轮询周期——拔线那一刻
/// `Wired` 直接从枚举里消失（瞬时不在场而不是一次三秒的超时），降级几乎不要钱。
/// 在场的 Endpoint 全都交不出一份可信的读数，才算[取数失败][NoReading::Failed]——**答了坏帧
/// 也算**（那是读取异常而不是失联：设备答了话，只是答的那一帧不可信），所以那一支的措辞不说
/// "都没读到"，见 [`full_reason`]。有没有哪一条答了坏帧（[`BadFrame`]）在试的时候记下，
/// 取数失败带着它是哪一种来路交出去（[`FailureCause`]）。
///
/// **让开的那几条不在"试过"之列**：厂商上位机在跑时它们压根没被打开，那是[暂停][NoReading::Paused]
/// 而不是取数失败，两者对用户是两句不同的话（[`NoReading`] 上写了理由）。
///
/// `pub` 是因为**它就是这个模块的接口**：「这一次取数拿得出什么读数」这个故事的入口只有它
/// 一条，调用方要的就是它（托盘的取数线程走的是它外面那一层 [`read_or_last_known_with_reason`]）。它顺带也让"拔线 →
/// 同周期降级"这条路径能在 `tests/readout.rs` 里被驱动——集成测试只看得见 pub 接口，
/// 而这条路径在真机上拔一次线才复现一次——但那是这个 `pub` 的好处，不是它的理由。
///
/// `now` 收的是一个**值**而不是 `&dyn Clock`：一次取数只在调用方的顶上（托盘的取数线程，`crate::shell`）问一次
/// "现在几点"，那一个"当下"再被取数与陈旧判定共用。问两次不是浪费而是**错的**——几条 Endpoint 依次
/// 试下来会花掉真实时间（一次鼠标超时就是三秒），两条 HID 各超时一次再落到 Ble，用第二个
/// "当下"去判第一个"当下"盖的时间戳，那一行写出来就不是"0 秒前"而是"6 秒前"。
/// 接缝本身（`crate::clock::Clock`）因此只出现在调用方的顶上，这一层往下全是纯函数。
///
/// `paused_by` 同理是个**值**而不是 `&dyn Processes`：本机在跑哪些进程在调用方的顶上问一次，
/// 一次取数问两次就是白花一次进程枚举（见 `crate::vendor_hub`）。`None` 即
/// 这一次取数不让开任何东西——开关关着、或者名单里一个都没在跑，两种都是它。
pub fn read(
    device: &Device,
    endpoints: &dyn Endpoints,
    paused_by: Option<&VendorHub>,
    now: Timestamp,
) -> Result<EndpointReading, NoReading> {
    let present = endpoints.present(device);
    let mut failures = Vec::new();
    let mut answered_a_bad_frame = false;
    let yielded = yielded_to(paused_by, &present);

    for kind in EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| present.contains(kind))
    {
        // **让开的那几条在这里就跳过，一个通道都不打开。**接缝定成"`present()` 答在场、
        // `open_transport()` 才打开"两个方法（parking lot Q16）就是为了这一刻——绕开
        // 的不只是那一次往返，还有打开通路本身。
        if yielded.contains(&kind) {
            continue;
        }
        // 挑哪个**驱动**不在这个函数里做，在 `read_with_driver` 里；这里只挑 Endpoint。
        let attempt = match kind {
            // Wired 与 Dongle24G 取数都是一次 HID 往返，要经过协议驱动。
            EndpointKind::Wired | EndpointKind::Dongle24G => endpoints
                .open_transport(device, kind)
                .and_then(|transport| read_with_driver(device, kind, transport.as_ref()))
                .map(|reading| EndpointReading::from_hid(kind, reading, now)),
            // Ble 不是往返：它读的是 Windows 攒的属性缓存，**不经过协议驱动**
            // （所以这一支根本不碰 `read_with_driver`——配置里的 `driver` 只作用于
            // HID），也没有 Transport 可开。接缝那一侧直接交成品，因为只有它知道
            // 那份缓存有多旧。这里仍**不写通配分支**：加第四种时编译器还会把人指到
            // 这一行来。
            EndpointKind::Ble => endpoints.read_ble(device, now),
        };
        match attempt {
            Ok(reading) => return Ok(reading),
            // 试过就记下原因，接着试下一条。全都失败时这些原因合起来就是取数失败的解释。
            Err(e) => {
                // 认的是类型，不是那句话里有没有"读取异常"几个字（`docs/adr/0004`）。
                answered_a_bad_frame |= e.downcast_ref::<BadFrame>().is_some();
                failures.push(format!("{kind}: {e:#}"));
            }
        }
    }

    // 让开过至少一条、而剩下的没交出读数：那是**暂停**，不是取数失败。两者要用户做的事相反
    // ——譬如失联那句话把他打发去找一个硬件故障（"设备没插？蓝牙没配对？"），而这里他
    // 该做的是关掉那个上位机。
    //
    // **剩下那几条自己的失败原因一起带走**（多半只有 `Ble` 一条）：它们是真去试过而失败了的，
    // 那一行得交代它们。这份列表空不空怎么改变措辞，写在 `NoReading::Paused` 的 `failures` 上。
    if let Some(hub) = paused_by.filter(|_| !yielded.is_empty()) {
        return Err(NoReading::Paused {
            hub: hub.clone(),
            yielded,
            failures,
        });
    }
    Err(full_reason(device, &failures, answered_a_bad_frame))
}

/// 这一次取数里这个 Device 让开的那几条 Endpoint：在场、且撞见的那个上位机会跟它抢的那些。
///
/// [`read`] 与 [`read_or_last_known`] 共用这一处，好让"什么算让开"只有一个答案——两处各写
/// 一遍，那一行末尾的标注与它解释的行为迟早会对不上。
///
/// 它非空同时是**"这个 Device 到底有没有被暂停"的判据**：本机有上位机在跑不等于这一台受
/// 影响（一副只配了 `Ble` 的耳机压根没有让开的东西），而对那台设备说一句"暂停中"就是一句
/// 与它无关的话。
fn yielded_to(paused_by: Option<&VendorHub>, present: &[EndpointKind]) -> Vec<EndpointKind> {
    paused_by
        .map(|hub| {
            present
                .iter()
                .copied()
                .filter(|kind| hub.pauses(*kind))
                .collect()
        })
        .unwrap_or_default()
}

/// 一份读数都没拿到时的两种下场：**取数失败**（交不出可以印的数，而且不是因为我们没去问），
/// 还是**暂停**（我们主动没去问）。
///
/// 为什么非要一个类型、不能继续压成一句 `anyhow::Error`（票 04 把这笔代价记在本票名下）：
/// 这两种要用户做的事**相反**。譬如失联那句话把他打发去查设备那一头（"设备没插？蓝牙没配对？"），
/// 而暂停期间设备好端端的，该关掉的是那个厂商上位机——把暂停说成
/// 失联，用户会去找一个不存在的硬件故障，而这张票的由来正是这句误导。
///
/// 压成字符串也能印出两句不同的话，但**下游分辨不了**：[`read_or_last_known`] 要按这一维
/// 决定那一行末尾标什么（parking lot Q40 给本票的原话：厂商上位机占着通路是个例外，
/// 那一维要自己说一句），而一个字符串只能被再解析一次。
#[derive(Debug)]
pub enum NoReading {
    /// 在场的 Endpoint 都试过了、一份可信的读数都没拿到，或者一条都不在场、一条都没配
    /// —— **取数失败**。
    ///
    /// **名字不说"失联"**：失联只是它的来路之一（`CONTEXT.md`「取数失败」）。"都试过了"里
    /// 也包括**答了坏帧**那一种，那是读取异常——设备答了话，拿失联说它是假话，所以那一支的
    /// 措辞也不说"都没读到"。几种来路对这一层是同一个下场（手上没有可印的数），所以同住一个
    /// 变体；要用户做的事各不相同，所以**来路跟着里面那个错误走**：[`NoReading::failure_cause`]
    /// 取得回来，菜单那一行按它说短原因（[`NoReading::short_reason`]）。
    ///
    /// 里面那句话由 [`full_reason`] 给出，**开头那个标记也在里面**：`Display` 对这一支一个
    /// 字都不加，往这里塞一个自己攒的 `anyhow::Error` 会印出一句没有标记的话，也说不出来路
    /// （按失联算，见 [`NoReading::failure_cause`]；要带来路就走 [`NoReading::failed`]）。措辞
    /// 与三支怎么分见那一处（parking lot Q19 / Q23 打磨过两轮）。
    Failed(anyhow::Error),
    /// 该走的那几条 HID 让开了，**这一次取数根本没去问** —— **暂停**。
    ///
    /// `yielded` 是真的让开了的那几条（在场、且这个上位机会跟它抢的）。它非空是这个变体
    /// 存在的前提：本机有上位机在跑不等于每一台设备都受影响。
    Paused {
        /// 撞见的那个上位机。那一行要点它的名，那是用户唯一能据以行动的东西。
        hub: VendorHub,
        /// 让开的那几条 Endpoint。
        yielded: Vec<EndpointKind>,
        /// 没让开、真去试过、而且失败了的那几条各自的原因（多半只有 `Ble` 一条）。
        ///
        /// **它空不空改变这一行的措辞**。全空只说明让开的那几条之外什么都没试过，**不说明试了
        /// 就会答话**：接收器插着、设备本体关着机，Dongle24G 照样在场、照样被让开，关掉上位机
        /// 之后它照旧超时（`docs/adr/0001`：接收器有自己一组 VID/PID）。所以全空时那一行只说
        /// 我们关掉它之后会做什么，不说设备会怎样；非空就把原因带上——那几条关掉上位机也不会
        /// 变好。
        failures: Vec<String>,
    },
}

impl NoReading {
    /// 一次取数失败：来路是 `cause`，完整原因是 `full_reason`（`Display` 原样印它，一个字不加）。
    ///
    /// 取数那一层自己造取数失败走的就是它。别处要一个说得出来路的取数失败，也走它：托盘内核的用例
    /// 罐装一次取数结果时不经假枚举，照样造得出三种来路。直接往 [`NoReading::Failed`] 里塞一个
    /// `anyhow::Error` 说不出来路，按失联算。
    pub fn failed(cause: FailureCause, full_reason: impl Into<String>) -> Self {
        Self::Failed(anyhow::Error::new(Failure {
            cause,
            full_reason: full_reason.into(),
        }))
    }

    /// 让开了 Endpoint 的那个上位机。`None` 即这不是暂停而是取数失败。
    ///
    /// [`read_or_last_known`] 拿它把暂停这一维带到交出去的那份读数上（[`RowReading::paused_by`]）：退到上次已知值的
    /// 那一行照样得说清"设备此刻不在"是因为我们没去问，而不是问不到。
    pub fn paused_by(&self) -> Option<&VendorHub> {
        match self {
            Self::Failed(_) => None,
            Self::Paused { hub, .. } => Some(hub),
        }
    }

    /// 取数失败是哪一种来路（[`FailureCause`]）。`None` 即这不是取数失败而是暂停。
    ///
    /// 不经 [`NoReading::failed`] 造的取数失败说不出来路，按**失联**算。今天只有一处：外壳连本机
    /// 有什么都枚举不了的时候自己攒的那一个——那也是去问了（问本机）、问不到。
    pub fn failure_cause(&self) -> Option<FailureCause> {
        match self {
            Self::Failed(reason) => Some(cause_of(reason)),
            Self::Paused { .. } => None,
        }
    }

    /// 菜单那一行的短原因（`CONTEXT.md`「短原因」）：取数失败按来路各一句，暂停一句并点名是哪个
    /// 上位机。
    ///
    /// **四句话只在这里定**，要说短原因的地方都来问它。它不从完整原因（`Display` 那一句）里截：
    /// 完整原因带着每条 Endpoint 各自的错误，一行菜单放不下，截一段又说不清是哪种情形（parking
    /// lot Q152）。所以每一句都**不带任何一条 Endpoint 的错误**，只说是哪种情形，再用问句指一下
    /// 该去哪儿看——问句而不是结论，理由与 [`full_reason`] 里那句"设备没插？"相同：我们只知道
    /// 没读到，不知道为什么。
    ///
    /// - 失联问插线、配对、开机：去问了问不到，最常见的就是这几样。
    /// - 读取异常问有没有上位机在抢通路：那是它最常见的来源（`CONTEXT.md`「读取异常」）。暂停的
    ///   开关开着时，名单上的上位机撞见了就已经是暂停，走到这一句的多半是名单外的那一个。
    /// - 一条 Endpoint 都没配那一句不问：该改的就是配置。
    /// - 暂停那一句与悬停提示说同一件事时是同一个说法（"已暂停（…正在运行）"）。
    pub fn short_reason(&self) -> String {
        match self {
            Self::Failed(reason) => match cause_of(reason) {
                FailureCause::Unreachable => "失联（没插？没配对？没开机？）".to_string(),
                FailureCause::ReadAnomaly => "读取异常（有上位机在抢通路？）".to_string(),
                FailureCause::NoEndpointConfigured => "一条 Endpoint 都没配置".to_string(),
            },
            Self::Paused { hub, .. } => format!("已暂停（{} 正在运行）", hub.process()),
        }
    }
}

/// 装在 [`NoReading::Failed`] 里那个错误的来路：经 [`NoReading::failed`] 造的带着它（[`Failure`]），
/// 别的按失联算（[`NoReading::failure_cause`] 上写了为什么）。认的是类型，不是那句话。
fn cause_of(reason: &anyhow::Error) -> FailureCause {
    reason
        .downcast_ref::<Failure>()
        .map_or(FailureCause::Unreachable, |failure| failure.cause)
}

/// 取数失败的三种来路（`CONTEXT.md`「取数失败」）。
///
/// 三种对"这一格印不印数字"、对 Primary 选择是同一个答案，所以同住 [`NoReading::Failed`]；要用户
/// 做的事各不相同——插线配对、去关上位机、改配置——所以菜单那一行按它各说一句
/// （[`NoReading::short_reason`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureCause {
    /// **失联**：在场的每一条都试过了、一条都没答话，或者一条都不在场——去问了、问不到。
    Unreachable,
    /// **读取异常**：至少一条答了话，而那一帧不可采信（[`BadFrame`]）。与超时同在一次取数里也是
    /// 它：有一条答了话，这一次就不是"一条都没读到"。
    ReadAnomaly,
    /// **配置里一条 Endpoint 都没有**：无处可取，该改的是配置。
    NoEndpointConfigured,
}

/// 取数那一层造的那句取数失败，连同它是哪一种来路。
///
/// 装进 [`NoReading::Failed`] 的就是它：`Display` 原样印那句完整原因、一个字不加，所以来路跟着
/// 走，完整原因逐字节不变。**来路装在错误里面，不在 `Failed` 旁边另开一个字段**：变体的形状不变，
/// 别处按 `Failed(_)` 匹配、自己造一个 `Failed` 的地方（托盘内核、外壳）一行不用改；代价是不经
/// [`NoReading::failed`] 造的那一个说不出来路，只好有个缺省（parking lot Q230、Q231）。
#[derive(Debug)]
struct Failure {
    cause: FailureCause,
    full_reason: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.full_reason)
    }
}

impl std::error::Error for Failure {}

impl fmt::Display for NoReading {
    /// 印出去的就是那一行 Device 名字之后的全部内容。
    ///
    /// **两句话各自完整、不共用前缀**：取数失败那一句整句由 [`full_reason`] 给出、连开头那个
    /// 标记一起，这里**一个字都不加**；暂停那一句以"暂停中"开头，在下面拼。共用一个前缀
    /// 再接不同的后半句，就等于把两种下场说成同一件事。
    ///
    /// 取数失败那个标记因此不在这里选：它按"试过了都失败 / 一条都不在场 / 一条都没配"分说，
    /// 那三条分支长在 [`full_reason`] 里，让措辞跟着判据走。
    ///
    /// 这里印的是**完整原因**，悬停提示与日志用它；菜单那一行放不下它，说的是
    /// 另一句短原因（[`NoReading::short_reason`]）。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(reason) => write!(f, "{reason:#}"),
            Self::Paused {
                hub,
                yielded,
                failures,
            } => {
                let names: Vec<String> = yielded.iter().map(EndpointKind::to_string).collect();
                write!(
                    f,
                    "暂停中 —— {} 正在运行，{} 让开（同时发命令会互相覆盖对方的应答）",
                    hub.process(),
                    names.join("、"),
                )?;
                // 列表空不空改变这半句，理由写在 `failures` 那个字段上：空的时候只说我们关掉
                // 它之后会做什么，不说设备会怎样；试过而失败的那几条带上自己的原因。
                match failures.is_empty() {
                    true => write!(f, "，关掉它之后这几条才试得到"),
                    false => write!(f, "；剩下的也没读到 —— {}", failures.join("；")),
                }
            }
        }
    }
}

/// 这一次取数拿得出来的读数：先按优先级取一次，取不到就退到状态文件里的上次已知值。
///
/// **"失联"与"没有已知值"是两回事，这个函数把它们分开。**设备收进抽屉、关了机、接收器拔了
/// ——这一次取数确实读不到，但用户仍然该看见"上次是多少电"（票面第 2 条）。只有连历史值都没有
/// （或者它已经过了 `very_stale_after`）才是那句"读不到"。
///
/// 交出来的 [`Provenance`] 是**下游要分辨的那一维**：菜单那一行与悬停提示拿它标注（一份十秒前的历史值
/// 照样不是现状）。把它塞进 [`EndpointReading`] 是另一条路，但那个结构描述的是"设备说了
/// 什么"，而这一维说的是"这个数从哪儿来的"，两者不同源（parking lot Q39）。
///
/// **记账也在这里：读到了就写进 `last_known`，退到历史值那一支一个字都不写。**两件事合在
/// 一个函数里不是图省事——"每次成功的 Reading 写入状态文件"和"失联时拿出上次已知值"是同一
/// 个决定的两面，分开写就出现一个只活在调用方里、用例碰不到的分支。把拿出来的历史值再
/// 写一遍会刷新写盘时刻，于是那条记录永远到不了 `very_stale_after`：一只收进抽屉半年的鼠标
/// 每天被看一眼就每天年轻一天，而"这个历史值必须会过期"这句话永久落空、没有任何症状。
///
/// 退不到历史值时交出来的错误是**那一次取数的原因**，不是"文件里没有"：用户要修的是设备
/// 那一头，而不是一个他不该知道存在的缓存文件。
///
/// `pub` 与 [`read`] 同理：它是这个模块交出去的另一半接口——一次真机上的失联要靠
/// 拔线或者等设备睡着才复现一次，而这条路径上错一步的症状是"重启就失忆"，没有任何报错。
pub fn read_or_last_known(
    device: &Device,
    endpoints: &dyn Endpoints,
    paused_by: Option<&VendorHub>,
    last_known: &mut LastKnown,
    general: &General,
    now: Timestamp,
) -> Result<RowReading, NoReading> {
    read_or_last_known_with_reason(device, endpoints, paused_by, last_known, general, now).row
}

/// [`read_or_last_known`] 交的那一份，连同它在退到上次已知值那一支里咽下去的那句原因。
///
/// 那一支交出去的是一份读数（上次已知值），这一次取数为什么没读到在那一份里没有位置；而托盘要把它写进日志、
/// 在菜单那一行写它的短原因（parking lot Q234）：图标上那只是一个灰的旧数，这却是最常见的那一种
/// 失败（设备收进了抽屉、接收器拔了，或者没有管理员权限，`docs/gaps.md`）。
pub struct Outcome {
    /// 交给这一轮的那一份，与 [`read_or_last_known`] 交的一模一样。
    pub row: Result<RowReading, NoReading>,
    /// 退到了上次已知值时，这一次取数自己为什么没读到。
    ///
    /// 读到了是 `None`；退不到时也是 `None`——那时原因就是 `row` 的 `Err`，不在两处各放一份。
    pub fell_back_because: Option<NoReading>,
}

/// 同 [`read_or_last_known`]，外加退到上次已知值时这一次取数自己的原因（[`Outcome`]）。
///
/// 记账、退路、暂停那一维的判法全在这一个函数里，[`read_or_last_known`] 只是丢掉那句原因的简写：
/// 两份各写一遍，退路那一支迟早会漂开。
pub fn read_or_last_known_with_reason(
    device: &Device,
    endpoints: &dyn Endpoints,
    paused_by: Option<&VendorHub>,
    last_known: &mut LastKnown,
    general: &General,
    now: Timestamp,
) -> Outcome {
    match read(device, endpoints, paused_by, now) {
        Ok(reading) => {
            last_known.record(&device.id, &reading, now);
            let row = Ok(RowReading {
                reading,
                provenance: Provenance::JustRead,
                // **读到了也可能有 Endpoint 让开过**：`Ble` 顶上的那一刻，两条 HID 一个都
                // 没被问。那一行照样要说一句——一个可能是几个月前的缓存数字突然顶上来，
                // 用户该知道那是因为上位机在跑，而不是设备出了事。
                //
                // 判据走的是 [`yielded_to`]——与 `read` 里跳过那几条时问的是同一个函数。
                // 这里再问一次 `present()` 是安全的：它是枚举快照上的纯筛选（幂等、不碰
                // 系统，见 `SystemEndpoints`），与 Clock 那一条"只许问一次"不是同一类问题
                // （parking lot Q35）。
                paused_by: paused_by
                    .filter(|_| !yielded_to(paused_by, &endpoints.present(device)).is_empty())
                    .cloned(),
            });
            Outcome {
                row,
                fell_back_because: None,
            }
        }
        // 历史值只在这一支里被问到：读得到的时候它不参与竞争。让它参与，一份存了半天的
        // 读数就可能盖掉当场问出来的那个数——而这条链路上其余每一处守的都是相反的规矩
        // （Endpoint 优先级、`Ble` 排最后）。
        Err(no_reading) => fall_back_to_last_known(device, no_reading, last_known, general, now),
    }
}

/// 这一次取数没读到（`no_reading`）：退到状态文件里的上次已知值，退不到就交出那句原因。
///
/// [`read_or_last_known_with_reason`] 读不到的那一支就是它。托盘在连本机都枚举不了、根本没法去读的时候
/// 也走它：那时一样该拿上次已知值顶上（`CONTEXT.md`「上次已知值」），而不是让一个灰的旧数跳成取数失败。
///
/// **它只读 `last_known`、不记账**：拿出来的历史值不许再喂回去（[`read_or_last_known`] 上写了为什么）。
pub fn fall_back_to_last_known(
    device: &Device,
    no_reading: NoReading,
    last_known: &LastKnown,
    general: &General,
    now: Timestamp,
) -> Outcome {
    // 暂停这一维要跟着历史值一起交出去：那一行末尾非说一句不可（parking lot Q40
    // 给本票的原话），而说得出话的只有取数那一步——它才知道这一次让开了什么。
    let paused_by = no_reading.paused_by().cloned();
    match last_known.reading_for(&device.id, general, now) {
        Some(reading) => Outcome {
            row: Ok(RowReading {
                reading,
                provenance: Provenance::LastKnown,
                paused_by,
            }),
            fell_back_because: Some(no_reading),
        },
        None => Outcome {
            row: Err(no_reading),
            fell_back_because: None,
        },
    }
}

/// 一行 Device 背后的那份读数，连同它写出去时要交代的两件事。
///
/// 名字说的是菜单里那一行（与悬停提示）的原料。
/// （不叫 `Readout`——parking lot Q16 与 Q31 里那个词指的是另一样东西：一个把"读到了 /
/// 暂停 / 取数失败"合成一个类型的三态枚举，也就是这里**没有**走的那条路。）
///
/// 给它一个名字而不是交一个三元组：靠位置解构的元组，加一样时每一处调用都要改。
///
/// 三样捆在一起是因为它们**同时产生、同时被用掉**（这一轮怎么判在 `crate::round`，那一行怎么写在 `crate::tray::hover`）：
/// 一个数、它从哪儿来的、以及这一次取数有没有 Endpoint 让开。后两样都不是 [`EndpointReading`] 的一部分——那个结构
/// 描述的是"设备说了什么"（parking lot Q39）。
pub struct RowReading {
    /// 要印出去的那一份读数。
    pub reading: EndpointReading,
    /// 它是这一次取数读到的，还是从状态文件里拿出来的上次已知值。
    pub provenance: Provenance,
    /// 这一次取数让开过 Endpoint 的那个厂商上位机，`None` 即这台设备没让开任何东西。
    pub paused_by: Option<VendorHub>,
}

/// 取数失败时印出去的**整句完整原因，开头那个标记也在内**，连同它是哪一种来路。
///
/// **整句只有这一处出处**：[`NoReading`] 的 `Display` 对取数失败那一支一个字都不加。原先标记
/// 在那边、这句话在这边，两处说的是同一件事——要改一句话得找齐两个前缀，而那两句还都不准。
///
/// 分三支，因为它们要用户做的事完全不同：一条都不在场多半是设备没插或配置写错，而在场却
/// 交不出一份可信的读数才是设备那头出了事。**标记两个**：
///
/// - **试过了、都失败** → "没有可信的读数"。中性是故意的：它对超时与读取异常都成立，而
///   "都没读到"对一台答了坏帧的设备是假话——我们读到了，只是读到的东西不可信
///   （`CONTEXT.md`「读取异常」）。卖掉的那点精确度，后面每条 Endpoint 自己的原因补上了，
///   所以那句总结本来没有加信息。
/// - **一条都不在场** / **一条都没配** → "读不到"，一个字没改：一条通路都没枚举到，我们
///   确实什么都没读到，这句话在这两支上是实话。
///
/// **这句话的标记不按来路分**（超时还是读取异常）：试过了都失败那一支照旧是中性的那一句，
/// 每条 Endpoint 自己的原因紧跟其后，是哪一种已经说清了。来路另交（[`FailureCause`]），给
/// 菜单那一行的短原因用——那里放不下每一条的原因，只能按来路说。有一条答了坏帧
/// （`answered_a_bad_frame`）就是读取异常：有一条答了话，这一次就不是"一条都没读到"。这个
/// 分类要穿过驱动的错误路径（[`BadFrame`]），`docs/adr/0004` 当初否掉的正是它，那份 ADR 末尾
/// 记着它怎么被推翻的。
///
/// "不在场"那句**不能只说"设备没插"**：本机枚举得到、却发不出这个协议要的那种报文的
/// 通路同样算不在场（键盘的 vendor collection 实测就是 `in:0 out:0 feat:65`），那时设备
/// 好端端插着，只是这条通路还没被接上。所以话要说到"枚举不到它们"为止，把没插当成一个
/// 问句而不是结论。
///
/// 三条 Endpoint 之后这句话不再点名"输出报文"：报文种类现在是**驱动**那一维的事
/// （键盘走 feature 报文），而 `Ble` 压根不是 HID——它不在场是因为本机的 BLE 设备里没有
/// 那个地址。一句话要同时对三种成立，就只能说到"枚举不到"，把三个可能的原因摆成问句。
fn full_reason(device: &Device, failures: &[String], answered_a_bad_frame: bool) -> NoReading {
    if !failures.is_empty() {
        return NoReading::failed(
            if answered_a_bad_frame {
                FailureCause::ReadAnomaly
            } else {
                FailureCause::Unreachable
            },
            format!("没有可信的读数 —— {}", failures.join("；")),
        );
    }
    // 问的是 `is_configured_in` 而不是 `hid_config_in`：后者对 Ble 恒为 None，拿它筛会把
    // 蓝牙从这份名单里漏掉，而这份名单就是要告诉用户"你配了这几条"。
    let configured: Vec<String> = EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| kind.is_configured_in(device))
        .map(|kind| kind.to_string())
        .collect();
    if configured.is_empty() {
        return NoReading::failed(
            FailureCause::NoEndpointConfigured,
            "读不到 —— 这个 Device 一条 Endpoint 都没配置",
        );
    }
    // "登记设备"是托盘右键菜单里那一项的名字（`crate::tray::register`）：本机扫到、还没登记的设备列在那里，
    // 成品里看得到本机有哪些设备的地方只有它。那一项改名，这句跟着改。
    NoReading::failed(
        FailureCause::Unreachable,
        format!(
            "读不到 —— 配置的 Endpoint（{}）一条都不在场：本机枚举不到它们（设备没插？蓝牙没配对？到\"登记设备\"里看看本机扫到了什么）",
            configured.join("、")
        ),
    )
}

/// 按配置里的 `driver` 挑协议驱动，取一次数。
///
/// **驱动分发只在这个函数里做，[`read`] 里不再做。**下面那几行是从 `read` 里原样
/// 搬过来的，一个字没改：`read` 现在只管 Endpoint 的优先级与降级，两维正交
/// ——加一个驱动改这里，加一条 Endpoint 改 `read`。合并时若看到 `read` 里还留着
/// 驱动分发，那是"搬家"和"改写"撞在了一起，正确的归宿是这里。
///
/// `_endpoint` 现在没人用：一个驱动要覆盖该协议家族的所有 Endpoint 种类，内部按
/// 种类分支（键盘的 Dongle24G 与 Wired 命令不同；鼠标两者相同，分支自然退化），
/// 所以这个参数是留给驱动那一维的，不是留给 `read` 的。
fn read_with_driver(
    device: &Device,
    _endpoint: EndpointKind,
    transport: &dyn Transport,
) -> Result<Reading> {
    // 配置里可以出现本次编译还没实现的驱动名，那该是这一行写着"尚未实现"。
    device.protocol_driver()?.read_battery(transport)
}
