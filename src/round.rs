//! 一轮：把每台 Device 的当下状态合起来、选出 Primary Device（`CONTEXT.md`「一轮」）。
//!
//! 这个模块交出**这一轮的结果**，而且是结构化的：每台 Device 的图标状态、电量（有的话）、
//! 来源 Endpoint、多久前、一句短原因；Primary Device 是谁、怎么选出来的（钉死 / 自动 / 保持上次
//! 的选择）；这一轮的告警。托盘要的就是它——图标画 Primary Device 的那一格，悬停提示与菜单行
//! 从它排出来。命令行在过渡期也从它排版（`cli::status`）：那几行一个字不变，两句 stderr 成了
//! 这一轮的告警（措辞跟着换成了"这一轮"）。
//!
//! **它是纯函数**：输入是配置、各台 Device 这一轮手上有的东西（一次取数的结果或上次已知值、
//! 暂停与否）、上一轮选的是谁、已经有了的告警、以及"当下"；不碰 Win32 界面、不碰硬件、不问时钟。
//!
//! 不在这里的两件事，都在它的两头：
//!
//! - **一次取数**在 `crate::readout`：先试哪一条 Endpoint、暂停时让开哪几条、读不到时退到上次
//!   已知值。这里只收它交出来的东西（[`InHand`]）。唯一住在这里的取数前置是 [`PauseCheck`]：
//!   问本机有没有厂商上位机在跑，它交给取数的是撞见的那个上位机，交给这一轮的是问不出来时的
//!   那一条告警——后者是这个模块的词，所以那条规则住在这里。
//! - **记进状态文件**在调用方：这一轮选出的 Primary Device 要不要记下来、读数什么时候写盘，
//!   都是外壳（今天是 `cli::status::run`）的事。这里只交出"选出了谁"。

use std::fmt;

use crate::clock::Timestamp;
use crate::config::{Device, General};
use crate::endpoints::EndpointKind;
use crate::primary::{self, Candidate, CandidateReading, Selection};
use crate::readout::{NoReading, RowReading};
use crate::sources::level::{Level, LevelSource, level_for};
use crate::staleness::{Freshness, Staleness};
use crate::state::Provenance;
use crate::vendor_hub::{Processes, VendorHub};

/// 托盘图标为一台 Device 画出的那一种状态，八选一（`CONTEXT.md`「图标状态」）。
///
/// 同时符合好几种时按这个顺序取第一个：**暂停 > 取数失败 / Unknown / 无已知值 > 充电中 >
/// 低电 > Stale > 正常**（判法见 [`DeviceState::assess`]）。每种画成什么颜色、什么符号由用户的
/// 图标设置决定（`docs/adr/0005`），所以这里的名字一个都不叫颜色。
///
/// **它与图标渲染器的输入 `icon::IconState` 是同一个东西**：渲染器那张票与这一张并行在做，
/// 各自先定义一份，名字与八个变体逐字相同；两边都落地后合成一份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    /// 新鲜的读数，电量不低。
    Normal,
    /// 电量低于这台 Device 的低电阈值（[`General::low_battery_for`]）。**压过 Stale**：一台读不到
    /// 了的设备，最常见的原因就是没电关机，那正是最该提醒的时候。
    Low,
    /// 这一次取数读到它正在充电。**压过低电**：插着线的那台，用户已经在处理了。
    Charging,
    /// 手上那份读数不能当现状：上次已知值（一律如此），或者当场读到而已经过了陈旧阈值的。
    Stale,
    /// 厂商上位机在跑，这台 Device 有 Endpoint 为它让开了——我们主动没去问。**压过其余每一种**：
    /// 用户该动手的地方是那个上位机。
    Paused,
    /// 这一次取数交不出可以印的数，又退不到上次已知值（`CONTEXT.md`「取数失败」）。
    FetchFailed,
    /// 手上有一份读数，而它的电量字段采信不了。**不等于 0%**，所以也不是低电。
    Unknown,
    /// 还没有一次取数有过结果，也没有上次已知值。它不是取数失败：没有什么失败了，只是还没读到。
    NoKnownValue,
}

