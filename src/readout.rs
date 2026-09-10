//! 取数：这一趟这台 Device 拿得出什么读数。
//!
//! 一个完整的故事，从"谁在场"到"这一行的原料"：按 [`EndpointKind::PRIORITY`] 依次试在场
//! 的每一条 Endpoint、按配置里的 `driver` 挑协议驱动、判定这一趟让开了哪几条、读不到时退
//! 到状态文件里的上次已知值。**终点是 [`RowReading`]**——再往后是排版，那在
//! `crate::cli::status`。
//!
//! **它不是 CLI 的一部分**，所以住在 crate 根、与 `crate::endpoints` 同级：`status` 只是
//! 今天唯一的调用方，常驻轮询那一步会是第二个。也因此这里**不 re-export**：
//! `cli::status::read` 搬家之后会指向一个不该拥有取数的模块，那是一句假话，调用点一律
//! 走这个模块名。

use std::fmt;

use anyhow::{Result, anyhow};

use crate::clock::Timestamp;
use crate::config::{Device, General};
use crate::endpoints::{EndpointKind, EndpointReading, Endpoints};
use crate::sources::{Reading, Transport, driver_for};
use crate::state::{LastKnown, Provenance};
use crate::vendor_hub::VendorHub;

/// 取一次数：按 [`EndpointKind::PRIORITY`] 依次试在场的每一条 Endpoint，遇第一个
/// 成功即停。
///
/// 靠前的一条不行就**在同一次调用里**接着试下一条，不等下一个轮询周期——拔线那一刻
/// `Wired` 直接从枚举里消失（瞬时不在场而不是一次三秒的超时），降级几乎不要钱。
/// 在场的 Endpoint 全都交不出一份可信的读数，才算[取数失败][NoReading::Failed]——**答了坏帧
/// 也算**（那是读取异常而不是失联：设备答了话，只是答的那一帧不可信），所以那一支的措辞不说
/// "都没读到"，见 [`lost_reason`]。
///
/// **让开的那几条不在"试过"之列**：厂商上位机在跑时它们压根没被打开，那是[暂停][NoReading::Paused]
/// 而不是取数失败，两者对用户是两句不同的话（[`NoReading`] 上写了理由）。
///
/// `pub` 是因为**它就是这个模块的接口**：「这一趟拿得出什么读数」这个故事的入口只有它
/// 一条，调用方要的就是它（今天是 `status`，常驻轮询会是第二个）。它顺带也让"拔线 →
/// 同周期降级"这条路径能在 `tests/readout.rs` 里被驱动——集成测试只看得见 pub 接口，
/// 而这条路径在真机上拔一次线才复现一次——但那是这个 `pub` 的好处，不是它的理由。
///
/// `now` 收的是一个**值**而不是 `&dyn Clock`：整条链路只在 [`run`] 里问一次"现在几点"，
/// 那一个"当下"再被取数与陈旧判定共用。问两次不是浪费而是**错的**——几条 Endpoint 依次
/// 试下来会花掉真实时间（一次鼠标超时就是三秒），两条 HID 各超时一次再落到 Ble，用第二个
/// "当下"去判第一个"当下"盖的时间戳，那一行印出来就不是"0 秒前"而是"6 秒前"。
/// 接缝本身（`crate::clock::Clock`）因此只出现在 [`run`] 的顶上，这一层往下全是纯函数。
///
/// `paused_by` 同理是个**值**而不是 `&dyn Processes`：本机在跑哪些进程只在 [`run`] 的顶上
/// 问一次，一趟 `status` 问两次就是白花一次进程枚举（见 `crate::vendor_hub`）。`None` 即
/// 这一趟不让开任何东西——开关关着、或者名单里一个都没在跑，两种都是它。
///
/// [`run`]: crate::cli::status::run
pub fn read(
    device: &Device,
    endpoints: &dyn Endpoints,
    paused_by: Option<&VendorHub>,
    now: Timestamp,
) -> Result<EndpointReading, NoReading> {
    let present = endpoints.present(device);
    let mut failures = Vec::new();
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
            Err(e) => failures.push(format!("{kind}: {e:#}")),
        }
    }

    // 让开过至少一条、而剩下的没交出读数：那是**暂停**，不是取数失败。两者要用户做的事相反
    // ——譬如失联那句话把他打发去找一个硬件故障（"设备没插？跑 `juicebar scan`"），而这里他
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
    Err(NoReading::Failed(lost_reason(device, &failures)))
}

