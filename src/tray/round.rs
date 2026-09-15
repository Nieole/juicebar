//! 一轮：某台 Device 的一次取数有了结果 → 合成新的一轮（`CONTEXT.md`「一轮」）→ 画出 Primary Device，
//! 更新悬停提示，决定要不要存状态文件，把取数失败的原因写进日志。
//!
//! 这一轮怎么判（图标状态、Primary Device、告警）不在这里，在 `crate::round`——那是一个纯函数，这里
//! 只替它记着每台 Device 手上有什么、上一轮选的是谁，再把它交出来的结果变成动作。

use std::collections::BTreeMap;

use crate::clock::Timestamp;
use crate::config::{Config, Device};
use crate::icon::{IconSettings, IconSize, IconState, Theme};
use crate::readout::{NoReading, RowReading};
use crate::round::{DeviceState, InHand, Round, Warning};
use crate::sources::level::Level;
use crate::staleness::Freshness;
use crate::state::{LastKnown, Provenance};

use super::{Fetched, Look, hover};

/// 一轮这一块的动作。
#[derive(Debug)]
pub enum Action {
    /// 按给定参数重画图标。
    DrawIcon(IconRequest),
    /// 更新悬停提示。
    Tooltip(String),
    /// 存状态文件。
    SaveState(SaveState),
}

/// 图标怎么画：外壳拿它原样调 [`crate::icon::render`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconRequest {
    /// 图标样式：手上那份配置的 `[tray]` 里管图标的六项。
    pub settings: IconSettings,
    /// Primary Device 的图标状态；选不出 Primary Device 时是无已知值。
    pub state: IconState,
    /// 图标上画的那个数；不画数时是 `None`。
    pub percent: Option<u8>,
    pub size: IconSize,
    pub theme: Theme,
}

/// 存一次状态文件。
///
/// 读数不在这里：每一次取数真读到的那一份，取数那一层当场就记进了取数线程手上的那一份状态
/// （`crate::readout::read_or_last_known`），这个动作让它落盘。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveState {
    /// 这一轮选出的 Primary Device，记进状态文件的 `last_primary`。
    ///
    /// `None` = 这一轮选不出来，**那一格不动**——不是清空。选不出来的那一轮正是最需要上次那个值的
    /// 时候（`CONTEXT.md`「保持上次的选择」），而这一格断了不会红，只会悄悄永远选不出来。
    pub remember_primary: Option<String>,
}

/// 一轮这一块记着的东西：每台 Device 手上有什么、上一轮选的是谁、托盘上此刻是什么样子。
pub struct Rounds {
    look: Look,
    /// 手上最近的那个"当下"：上一格时钟，或者上一次取数。不是由取数开的那一轮（菜单里换了 Primary Device）照它判。
    now: Timestamp,
    /// 每台 Device 手上有的东西，一轮一轮地留着：一台两次取数之间会过好几轮。
    in_hand: BTreeMap<String, InHand>,
    /// 上一轮按规则选出的那一台，起手是状态文件里的 `last_primary`（`CONTEXT.md`「保持上次的选择」）。
    /// 只有"设"没有"清"，理由同 [`SaveState::remember_primary`]。
    remembered: Option<String>,
    /// 这一轮的 Primary Device，选不出来是 `None`。两轮之间时钟走一格，悬停提示照这一台重排。
    primary: Option<String>,
    /// 此刻托盘上画着的图标与悬停提示：一样的就不再发一遍（时钟每秒一格，"5 分钟前"要一分钟才变）。
    shown_icon: Option<IconRequest>,
    shown_tooltip: Option<String>,
}

impl Rounds {
    /// 启动：每台 Device 手上先是状态文件里的上次已知值，没有就是无已知值；画出这一刻的样子。
    ///
    /// **起手这一次不存状态文件**：手上的东西全是从那个文件里读出来的，没有一样新的要记。
    pub fn new(
        config: &Config,
        last_known: &LastKnown,
        look: Look,
        now: Timestamp,
        out: &mut Vec<super::Action>,
    ) -> Self {
        let in_hand = config
            .devices
            .iter()
            .map(|device| {
                (
                    device.id.clone(),
                    in_hand_from_state_file(last_known, device, config, now),
                )
            })
            .collect();
        let mut rounds = Self {
            look,
            now,
            in_hand,
            remembered: last_known.last_primary().map(str::to_owned),
            primary: None,
            shown_icon: None,
            shown_tooltip: None,
        };
        rounds.compose(config, now, Vec::new(), out);
        rounds
    }

