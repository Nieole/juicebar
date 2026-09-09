//! `juicebar status` —— 一行一个 Device，打印当前电量。
//!
//! 这是取数链路的收口：配置 → 驱动 → 设备 → 一行看得懂的字。托盘将来显示的是
//! 同一组信息。

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::bluetooth::{self, BleBattery};
use crate::cli::{config_refresh, resolve_config_path, state_path_beside_config};
use crate::clock::{Clock, SystemClock, Timestamp};
use crate::config::{Config, Device, General};
use crate::endpoints::{EndpointKind, EndpointReading, Endpoints, SystemEndpoints};
use crate::primary::{self, Candidate, CandidateReading, PrimaryRule};
use crate::sources::level::{LevelSource, level_for};
use crate::sources::{Reading, Transport, driver_for};
use crate::staleness::{Freshness, Staleness};
use crate::state::{LastKnown, Provenance};

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    if !path.exists() {
        // 首次运行：先给一份扫出来的草稿，而不是打发用户去抄样例。草稿里扫不到的
        // Endpoint 是注释掉的占位，得他自己看一眼，所以这一趟到此为止、不接着取数。
        return config_refresh::bootstrap(&path);
    }
    let config = Config::load(&path)?;

    // 枚举一次，全部 Device 共用：一次枚举要打开本机每一条 HID collection。
    let endpoints = SystemEndpoints::enumerate()?;
    let clock = SystemClock;
    // 上次已知的读数。**读不到、或者读到的东西坏了，都当成没有已知值**，不影响下面那几行
    // （票面第 5 条）——它是缓存，不是用户的数据。
    let state_path = state_path_beside_config(&path);
    let mut last_known = LastKnown::load(&state_path);
    // 一个 Device 都没配不是就此收摊：**那正是最需要下面那段未登记名单的时刻**（本机扫到的
    // 每一台 BLE 设备都还没登记，而 MAC 就在那几行里等着抄）。所以这里不再提前 return。
    if config.devices.is_empty() {
        println!("配置 {} 里一个 Device 都没有。", path.display());
    }
    // **先把每台都读完，再一起印**：`primary = "lowest"` 得看过这一轮的全部读数才知道
    // 该标哪一行，而那个标注印在每一行上。读的次序和印的次序都还是配置里的书写顺序。
    let mut rows: Vec<DeviceRow<'_>> = Vec::new();
    for device in &config.devices {
        // 一个 Device 一个"当下"，取数与陈旧判定共用它。几台设备依次读下来会花掉真实
        // 时间，所以这个值在循环里取而不在循环外——但**一台设备只取一次**。
        let now = clock.now();
        let (line, candidate) =
            match read_or_last_known(device, &endpoints, &mut last_known, &config.general, now) {
                // 读到的那一份已经由 `read_or_last_known` 记进 `last_known` 了（票面第 1 条），
                // 写盘在这一趟的末尾一次写完：几台设备的记录住在同一份文件里。
                Ok((reading, provenance)) => {
                    let staleness = Staleness::assess(&reading, &config.general, now);
                    (
                        render(&reading, device.level_source, &staleness, provenance),
                        // Primary 选择只要两件事：这一行显示的是哪个百分比（`Level::Unknown`
                        // 即采信不了），以及这份读数还算不算现状。`level_for` 在 `render` 里
                        // 也调了一次——同一个纯函数、同样的入参，两处必然是同一个答案，
                        // 所以不必把它提进 `EndpointReading`（parking lot Q43）。
                        //
                        // **拿出来顶上的历史值不会因为这一步而混进 lowest**：它一律是
                        // Stale（`CONTEXT.md`「上次已知值」那一条），而下面的筛子只收
                        // `Freshness::Fresh`。所以这里不必再按 `provenance` 分一次支。
                        Some(CandidateReading {
                            level: level_for(&reading.reading, device.level_source),
                            freshness: staleness.freshness,
                        }),
                    )
                }
                // 一台读不到不该拖累别的 Device，把原因印在它自己那一行上。**读不到就是
                // 失联，它不参与 lowest 比较**——那正是这里 `reading` 为 `None` 的意思，
                // 而失联与 `Level::Unknown` 是两回事（`CONTEXT.md`：Unknown 也不等于设备离线）。
                Err(e) => (format!("读不到 —— {e:#}"), None),
            };
        rows.push(DeviceRow {
            device,
            line,
            candidate,
        });
    }

    // "上次选出的是谁"从状态文件里取，这一趟选出的再记回去——票 09 的"全都不可信时保持
    // 上次的选择"靠的就是这一条线（parking lot Q41）。`last_primary` 借的是 `last_known`，
    // 而下面 `remember_primary` 要可变借它，所以这里先把那个 id 拷出来。
    let previous_primary = last_known.last_primary().map(str::to_owned);
    let (lines, chosen_primary) =
        primary_lines(&config.general.primary, &rows, previous_primary.as_deref());
    // 选出了一台真的 Primary 才记。选不出来的那一趟**什么都不动**：那正是最需要上次那个
    // 值的时候，而把它清掉恰好会让"保持上次的选择"永久落空。
    if let Some(id) = chosen_primary {
        last_known.remember_primary(id);
    }
    for line in lines {
        println!("{line}");
    }

    // 未登记的 BLE 设备是**另一条输出维度**，不属于上面任何一行：它们是本机扫得到、
    // 而配置里没有对应 `[[device]]` 的那些。缺省不印。
    let unregistered = unregistered_ble_lines(&config, endpoints.scanned_ble());
    if !unregistered.is_empty() {
        println!();
        println!("未登记的 BLE 设备：");
        for line in unregistered {
            println!("  {line}");
        }
    }

    // 写盘放在最后，一趟一次。**写不进去只在 stderr 上说一句，不让整条命令失败**：上面
    // 那几行已经印出去了，而用户要的东西就是那几行。丢的是下一趟的记忆，不是这一趟的输出。
    if let Err(e) = last_known.save(&state_path) {
        eprintln!("记不下这一趟的读数（下次启动就没有上次已知值了）—— {e:#}");
    }
    Ok(())
}

