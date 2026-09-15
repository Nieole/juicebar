//! 设备行：一台 Device 在右键菜单一级里的那一行（`menu-as-designed` spec「设备行」）。
//!
//! **一行三段：〔勾〕名字 ｜ 中段 ｜ 右列。**普通菜单项只有一个右列（`docs/research/native-menu-research.md` 8.1），
//! 所以中段接在名字后面、隔两个空格，写进左边那段字里；右列用普通项自带的那一列。
//!
//! - **中段**：这一次取数拿到了读数 → 来源和多久前，连同悬停提示上的那几个标注（陈旧、充电中、为上位机让开过时
//!   点它的名），说法与悬停提示同一份（[`hover::source_and_age`] 等）；没拿到读数（取数失败，或者暂停，手上有没有
//!   上次已知值都一样）→ 短原因（[`crate::readout::NoReading::short_reason`]）；无已知值 → 空着。
//! - **右列**：手上有数 → 电量；没有数可写 → 这一台的图标状态的名字。
//! - **变灰**：图标状态是 Stale、暂停、取数失败、Unknown、无已知值之一。图标状态的优先顺序照旧，所以读不到了、
//!   上次已知值低电的那一台按低电走，不变灰。
//! - **勾**：这一轮的 Primary Device 那一行。
//!
//! 设备行没有点击动作。措辞归代码，设计稿基准里只占位（parking lot Q283）。

use crate::icon::IconState;
use crate::round::DeviceState;

use super::hover;
use super::menu::{Entry, Kind, Placeholder};
use super::round::level_to_show;

/// "菜单显示"那两项怎么设（ADR-0005 的 `menu_source`、`primary_mark`）。票 07 读 `[tray]` 表来填它；在那之前一律是缺省。
#[derive(Debug, Clone, Copy)]
pub(super) struct RowDisplay {
    /// "写出来源和多久前"：关着时，有读数的那一行中段空着，短原因照写（spec 用户故事 14）。
    pub source_and_age: bool,
    /// "在设备列表里标出 Primary Device"（`primary_mark = "both"`）：`"radio"` 时设备行不打勾，只留子菜单里的圆点。
    pub mark_primary: bool,
}

impl Default for RowDisplay {
    fn default() -> Self {
        Self {
            source_and_age: true,
            mark_primary: true,
        }
    }
}

/// 中段写的是哪一种：它决定那一句，也决定设计稿基准里的占位名。
enum Middle {
    SourceAndAge(String),
    ShortReason(String),
    Empty,
}

/// 一台 Device 此刻的那一行。`is_primary`：它是这一轮的 Primary Device。
pub(super) fn entry(state: &DeviceState<'_>, is_primary: bool, display: RowDisplay) -> Entry {
    let name = &state.device.name;
    let (text, text_placeholder) = match middle(state, display) {
        Middle::SourceAndAge(words) => (
            format!("{name}  {words}"),
            format!("〔设备行：{name} · 来源和多久前〕"),
        ),
        Middle::ShortReason(words) => (
            format!("{name}  {words}"),
            format!("〔设备行：{name} · 短原因〕"),
        ),
        Middle::Empty => (name.clone(), format!("〔设备行：{name}〕")),
    };
    let (right, right_placeholder) = match level_to_show(state) {
        Some(level) => (level.to_string(), "〔电量〕".to_string()),
        None => (
            state.icon_state.to_string(),
            format!("〔图标状态：{}〕", state.icon_state),
        ),
    };
    Entry {
        right: Some(right),
        checked: is_primary && display.mark_primary,
        grayed: grayed(state.icon_state),
        placeholder: Some(Placeholder {
            text: text_placeholder,
            right: Some(right_placeholder),
        }),
        ..Entry::new(Kind::Normal, text)
    }
}

fn middle(state: &DeviceState<'_>, display: RowDisplay) -> Middle {
    if let Some(no_reading) = state.reason() {
        return Middle::ShortReason(no_reading.short_reason());
    }
    match state.reading() {
        Some((row, staleness)) if display.source_and_age => Middle::SourceAndAge(format!(
            "{}{}{}",
            hover::source_and_age(row, staleness),
            hover::charging_marker(row),
            hover::paused_note(row)
                .map(|note| format!("，{note}"))
                .unwrap_or_default()
        )),
        Some(_) | None => Middle::Empty,
    }
}

/// 这一行变不变灰：一个不是此刻的数、或者没有数，不该被当成现状。对图标状态穷举，加一种状态时编译器会指过来。
fn grayed(state: IconState) -> bool {
    match state {
        IconState::Stale
        | IconState::Paused
        | IconState::FetchFailed
        | IconState::Unknown
        | IconState::NoKnownValue => true,
        IconState::Normal | IconState::Low | IconState::Charging => false,
    }
}