    /// 某台的一次取数有了结果：新的一轮。
    ///
    /// 这一轮带着此刻还挂着的全部告警（`warnings`，路由那一层交来的 `super::warnings::Warnings::hanging`），
    /// 不只是这一次取数问进程时的那一条（parking lot Q162）。
    ///
    /// 状态文件只在有新东西要记的时候存：这一次取数真读到了一份（它已经记进取数线程手上那一份状态），
    /// 或者这一轮选出的 Primary Device 换了人。两样都没有就一个字节都不写。
    pub fn on_fetched(
        &mut self,
        config: &Config,
        warnings: Vec<Warning>,
        fetched: Fetched,
        out: &mut Vec<super::Action>,
    ) {
        log_failure(config, &fetched, out);
        let just_read = matches!(
            &fetched.in_hand,
            InHand::Reading(row) if row.provenance == Provenance::JustRead
        );
        self.in_hand.insert(fetched.device_id, fetched.in_hand);
        self.now = fetched.at;
        self.new_round(config, warnings, just_read, out);
    }

    /// 菜单里换了"托盘上画哪一台"（`config` 里的 `primary` 已经是新的那一条）：照它**当场**重算这一轮、重画——用户点
    /// 了就该看见图标换人，不等下一次取数（parking lot Q252 在菜单这一头的那一半）。
    ///
    /// 各台手上的东西不变，"当下"是手上最近的那一刻（[`Self::now`]）。存不存状态文件照取数开的那一轮的规矩，只是没有
    /// 新读到的东西要记：这一轮选出的换了人才存（记下 `last_primary`）。
    pub fn on_primary_changed(
        &mut self,
        config: &Config,
        warnings: Vec<Warning>,
        out: &mut Vec<super::Action>,
    ) {
        self.new_round(config, warnings, false, out);
    }

    /// 合成新的一轮、画出来；这一轮选出的换了人，或者 `just_read`（这一次取数真读到了一份），就存状态文件。
    fn new_round(
        &mut self,
        config: &Config,
        warnings: Vec<Warning>,
        just_read: bool,
        out: &mut Vec<super::Action>,
    ) {
        let selected = self.compose(config, self.now, warnings, out);
        let changed = selected.is_some() && selected != self.remembered;
        if changed {
            self.remembered.clone_from(&selected);
        }
        if just_read || changed {
            out.push(super::Action::Round(Action::SaveState(SaveState {
                remember_primary: selected,
            })));
        }
    }

    /// 时钟走到了：不是新的一轮——Primary Device 不换人，图标不重判——只让悬停提示里的"多久前"跟上
    /// 当下。悬停提示是一段死字，不跟着走，它就一直说"0 秒前"。
    pub fn on_tick(&mut self, config: &Config, now: Timestamp, out: &mut Vec<super::Action>) {
        self.now = now;
        let Some(id) = &self.primary else {
            return;
        };
        let Some((device, in_hand)) = config
            .devices
            .iter()
            .find(|device| &device.id == id)
            .and_then(|device| {
                self.in_hand
                    .get(&device.id)
                    .map(|in_hand| (device, in_hand))
            })
        else {
            return;
        };
        let state = DeviceState::assess(device, in_hand, &config.general, now);
        self.show_tooltip(hover::device_text(&state), out);
    }