/// 一台 Device 这一轮手上有的东西——这一轮的输入。
///
/// 前两种就是一次取数交出来的那个 `Result`（[`crate::readout::read_or_last_known`]，见下面的
/// `From`）；第三种是一次取数都还没有结果的时候，常驻之后每台设备刚启动时都是它。
///
/// 它归调用方所有、一轮一轮地留着（托盘里某台 Device 两次取数之间会过好几轮），这一轮只借它：
/// [`NoReading`] 装着一个 `anyhow::Error`，拷不了。
pub enum InHand {
    /// 一份读数：这一次取数读到的，或者读不到时退到的上次已知值——哪一种由
    /// [`RowReading::provenance`] 分辨。
    Reading(RowReading),
    /// 一次取数什么都没交出来，又退不到上次已知值：取数失败，或者暂停。
    NoReading(NoReading),
    /// 无已知值：一次取数都还没有结果，状态文件里也没有它。
    ///
    /// 它不带暂停这一维：还没取过数，就还没有什么为上位机让开过（parking lot Q153）。
    NoKnownValue,
}

impl From<Result<RowReading, NoReading>> for InHand {
    /// 一次取数的结果就是这一轮手上的东西。
    fn from(fetched: Result<RowReading, NoReading>) -> Self {
        match fetched {
            Ok(row) => Self::Reading(row),
            Err(no_reading) => Self::NoReading(no_reading),
        }
    }
}

/// 暂不暂停：问一次本机撞见了哪个厂商上位机，连同问不出来时的那一条告警。
///
/// 它要在取数**之前**问——撞见的那个上位机递给随后的每一次取数，好让它们让开那几条 HID
/// （[`Self::paused_by`]）；问不出来时的那一条告警（[`Self::warning`]）由调用方交给这一轮
/// （[`Round::assess`] 的 `warnings`）。命令行一次 `status` 只问一次，全部 Device 共用：一次进程
/// 枚举不便宜，而这一次里它的答案不会变（parking lot Q27）。
pub struct PauseCheck {
    paused_by: Option<VendorHub>,
    warning: Option<Warning>,
}

impl PauseCheck {
    /// 问一次本机。开关关着时连进程都不去枚举（[`VendorHub::detect`] 的第一行）。
    ///
    /// **问不出来时按"没在跑"走**（fail open），并交出一条告警（[`Warning::ProcessesUnknown`]）。
    /// 反过来——问不出来就一律暂停——会让一台 Win32 调用失败的机器永久显示暂停，而那句话指名的
    /// 进程根本没在跑：一句用户查不下去的假话，比一次可能读错的数字更难修（parking lot Q34）。
    /// 而那件没问出来的事也不能咽掉：压成一个不声不响的"没在跑"，暂停就会一声不吭地永远不触发。
    pub fn detect(general: &General, processes: &dyn Processes) -> Self {
        match VendorHub::detect(general, processes) {
            Ok(paused_by) => Self {
                paused_by,
                warning: None,
            },
            Err(e) => Self {
                paused_by: None,
                warning: Some(Warning::ProcessesUnknown(format!("{e:#}"))),
            },
        }
    }

    /// 撞见的那个上位机，递给随后的每一次取数。`None` 即那几次取数什么都不让开。
    pub fn paused_by(&self) -> Option<&VendorHub> {
        self.paused_by.as_ref()
    }

    /// 问不出来时的那一条告警，交给这一轮。问得出来（撞没撞见上位机都一样）就没有。
    pub fn warning(&self) -> Option<Warning> {
        self.warning.clone()
    }
}

