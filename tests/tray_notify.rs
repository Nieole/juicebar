//! 托盘内核里「通知」这一块的低电通知（`juicebar::tray::notify`，spec「低电通知」）：任何一台已登记的
//! Device 的一次取数读到低于它自己阈值的电量、而此前没为它提醒过，就弹一条通知；之后读到不低于阈值、
//! 或者读到充电中，才重新武装。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），动作出；外壳由 `common::tray::Screen` 顶替，弹过的
//! 通知攒在 [`Screen::notices`]。一次取数的结果是罐装的，时钟是一串字面量。
//!
//! [`Screen::notices`]: common::tray::Screen::notices

mod common;

use common::tray::{
    MOUSE_AND_KEYBOARD, failed, feed, fell_back, fetched, just_read, last_known_value, later,
    reading, start,
};
use common::{NOW, vendor_hub_running};
use juicebar::clock::Timestamp;
use juicebar::endpoints::EndpointKind;
use juicebar::icon::IconState;
use juicebar::readout::RowReading;
use juicebar::round::InHand;
use juicebar::state::{LastKnown, Provenance};
use juicebar::tray::notify::Notice;

/// 跌破阈值（缺省 20）的那一次取数弹一条通知：写明是哪一台、电量多少、低于多少。
#[test]
fn a_fetch_that_reads_a_level_below_the_threshold_raises_one_low_battery_notice() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 15, NOW),
        ),
    );

    assert_eq!(
        screen.notices,
        [Notice {
            title: "Dragonfly 3 Master+ 电量低".to_string(),
            body: "15%（Reported Level），低于低电量阈值 20%".to_string(),
        }]
    );
}

/// 同一次跌破只提醒一次：之后一次次取数读到的仍然低于阈值（哪怕更低了），不再弹。
#[test]
fn staying_below_the_threshold_does_not_repeat_the_notice() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    for (secs, level) in [(0, 15), (60, 14), (120, 13)] {
        let at = later(NOW, secs);
        feed(
            &mut tray,
            &mut screen,
            fetched(
                "dragonfly3",
                at,
                just_read(EndpointKind::Dongle24G, level, at),
            ),
        );
    }

    assert_eq!(screen.notices.len(), 1, "{:?}", screen.notices);
    assert_eq!(
        screen.notices[0].body,
        "15%（Reported Level），低于低电量阈值 20%"
    );
}

/// 回升到不低于阈值就重新武装，再跌破再提醒。刚好等于阈值就算回升了：低电是"低于"阈值
/// （`CONTEXT.md`「图标状态」）。
#[test]
fn reading_back_at_the_threshold_rearms_so_the_next_drop_notifies_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    for (secs, level) in [(0, 15), (60, 20), (120, 19)] {
        let at = later(NOW, secs);
        feed(
            &mut tray,
            &mut screen,
            fetched(
                "dragonfly3",
                at,
                just_read(EndpointKind::Dongle24G, level, at),
            ),
        );
    }

    let bodies: Vec<&str> = screen.notices.iter().map(|n| n.body.as_str()).collect();
    assert_eq!(
        bodies,
        [
            "15%（Reported Level），低于低电量阈值 20%",
            "19%（Reported Level），低于低电量阈值 20%",
        ]
    );
}

/// 读到它正在充电、电量还低于阈值：不提醒——用户已经在给它充了。
#[test]
fn a_device_read_charging_below_the_threshold_is_not_reminded() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched("dragonfly3", NOW, charging(10, NOW)),
    );

    assert_eq!(screen.notices, []);
}

/// 读到充电中就重新武装：插上线充了一会儿（电量仍在阈值以下）、拔掉之后再读到低于阈值，再提醒一次。
#[test]
fn reading_it_charging_rearms_so_the_next_drop_after_unplugging_notifies_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 15, NOW),
        ),
    );
    let plugged_in = later(NOW, 60);
    feed(
        &mut tray,
        &mut screen,
        fetched("dragonfly3", plugged_in, charging(16, plugged_in)),
    );
    let unplugged = later(NOW, 120);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            unplugged,
            just_read(EndpointKind::Dongle24G, 17, unplugged),
        ),
    );

    let bodies: Vec<&str> = screen.notices.iter().map(|n| n.body.as_str()).collect();
    assert_eq!(
        bodies,
        [
            "15%（Reported Level），低于低电量阈值 20%",
            "17%（Reported Level），低于低电量阈值 20%",
        ]
    );
}