    /// 合成这一轮、画出来，交出这一轮选出的 Primary Device。
    ///
    /// 全部 Device 在同一个"当下"判（[`Round::assess`]）：一轮只看各台当下手上有的东西，一份几分钟前
    /// 取到的读数此刻就是几分钟前的。上一轮是谁取自 [`Self::remembered`]。`warnings` 是此刻还挂着的全部；
    /// 它们在挂上的那一刻就写过日志了（`super::warnings`），这里不再写——挂着的每一轮都记一行，只会把真的
    /// 发生淹掉。
    fn compose(
        &mut self,
        config: &Config,
        now: Timestamp,
        warnings: Vec<Warning>,
        out: &mut Vec<super::Action>,
    ) -> Option<String> {
        let devices = config.devices.iter().filter_map(|device| {
            self.in_hand
                .get(&device.id)
                .map(|in_hand| (device, in_hand))
        });
        let round = Round::assess(
            &config.general,
            devices,
            self.remembered.as_deref(),
            warnings,
            now,
        );
        let icon = self.icon_for(config.tray.icon, round.primary_state());
        let tooltip = hover::text(&round);
        let selected = round.primary.primary_id().map(str::to_owned);
        if self.shown_icon != Some(icon) {
            self.shown_icon = Some(icon);
            out.push(super::Action::Round(Action::DrawIcon(icon)));
        }
        self.show_tooltip(tooltip, out);
        self.primary.clone_from(&selected);
        selected
    }

    fn show_tooltip(&mut self, text: String, out: &mut Vec<super::Action>) {
        if self.shown_tooltip.as_ref() != Some(&text) {
            self.shown_tooltip = Some(text.clone());
            out.push(super::Action::Round(Action::Tooltip(text)));
        }
    }

    /// 托盘上画什么：Primary Device 的图标状态与电量。
    ///
    /// 选不出 Primary Device（选不出来，或者钉的 id 不在册）时画无已知值：图标上没有哪一台，"没有读数
    /// 时画什么"那个符号说的正是这件事；为什么选不出来，由悬停提示说。
    fn icon_for(&self, settings: IconSettings, primary: Option<&DeviceState<'_>>) -> IconRequest {
        let (state, percent) = match primary {
            Some(state) => (state.icon_state, percent_to_draw(state)),
            None => (IconState::NoKnownValue, None),
        };
        IconRequest {
            settings,
            state,
            percent,
            size: self.look.size,
            theme: self.look.theme,
        }
    }

    /// 图标样式换了（菜单里点的，或者重读进来的配置里 `[tray]` 变了）：**不开新的一轮**，把托盘上此刻画着的那一张换成
    /// 新的样式重画。Primary Device、图标状态、电量都不变，所以也没有状态文件要存；样式其实没变就什么都不发。
    pub(super) fn restyle(&mut self, settings: IconSettings, out: &mut Vec<super::Action>) {
        let Some(shown) = self.shown_icon else {
            return;
        };
        self.redraw(IconRequest { settings, ..shown }, out);
    }

    /// 任务栏的样子变了（深浅色、显示缩放）：换掉手上那一份——图标照它画，菜单此刻实际的深浅也照它答（parking lot
    /// Q261）——再把托盘上此刻画着的那一张换成新的调色与尺寸重画。与 [`Self::restyle`] 一样不开新的一轮；样子没变就
    /// 什么都不发。
    pub(super) fn on_look_changed(&mut self, look: Look, out: &mut Vec<super::Action>) {
        self.look = look;
        let Some(shown) = self.shown_icon else {
            return;
        };
        self.redraw(
            IconRequest {
                size: look.size,
                theme: look.theme,
                ..shown
            },
            out,
        );
    }

    /// 照这张重画，与此刻画着的一样就不发。
    fn redraw(&mut self, icon: IconRequest, out: &mut Vec<super::Action>) {
        if self.shown_icon != Some(icon) {
            self.shown_icon = Some(icon);
            out.push(super::Action::Round(Action::DrawIcon(icon)));
        }
    }

    /// 此刻托盘的样子：图标照它画，菜单实际的深浅也照它答（[`super::Tray::menu_theme`]）。
    pub(super) fn look(&self) -> Look {
        self.look
    }