/// 这一轮的一条告警：不属于任何一台 Device、而用户该知道的事。
///
/// 各台 Device 自己的事（取数失败、暂停……）不在这里，在它们各自的图标状态与短原因上。
///
/// 装的是一句已经说完整的原因（`{:#}` 排好的错误链），不是 `anyhow::Error` 本身：一条告警要能
/// 拷、能比，而它往下只会被印出去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// 问不出本机在跑哪些进程，取数按"没在跑"走（[`PauseCheck::detect`]）。
    ProcessesUnknown(String),
    /// 这一轮的读数写不进状态文件：丢的是下次启动时的上次已知值，不是这一轮的结果。
    ///
    /// **这一条不经过 [`Round`]**：写盘在这一轮合成之后，而这个纯函数碰不到磁盘。它由写盘的
    /// 那一方造出来（今天是 `cli::status::run`），与上面那一条同一种东西，说法也在这里定。
    StateNotSaved(String),
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProcessesUnknown(reason) => {
                write!(f, "认不出本机在跑哪些进程，这一轮不暂停 —— {reason}")
            }
            Self::StateNotSaved(reason) => {
                write!(
                    f,
                    "记不下这一轮的读数（下次启动就没有上次已知值了）—— {reason}"
                )
            }
        }
    }
}

/// 这一轮的结果。
pub struct Round<'a> {
    /// 每台 Device 这一轮的样子，次序即配置里的书写顺序。
    pub devices: Vec<DeviceState<'a>>,
    /// Primary Device 是谁、怎么选出来的：钉死（`Pinned`）、自动（`Lowest`）、保持上次的选择
    /// （`HeldOver`），或者选不出来（`PinnedNotFound` / `Undecided`）。规则在 `crate::primary`。
    pub primary: Selection<'a>,
    /// 这一轮的告警。
    pub warnings: Vec<Warning>,
}

impl<'a> Round<'a> {
    /// 一轮。纯函数：配置、各台 Device 这一轮手上有的东西、上一轮选出的是谁、已经有了的告警、
    /// 当下 → 这一轮的结果。
    ///
    /// 全部 Device 在**同一个"当下"**判：一轮只看各台当下手上有的东西，而一份几分钟前取到的读数
    /// 此刻就是几分钟前的（`CONTEXT.md`「一轮」）。各台各自在自己的"当下"判、再合成，走
    /// [`DeviceState::assess`] 加 [`Self::compose`]。
    ///
    /// `previous_primary` 是上一轮按规则选出的那台的 id（状态文件里的 `last_primary`），`None` =
    /// 没有上一轮。`warnings` 是这一轮之前就有了的告警（今天只有 [`PauseCheck::warning`]），原样
    /// 收进这一轮——几次取数各问过几次本机、该带哪几条，是调用方的事。
    pub fn assess(
        general: &'a General,
        devices: impl IntoIterator<Item = (&'a Device, &'a InHand)>,
        previous_primary: Option<&'a str>,
        warnings: Vec<Warning>,
        now: Timestamp,
    ) -> Self {
        let devices = devices
            .into_iter()
            .map(|(device, in_hand)| DeviceState::assess(device, in_hand, general, now))
            .collect();
        Self::compose(general, devices, previous_primary, warnings)
    }

    /// 已经各自判完的几台合成一轮：选出 Primary Device，收下这一轮的告警。纯函数。
    ///
    /// 与 [`Self::assess`] 的差别只在"当下"：这里每台带着它自己判过的那个。命令行走这一条——
    /// 它一台一台依次取数，每台的"当下"就是它自己取数的那一刻（`cli::status::run` 上写了为什么）。
    pub fn compose(
        general: &'a General,
        devices: Vec<DeviceState<'a>>,
        previous_primary: Option<&'a str>,
        warnings: Vec<Warning>,
    ) -> Self {
        // 候选就是登记在册的这几台。未登记的 BLE 设备天然不参与：它们不是 Device，进不了这份
        // 名单（parking lot Q24）。
        let candidates: Vec<Candidate<'a>> = devices
            .iter()
            .map(|state| Candidate {
                id: state.device.id.as_str(),
                reading: state.candidate(),
            })
            .collect();
        let primary = primary::select(&general.primary, &candidates, previous_primary);
        Self {
            devices,
            primary,
            warnings,
        }
    }
}

