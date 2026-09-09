//! 选出 Primary Device：托盘图标当下画的那一个。
//!
//! `CONTEXT.md` 的 **Primary Device** 逐字是：「托盘图标当下画的那一个 Device。可以钉死
//! 某一个，也可以按规则动态选出。」两种形态都住在这里：钉死的那一种是配置里写了一个
//! Device id，动态那一种是 `"lowest"`——当前电量最低的那个。
//!
//! 全是纯函数。它收的是**已经定下来的结论**（这一轮显示的是哪个百分比、这份读数还算不算
//! 现状），不自己去判：那两件事分别在 `crate::sources::level` 与 `crate::staleness` 里，
//! 各有各的用例。

use serde::{Deserialize, Deserializer};

use crate::sources::level::Level;
use crate::staleness::Freshness;

/// 配置里的 `primary`：Primary Device 是钉死的，还是按规则选出来的。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PrimaryRule {
    /// 当前电量最低的那一个。缺省。
    #[default]
    Lowest,
    /// 钉死配置里这个 id 的 Device。
    Pinned(String),
}

/// `primary` 里那条动态规则的写法。别的任何字符串都是一个 Device id。
const LOWEST: &str = "lowest";

/// 配置里 `primary` 那个字符串说的是哪一种。
///
/// **认不出的值不让整份配置读不动**，与 `level_source` 的处置相反（parking lot Q15 与
/// Q42）：`level_source` 只有两个合法值，写别的一定是错的；而这一项的合法值里有一个是
/// **任意 Device id**，id 是自由文本，"dragonfly4" 分辨不出是笔误还是一台还没登记的设备。
/// 所以这里一律当 id 收下，"这个 id 不在册"留到选的时候说
/// （[`Selection::PinnedNotFound`]）——那时候手上才有那份名单。
fn from_config_value(raw: &str) -> PrimaryRule {
    if raw == LOWEST {
        PrimaryRule::Lowest
    } else {
        PrimaryRule::Pinned(raw.to_string())
    }
}

impl<'de> Deserialize<'de> for PrimaryRule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(from_config_value(&String::deserialize(deserializer)?))
    }
}

/// 一个 Device 这一轮在 Primary 选择里的样子。
///
/// 候选就是**登记在册的那些 Device**。未登记的 BLE 设备天然不在这里——它们不是 Device，
/// 没有 id、不进 `config.devices`（parking lot Q24，名单是怎么来的见 `cli::status::run`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// 配置里那个 Device 的 id。选出来的结果就是它。
    pub id: &'a str,
    /// 这一轮从这台 Device 读到的东西。
    ///
    /// `None` = **失联**，在场的 Endpoint 一条都没读到。它与一份 [`Level::Unknown`] 的读数
    /// 是两回事（`CONTEXT.md`：「Unknown 不等于 0%，**也不等于设备离线**」），所以这里是
    /// 一个 `Option` 而不是拿 Unknown 顶上去：两者在 `lowest` 里的结论恰好相同（都不参与），
    /// 但它们在那一行上印的话完全不同，混成一个下游就再也分不清。
    pub reading: Option<CandidateReading>,
}

/// 一个候选这一轮读到的东西——Primary 选择只看这两件事。
///
/// 两个字段各是**一道独立的筛子**，见 [`select`]。它收的都是别处判完的结论：这一步不认识
/// 电压、不认识阈值、也不认识"现在几点"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateReading {
    /// 这一轮**这台设备那一行显示的**那个百分比，连同它取自哪里。
    ///
    /// 取自哪里由该 Device 的 `level_source` 和电压落在哪一段定（`crate::sources::level`）。
    /// 比较时只用那个数字，不管它是 Reported 还是 Derived：两个来源的数可能不一致，而
    /// **用户看到的就是这一个**，拿另一个去比会让托盘上的名字和屏幕上的数字对不上。
    pub level: Level,
    /// 这份读数还算不算现状。由 `crate::staleness` 判完。
    pub freshness: Freshness,
}

