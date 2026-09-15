//! 一轮：把每台 Device 的当下状态合起来、选出 Primary Device（`CONTEXT.md`「一轮」）。
//!
//! 这个模块交出**这一轮的结果**，而且是结构化的：每台 Device 的图标状态、电量（有的话）、
//! 来源 Endpoint、多久前、一句短原因；Primary Device 是谁、怎么选出来的（钉死 / 自动 / 保持上次
//! 的选择）；这一轮的告警。托盘要的就是它——图标画 Primary Device 的那一格，悬停提示与菜单行
//! 从它排出来。
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
//!   都是托盘那一侧的事（`crate::tray::round` 决定，外壳去写）。这里只交出"选出了谁"。

use std::fmt;

use crate::clock::Timestamp;
use crate::config::{Device, General};
use crate::endpoints::EndpointKind;
use crate::icon::IconState;
use crate::primary::{self, Candidate, CandidateReading, Selection};
use crate::readout::{NoReading, Outcome, RowReading};
use crate::sources::level::{Level, LevelSource, level_for};
use crate::staleness::{Freshness, Staleness};
use crate::state::Provenance;
use crate::vendor_hub::{Processes, VendorHub};

/// 一台 Device 这一轮手上有的东西——这一轮的输入。
///
/// 一次取数交出来的就是它：托盘拿的是连同"退到上次已知值时这一次为什么没读到"的那一份（[`Outcome`]）；只有那个
/// `Result` 的（[`crate::readout::read_or_last_known`]）也转得过来，见下面两个 `From`。无已知值是一次取数都还没有
/// 结果的时候，常驻之后每台设备刚启动时都是它。
///
/// 它归调用方所有、一轮一轮地留着（托盘里某台 Device 两次取数之间会过好几轮），这一轮只借它：
/// [`NoReading`] 装着一个 `anyhow::Error`，拷不了。
pub enum InHand {
    /// 一份读数：这一次取数读到的，或者上次已知值——哪一种由 [`RowReading::provenance`] 分辨。
    ///
    /// 上次已知值走这里的，是说不出"这一次为什么没读到"的那几种：刚启动时从状态文件里拿出来的（还没有"这一次"），
    /// 以及只交了那个 `Result` 的那一份（[`crate::readout::read_or_last_known`]）。
    Reading(RowReading),
    /// 这一次取数没读到（`because`：取数失败的某一种来路，或者暂停），退到了上次已知值（`last_known`）。
    ///
    /// 与 [`Self::Reading`] 分开，是因为那一行要说**这一次**为什么没读到：菜单设备行的中段写它的短原因，右列照样
    /// 写上次已知值的电量（`menu-as-designed` spec「设备行」，parking lot Q234）。图标状态、电量、来源、多久前都
    /// 照那份上次已知值判，与 [`Self::Reading`] 一模一样。
    FellBack {
        last_known: RowReading,
        because: NoReading,
    },
    /// 一次取数什么都没交出来，又退不到上次已知值：取数失败，或者暂停。
    NoReading(NoReading),
    /// 无已知值：一次取数都还没有结果，状态文件里也没有它。
    ///
    /// 它不带暂停这一维：还没取过数，就还没有什么为上位机让开过（parking lot Q153）。
    NoKnownValue,
}

impl From<Result<RowReading, NoReading>> for InHand {
    /// 一次取数的结果就是这一轮手上的东西。退到上次已知值时的原因不在这个 `Result` 里，所以那一份是 [`Self::Reading`]。
    fn from(fetched: Result<RowReading, NoReading>) -> Self {
        match fetched {
            Ok(row) => Self::Reading(row),
            Err(no_reading) => Self::NoReading(no_reading),
        }
    }
}

impl From<Outcome> for InHand {
    /// 托盘那一次取数的结果（[`crate::readout::read_or_last_known_with_reason`]）：退到了上次已知值，就连同这一次的原因。
    fn from(outcome: Outcome) -> Self {
        match (outcome.row, outcome.fell_back_because) {
            (Ok(last_known), Some(because)) => Self::FellBack {
                last_known,
                because,
            },
            (Ok(row), None) => Self::Reading(row),
            // 退不到时原因就是那个 `Err`，`fell_back_because` 那时是 `None`（`Outcome` 上写着）。
            (Err(no_reading), _) => Self::NoReading(no_reading),
        }
    }
}

/// 暂不暂停：问一次本机撞见了哪个厂商上位机，连同问不出来时的那一条告警。
///
/// 它要在取数**之前**问——撞见的那个上位机递给随后的每一次取数，好让它们让开那几条 HID
/// （[`Self::paused_by`]）；问不出来时的那一条告警（[`Self::warning`]）由调用方交给这一轮
/// （[`Round::assess`] 的 `warnings`）。托盘的取数线程每次取数之前问一次（`crate::shell`）：一次进程
/// 枚举不便宜，而这一次取数里它的答案不会变（parking lot Q27）。
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
    /// **它由写盘的那一方造出来**：写盘在这一轮合成之后，而这个纯函数碰不到磁盘。外壳写完盘把结果交给内核，
    /// 这一条挂在之后的每一轮上，直到某一次写成（`crate::tray::warnings`）。与上面那一条同一种东西，说法也在这里定。
    StateNotSaved(String),
    /// 配置文件读不了（写坏了，或者读不到），托盘沿用上一份读好的。
    ///
    /// 托盘运行中配置文件变了就重读（`crate::tray::config`），读不了不停摆，这一条挂到下一次读好为止。启动时读不了
    /// 没有上一份可沿用，托盘起不来（parking lot Q251）。
    ConfigUnreadable(String),
}