/// 一台 Device 在这一轮里的样子。
pub struct DeviceState<'a> {
    /// 配置里那个 Device。
    pub device: &'a Device,
    /// 它的图标状态。
    pub icon_state: IconState,
    /// 判它低不低电用的那个阈值：它自己写了就是它自己的，没写就是 `[general]` 的。
    ///
    /// 交出来是因为这一轮之后还有人要问同一个问题（低电通知），答案得是同一个。
    pub low_battery: u8,
    /// 它这一轮拿什么出来给人看。电量、来源、多久前、短原因都从这里读（下面那几个方法）。
    pub shown: Shown<'a>,
}

/// 一台 Device 这一轮拿什么出来给人看：[`InHand`] 借过来，读数那一种再带上它在"当下"有多旧。
///
/// 与 [`InHand`] 一一对应，差的只是那个陈旧判定——它要"当下"，而 [`InHand`] 是一轮一轮留着的、
/// 不知道哪一轮在看它。
pub enum Shown<'a> {
    /// 一份读数，连同它在这一轮的"当下"有多旧。
    Reading {
        row: &'a RowReading,
        staleness: Staleness,
    },
    /// 没有读数：取数那一层交出的原因。
    NoReading(&'a NoReading),
    /// 无已知值。
    NoKnownValue,
}

impl<'a> DeviceState<'a> {
    /// 一台 Device、它手上这一份、配置、当下 → 它这一轮的样子。纯函数。
    ///
    /// 图标状态照 [`IconState`] 上那个顺序取第一个符合的。几处不是一眼能看出来的：
    ///
    /// - **暂停看的是"这一次取数真让开过 Endpoint"**（[`RowReading::paused_by`]，或者
    ///   [`NoReading::Paused`]），不是"本机有上位机在跑"：一副只配了 `Ble` 的耳机压根没有让开的
    ///   东西。
    /// - **上次已知值里记着的"充电中"不算充电中**：那是当时的状态，不是现在的
    ///   （[`Provenance::charging_now`]，命令行那一行守的是同一条）。
    /// - **上次已知值一律 Stale**，哪怕它是十秒前取的：它有多新，和设备此刻在不在，是两件事
    ///   （`CONTEXT.md`「上次已知值」）。当场读到的按陈旧阈值判。
    /// - 陈旧到不该再显示数字的那一档（只有 `Ble` 到得了）在这里仍是 Stale 一类，照样参与低电判定
    ///   （parking lot Q151）。
    pub fn assess(
        device: &'a Device,
        in_hand: &'a InHand,
        general: &General,
        now: Timestamp,
    ) -> Self {
        let low_battery = general.low_battery_for(device);
        let shown = match in_hand {
            InHand::Reading(row) => Shown::Reading {
                row,
                staleness: Staleness::assess(&row.reading, general, now),
            },
            InHand::NoReading(no_reading) => Shown::NoReading(no_reading),
            InHand::NoKnownValue => Shown::NoKnownValue,
        };
        Self {
            device,
            icon_state: icon_state(&shown, device.level_source, low_battery),
            low_battery,
            shown,
        }
    }

    /// 手上那份读数，连同它在这一轮的"当下"有多旧。没有读数就没有。
    pub fn reading(&self) -> Option<(&'a RowReading, &Staleness)> {
        match &self.shown {
            Shown::Reading { row, staleness } => Some((row, staleness)),
            Shown::NoReading(_) | Shown::NoKnownValue => None,
        }
    }

    /// 电量：这一行显示的那个百分比，连同它取自哪里（`crate::sources::level`）。没有读数就没有。
    pub fn level(&self) -> Option<Level> {
        self.reading()
            .map(|(row, _)| level_for(&row.reading.reading, self.device.level_source))
    }

    /// 来源 Endpoint：手上那份读数是从哪一条上取到的。没有读数就没有。
    pub fn source(&self) -> Option<EndpointKind> {
        self.reading().map(|(row, _)| row.reading.endpoint)
    }