/// 一个 Device 这一轮的成品：那一行印什么，以及它在 Primary 选择里的样子。
///
/// 三样捆在一起是因为它们**同时产生、同时被用掉**：`lowest` 得看过全部读数才知道该标哪一行，
/// 所以那一行的文本必须先攒着。给它一个名字而不是用一个三元组，理由与这个仓库里
/// [`EndpointReading`] / `Staleness` 同一条——靠位置解构的元组，加第四样时每一处都要改，
/// 而票 08 正要往这里加东西（parking lot Q44 末段）。
pub struct DeviceRow<'a> {
    /// 配置里那个 Device。
    pub device: &'a Device,
    /// 这一行 Device 名字**之后**的全部内容，已经排好版（[`render`]，或者读不到时的原因）。
    pub line: String,
    /// 它在 Primary 选择里的样子。`None` = 失联，不参与 `lowest` 比较。
    pub candidate: Option<CandidateReading>,
}

/// `status` 印出去的那几行：每个 Device 一行，末尾可能多一句交代。
///
/// 这一半是纯的，理由与 [`unregistered_ble_lines`] 同一条：选出谁、标在哪一行、要不要补
/// 那句话，都是 `status` 的外部行为，而"本机有哪些设备"那一步才躲不开真系统。
///
/// **"上次的选择"从状态文件里来，这一趟选出的又记回去。**`status` 是一次性命令——枚举、
/// 取数、印几行、退出，它自己手上没有"上次"，所以那份记忆住在票 08 的 `state.toml`
/// （parking lot Q41：不能住 `config.toml`，因为 `primary` 只有一个格子，把自动选出的 id
/// 写回去会把 `"lowest"` 这条规则本身洗掉）。
///
/// 记回去的只是"选出了一台真的 Primary"那几种（[`Selection::primary_id`] 给 `Some` 的那些）。
/// 选不出来的那一趟**不动**这一格——那正是最需要上次那个值的时候，而 `remember_primary`
/// 只有"设"没有"清"就是为此。
///
/// 这一半仍然是纯的：它收一个 `previous` 值、交出这一轮选出的 id，读盘写盘都在 [`run`] 那一头。
/// 交出去的那个 id 可能借自三处任一：配置里钉死的那个（`rule`）、这一轮的候选之一
/// （`rows`）、或者上次记下的那个（`previous`）。三者因此共一个生命周期。
pub fn primary_lines<'a>(
    rule: &'a PrimaryRule,
    rows: &'a [DeviceRow<'a>],
    previous: Option<&'a str>,
) -> (Vec<String>, Option<&'a str>) {
    // 候选就是登记在册的这几台。未登记的 BLE 设备**天然不参与**：它们不是 Device，
    // 不在 `config.devices` 里，所以压根进不了 `rows`（parking lot Q24）——一台没人登记的
    // 耳机不该抢走托盘图标。
    let candidates: Vec<Candidate<'_>> = rows
        .iter()
        .map(|row| Candidate {
            id: &row.device.id,
            reading: row.candidate,
        })
        .collect();
    let selection = primary::select(rule, &candidates, previous);
    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| {
            format!(
                "{}  {}",
                selection.label(&row.device.id, &row.device.name),
                row.line
            )
        })
        .collect();
    // 光看那几行看不出发生了什么的时候补一句（保持了上次的选择、钉的 id 不在册、
    // 一个都选不出来）。**一个 Device 都没有时不补**：那时 `run` 已经印过"一个 Device
    // 都没有"，把原因说完了，再说一句"选不出 Primary Device"只是同一件事的第二遍。
    if !rows.is_empty()
        && let Some(note) = selection.note()
    {
        lines.push(note);
    }
    (lines, selection.primary_id())
}

