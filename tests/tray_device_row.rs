//! 菜单设备行的确切字符串（`menu-as-designed` spec「设备行」）：一行三段——名字；来源和多久前，这一次没拿到读数时
//! 换成短原因；右列写电量或图标状态的名字——连同该变灰的行变灰、Primary Device 那一行打勾。
//!
//! 设计稿基准里设备行只占位，所以字从这里断言。全程只经内核的公开面：罐装的一次取数结果喂进去（`Tray::handle`），
//! 此刻的菜单从 `Tray::menu` 看。

mod common;

use common::menu::device_row;
use common::tray::{
    MOUSE_AND_KEYBOARD, failed_because, feed, fell_back, fell_back_because, fetched, just_read,
    last_known_value, later, reading, start,
};
use common::{NOW, vendor_hub_running};
use juicebar::clock::Timestamp;
use juicebar::endpoints::EndpointKind;
use juicebar::readout::{FailureCause, NoReading, RowReading};
use juicebar::round::InHand;
use juicebar::state::{LastKnown, Provenance};
use juicebar::tray::Tray;

/// `at` 那一刻弹出的菜单里，名字是 `name` 的那一行：左边那段字、右列、变没变灰。
fn row(tray: &Tray, at: Timestamp, name: &str) -> (String, String, bool) {
    let menu = tray.menu(at);
    let entry = device_row(&menu, name);
    (
        entry.text.clone(),
        entry.right.clone().expect("设备行总有右列"),
        entry.grayed,
    )
}

fn row_of(text: &str, right: &str, grayed: bool) -> (String, String, bool) {
    (text.to_string(), right.to_string(), grayed)
}

/// 这一次取数当场读到、新鲜：名字隔两个空格接来源和多久前（与悬停提示同一句）；右列写电量，连同它是哪一个
/// 百分比。不变灰。
#[test]
fn a_fresh_reading_writes_where_and_when_it_came_from_and_its_level() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 57, NOW),
        ),
    );

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  来自 Dongle24G（0 秒前）",
            "57%（Reported Level）",
            false
        )
    );
}

/// 多久前照菜单弹出的那一刻算，不是取数的那一刻；Ble 那一条照悬停提示的说法带着"Windows 缓存"。
#[test]
fn the_age_is_counted_to_the_moment_the_menu_pops_up() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 80, NOW)),
    );

    assert_eq!(
        row(&tray, later(NOW, 60), "VGN Neon75"),
        row_of(
            "VGN Neon75  来自 Ble（Windows 缓存，60 秒前）",
            "80%（Reported Level）",
            false
        )
    );
}

/// 当场读到、而已经过了陈旧阈值：中段带"已陈旧"，这一行变灰（Stale）。
#[test]
fn a_stale_reading_is_grayed() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 57, NOW),
        ),
    );

    assert_eq!(
        row(&tray, later(NOW, 600), "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  来自 Dongle24G（10 分钟前），已陈旧",
            "57%（Reported Level）",
            true
        )
    );
}

/// 刚启动、第一次取数还没回来：手上是状态文件里的上次已知值——还没有哪一次取数失败，中段照写来源和多久前，标着
/// 上次已知值；变灰。
#[test]
fn at_startup_the_last_known_value_from_the_state_file_is_grayed() {
    let taken = NOW.minus_secs(600);
    let mut last_known = LastKnown::default();
    last_known.record(
        "dragonfly3",
        &reading(EndpointKind::Dongle24G, 62, taken),
        taken,
    );
    let (tray, _screen) = start(MOUSE_AND_KEYBOARD, &last_known, NOW);

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  来自 Dongle24G（10 分钟前），已陈旧，上次已知值",
            "62%（Reported Level）",
            true
        )
    );
}

/// 陈旧到不显示百分比的那一档（只有 Ble 到得了）：右列没有数可写，写图标状态的名字；中段照悬停提示标着"不显示
/// 百分比"。
#[test]
fn a_reading_too_stale_to_show_its_percentage_writes_the_icon_state_instead() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "neon75",
            NOW,
            just_read(EndpointKind::Ble, 62, NOW.minus_secs(10 * 86_400)),
        ),
    );

    assert_eq!(
        row(&tray, NOW, "VGN Neon75"),
        row_of(
            "VGN Neon75  来自 Ble（Windows 缓存，10 天前），已陈旧，不显示百分比",
            "Stale",
            true
        )
    );
}

/// 取数失败的三种来路：中段各写各的短原因，不带任何一条 Endpoint 的错误；右列写图标状态的名字，变灰。
#[test]
fn a_failed_fetch_writes_the_short_reason_of_its_cause() {
    for (cause, short_reason) in [
        (FailureCause::Unreachable, "失联（没插？没配对？没开机？）"),
        (FailureCause::ReadAnomaly, "读取异常（有上位机在抢通路？）"),
        (FailureCause::NoEndpointConfigured, "一条 Endpoint 都没配置"),
    ] {
        let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
        feed(
            &mut tray,
            &mut screen,
            fetched(
                "neon75",
                NOW,
                failed_because(cause, "读不到 —— 用例里的完整原因，菜单上不该出现"),
            ),
        );

        assert_eq!(
            row(&tray, NOW, "VGN Neon75"),
            row_of(&format!("VGN Neon75  {short_reason}"), "取数失败", true),
            "{cause:?}"
        );
    }
}