    /// 多久前：手上那份读数的取得时刻距这一轮的"当下"多少秒。
    ///
    /// `None` = 没有读数，或者那份读数说不出取得时刻（那台 BLE 设备没有更新时间戳）。
    ///
    /// 命令行那一行在 `Ble` 上印的是 Windows 报的那个缓存年龄（parking lot Q21），与这个数在
    /// 命令行里恰好相等：它每台用自己取数那一刻的"当下"判，而取得时刻正是那一刻减去缓存年龄。
    /// 两者只在"当下"晚于取数时分开——那时这里多出来的正是真实流逝的那几秒。
    pub fn age_secs(&self) -> Option<u64> {
        self.reading().and_then(|(_, staleness)| staleness.age_secs)
    }

    /// 一句短原因：手上没有读数时，取数那一层说的那一句——为什么交不出数（取数失败的三种来路，
    /// 或者暂停）。有读数、或者无已知值（没有什么失败了）时没有。
    ///
    /// 交的是那个值本身，不是一句另写的短话：它的 `Display` 就是那一句（命令行那一行印的正是它），
    /// 而它的变体还分得出是暂停还是取数失败。菜单上要多短，由排菜单的那一侧定（parking lot Q152）。
    pub fn reason(&self) -> Option<&'a NoReading> {
        match self.shown {
            Shown::NoReading(no_reading) => Some(no_reading),
            Shown::Reading { .. } | Shown::NoKnownValue => None,
        }
    }

    /// 它在 Primary 选择里的样子：这一行显示的那个百分比，以及这份读数还算不算现状。没有读数
    /// 就不参与 `lowest` 比较——那与一份 Unknown 的读数是两回事（`CONTEXT.md`：Unknown 也不等于
    /// 设备离线）。
    ///
    /// **"还算不算现状"只看陈旧阈值，不看来路**，与命令行一直以来的判法一个字不差：拿出来顶上的
    /// 上次已知值，若取得时刻还在阈值以内，照样以 `Fresh` 参与 `lowest`——而图标状态说它 Stale。
    /// 两处对不上，记在 parking lot Q154。
    pub fn candidate(&self) -> Option<CandidateReading> {
        let (_, staleness) = self.reading()?;
        Some(CandidateReading {
            level: self.level()?,
            freshness: staleness.freshness,
        })
    }
}

/// 图标状态的判法，照 [`IconState`] 上那个顺序。
fn icon_state(shown: &Shown<'_>, level_source: LevelSource, low_battery: u8) -> IconState {
    match shown {
        Shown::Reading { row, .. } if row.paused_by.is_some() => IconState::Paused,
        Shown::Reading { row, staleness } => {
            let low = match level_for(&row.reading.reading, level_source) {
                // "低于"阈值才算：`CONTEXT.md` 的原话，刚好等于阈值还不算低电。
                Level::Reported(percent) | Level::Derived(percent) => percent < low_battery,
                Level::Unknown => return IconState::Unknown,
            };
            if row.provenance.charging_now(row.reading.reading.charging) == Some(true) {
                IconState::Charging
            } else if low {
                IconState::Low
            } else if is_stale(row.provenance, staleness.freshness) {
                IconState::Stale
            } else {
                IconState::Normal
            }
        }
        Shown::NoReading(NoReading::Paused { .. }) => IconState::Paused,
        Shown::NoReading(NoReading::Failed(_)) => IconState::FetchFailed,
        Shown::NoKnownValue => IconState::NoKnownValue,
    }
}

/// 手上那份读数不能当现状了吗。
///
/// 对两维都穷举、不写通配分支，理由与 `cli::status` 里 `stale_marker` 那一条相同：加一档陈旧
/// 或者加一种来路时，编译器会把人指到这里来。那一处要把五格排成四句不同的话，这里只要一个
/// 是非，所以两边各写一张穷举的表，不合成一个。
fn is_stale(provenance: Provenance, freshness: Freshness) -> bool {
    match (provenance, freshness) {
        (Provenance::JustRead, Freshness::Fresh) => false,
        (Provenance::JustRead, Freshness::Stale | Freshness::VeryStale) => true,
        // 上次已知值一律 Stale：它有多新，和设备此刻在不在，是两件事。
        (Provenance::LastKnown, Freshness::Fresh | Freshness::Stale | Freshness::VeryStale) => true,
    }
}