/// 拿上次已知值顶上的结果不触发：这一次取数没读到，顶上来的那个低于阈值的数是上次存下的，不是新消息
/// ——但图标照样按低电画（spec「低电通知」）。钉死鼠标，好让图标上画的就是它，与 `lowest` 怎么选无关。
#[test]
fn a_result_topped_up_with_the_last_known_value_does_not_notify_but_the_icon_still_draws_low() {
    let (mut tray, mut screen) = start(&pinned("dragonfly3"), &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "dragonfly3",
            NOW,
            last_known_value(EndpointKind::Dongle24G, 15, NOW.minus_secs(30)),
            "读超时",
        ),
    );

    assert_eq!(screen.notices, []);
    assert_eq!(screen.icon().state, IconState::Low);
}

#[test]
fn a_device_that_is_not_the_primary_device_is_reminded_too() {
    let (mut tray, mut screen) = start(&pinned("neon75"), &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 80, NOW)),
    );
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 15, NOW),
        ),
    );

    assert_eq!(screen.icon().percent, Some(80), "图标上画的是钉死的键盘");
    assert_eq!(
        screen.notices,
        [Notice {
            title: "Dragonfly 3 Master+ 电量低".to_string(),
            body: "15%（Reported Level），低于低电量阈值 20%".to_string(),
        }]
    );
}

/// "提醒过"每台各记各的：为鼠标提醒过，不挡着键盘跌破时的那一条。
#[test]
fn each_device_is_reminded_on_its_own() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 15, NOW),
        ),
    );
    let then = later(NOW, 10);
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", then, just_read(EndpointKind::Ble, 10, then)),
    );

    let titles: Vec<&str> = screen.notices.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["Dragonfly 3 Master+ 电量低", "VGN Neon75 电量低"]);
}

/// 单台阈值覆盖生效：鼠标写了 `low_battery = 30`，25% 就提醒，通知里写的是它自己的阈值；键盘没写，
/// 跟着 `[general]`（缺省 20），同一个 25% 不提醒。
#[test]
fn a_per_device_threshold_overrides_the_general_one() {
    let (mut tray, mut screen) = start(
        MOUSE_AT_30_AND_KEYBOARD_AT_DEFAULT,
        &LastKnown::default(),
        NOW,
    );

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 25, NOW),
        ),
    );
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 25, NOW)),
    );

    assert_eq!(
        screen.notices,
        [Notice {
            title: "Dragonfly 3 Master+ 电量低".to_string(),
            body: "25%（Reported Level），低于低电量阈值 30%".to_string(),
        }]
    );
}

/// "提醒过"只记在内存里：程序重启算新的开始，仍然低电就再提醒一次。
///
/// 重启后状态文件里有那一份低于阈值的读数（取数线程读到它时当场就记下了），起手画的是它——那是上次
/// 已知值，不提醒；重启后第一次取数真读到仍然低电，才提醒。
#[test]
fn after_a_restart_a_device_still_below_the_threshold_is_reminded_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 15, NOW),
        ),
    );
    assert_eq!(screen.notices.len(), 1, "重启之前提醒过一次");

    let mut last_known = LastKnown::default();
    last_known.record(
        "dragonfly3",
        &reading(EndpointKind::Dongle24G, 15, NOW),
        NOW,
    );
    let restarted_at = later(NOW, 300);
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &last_known, restarted_at);
    assert_eq!(screen.notices, [], "起手画的是状态文件里的上次已知值");

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            restarted_at,
            just_read(EndpointKind::Dongle24G, 15, restarted_at),
        ),
    );

    assert_eq!(
        screen.notices,
        [Notice {
            title: "Dragonfly 3 Master+ 电量低".to_string(),
            body: "15%（Reported Level），低于低电量阈值 20%".to_string(),
        }]
    );
}