/// 这一趟这个 Device 让开的那几条 Endpoint：在场、且撞见的那个上位机会跟它抢的那些。
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
/// 这两种要用户做的事**相反**。譬如失联那句话把他打发去查设备那一头（"设备没插？蓝牙没配对？
/// 跑 `juicebar scan`"），而暂停期间设备好端端的，该关掉的是那个厂商上位机——把暂停说成
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
    /// 变体；要用户做的事各不相同，那交给里面那句话去分说。
    ///
    /// 里面那句话由 [`lost_reason`] 给出，**开头那个标记也在里面**：`Display` 对这一支一个
    /// 字都不加，往这里塞一个自己攒的 `anyhow::Error` 会印出一句没有标记的话。措辞与三支
    /// 怎么分见那一处（parking lot Q19 / Q23 打磨过两轮）。
    Failed(anyhow::Error),
    /// 该走的那几条 HID 让开了，**这一趟根本没去问** —— **暂停**。
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
    /// 让开了 Endpoint 的那个上位机。`None` 即这不是暂停而是取数失败。
    ///
    /// [`read_or_last_known`] 拿它把暂停这一维带给 [`render`]：退到上次已知值的那一行
    /// 照样得说清"设备此刻不在"是因为我们没去问，而不是问不到。
    ///
    /// [`render`]: crate::cli::status::render
    pub fn paused_by(&self) -> Option<&VendorHub> {
        match self {
            Self::Failed(_) => None,
            Self::Paused { hub, .. } => Some(hub),
        }
    }
}

