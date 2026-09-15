//! 一份 [`Reading`] 里那几个数字可不可信，以及该显示哪一个。
//!
//! 这个模块不属于任何一个协议家族——两个驱动交出来的都是 [`Reading`]，而"这个
//! Reading 可信吗"、"要显示的那个百分比取自哪里"两问的答案对每个家族都一样。
//! 它放在 `sources` 下面是因为它答的是**取数**这一侧的问题，而不是呈现那一侧的：
//! 悬停提示与菜单那一行只是把结论写出来。
//!
//! 规格见 `.scratch/battery-readout/spec.md`「电量数值」，两处**故意偏离厂商上位机**
//! 的地方记在 `docs/adr/0002-battery-level-source.md`，`voltageToLevel` 的原始算法与
//! 那张 21 档换算表记在 `docs/protocol.md` 第 1 节。

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::sources::Reading;

/// 用户看到的那个百分比，连同它取自哪里。
///
/// **两个来源必须一路带到界面上。**项目里有两个来源不同的百分比（`CONTEXT.md` 的
/// Reported Level 与 Derived Level），它们可能不一致，混用过一次就已经导致过一个
/// 错误结论。裸交一个 `u8` 出去，那次混用迟早再发生一遍。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 设备固件自己上报的百分比。
    Reported(u8),
    /// 由电压查表加插值算出的百分比。
    Derived(u8),
    /// 电量字段无法采信。**Unknown 不等于 0%，也不等于设备离线**（`CONTEXT.md`）。
    Unknown,
}

impl std::fmt::Display for Level {
    /// 用 `CONTEXT.md` 的词，原样。这几个字会出现在悬停提示与菜单那一行上
    /// （`EndpointKind` 的 `Display` 是同一个理由）。
    ///
    /// **措辞与产生它的规则住在同一个文件里。**Unknown 后面那句解释只有
    /// [`reported_or_unknown`] 说得准——它是全 crate 唯一产生 Unknown 的地方。
    /// 把这句话写在呈现层，等哪天 Unknown 多了第二个来源，那一行就开始说谎。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reported(percent) => write!(f, "{percent}%（Reported Level）"),
            Self::Derived(percent) => write!(f, "{percent}%（Derived Level）"),
            Self::Unknown => f.write_str("电量 Unknown（固件报的是 0，采信不了）"),
        }
    }
}

/// 一个 Device 的电量数值取自哪里。配置里每个 Device 一项，缺省 [`Auto`](Self::Auto)。
///
/// **没有全局默认**（spec「电量数值」）：想切的理由是"某型号固件 level 不准"，
/// 那本质上是设备相关的，不该有一个能连带影响其它设备的全局值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevelSource {
    /// 电压越过查表的表顶时用 Reported Level，其余区间用 Derived Level。
    #[default]
    Auto,
    /// 一律用 Reported Level。
    ///
    /// 留这个开关是因为厂商上位机特意绕开固件自报值，很可能是踩过某些型号的坑，
    /// 而我们只有一台设备的证据，不足以推翻它（`docs/adr/0002`）。
    Reported,
}

/// 这一份 Reading 该显示哪个百分比。
///
/// 前提是它已经过了 [`require_plausible`]——驱动交出来的每一份都过了。
pub fn level_for(reading: &Reading, source: LevelSource) -> Level {
    match (source, reading.voltage_mv) {
        // 表顶以上查表分辨不出 95 与 100（两个电压 clamp 成同一个数），而固件跟得上
        // 真实充放电。只在这一段偏离厂商上位机，理由见 `docs/adr/0002` 第一节。
        (LevelSource::Auto, Some(mv)) if mv > TABLE_TOP_MV => {
            reported_or_unknown(reading.reported_level)
        }
        // 表顶以下（含表顶那一档本身）查表仍然可用，就仍然用它：只在**已知**查表失效
        // 的那一段偏离上位机。
        (LevelSource::Auto, Some(mv)) => derived_level(mv, reading.charging),
        // **没有电压就没有 Derived Level**，`auto` 于是自然退化成 Reported Level。
        // 键盘走的就是这一臂：它的 `0xF7` 回包里根本没有电压这一项。这里没有"如果是
        // 键盘就……"的设备清单——只有"那个数据不存在"，而类型把这件事摆到了明面上，
        // 编译器要求这一臂被写出来（parking lot Q7、Q11）。
        (LevelSource::Auto, None) => reported_or_unknown(reading.reported_level),
        // 整台设备的逃生门：连查表可用的区间也不查。
        (LevelSource::Reported, _) => reported_or_unknown(reading.reported_level),
    }
}

