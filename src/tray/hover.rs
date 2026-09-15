//! 悬停提示的文字：从这一轮的结果排出来（spec 用户故事 16：名字、电量、来源和多久前）。
//!
//! 一行一件事：名字；电量（或者为什么没有电量）；来源与多久前，连同它能不能当现状；暂停时再加一行点
//! 那个上位机的名。托盘的悬停提示认换行，一行一件事比挤成一长行好读。
//!
//! **这几句说法只在这里定一份**：电量那一段、"来自 X（多久前）"、陈旧与暂停两个标注、读不到时的那句原因。菜单
//! 设备行的中段说同一件事时用的就是这里的函数（[`source_and_age`] 等，`super::device_row`）：同一个用户读的同一件事，
//! 说法漂开，他就得学两套。命令行 `status` 那一行原先也说这几句，它随命令行退场（`docs/adr/0008`），那批排版用例
//! 迁到了 `tests/tray_hover.rs`。
//!
//! 有两样不写：电压与"未充电"（悬停提示放不下，也不是用户停下来看它时要的东西）。"多久前"从取得时刻与当下减出来
//! （`Ble` 那一级也是：托盘常驻，Windows 当时报的缓存年龄过一分钟就不对了）。

use crate::bluetooth::age_text;
use crate::endpoints::EndpointKind;
use crate::primary::Selection;
use crate::readout::RowReading;
use crate::round::{DeviceState, Round, Shown};
use crate::sources::level::level_for;
use crate::staleness::{Freshness, Staleness};
use crate::state::Provenance;

use super::register::{NEW_DEVICE, REGISTER};

/// 悬停提示最长多少个 UTF-16 单元：`NOTIFYICONDATAW::szTip` 是 128 格，末尾一格要留给 NUL。
///
/// 超出的部分 Windows 不会替我们截，外壳硬塞进去只会截在一个随便的地方；所以在这里截，截处留一个
/// 省略号（[`text`]）。完整的原因在日志里。
pub const MAX_UTF16: usize = 127;

/// 配置里一台 Device 都没登记时说的那一句，接着指路：插上之后到"登记设备"里新建。悬停提示写它，菜单设备行那一块也写它
/// （`super::menu`）——用户先看到的往往是悬停提示（parking lot Q333）。两个名字取菜单里那两项的常量，改名跟着走。
pub(super) fn no_device() -> String {
    format!(
        "配置里一个 Device 都没有：插上设备或配对蓝牙之后，到\"{REGISTER}\"里点\"{NEW_DEVICE}\""
    )
}

/// 这一轮的悬停提示：Primary Device 那一台的样子；选不出 Primary Device 时，说为什么。
pub fn text(round: &Round<'_>) -> String {
    // 一台都没登记是根上的原因：那时说"选不出谁"只是同一件事的第二遍。
    if round.devices.is_empty() {
        return no_device();
    }
    fit(match (round.primary_state(), round.primary) {
        (Some(state), _) => device_lines(state).join("\n"),
        // 一次笔误，得说出来：用户以为钉住了。
        (None, Selection::PinnedNotFound(id)) => {
            format!("配置里 primary 钉的 id \"{id}\" 不在登记的 Device 里")
        }
        (None, _) => {
            "选不出 Primary Device：没有一个新鲜且可信的读数，也没有上次的选择".to_string()
        }
    })
}

/// Primary Device 那一台的悬停提示，在它自己的"当下"判过的样子。
///
/// 两轮之间时钟每走一格，内核拿这一台重判一次、重排一次：Primary Device 不换人，"多久前"跟着走。
pub fn device_text(state: &DeviceState<'_>) -> String {
    fit(device_lines(state).join("\n"))
}

/// 放得进托盘的悬停提示：超出 [`MAX_UTF16`] 就截断，末尾一个省略号。按字符截，不会把一个字劈成两半。
fn fit(text: String) -> String {
    if text.encode_utf16().count() <= MAX_UTF16 {
        return text;
    }
    let mut kept = String::new();
    let mut units = 0;
    for c in text.chars() {
        // 留一格给省略号。
        if units + c.len_utf16() > MAX_UTF16 - 1 {
            break;
        }
        units += c.len_utf16();
        kept.push(c);
    }
    kept.push('…');
    kept
}