/// 本机扫到、而配置里没有登记的那些 BLE 设备，一台一行。
///
/// **名字里是"未登记"而不是"Unknown"**：`CONTEXT.md` 的 Unknown 说的是"一份 Reading 的电量
/// 字段无法采信"，是读数的状态；这里说的是"这台设备配置里没写"，两回事。配置键叫
/// `show_unknown_ble` 是用户看得见的名字、已经定了，代码这一侧不跟着漂。
///
/// 关着时是空的——那是缺省，因为一台机器上的 BLE 设备多数跟键鼠无关（耳机、手机、手环），
/// 全列出来只会把两行有用的埋掉。打开它的用处是接新设备：想登记的那台就在这几行里，
/// MAC 可以直接抄进 `[device.bluetooth]`。
///
/// **"登记过没有"看的是 MAC 对不对得上**，不是名字：`friendly_name` 虽然是唯一可靠的
/// 产品标识，但同型号两台会重名，而配置里认设备本来就是靠 MAC。认不出来的症状是同一台
/// 设备既在上面那一行、又在这一段里出现一次，像是本机有两只鼠标。
///
/// `pub` 与 [`read`] / [`render`] 同理：印出哪几台是 `status` 的外部行为。而"本机有哪些
/// BLE 设备"那一步要真去问 Windows，只有把这一半留成纯函数它才断言得到。
pub fn unregistered_ble_lines(config: &Config, scanned: &[BleBattery]) -> Vec<String> {
    if !config.general.show_unknown_ble {
        return Vec::new();
    }
    scanned
        .iter()
        .filter(|found| !is_registered(config, found))
        .map(|found| {
            let name = if found.friendly_name.is_empty() {
                "(无名)"
            } else {
                found.friendly_name.as_str()
            };
            // 电量那一格：`scanned_ble()` 交出来的快照来自 `bluetooth::enumerate`，它就地
            // 滤掉了没有电量属性的设备，所以实际上走不到 `None` 那一支。留着它是因为类型要
            // 交代一句，而**交代的话得是实话**：那种情形是"这台设备答不出电量"，不是"0%"。
            //
            // "（Reported Level）"这半句与 `render` 里那一段字面相同，**故意不抽成共用
            // 函数**：票 03 此刻正在改 `render` 里显示哪个百分比那一段（Reported 还是
            // Derived），抽一个两边共用的东西正好撞在它手上。等它落地，那时才知道该抽的
            // 是什么形状。
            let level = match found.level {
                Some(level) => format!("{level}%（Reported Level）"),
                None => "没有电量属性".to_string(),
            };
            // 不印"来自 Ble"：这一整段都是 BLE 设备，说一遍就够。留下的是"多久以前"
            // ——`scan` 的提示里那句话对这里同样成立：那一列才是它的真实可信度。
            format!(
                "{name}  {level}  {}  MAC {}",
                bluetooth::age_text(found.age_secs),
                found.address
            )
        })
        .collect()
}

