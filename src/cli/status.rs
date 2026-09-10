//! `juicebar status` —— 一行一个 Device，打印当前电量。
//!
//! 这是取数链路的收口：配置 → 驱动 → 设备 → 一行看得懂的字。托盘将来显示的是
//! 同一组信息。
//!
//! **取数那一步不在这里**：先试哪一条 Endpoint、怎么降级、什么时候才算失联、读不到时
//! 退到上次已知值，全在 `crate::readout`。**这一轮判成什么也不在这里**：每台 Device 的
//! 图标状态、Primary Device 是谁、这一轮的告警，全在 `crate::round`——托盘要的是同一个结构。
//! 这个模块剩下的只有"印出这几行"：`run` 的接线、一行 Device 的排版、以及那几行的合成，
//! 都从那个结构排出来。

use std::path::PathBuf;

use anyhow::Result;

use crate::bluetooth::{self, BleBattery};
use crate::cli::{config_refresh, resolve_config_path, state_path_beside_config};
use crate::clock::{Clock, SystemClock, Timestamp};
use crate::config::{Config, Device};
use crate::endpoints::{EndpointKind, EndpointReading, SystemEndpoints};
use crate::primary::{self, Candidate, CandidateReading, PrimaryRule, Selection};
use crate::readout;
use crate::round::{DeviceState, InHand, PauseCheck, Round, Shown, Warning};
use crate::sources::level::{LevelSource, level_for};
use crate::staleness::{Freshness, Staleness};
use crate::state::{LastKnown, Provenance};
use crate::vendor_hub::{SystemProcesses, VendorHub};

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    if !path.exists() {
        // 首次运行：先给一份扫出来的草稿，而不是打发用户去抄样例。草稿里扫不到的
        // Endpoint 是注释掉的占位，得他自己看一眼，所以这一次到此为止、不接着取数。
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
    // 厂商上位机在跑吗。**问一次，全部 Device 共用**——与枚举、与"当下"同一条规矩
    // （parking lot Q27）：一次进程枚举不便宜，而这一轮里它的答案不会变。开关关着时
    // 这一句连进程都不去枚举（`VendorHub::detect` 的第一行）。问不出来时按"没在跑"走、
    // 交一条这一轮的告警——那条规则住在 `PauseCheck` 里，这里只把它的答案递下去。
    let pause = PauseCheck::detect(&config.general, &SystemProcesses);
    // **先把每台都取完数，再合成这一轮**：`primary = "lowest"` 得看过这一轮的全部读数才知道
    // 该标哪一行，而那个标注印在每一行上。取数的次序和印的次序都还是配置里的书写顺序。
    let fetched: Vec<(InHand, Timestamp)> = config
        .devices
        .iter()
        .map(|device| {
            // 一个 Device 一个"当下"，取数与陈旧判定共用它。几台设备依次读下来会花掉真实
            // 时间，所以这个值在循环里取而不在循环外——但**一台设备只取一次**。
            let now = clock.now();
            // 读到的那一份已经由 `read_or_last_known` 记进 `last_known` 了（票面第 1 条），
            // 写盘在这一轮的末尾一次写完：几台设备的记录住在同一份文件里。
            let in_hand = InHand::from(readout::read_or_last_known(
                device,
                &endpoints,
                pause.paused_by(),
                &mut last_known,
                &config.general,
                now,
            ));
            (in_hand, now)
        })
        .collect();

    // "上次选出的是谁"从状态文件里取，这一轮选出的再记回去——"全都不可信时保持上次的
    // 选择"靠的就是这一条线（parking lot Q41）。`last_primary` 借的是 `last_known`，
    // 而下面 `remember_primary` 要可变借它，所以这里先把那个 id 拷出来。
    let previous_primary = last_known.last_primary().map(str::to_owned);
    // **每台在它自己那个"当下"判，再合成一轮**（`Round::compose`），不拿一个统一的"当下"
    // 判全部（`Round::assess`，托盘要的是那一种）：那样先取完数的那几台会被后面那台的超时
    // 拖老几秒，一行当场问出来的"0 秒前"就成了"3 秒前"。
    let states = config
        .devices
        .iter()
        .zip(&fetched)
        .map(|(device, (in_hand, now))| DeviceState::assess(device, in_hand, &config.general, *now))
        .collect();
    let round = Round::compose(&config.general, states, previous_primary.as_deref(), &pause);

    // 这一轮的告警印在 stderr 上、先于那几行：它们还是两句 `eprintln!` 时就是这个次序。
    for warning in &round.warnings {
        eprintln!("{warning}");
    }
    // 一个 Device 都没配不是就此收摊：**那正是最需要下面那段未登记名单的时刻**（本机扫到的
    // 每一台 BLE 设备都还没登记，而 MAC 就在那几行里等着抄）。所以这里不再提前 return。
    if config.devices.is_empty() {
        println!("配置 {} 里一个 Device 都没有。", path.display());
    }
    let rows: Vec<DeviceRow<'_>> = round.devices.iter().map(row_of).collect();
    // 选出了一台真的 Primary 才记。选不出来的那一轮**什么都不动**：那正是最需要上次那个
    // 值的时候，而把它清掉恰好会让"保持上次的选择"永久落空。
    if let Some(id) = round.primary.primary_id() {
        last_known.remember_primary(id);
    }
    for line in compose(&round.primary, &rows) {
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

    // 写盘放在最后，一轮一次。**写不进去只说一句（这一轮的又一条告警），不让整条命令失败**：
    // 上面那几行已经印出去了，而用户要的东西就是那几行。丢的是下次启动时的记忆，不是这一轮的
    // 输出。这一条由这里造，不由 `Round` 交出来：写盘在这一轮合成之后，那个纯函数碰不到磁盘。
    if let Err(e) = last_known.save(&state_path) {
        eprintln!("{}", Warning::StateNotSaved(format!("{e:#}")));
    }
    Ok(())
}

/// 这一轮里的一台 Device 排成的那一行成品。
fn row_of<'a>(state: &DeviceState<'a>) -> DeviceRow<'a> {
    DeviceRow {
        device: state.device,
        line: line_of(state),
        candidate: state.candidate(),
    }
}