impl Warning {
    /// 那件事没办成的那半句，不带完整原因：菜单顶上一条告警一行，写的就是它（`crate::tray::menu`）；`Display` 在它
    /// 后面接完整原因，日志记那一整句。两处说的是同一件事，所以前半句只在这里写一次。
    pub fn headline(&self) -> &'static str {
        match self {
            Self::ProcessesUnknown(_) => "认不出本机在跑哪些进程，这一轮不暂停",
            Self::StateNotSaved(_) => "记不下这一轮的读数（下次启动就没有上次已知值了）",
            Self::ConfigUnreadable(_) => "读不了配置文件，沿用上一份读好的",
        }
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headline = self.headline();
        match self {
            Self::ProcessesUnknown(reason) | Self::ConfigUnreadable(reason) => {
                write!(f, "{headline} —— {reason}")
            }
            // 半句收在全角括号上，破折号前面不留空格。
            Self::StateNotSaved(reason) => write!(f, "{headline}—— {reason}"),
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
    /// 这一轮的告警：此刻还挂着的全部。托盘里一条挂到那件事下一次办成为止（`crate::tray::warnings`）。
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
    /// 没有上一轮。`warnings` 是此刻还挂着的告警，原样收进这一轮——该带哪几条是调用方的事：托盘带挂到那件事
    /// 下一次办成为止的全部（`crate::tray::warnings`；[`PauseCheck::warning`] 是其中一条的来处）。
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
    /// 与 [`Self::assess`] 的差别只在"当下"：这里每台带着它自己判过的那个。[`Self::assess`] 在同一个"当下"判完
    /// 全部之后走的也是这里。
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

    /// Primary Device 那一台这一轮的样子。选不出来（`PinnedNotFound` / `Undecided`）就没有。
    ///
    /// 托盘上画的、悬停提示写的都是它：两处各自拿 id 回名单里找一遍，迟早会有一处找的不是同一台。
    pub fn primary_state(&self) -> Option<&DeviceState<'a>> {
        let id = self.primary.primary_id()?;
        self.devices.iter().find(|state| state.device.id == id)
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
    /// 退到了上次已知值时，这一次取数为什么没读到（[`InHand::FellBack`]）；别的时候是 `None`。从 [`Self::reason`] 读。
    fell_back_because: Option<&'a NoReading>,
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
    ///   （[`Provenance::charging_now`]，悬停提示与菜单那一行守的是同一条）。
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
        let reading = |row: &'a RowReading| Shown::Reading {
            row,
            staleness: Staleness::assess(&row.reading, general, now),
        };
        let (shown, fell_back_because) = match in_hand {
            InHand::Reading(row) => (reading(row), None),
            InHand::FellBack {
                last_known,
                because,
            } => (reading(last_known), Some(because)),
            InHand::NoReading(no_reading) => (Shown::NoReading(no_reading), None),
            InHand::NoKnownValue => (Shown::NoKnownValue, None),
        };
        Self {
            device,
            icon_state: icon_state(&shown, device.level_source, low_battery),
            low_battery,
            shown,
            fell_back_because,
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
    /// `Ble` 那一级也是从取得时刻算到"当下"，不是 Windows 当时报的那个缓存年龄（parking lot Q21）：两者在取数那一刻
    /// 相等，之后这里多出来的正是真实流逝的那几秒——托盘常驻，那正是要写出来的数。
    pub fn age_secs(&self) -> Option<u64> {
        self.reading().and_then(|(_, staleness)| staleness.age_secs)
    }

    /// 这一次取数为什么没拿到读数（取数失败的三种来路，或者暂停）：手上没有读数时是取数那一层交出的那个；退到了
    /// 上次已知值时也有，是这一次取数自己的原因（parking lot Q234）。这一次读到了、无已知值（没有什么失败了），
    /// 或者手上的上次已知值说不出"这一次"（刚启动时从状态文件里拿的那一份）时没有。
    ///
    /// 交的是那个值本身：菜单设备行说它的短原因（[`NoReading::short_reason`]），悬停提示与日志说完整原因
    /// （`Display`），而它的变体还分得出是暂停还是取数失败。
    pub fn reason(&self) -> Option<&'a NoReading> {
        match self.shown {
            Shown::NoReading(no_reading) => Some(no_reading),
            Shown::Reading { .. } | Shown::NoKnownValue => self.fell_back_because,
        }
    }

    /// 它在 Primary 选择里的样子：这一行显示的那个百分比，以及这份读数还算不算现状。没有读数
    /// 就不参与 `lowest` 比较——那与一份 Unknown 的读数是两回事（`CONTEXT.md`：Unknown 也不等于
    /// 设备离线）。
    ///
    /// **"还算不算现状"看陈旧阈值，也看来路**：上次已知值一律不算，哪怕它的取得时刻还在阈值以内——与图标状态说它
    /// Stale 是同一个判断（[`is_stale`]）。一台刚失联的设备因此不会拿几十秒前的数去抢 `lowest`，全都不可信时"保持上次
    /// 的选择"也才走得到（parking lot Q154、Q166）。当场读到的照陈旧阈值判。
    pub fn candidate(&self) -> Option<CandidateReading> {
        let (row, staleness) = self.reading()?;
        // 对两维都穷举：加一档陈旧或者加一种来路时，编译器会把人指到这里来问一句"它参与 lowest 吗"。
        let freshness = match (row.provenance, staleness.freshness) {
            (Provenance::JustRead, freshness) => freshness,
            (Provenance::LastKnown, Freshness::Fresh | Freshness::Stale) => Freshness::Stale,
            (Provenance::LastKnown, Freshness::VeryStale) => Freshness::VeryStale,
        };
        Some(CandidateReading {
            level: self.level()?,
            freshness,
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
/// 对两维都穷举、不写通配分支，理由与悬停提示里那张陈旧标注的表（`crate::tray::hover`）相同：加一档陈旧
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