/// 这台扫到的设备已经登记在某个 Device 的 `[device.bluetooth]` 里了吗。
fn is_registered(config: &Config, found: &BleBattery) -> bool {
    config.devices.iter().any(|device| {
        device
            .bluetooth
            .as_ref()
            .is_some_and(|configured| configured.matches(&found.address))
    })
}

/// 取一次数：按 [`EndpointKind::PRIORITY`] 依次试在场的每一条 Endpoint，遇第一个
/// 成功即停。
///
/// 靠前的一条不行就**在同一次调用里**接着试下一条，不等下一个轮询周期——拔线那一刻
/// `Wired` 直接从枚举里消失（瞬时不在场而不是一次三秒的超时），降级几乎不要钱。
/// 在场的 Endpoint 全都没读到，才算失联。
///
/// `pub` 是为了让"拔线 → 同周期降级"这条路径能在 `tests/endpoints.rs` 里被驱动：
/// 集成测试只看得见 pub 接口，而这条路径在真机上拔一次线才复现一次。
///
/// `now` 收的是一个**值**而不是 `&dyn Clock`：整条链路只在 [`run`] 里问一次"现在几点"，
/// 那一个"当下"再被取数与陈旧判定共用。问两次不是浪费而是**错的**——几条 Endpoint 依次
/// 试下来会花掉真实时间（一次鼠标超时就是三秒），两条 HID 各超时一次再落到 Ble，用第二个
/// "当下"去判第一个"当下"盖的时间戳，那一行印出来就不是"0 秒前"而是"6 秒前"。
/// 接缝本身（`crate::clock::Clock`）因此只出现在 [`run`] 的顶上，这一层往下全是纯函数。
pub fn read(device: &Device, endpoints: &dyn Endpoints, now: Timestamp) -> Result<EndpointReading> {
    let present = endpoints.present(device);
    let mut failures = Vec::new();

    for kind in EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| present.contains(kind))
    {
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
            // 试过就记下原因，接着试下一条。全都失败时这些原因合起来就是失联的解释。
            Err(e) => failures.push(format!("{kind}: {e:#}")),
        }
    }

    Err(lost_reason(device, &failures))
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
/// `pub` 与 [`read`] / [`render`] 同理：这一维是 `status` 的外部行为——一次真机上的失联要靠
/// 拔线或者等设备睡着才复现一次，而这条路径上错一步的症状是"重启就失忆"，没有任何报错。
pub fn read_or_last_known(
    device: &Device,
    endpoints: &dyn Endpoints,
    last_known: &mut LastKnown,
    general: &General,
    now: Timestamp,
) -> Result<(EndpointReading, Provenance)> {
    match read(device, endpoints, now) {
        Ok(reading) => {
            last_known.record(&device.id, &reading, now);
            Ok((reading, Provenance::JustRead))
        }
        // 历史值只在这一支里被问到：读得到的时候它不参与竞争。让它参与，一份存了半天的
        // 读数就可能盖掉当场问出来的那个数——而这条链路上其余每一处守的都是相反的规矩
        // （Endpoint 优先级、`Ble` 排最后）。
        Err(lost) => last_known
            .reading_for(&device.id, general, now)
            .map(|last| (last, Provenance::LastKnown))
            .ok_or(lost),
    }
}