/// 一台 Device 名字之后的那一段：有读数就排版那份读数（[`render`]），没有就印取数那一层交出的
/// 那句话。
///
/// 两支的措辞都不在这里定：前者是 [`render`] 的事，后者由 [`readout::NoReading`] 自己的
/// `Display` 给出——失联与暂停对 Primary 选择是同一件事（手上没有数），而印出去那句话两者
/// 完全不同，那一维由它分开。
fn line_of(state: &DeviceState<'_>) -> String {
    match &state.shown {
        Shown::Reading { row, staleness } => render(
            &row.reading,
            state.device.level_source,
            staleness,
            row.provenance,
            row.paused_by.as_ref(),
        ),
        Shown::NoReading(no_reading) => no_reading.to_string(),
        // 走不到：`run` 给每台都取过一次数，手上的东西只会是一次取数的结果。留着这一支是因为
        // 类型要交代一句，而**交代的话得是实话**。
        Shown::NoKnownValue => "无已知值 —— 还没读到过，也没有上次已知值".to_string(),
    }
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
    /// 它在 Primary 选择里的样子。`None` = 取数失败或者暂停、又退不到上次已知值，
    /// 不参与 `lowest` 比较。
    pub candidate: Option<CandidateReading>,
}

/// `status` 印出去的那几行：每个 Device 一行，末尾可能多一句交代——连同这一轮选出了谁。
///
/// 这一半是纯的，理由与 [`unregistered_ble_lines`] 同一条：选出谁、标在哪一行、要不要补
/// 那句话，都是 `status` 的外部行为，而"本机有哪些设备"那一步才躲不开真系统。
///
/// **`run` 不走这里的那一步选择**：它手上已经有这一轮选出的那个 [`Selection`]
/// （`crate::round`，与托盘同一个结构），直接交给 [`compose`] 排版。这个函数是同一段合成连同
/// 那一步选择（[`primary::select`]，`Round` 用的也是它），那几条排版用例从这里进来。
///
/// **"上次的选择"从状态文件里来，这一轮选出的又记回去。**`status` 是一次性命令——枚举、
/// 取数、印几行、退出，它自己手上没有"上次"，所以那份记忆住在票 08 的 `state.toml`
/// （parking lot Q41：不能住 `config.toml`，因为 `primary` 只有一个格子，把自动选出的 id
/// 写回去会把 `"lowest"` 这条规则本身洗掉）。
///
/// 记回去的只是"选出了一台真的 Primary"那几种（[`Selection::primary_id`] 给 `Some` 的那些）。
/// 选不出来的那一轮**不动**这一格——那正是最需要上次那个值的时候，而 `remember_primary`
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
    (compose(&selection, rows), selection.primary_id())
}