/// 暂停、手上没有上次已知值：中段点名是哪个上位机，右列"暂停"，变灰。
#[test]
fn a_paused_fetch_names_the_vendor_hub() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let paused = InHand::NoReading(NoReading::Paused {
        hub: vendor_hub_running(),
        yielded: vec![EndpointKind::Dongle24G],
        failures: Vec::new(),
    });
    feed(&mut tray, &mut screen, fetched("dragonfly3", NOW, paused));

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  已暂停（VGN VHUB.exe 正在运行）",
            "暂停",
            true
        )
    );
}

/// 这一次取数没读到、退到了上次已知值（parking lot Q234）：中段写的是**这一次**为什么没读到——短原因——右列照样
/// 写着上次已知值的电量；变灰（上次已知值一律 Stale）。时钟走过几格、下一次取数之前，那一行照旧这样说。
#[test]
fn a_fetch_that_fell_back_writes_its_short_reason_beside_the_last_known_level() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "dragonfly3",
            NOW,
            last_known_value(EndpointKind::Dongle24G, 62, NOW.minus_secs(600)),
            "读不到 —— 用例里的完整原因",
        ),
    );

    assert_eq!(
        row(&tray, later(NOW, 30), "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  失联（没插？没配对？没开机？）",
            "62%（Reported Level）",
            true
        )
    );
}

/// 读不到了、而上次已知值是低电的那一台：图标状态按低电走，这一行**不变灰**——它没电关机时照样显眼。
#[test]
fn a_fetch_that_fell_back_to_a_low_last_known_value_is_not_grayed() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "dragonfly3",
            NOW,
            last_known_value(EndpointKind::Dongle24G, 12, NOW.minus_secs(600)),
            "读不到 —— 用例里的完整原因",
        ),
    );

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  失联（没插？没配对？没开机？）",
            "12%（Reported Level）",
            false
        )
    );
}

/// 暂停、手上有上次已知值：中段点名是哪个上位机，右列写上次已知值的电量；变灰（暂停）。
#[test]
fn a_paused_fetch_that_fell_back_names_the_vendor_hub_beside_the_last_known_level() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let hub = vendor_hub_running();
    let last_known = InHand::Reading(RowReading {
        reading: reading(EndpointKind::Dongle24G, 62, NOW.minus_secs(600)),
        provenance: Provenance::LastKnown,
        paused_by: Some(hub.clone()),
    });
    let because = NoReading::Paused {
        hub,
        yielded: vec![EndpointKind::Dongle24G],
        failures: Vec::new(),
    };
    feed(
        &mut tray,
        &mut screen,
        fell_back_because("dragonfly3", NOW, last_known, because),
    );

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  已暂停（VGN VHUB.exe 正在运行）",
            "62%（Reported Level）",
            true
        )
    );
}

/// 当场读到它正在充电：中段照悬停提示的说法标"充电中"；右列照写电量，不变灰（充电中不在变灰的那几种里）。
#[test]
fn a_charging_reading_says_so_after_where_it_came_from() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let mut charging = reading(EndpointKind::Dongle24G, 57, NOW);
    charging.reading.charging = Some(true);
    let in_hand = InHand::Reading(RowReading {
        reading: charging,
        provenance: Provenance::JustRead,
        paused_by: None,
    });
    feed(&mut tray, &mut screen, fetched("dragonfly3", NOW, in_hand));

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  来自 Dongle24G（0 秒前），充电中",
            "57%（Reported Level）",
            false
        )
    );
}

/// 这一次取数读到了、而有 Endpoint 为上位机让开过（Ble 顶了上来）：中段照写来源和多久前，再照悬停提示点名是哪个
/// 上位机——这一行变灰（暂停），得说得出为什么。
#[test]
fn a_reading_taken_while_paused_names_the_vendor_hub_after_where_it_came_from() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let in_hand = InHand::Reading(RowReading {
        reading: reading(EndpointKind::Ble, 80, NOW),
        provenance: Provenance::JustRead,
        paused_by: Some(vendor_hub_running()),
    });
    feed(&mut tray, &mut screen, fetched("neon75", NOW, in_hand));

    assert_eq!(
        row(&tray, NOW, "VGN Neon75"),
        row_of(
            "VGN Neon75  来自 Ble（Windows 缓存，0 秒前），已暂停（VGN VHUB.exe 正在运行）",
            "80%（Reported Level）",
            true
        )
    );
}

/// 电量 Unknown：有读数，中段照写来源和多久前；右列没有数可写，写"Unknown"，变灰。
#[test]
fn an_unknown_level_writes_unknown_and_is_grayed() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 0, NOW),
        ),
    );

    assert_eq!(
        row(&tray, NOW, "Dragonfly 3 Master+"),
        row_of(
            "Dragonfly 3 Master+  来自 Dongle24G（0 秒前）",
            "Unknown",
            true
        )
    );
}

/// 无已知值：还没取过数、状态文件里也没有——没有什么失败了，中段空着；右列"无已知值"，变灰。
#[test]
fn a_device_with_no_known_value_writes_only_its_name() {
    let (tray, _screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    assert_eq!(
        row(&tray, NOW, "VGN Neon75"),
        row_of("VGN Neon75", "无已知值", true)
    );
}

/// 这一轮的 Primary Device 那一行打勾（`primary_mark` 缺省"两处都标"），别的不打。
#[test]
fn the_row_of_the_primary_device_is_checked() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 57, NOW),
        ),
    );
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 80, NOW)),
    );

    let menu = tray.menu(NOW);

    assert!(
        device_row(&menu, "Dragonfly 3 Master+").checked,
        "电量最低的那一台"
    );
    assert!(!device_row(&menu, "VGN Neon75").checked);
}