impl fmt::Display for NoReading {
    /// 印出去的就是那一行 Device 名字之后的全部内容。
    ///
    /// **两句话各自完整、不共用前缀**：取数失败那一句整句由 [`lost_reason`] 给出、连开头那个
    /// 标记一起，这里**一个字都不加**；暂停那一句以"暂停中"开头，在下面拼。共用一个前缀
    /// 再接不同的后半句，就等于把两种下场说成同一件事。
    ///
    /// 取数失败那个标记因此不在这里选：它要按"试过了都失败 / 一条都不在场 / 一条都没配"分说，
    /// 而这一层只看得见 `Failed` 这一个变体。**这不是把话藏起来**——那三条分支本来就长在
    /// [`lost_reason`] 里，让措辞跟着判据走；这一层要再问一次"那到底是哪一种"的代价
    /// 写在那个函数上。
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

/// 这一趟拿得出来的读数：先按优先级取一次，取不到就退到状态文件里的上次已知值。
///
/// **"失联"与"没有已知值"是两回事，这个函数把它们分开。**设备收进抽屉、关了机、接收器拔了
/// ——这一趟确实读不到，但用户仍然该看见"上次是多少电"（票面第 2 条）。只有连历史值都没有
/// （或者它已经过了 `very_stale_after`）才是那句"读不到"。
///
/// 交出来的 [`Provenance`] 是**下游要分辨的那一维**：[`render`] 拿它标注（一份十秒前的历史值
/// 照样不是现状）。把它塞进 [`EndpointReading`] 是另一条路，但那个结构描述的是"设备说了
/// 什么"，而这一维说的是"这个数从哪儿来的"，两者不同源（parking lot Q39）。
///
/// **记账也在这里：读到了就写进 `last_known`，退到历史值那一支一个字都不写。**两件事合在
/// 一个函数里不是图省事——"每次成功的 Reading 写入状态文件"和"失联时拿出上次已知值"是同一
/// 个决定的两面，分开写就出现一个只活在 [`run`] 里、用例碰不到的分支。把拿出来的历史值再
/// 写一遍会刷新写盘时刻，于是那条记录永远到不了 `very_stale_after`：一只收进抽屉半年的鼠标
/// 每天被看一眼就每天年轻一天，而"这个历史值必须会过期"这句话永久落空、没有任何症状。
///
/// 退不到历史值时交出来的错误是**取数那一趟的原因**，不是"文件里没有"：用户要修的是设备
/// 那一头，而不是一个他不该知道存在的缓存文件。
///
/// `pub` 与 [`read`] 同理：它是这个模块交出去的另一半接口——一次真机上的失联要靠
/// 拔线或者等设备睡着才复现一次，而这条路径上错一步的症状是"重启就失忆"，没有任何报错。
///
/// [`render`]: crate::cli::status::render
/// [`run`]: crate::cli::status::run
pub fn read_or_last_known(
    device: &Device,
    endpoints: &dyn Endpoints,
    paused_by: Option<&VendorHub>,
    last_known: &mut LastKnown,
    general: &General,
    now: Timestamp,
) -> Result<RowReading, NoReading> {
    match read(device, endpoints, paused_by, now) {
        Ok(reading) => {
            last_known.record(&device.id, &reading, now);
            Ok(RowReading {
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
            })
        }
        // 历史值只在这一支里被问到：读得到的时候它不参与竞争。让它参与，一份存了半天的
        // 读数就可能盖掉当场问出来的那个数——而这条链路上其余每一处守的都是相反的规矩
        // （Endpoint 优先级、`Ble` 排最后）。
        Err(no_reading) => {
            // 暂停这一维要跟着历史值一起交出去：那一行末尾非说一句不可（parking lot Q40
            // 给本票的原话），而说得出话的只有取数那一步——它才知道这一趟让开了什么。
            let paused_by = no_reading.paused_by().cloned();
            last_known
                .reading_for(&device.id, general, now)
                .map(|reading| RowReading {
                    reading,
                    provenance: Provenance::LastKnown,
                    paused_by,
                })
                .ok_or(no_reading)
        }
    }
}

/// 一行 Device 背后的那份读数，连同它印出去时要交代的两件事。
///
/// 名字跟着 [`DeviceRow`] 走：那个结构是**印出来的那一行**，这个是它的原料。
/// （不叫 `Readout`——parking lot Q16 与 Q31 里那个词指的是另一样东西：一个把"读到了 /
/// 暂停 / 取数失败"合成一个类型的三态枚举，也就是这里**没有**走的那条路。）
///
/// 给它一个名字而不是交一个三元组，理由与 [`DeviceRow`] 那一条相同：靠位置解构的元组，
/// 加第三样时每一处调用都要改——而本票就是来加第三样的那张票。
///
/// 三样捆在一起是因为它们**同时产生、同时被 [`render`] 用掉**：一个数、它从哪儿来的、
/// 以及这一趟有没有 Endpoint 让开。后两样都不是 [`EndpointReading`] 的一部分——那个结构
/// 描述的是"设备说了什么"（parking lot Q39）。
///
/// [`DeviceRow`]: crate::cli::status::DeviceRow
/// [`render`]: crate::cli::status::render
pub struct RowReading {
    /// 要印出去的那一份读数。
    pub reading: EndpointReading,
    /// 它是这一趟读到的，还是从状态文件里拿出来的上次已知值。
    pub provenance: Provenance,
    /// 这一趟让开过 Endpoint 的那个厂商上位机，`None` 即这台设备没让开任何东西。
    pub paused_by: Option<VendorHub>,
}

/// 一份读数都没拿到时印出去的**整句话，开头那个标记也在内**。
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
/// **不按失败的种类分**（超时还是读取异常）：那要一个分类枚举穿过每个驱动的错误路径，
/// 而 `docs/adr/0004` 否掉的正是它——外层措辞要的那个区分，上面这三支已经免费给了。
///
/// "不在场"那句**不能只说"设备没插"**：本机枚举得到、却发不出这个协议要的那种报文的
/// 通路同样算不在场（键盘的 vendor collection 实测就是 `in:0 out:0 feat:65`），那时设备
/// 好端端插着，只是这条通路还没被接上。所以话要说到"枚举不到它们"为止，把没插当成一个
/// 问句而不是结论。
///
/// 三条 Endpoint 之后这句话不再点名"输出报文"：报文种类现在是**驱动**那一维的事
/// （键盘走 feature 报文），而 `Ble` 压根不是 HID——它不在场是因为本机的 BLE 设备里没有
/// 那个地址。一句话要同时对三种成立，就只能说到"枚举不到"，把三个可能的原因摆成问句。
fn lost_reason(device: &Device, failures: &[String]) -> anyhow::Error {
    if !failures.is_empty() {
        return anyhow!("没有可信的读数 —— {}", failures.join("；"));
    }
    // 问的是 `is_configured_in` 而不是 `hid_config_in`：后者对 Ble 恒为 None，拿它筛会把
    // 蓝牙从这份名单里漏掉，而这份名单就是要告诉用户"你配了这几条"。
    let configured: Vec<String> = EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| kind.is_configured_in(device))
        .map(|kind| kind.to_string())
        .collect();
    if configured.is_empty() {
        return anyhow!("读不到 —— 这个 Device 一条 Endpoint 都没配置");
    }
    anyhow!(
        "读不到 —— 配置的 Endpoint（{}）一条都不在场：本机枚举不到它们（设备没插？蓝牙没配对？跑 `juicebar scan` 看本机有什么）",
        configured.join("、")
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
    let driver =
        driver_for(&device.driver).ok_or_else(|| anyhow!("驱动 {} 尚未实现", device.driver))?;
    driver.read_battery(transport)
}