/// 那几行的合成：每个 Device 一行，Primary Device 那一行的名字后面带标注，末尾可能多一句交代。
fn compose(selection: &Selection<'_>, rows: &[DeviceRow<'_>]) -> Vec<String> {
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
    lines
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
///
/// [`read`]: crate::readout::read
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
///
/// [`read`]: crate::readout::read
pub fn render(
    sourced: &EndpointReading,
    level_source: LevelSource,
    staleness: &Staleness,
    provenance: Provenance,
    paused_by: Option<&VendorHub>,
) -> String {
    let EndpointReading {
        endpoint: _,
        reading,
        cache_age_secs: _,
        taken_at,
    } = sourced;
    // 充电态那一格空着有**两个各自独立的理由**，而没有一个是"这台设备没在充电"：
    //
    // 一、**这条协议说不上来。**键盘那一位至今没有实测样本，spec 明写「键盘暂不显示充电
    //     态」——印"未充电"就是替设备做了一个没人验过的断言。`Ble` 也落在这一条：Windows
    //     攒的那份电量属性里根本没有充电这一项。
    // 二、**这个数是上次已知值。**电量与电压是那一刻的测量值，行尾"已陈旧，上次已知值"
    //     已经把它们限定完整了；而充电态是一个**现在时的状态断言**，它恰恰在设备被收进
    //     抽屉、或者刚插上线的那一刻变掉（`CONTEXT.md`「上次已知值」：它有多新，和设备
    //     此刻在不在，是两件事）。**暂停里退到历史值的那一行也落在这一条**（两条 HID 都
    //     让开了、又没有别的 Endpoint 顶上）：那一次取数根本没问过它在不在充电，而那时设备
    //     **就在手边**、可能一分钟前刚插上线，比失联更不该印（parking lot Q98）。`Ble`
    //     顶上的那一种暂停是当场读到的，它落在上面第一条。
    //
    // 两条合起来是同一句话——**这一次取数没读到这个状态，就不说它**——所以"说不上来的那一格整个
    // 不印"这条规矩到此在所有情形下一致：一格有值，那个值就是真的。**不改成过去时措辞**
    // （"当时在充电"）：一个只为措辞存在的分支不值得，而这里能省下的正是那个分支。
    //
    // 对 `Provenance` **穷举**，理由与 [`stale_marker`] 那一条相同：加第三种来路时编译器会
    // 把人指到这一行来。（`Option<bool>` 那一维加不出变体，所以第二条那一支取通配。）
    let charging = match (provenance, reading.charging) {
        (Provenance::JustRead, Some(true)) => "  充电中",
        (Provenance::JustRead, Some(false)) => "  未充电",
        // 理由一：这条协议说不上来。
        (Provenance::JustRead, None) => "",
        // 理由二：这个数是上次已知值。
        (Provenance::LastKnown, _) => "",
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
        "{measured}  {}{}{}",
        source_of(sourced, staleness),
        stale_marker(staleness.freshness, provenance),
        pause_marker(paused_by),
    )
}

/// 暂停标注：这一次取数有 Endpoint 让开时说一句，否则一个字都不加。
///
/// **它与陈旧标注是两维**，所以是另一段而不是塞进 [`stale_marker`]：一份读数有多旧，和
/// "我们这一次取数根本没去问"，是两个独立的事实。同一个 44% 可能既是半小时前的（陈旧），
/// 又是因为上位机在跑才没被刷新（暂停）——两句话都得说。
///
/// **点名那个进程**，不只说"已暂停"：用户唯一能动手的地方就是关掉它，而"哪一个"这件事只有
/// 这一维答得出（`crate::vendor_hub`）。
///
/// 它**只在真让开过 Endpoint 的那一行出现**（判据见 [`read_or_last_known`]），所以不是
/// 一句常态化的提醒——那种提醒会把真正该看的话一起淹掉。
///
/// [`read_or_last_known`]: crate::readout::read_or_last_known
fn pause_marker(paused_by: Option<&VendorHub>) -> String {
    match paused_by {
        None => String::new(),
        Some(hub) => format!("  已暂停（{} 正在运行）", hub.process()),
    }
}

/// 陈旧标注：新鲜、这一次取数读到的读数什么都不加，其余每一种都以"已陈旧"开头。
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