/// 采信固件自报的那个百分比——**除了 0**。
///
/// 上位机原文是 `battery.level = result[1] == 0 ? 100 : result[1]`，把 0 当满电。照抄的
/// 后果是电量真读到 0 时显示 100%：不崩、不报警、每次看都合情合理，而且偏偏发生在最该
/// 提醒充电的时刻。也不显示成 0%——固件那个 0 与"这一格没填好"在字节上不可区分，
/// 而回包里 `[0]` 已经是独立的就绪标志了。所以是 Unknown（`docs/adr/0002` 第二节）。
///
/// **只管固件这一个字段。**查表算出的 0% 是一个测出来的电压换算来的，那是真读数。
fn reported_or_unknown(level: u8) -> Level {
    if level == 0 {
        Level::Unknown
    } else {
        Level::Reported(level)
    }
}

/// 一份 [`Reading`] 里的每个数字都落在物理上说得通的区间里，否则这一帧整个不可信。
///
/// **不过就是"读取异常"，不是一个电量。**协议是从某一版厂商上位机逆出来的，
/// 厂商推个新固件就可能让帧结构变样，而那时按老下标取出来的仍然是一串看着像数字的
/// 字节。校验清单和每一条的理由见 `docs/protocol.md` 第 6 节。
///
/// 它答的是"这一帧是不是我以为的那一帧"，所以任何一项不过，**整份 Reading 都不能用**
/// ——包括充电位和电压。这与 `reported_level == 0` 是两回事：那一帧结构没问题，只是
/// 电量那一个字段没填好，结论是 Unknown（见 [`level_for`]）而不是读取异常。
///
/// 两个驱动共用这一句，各自在解析的最后调一次。
pub fn require_plausible(reading: &Reading) -> Result<()> {
    if reading.reported_level > 100 {
        bail!(
            "读取异常：电量 {}% 不在 0..=100 —— 这一帧不是我们以为的那一帧",
            reading.reported_level
        );
    }
    // 没有电压的协议（键盘）跳过这一条，**不拿 0 顶上去**：0 不是"没有电压"，
    // 它是一个会被这里当真的数字，会把每一条键盘读数判成异常（parking lot Q7）。
    if let Some(mv) = reading.voltage_mv
        && !PLAUSIBLE_VOLTAGE_MV.contains(&mv)
    {
        bail!(
            "读取异常：电压 {mv} mV 不在 {}..={} mV —— 这一帧不是我们以为的那一帧",
            PLAUSIBLE_VOLTAGE_MV.start(),
            PLAUSIBLE_VOLTAGE_MV.end()
        );
    }
    Ok(())
}

/// 电池电压说得通的区间，毫伏。
///
/// **上界不是查表的表顶 4110。**满电锂电静置就在 4.15 V 上下（实测 4154–4190 mV），
/// 充电中更高（实测 4231–4235 mV），而那些帧的 CRC 全部验通。照 4110 的上界，每一次
/// 充完电都会被判成"帧结构变了"。取 4350 而不是 4200：锂电充电终止电压标称
/// 4.20 V ±1%，4350 留够余量又仍然识别得出帧错乱。理由全文见 `docs/protocol.md`
/// 第 6 节。下界就是表底那一档。
const PLAUSIBLE_VOLTAGE_MV: std::ops::RangeInclusive<u16> = VOLTAGE_TABLE_MV[0]..=4350;