/// 一台 Device 的悬停提示，一行一个元素。
fn device_lines(state: &DeviceState<'_>) -> Vec<String> {
    let mut lines = vec![state.device.name.clone()];
    match &state.shown {
        Shown::Reading { row, staleness } => {
            let charging = charging_marker(row);
            // 陈旧到不该再显示数字的那一档，电量整段换成它是哪一天的——一个几个月前的百分比写出来就是
            // 一句假话。说不出取得时刻就判不出这一档，那一支走不到，照常写。
            lines.push(match (staleness.freshness, row.reading.taken_at) {
                (Freshness::VeryStale, Some(taken_at)) => format!("{taken_at} 的读数"),
                (Freshness::VeryStale, None) | (Freshness::Fresh | Freshness::Stale, _) => {
                    let level = level_for(&row.reading.reading, state.device.level_source);
                    format!("{level}{charging}")
                }
            });
            lines.push(source_and_age(row, staleness));
            if let Some(note) = paused_note(row) {
                lines.push(note);
            }
        }
        // 取数失败的三种来路与暂停，那一句由取数那一层说（`NoReading` 的 `Display`）。太长就被截在
        // [`MAX_UTF16`]：短原因归菜单那一侧定（parking lot Q152），悬停提示照原句写到放不下为止。
        Shown::NoReading(no_reading) => lines.push(no_reading.to_string()),
        Shown::NoKnownValue => {
            lines.push("无已知值 —— 还没读到过，也没有上次已知值".to_string());
        }
    }
    lines
}

/// 来源与多久前，连同它能不能当现状："来自 Dongle24G（3 分钟前），已陈旧"。
///
/// 菜单设备行的中段说的是同一件事，用的就是这一句（`super::device_row`，spec 用户故事 16）。
pub(super) fn source_and_age(row: &RowReading, staleness: &Staleness) -> String {
    let source = row.reading.endpoint;
    let from = match source {
        EndpointKind::Wired | EndpointKind::Dongle24G => {
            format!("来自 {source}（{}）", age_text(staleness.age_secs))
        }
        EndpointKind::Ble => {
            format!(
                "来自 {source}（Windows 缓存，{}）",
                age_text(staleness.age_secs)
            )
        }
    };
    format!(
        "{from}{}",
        stale_marker(row.provenance, staleness.freshness)
    )
}

/// "，充电中"：这一次取数读到它正在充电时接在电量后面。上次已知值里记着的"充电中"不算（`Provenance::charging_now`，
/// 图标状态问的是同一条）。菜单设备行的中段接的也是它（`super::device_row`）。
pub(super) fn charging_marker(row: &RowReading) -> &'static str {
    match row.provenance.charging_now(row.reading.reading.charging) {
        Some(true) => "，充电中",
        Some(false) | None => "",
    }
}

/// 这一次取数有 Endpoint 为上位机让开过：点那个上位机的名，用户能动手的地方只有它。菜单设备行的中段接的也是它。
pub(super) fn paused_note(row: &RowReading) -> Option<String> {
    row.paused_by
        .as_ref()
        .map(|hub| format!("已暂停（{} 正在运行）", hub.process()))
}

/// 陈旧标注：新鲜、这一次取数读到的什么都不加，其余每一种都以"已陈旧"开头。
///
/// 几档共用一个词：`CONTEXT.md` 只给了一个 **Stale**，用户要分辨的也只有"能不能当现状"。"不显示百分比"那半句交代百分比
/// 去哪了（电量那一行已经换成了日期）。**上次已知值一律带标注，新鲜那一档也带**：阈值答的是"这个数多新"，而上次已知值的
/// 问题是设备此刻不在——等一等不会自己变新，得先把设备找出来（`crate::state::Provenance`）。对两维都穷举、不写通配分支：
/// 加一档陈旧或者加一种来路时，编译器会把人指到这里来。
fn stale_marker(provenance: Provenance, freshness: Freshness) -> &'static str {
    match (provenance, freshness) {
        (Provenance::JustRead, Freshness::Fresh) => "",
        (Provenance::JustRead, Freshness::Stale) => "，已陈旧",
        (Provenance::JustRead, Freshness::VeryStale) => "，已陈旧，不显示百分比",
        (Provenance::LastKnown, Freshness::Fresh | Freshness::Stale) => "，已陈旧，上次已知值",
        // 走不到：超过 `very_stale_after` 的上次已知值在状态文件那一步就被丢掉了。留着这一支是因为
        // 类型要交代一句，而交代的话得是实话。
        (Provenance::LastKnown, Freshness::VeryStale) => "，已陈旧，上次已知值，不显示百分比",
    }
}