    /// 每台 Device 在 `now` 这一刻的样子（菜单的设备行照它排）：与合成这一轮时同一组、同一个次序（配置里的书写
    /// 顺序），只是"当下"换成了此刻——"多久前"跟着走，与两轮之间的悬停提示同一个道理。
    pub(super) fn device_states<'a>(
        &'a self,
        config: &'a Config,
        now: Timestamp,
    ) -> Vec<DeviceState<'a>> {
        config
            .devices
            .iter()
            .filter_map(|device| {
                self.in_hand
                    .get(&device.id)
                    .map(|in_hand| DeviceState::assess(device, in_hand, &config.general, now))
            })
            .collect()
    }

    /// 这一轮的 Primary Device，选不出来是 `None`：托盘上此刻画的就是它。
    pub(super) fn primary(&self) -> Option<&str> {
        self.primary.as_deref()
    }

    /// 配置换了一份（重读读好了）：新登记的 Device 手上先是无已知值——状态文件只在启动时读，之后没有它可以问——好让它
    /// 在第一次取数回来之前就在这一轮里，菜单上照样有它那一行（本票 Spec review 指出：不然从一台都没有变成有一台时，
    /// 设备行那一块整块是空的）。已经记着的一样不动；配置里删掉的留着也无妨，合成这一轮只照配置里的名单取。
    pub(super) fn track_devices(&mut self, config: &Config) {
        for device in &config.devices {
            self.in_hand
                .entry(device.id.clone())
                .or_insert(InHand::NoKnownValue);
        }
    }
}

/// 状态文件里这台 Device 的上次已知值，交成这一轮手上的东西；没有（或者已经过了 `very_stale_after`）
/// 就是无已知值。
///
/// 这是托盘刚启动、第一次取数还没回来的那几秒：`CONTEXT.md` 的无已知值是"也没有上次已知值"，有就得
/// 拿出来——一律标成上次已知值（因而一律 Stale），它没有为谁让开过什么（parking lot Q153 那一面：还没
/// 取过数，就还没有什么让开过）。
fn in_hand_from_state_file(
    last_known: &LastKnown,
    device: &Device,
    config: &Config,
    now: Timestamp,
) -> InHand {
    match last_known.reading_for(&device.id, &config.general, now) {
        Some(reading) => InHand::Reading(RowReading {
            reading,
            provenance: Provenance::LastKnown,
            paused_by: None,
        }),
        None => InHand::NoKnownValue,
    }
}

/// 把这一次取数失败的完整原因写进日志。
///
/// 两种都记：交不出数的（图标上是取数失败），与退到了上次已知值的（图标上只是一个灰的旧数，而这是
/// 最常见的那一种——设备收进了抽屉、接收器拔了，或者没有管理员权限）。**暂停不记**：那不是失败，是
/// 我们主动没去问（`CONTEXT.md`「暂停」），上位机开着的每一分钟都记一行，只会把真的失败淹掉。
fn log_failure(config: &Config, fetched: &Fetched, out: &mut Vec<super::Action>) {
    let Some(device) = config.devices.iter().find(|d| d.id == fetched.device_id) else {
        return;
    };
    if let InHand::NoReading(no_reading @ NoReading::Failed(_)) = &fetched.in_hand {
        out.push(super::Action::Log(format!(
            "{} 取数失败：{no_reading}",
            device.name
        )));
    }
    if let InHand::FellBack {
        because: reason @ NoReading::Failed(_),
        ..
    } = &fetched.in_hand
    {
        out.push(super::Action::Log(format!(
            "{} 取数失败，退到上次已知值：{reason}",
            device.name
        )));
    }
}

/// 图标上画的那个数。
///
/// 陈旧到只该说日期的那一档不画数（parking lot Q151 交给本票的那一半）：命令行那一行在这一档只印日期，
/// 理由对图标一字不差——十天前的一个 62 画在托盘上就是一句假话。图标状态照旧（Stale，或者低电），
/// 渲染器在有状态、没有数的时候画"没有读数时"那个符号，日期由悬停提示说。
fn percent_to_draw(state: &DeviceState<'_>) -> Option<u8> {
    match level_to_show(state)? {
        Level::Reported(percent) | Level::Derived(percent) => Some(percent),
        Level::Unknown => None,
    }
}

/// 这一台此刻拿得出来给人看的那个电量：图标上画的数（[`percent_to_draw`]）与菜单设备行右列写的电量
/// （`super::device_row`）是同一个，所以只在这里判一次。
///
/// 没有读数、陈旧到只该说日期的那一档、电量 Unknown，都没有数可给人看。
pub(super) fn level_to_show(state: &DeviceState<'_>) -> Option<Level> {
    let (_, staleness) = state.reading()?;
    match staleness.freshness {
        Freshness::Fresh | Freshness::Stale => {}
        Freshness::VeryStale => return None,
    }
    match state.level()? {
        level @ (Level::Reported(_) | Level::Derived(_)) => Some(level),
        Level::Unknown => None,
    }
}