/// 一条 Endpoint 都没读到时印出去的那句话。
///
/// 分两种说法，因为它们要用户做的事完全不同：一条都不在场多半是设备没插或配置写错，
/// 而在场却读不到才是设备那头出了事。
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
        return anyhow!("全部 Endpoint 都没读到 —— {}", failures.join("；"));
    }
    // 问的是 `is_configured_in` 而不是 `hid_config_in`：后者对 Ble 恒为 None，拿它筛会把
    // 蓝牙从这份名单里漏掉，而这份名单就是要告诉用户"你配了这几条"。
    let configured: Vec<String> = EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| kind.is_configured_in(device))
        .map(|kind| kind.to_string())
        .collect();
    if configured.is_empty() {
        return anyhow!("这个 Device 一条 Endpoint 都没配置");
    }
    anyhow!(
        "配置的 Endpoint（{}）一条都不在场：本机枚举不到它们（设备没插？蓝牙没配对？跑 `juicebar scan` 看本机有什么）",
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

/// 电量后面那句 Reported Level / Derived Level 不是啰嗦：项目里有两个来源不同的
/// 百分比，它们可能不一致，而混用过一次就已经导致过一个错误结论。这一行印的是哪一个，
/// 取决于这台设备的 `level_source` 和电压落在哪一段（见 `sources::level::level_for`），
/// 所以必须标出来。
///
/// 末尾那句"来自 X"是另一种来源：同一个 Device 的几条 Endpoint 在不同时刻各自可用，
/// 不写出来就看不出这一行是插着线读的还是走 2.4G 读的——而"插着线"恰恰是那个最容易
/// 被误报成离线的时刻。
///
/// `pub` 与 [`read`] 同理：这一行印成什么样是 `status` 的外部行为。
pub fn render(
    sourced: &EndpointReading,
    level_source: LevelSource,
    staleness: &Staleness,
    provenance: Provenance,
) -> String {
    let EndpointReading {
        endpoint: _,
        reading,
        cache_age_secs: _,
        taken_at,
    } = sourced;
    // 键盘那一位至今没有实测样本，spec 明写「键盘暂不显示充电态」，所以说不上来时
    // 这一格整个不印——印"未充电"就是替设备做了一个没人验过的断言。
    let charging = match reading.charging {
        Some(true) => "  充电中",
        Some(false) => "  未充电",
        None => "",
    };
    // 键盘的回包里根本没有电压这一项，那一格同样整个不印。印一个 0 mV 会被当成读数。
    let voltage = match reading.voltage_mv {
        Some(mv) => format!("  {mv} mV"),
        None => String::new(),
    };
    // 两个百分比里印哪一个，以及"一个都印不出来"。措辞由 [`Level`] 自己的 `Display`
    // 给出——**Unknown 后面那句解释只有产生它的那条规则说得准**；写在这里等于让呈现层
    // 替取数层解释因果，Unknown 哪天多了第二个来源，这一行就开始说谎。
    //
    // 陈旧到不该再显示数字的那一档，把电量、充电态、电压**整段换掉**：一个几个月前的
    // 百分比印出来就是一句假话，而"充电中"是同一句假话的另一半（几个月前在充电，
    // 现在呢？）。换上去的是那时唯一还成立的事实——这个数是哪一天的。
    // 百分比为什么不见了由 [`stale_marker`] 那半句交代。
    let level = format!("{}{charging}{voltage}", level_for(reading, level_source));
    // 对 `Freshness` **穷举、不写通配分支**：加一档时编译器会把人指到这一行来
    // （parking lot Q29 承诺的"翻案是加一个变体，波及 render 的一个 match"靠的正是这个，
    // 一个 `_` 就会让那句承诺落空）。
    let measured = match (staleness.freshness, taken_at) {
        (Freshness::VeryStale, Some(taken_at)) => format!("{taken_at} 的读数"),
        // VeryStale 而说不出取得时刻走不到：那一档要算出年龄才判得出来，而算得出年龄就
        // 有时刻。留着这一支是因为类型要交代一句，而**交代的话得是实话**：说不出是哪天的
        // 就只能照常印数字，由末尾那个陈旧标注去拦。
        (Freshness::VeryStale, None) => level,
        (Freshness::Fresh | Freshness::Stale, _) => level,
    };
    format!(
        "{measured}  {}{}",
        source_of(sourced, staleness),
        stale_marker(staleness.freshness, provenance),
    )
}

/// 陈旧标注：新鲜、这一趟读到的读数什么都不加，其余每一种都以"已陈旧"开头。
///
/// 几档共用同一个词是有意的：`CONTEXT.md` 只给了一个 **Stale**，而用户要分辨的也只有
/// "能不能当现状"这一件事。给第二档另造一个词（"极旧"？"失效"？）只会多一个
/// `CONTEXT.md` 里没有的说法，而 Stale 那一条的 _Avoid_ 恰恰就是"过期、失效"。
///
/// "不显示百分比"那半句是**交代那个百分比去哪了**：前面那一段已经换成了一个日期，不说一句
/// 用户会以为读数没读到——而它读到了，只是旧得不该再当数字报出来。
///
/// **上次已知值一律带标注，新鲜那一档也带**（[`Provenance`] 上写了理由）：阈值答的是
/// "这个数多新"，而历史值的问题是设备此刻不在，一份十秒前的读数照样不是现状。多的那半句
/// "上次已知值"是这一维自己要说的话——一份当场读到的陈旧读数（Windows 缓存里两小时前的
/// 那个数）和一份失联后从磁盘上捞出来的读数，用户要做的事不同：前者等一等会自己变新，
/// 后者得先把设备找出来。
///
/// **对两维都穷举、不写通配分支**，理由与 parking lot Q29 那一条相同：加一档（或者加第三种
/// 来路）时编译器会把人指到这里来，而一个 `_` 会让那句承诺落空。
fn stale_marker(freshness: Freshness, provenance: Provenance) -> &'static str {
    match (provenance, freshness) {
        (Provenance::JustRead, Freshness::Fresh) => "",
        (Provenance::JustRead, Freshness::Stale) => "  已陈旧",
        (Provenance::JustRead, Freshness::VeryStale) => "  已陈旧，不显示百分比",
        (Provenance::LastKnown, Freshness::Fresh | Freshness::Stale) => "  已陈旧，上次已知值",
        // 走不到：超过 `very_stale_after` 的历史值在 `LastKnown::reading_for` 那一步就被
        // 丢掉了，而两条 HID 压根没有这一档（Q29）。留着这一支是因为类型要交代一句，
        // 而**交代的话得是实话**——那时前面那一段已经换成了一个日期。
        (Provenance::LastKnown, Freshness::VeryStale) => "  已陈旧，上次已知值，不显示百分比",
    }
}