/// 这一轮的 Primary Device 是谁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection<'a> {
    /// 配置把 Primary Device 钉死在这一个上。
    Pinned(&'a str),
    /// 新鲜且可信的读数里电量最低的那一个。
    Lowest(&'a str),
    /// 这一轮没有一个新鲜且可信的读数，保持上次的选择。
    HeldOver(&'a str),
    /// 配置钉死的那个 id 在登记的 Device 里根本没有。一次笔误，得说出来。
    PinnedNotFound(&'a str),
    /// 选不出 Primary Device。
    Undecided,
}

impl<'a> Selection<'a> {
    /// 这一轮的 Primary Device 是哪个 id，选不出来就是 `None`。
    pub fn primary_id(&self) -> Option<&'a str> {
        match self {
            Self::Pinned(id) | Self::Lowest(id) | Self::HeldOver(id) => Some(id),
            // 钉死的 id 不在册时**不退回别的选择**：那一行不存在，标不出来。
            Self::PinnedNotFound(_) | Self::Undecided => None,
        }
    }

    /// 一个 Device 那一行开头印的名字：Primary Device 后面多一句它的名分，别的原样。
    ///
    /// 标注**印在 Device 名字这一格、不进 `cli::status::render`**：那个函数排版的是一份
    /// Reading，而**一台失联的 Device 照样可以是 Primary Device**（钉死的那一种，或者
    /// 保持上次的选择），那一行根本没有 Reading 可排（见 parking lot Q44）。
    ///
    /// 措辞是 `CONTEXT.md` 的 **Primary Device**，原样——那一条的 _Avoid_ 正是
    /// 「主设备、默认设备、当前设备混用」。它与产生它的规则住在同一个文件里，
    /// 理由与 [`Level`] 的 `Display` 同一条。
    ///
    /// 收 `id` 与 `name` 两个 `&str` 而不是一个 `&Device`：这一步只用得到这两样，
    /// 收整个结构会让这个模块反过来依赖 `crate::config`（而 `config` 已经为了 `primary`
    /// 那个字段依赖了这里）。
    pub fn label(&self, id: &str, name: &str) -> String {
        if self.primary_id() == Some(id) {
            format!("{name}（Primary Device）")
        } else {
            name.to_string()
        }
    }

    /// 这一轮还要额外对用户说的一句话，没有就是 `None`。
    ///
    /// 三种情形要说话，它们都有一个共同点：**光看那几行看不出发生了什么**。
    /// 正常选出来的两种（[`Self::Lowest`]、[`Self::Pinned`]）不说——那一行上的标注
    /// 已经把话说完了，再补一句只是噪音。
    pub fn note(&self) -> Option<String> {
        match self {
            Self::Pinned(_) | Self::Lowest(_) => None,
            // 标注这时落在一行"已陈旧"、Unknown 或者"读不到"上。不说一句，它看着像 bug。
            Self::HeldOver(_) => Some(
                "（这一轮没有一个新鲜且可信的读数，Primary Device 保持上次的选择）".to_string(),
            ),
            // 一行都没标，而原因是配置写错了——这是最要紧的一句：用户以为钉住了。
            Self::PinnedNotFound(id) => Some(format!(
                "（配置里 primary 钉的 id \"{id}\" 不在登记的 Device 里，所以一行都没标）"
            )),
            // 一行都没标，而原因是这一轮真的没有可信的读数。
            Self::Undecided => Some(
                "（这一轮选不出 Primary Device：没有一个新鲜且可信的读数，也没有上次的选择）"
                    .to_string(),
            ),
        }
    }
}

/// 这一轮的 Primary Device。纯函数。
///
/// 两条规则的形状完全不同：
///
/// - [`PrimaryRule::Pinned`] **一道筛子都不过**。用户钉住某一台的理由恰恰是"别的我不
///   关心"，让一份读数把它挤掉就是没钉住。
/// - [`PrimaryRule::Lowest`] 只让**新鲜且可信**的候选参与比较，全都不参与时保持上次的
///   选择。
///
/// **"新鲜"和"可信"是两道独立的筛子，不是一个判断**（见 [`comparable`]）：新鲜是
/// [`Freshness::Fresh`]（`crate::staleness`），可信是 [`Level`] 不是
/// [`Unknown`](Level::Unknown)（`crate::sources::level`）。
///
/// `previous` 是**上次选出来的是谁**，`None` = 没有上次。`status` 传的恒是 `None`：它是
/// 一次性命令，手上没有"上次"。那份记忆的家见 parking lot Q41。
pub fn select<'a>(
    rule: &'a PrimaryRule,
    candidates: &[Candidate<'a>],
    previous: Option<&'a str>,
) -> Selection<'a> {
    match rule {
        PrimaryRule::Lowest => {
            let lowest = candidates
                .iter()
                .filter_map(|candidate| {
                    comparable(candidate).map(|percent| (percent, candidate.id))
                })
                .min_by_key(|(percent, _)| *percent)
                .map(|(_, id)| id);
            match lowest {
                Some(id) => Selection::Lowest(id),
                // 保持上次的选择，**但只保持一个还在册的 id**：上次那台已经从配置里
                // 删掉时，保持它等于说一句"标在那一行上"而那一行根本不存在。
                None => match previous.filter(|id| is_configured(candidates, id)) {
                    Some(previous) => Selection::HeldOver(previous),
                    None => Selection::Undecided,
                },
            }
        }
        // 一个筛子都不过（理由见上）。只有"这个 id 压根不在册"是另一回事——那不是设备的
        // 状态，是配置写错了，而静默退回 `lowest` 会让用户以为钉住了、其实没有（Q42）。
        PrimaryRule::Pinned(id) if is_configured(candidates, id) => Selection::Pinned(id),
        PrimaryRule::Pinned(id) => Selection::PinnedNotFound(id),
    }
}

/// 这个 id 是这一轮某个候选的 id 吗。
fn is_configured(candidates: &[Candidate<'_>], id: &str) -> bool {
    candidates.iter().any(|candidate| candidate.id == id)
}

/// 这个候选拿得出一个参与 `lowest` 比较的百分比吗。
///
/// 票面那句"只有新鲜且可信的 Reading 参与比较"是**两道独立的筛子**，下面两个 `match`
/// 各是一道。混成一个判断会在两个方向上都出错：一份刚当场问出来的读数照样可能 Unknown
/// （固件把电量那一格填了 0），而一份陈旧读数里的那个百分比本身一点问题都没有——它只是
/// 不该再当作现状。
fn comparable(candidate: &Candidate<'_>) -> Option<u8> {
    // 失联的连读数都没有，两道筛子都无从谈起。
    let reading = candidate.reading?;
    // 第一道：**新鲜**。用 [`Freshness`]，不自己去比时刻（parking lot Q26 给本票的原话）。
    //
    // 对枚举穷举、不写通配分支，与这个仓库别处同一个理由：加一档陈旧时，编译器会把人指到
    // 这里来问一句"这一档参与比较吗"，而不是让它悄悄继承 `Stale` 的处置。
    match reading.freshness {
        Freshness::Fresh => {}
        // 陈旧的不参与——票面那句"一个几天前的蓝牙缓存值不该抢走这个位置"就是这一行。
        Freshness::Stale | Freshness::VeryStale => return None,
    }
    // 第二道：**可信**。同样穷举。
    match reading.level {
        // 正在充电的照常参与（票面第 3 条）：这里根本没有充电这一维，而那不是遗漏——
        // 一台插着线还剩 5% 的设备恰恰是最该盯的，它可能压根没充上。
        Level::Reported(percent) | Level::Derived(percent) => Some(percent),
        // `CONTEXT.md`：「Unknown 不等于 0%」。当 0 会让一台答不出电量的设备永久占住
        // 托盘图标，而它其实什么都没说。
        Level::Unknown => None,
    }
}