/// 电量 Unknown、取数失败，既不提醒、也不重新武装：Unknown 不等于 0%（`CONTEXT.md`「Unknown」），也不
/// 等于回升了；取数失败什么都没读到。所以一台时读得到、时读不到的设备，不会一次次地提醒。
#[test]
fn an_unknown_level_or_a_failed_fetch_neither_reminds_nor_rearms() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let unknown = |at| just_read(EndpointKind::Dongle24G, 0, at);

    feed(
        &mut tray,
        &mut screen,
        fetched("dragonfly3", NOW, unknown(NOW)),
    );
    assert_eq!(screen.notices, [], "电量 Unknown 不算低电");

    let steps = [
        (60, just_read(EndpointKind::Dongle24G, 15, later(NOW, 60))),
        (120, failed("读超时")),
        (180, unknown(later(NOW, 180))),
        (240, just_read(EndpointKind::Dongle24G, 14, later(NOW, 240))),
    ];
    for (secs, in_hand) in steps {
        feed(
            &mut tray,
            &mut screen,
            fetched("dragonfly3", later(NOW, secs), in_hand),
        );
    }

    let bodies: Vec<&str> = screen.notices.iter().map(|n| n.body.as_str()).collect();
    assert_eq!(bodies, ["15%（Reported Level），低于低电量阈值 20%"]);
}

/// 上位机在跑、2.4G 让开了，`Ble` 顶上来的那一份低于阈值：照样提醒。它是这一次取数真读到的；图标上
/// 画暂停，那是图标状态的优先顺序，不是这一份不算数。
#[test]
fn a_reading_taken_while_the_vendor_hub_is_running_still_reminds() {
    let (mut tray, mut screen) = start(KEYBOARD_ON_24G_AND_BLE, &LastKnown::default(), NOW);
    let InHand::Reading(row) = just_read(EndpointKind::Ble, 12, NOW) else {
        unreachable!()
    };
    let behind_the_hub = InHand::Reading(RowReading {
        paused_by: Some(vendor_hub_running()),
        ..row
    });

    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, behind_the_hub),
    );

    assert_eq!(screen.icon().state, IconState::Paused, "图标上是暂停");
    assert_eq!(
        screen.notices,
        [Notice {
            title: "VGN Neon75 电量低".to_string(),
            body: "12%（Reported Level），低于低电量阈值 20%".to_string(),
        }]
    );
}

/// 一份陈旧到不该再显示百分比的 `Ble` 缓存（十天前）低于阈值：照样提醒。它是这一次取数从系统里读到的，
/// 不是上次已知值；读不到新数的设备，最常见的原因就是没电了（parking lot Q200）。
#[test]
fn a_ble_reading_too_stale_to_show_its_percentage_still_reminds() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let ten_days_ago = NOW.minus_secs(10 * 86_400);

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "neon75",
            NOW,
            just_read(EndpointKind::Ble, 12, ten_days_ago),
        ),
    );

    assert_eq!(
        screen.notices,
        [Notice {
            title: "VGN Neon75 电量低".to_string(),
            body: "12%（Reported Level），低于低电量阈值 20%".to_string(),
        }]
    );
}

/// 同 [`MOUSE_AND_KEYBOARD`]，但钉死 `id` 那一台当 Primary Device。
fn pinned(id: &str) -> String {
    format!("[general]\nprimary = \"{id}\"\n{MOUSE_AND_KEYBOARD}")
}

/// 同 [`MOUSE_AND_KEYBOARD`]，但鼠标自己写了低电阈值 30。
const MOUSE_AT_30_AND_KEYBOARD_AT_DEFAULT: &str = r#"
[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"
low_battery = 30

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.bluetooth]
  address = "e452430072a9"
"#;

/// 这一次取数当场读到的一份，而且读到它正在充电：Dongle24G 上的电量 `level`，取得时刻 `at`。
fn charging(level: u8, at: Timestamp) -> InHand {
    let mut reading = reading(EndpointKind::Dongle24G, level, at);
    reading.reading.charging = Some(true);
    InHand::Reading(RowReading {
        reading,
        provenance: Provenance::JustRead,
        paused_by: None,
    })
}

/// 一台键盘，配了 Dongle24G 与 Ble 两条：上位机在跑时 2.4G 让开，`Ble` 顶上。
const KEYBOARD_ON_24G_AND_BLE: &str = r#"
[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0

  [device.bluetooth]
  address = "e452430072a9"
"#;