/// 来源那一段：这个数是从哪条 Endpoint 上来的，以及它有多旧。
///
/// `Ble` 与另两级的差别不是装饰。Wired 与 Dongle24G 是当场问出来的，`Ble` 读的是
/// Windows 攒的缓存，可能是几个月前的（`CONTEXT.md`：「数据取自 Windows 缓存而非当场问
/// 设备，因此天然可能陈旧」）。只印一个"来自 Ble"，用户分不出"刚问出来的 62%"和"三月份
/// 那个 62%"——而 spec 的第 6 条 user story 要的正是这个分辨。
///
/// **"多久前"三级都印**，不只蓝牙那一级：`status` 今天是一次性命令，HID 那一格恒为
/// "0 秒前"，但常驻轮询之后"这一行是几秒前还是几分钟前取的"才是真正要分辨的事。
/// 只有"（Windows 缓存，…）"那半句是 Ble 独有的——它说的是这个数**不是当场问出来的**。
///
/// **那个秒数两级各有各的来源，这不是重复而是两个不同的事实。**`Ble` 印的是
/// [`EndpointReading::cache_age_secs`]，也就是 **Windows 自己报的**那份缓存的年龄
/// ——它**不经过 Clock**（parking lot Q21 给本票的原话：「它是 Windows 报的秒数、不经过
/// Clock」；那份时间戳是系统攒的，本机时钟只能用来算差值，而 `bluetooth.rs` 已经算过了）。
/// 两条 HID 没有"缓存年龄"这回事，它们的秒数只能由取得时刻与当下相减得来，那正是
/// [`Staleness::age_secs`]。
///
/// **这里只排版，不判定**：阈值怎么定、够不够陈旧，都在 `crate::staleness` 里判完了。
fn source_of(sourced: &EndpointReading, staleness: &Staleness) -> String {
    let endpoint = sourced.endpoint;
    match endpoint {
        EndpointKind::Wired | EndpointKind::Dongle24G => {
            format!(
                "来自 {endpoint}（{}）",
                bluetooth::age_text(staleness.age_secs)
            )
        }
        EndpointKind::Ble => format!(
            "来自 {endpoint}（Windows 缓存，{}）",
            bluetooth::age_text(sourced.cache_age_secs)
        ),
    }
}