/// 电压换算表：21 档，每档 5%。`docs/protocol.md`「电压换算表」那一节的上排。
///
/// 下标乘 5 就是那一档的百分比（第 0 档 0%、第 20 档 100%），所以不必再存一张
/// 百分比表——存两张就多一处会走样的地方。
const VOLTAGE_TABLE_MV: [u16; 21] = [
    3050, 3420, 3480, 3540, 3600, 3660, 3720, 3760, 3800, 3840, 3880, 3920, 3940, 3960, 3980, 4000,
    4020, 4040, 4060, 4080, 4110,
];

/// 表顶那一档的电压，也就是查表能分辨的上限。
///
/// **从表本身取，不另写一个 4110。**`docs/protocol.md` 第 6 节点名了这件事：查表的
/// 表顶与合理性校验的电压上界"两处必须一起改"，而这个数还同时是 `auto` 分流的界。
/// 让它们指向同一个字面量，就没有"改了一处忘了另一处"这回事。
const TABLE_TOP_MV: u16 = VOLTAGE_TABLE_MV[VOLTAGE_TABLE_MV.len() - 1];

/// 由电压查表加档间插值算出的 Derived Level。
///
/// 照 `docs/protocol.md`「`voltageToLevel` 完整算法」实现，含它那两处怪癖，
/// **原样保留**：对着 `app.asar` 核对的人应当逐行对得上。
///
/// 交出 [`Level`] 而不是裸 `u8`：项目里有两个来源不同的百分比，混用过一次就已经导致过
/// 一个错误结论，所以跨出这个函数的那一刻它就得带着自己的来源。
///
/// `pub` 的理由是**表顶那段 clamp 只有它自己能被断言到**：`auto` 在 > 4110 mV 时已经
/// 改用 Reported Level，所以经 [`level_for`] 走得到 clamp 的只有 `== 4110` 这一个电压
/// （见 parking lot Q12），而票面要求这个算法实现完整。
pub fn derived_level(voltage_mv: u16, charging: Option<bool>) -> Level {
    // 表顶那一档以上没有可插值的区间，硬 clamp。
    //
    // 原文的条件是 `> 4110`，把 `== 4110` 漏给了下面的查表——而查表在那里会算出
    // NaN（`findIndex` 返回 -1，`voltages[-1]` 是 undefined）。4110 就是表顶那一档的
    // 100%，两条路本该给同一个数，所以这里把它并进 clamp 而不是照抄那个洞。
    //
    // 充电中给 99 是**故意偏离表格**的一步（"充电中别显示满电"），唯一的依据是
    // "知道它在充电"。说不上来的时候不取 99：那就成了替设备做一个没人验过的断言。
    let Some(index) = VOLTAGE_TABLE_MV.iter().position(|mv| *mv > voltage_mv) else {
        return Level::Derived(if matches!(charging, Some(true)) {
            99
        } else {
            100
        });
    };
    let level = if index == 0 {
        // 表底以下没有下界可插值。上位机在这里给 0，紧接着又被下面那条怪癖抬成 1。
        0.0
    } else {
        let low = f64::from(VOLTAGE_TABLE_MV[index - 1]);
        // 一档 5%，所以除以 5 得到"每 1% 多少毫伏"。
        let interval = (f64::from(VOLTAGE_TABLE_MV[index]) - low) / 5.0;
        (f64::from(voltage_mv) - low) / interval + (index - 1) as f64 * 5.0
    };
    // 正好 0 或正好 15 就加一。**没有物理含义，是上位机的怪癖，原样保留**：抹平它
    // 会让两份实现在这两个电压上悄悄分岔，而对着 `app.asar` 核对的人先看到的是这里。
    // 比的是**取整之前**那个浮点数，原文如此——所以只有电压正好落在档位上才会触发。
    let level = if level == 0.0 || level == 15.0 {
        level + 1.0
    } else {
        level
    };
    Level::Derived(level.round() as u8)
}
