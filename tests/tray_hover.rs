//! 悬停提示的确切文字（`juicebar::tray::hover`）：从"这一轮的结果"排出来，写的是 Primary Device 的名字、
//! 电量、来源和多久前（spec 用户故事 16）。
//!
//! 被测的是一个纯函数：一轮 → 一段字。这一轮由 `Round::assess` 从罐装的手上那一份合成，不走内核的
//! 事件——"什么时候更新悬停提示"在 `tests/tray_round.rs`，这里只管"它写成什么样"。
//!
//! 末尾那一批从**真的一次取数**排出来：经假枚举与假 Transport，走托盘取数线程走的那一条
//! （`readout::read_or_last_known_with_reason`）。它们是命令行 `status` 那批排版用例迁过来的落点（resident-tray 票 14）：
//! 排版与取数对一份读数的理解只有一份，两处各自可能漂开的知识里只有一份是被断言过的。

mod common;

use common::fixtures::{
    KEYBOARD_REPORT_ID, KEYBOARD_RESTING_FULL, MOUSE_CHARGING, MOUSE_REPORT_ID, MOUSE_RESTING_FULL,
    mouse_frame_with,
};
use common::tray::{MOUSE_AND_KEYBOARD, failed, just_read, last_known_value, reading};
use common::{
    FakeEndpoints, NOW, default_general, keyboard_with_dongle_endpoint,
    mouse_with_all_three_endpoints, mouse_with_both_endpoints, scanned_ble, vendor_hub_running,
};
use juicebar::clock::Timestamp;
use juicebar::config::{Config, Device, General};
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::primary::PrimaryRule;
use juicebar::readout::{self, NoReading, RowReading};
use juicebar::round::{InHand, Round};
use juicebar::sources::Reading;
use juicebar::state::{LastKnown, Provenance};
use juicebar::tray::hover;
use juicebar::vendor_hub::VendorHub;

/// 把 `id` 那一台钉成 Primary Device、手上是 `in_hand`，这一轮的悬停提示。另一台什么都还没有。
///
/// 钉死是为了让那一台一定是 Primary Device——`lowest` 只让新鲜且可信的读数参与，而这里有一半用例
/// 要看的恰恰是陈旧与读不到的那几种。
fn hover_of(id: &str, in_hand: InHand) -> String {
    let config = Config::parse(&format!(
        "[general]\nprimary = \"{id}\"\n{MOUSE_AND_KEYBOARD}"
    ))
    .expect("用例里的配置应当解析得动");
    let nothing = InHand::NoKnownValue;
    let devices = config.devices.iter().map(|device| {
        let hand = if device.id == id { &in_hand } else { &nothing };
        (device, hand)
    });
    let round = Round::assess(&config.general, devices, None, Vec::new(), NOW);
    hover::text(&round)
}

/// 这一次取数当场读到它正在充电。
fn charging(endpoint: EndpointKind, level: u8, at: Timestamp) -> InHand {
    let mut reading = reading(endpoint, level, at);
    reading.reading.charging = Some(true);
    InHand::Reading(RowReading {
        reading,
        provenance: Provenance::JustRead,
        paused_by: None,
    })
}

/// 按这份配置、每台都还什么都没有，这一轮的悬停提示。
fn hover_with_nothing_in_hand(config: &str) -> String {
    let config = Config::parse(config).expect("用例里的配置应当解析得动");
    let nothing = InHand::NoKnownValue;
    let devices = config.devices.iter().map(|device| (device, &nothing));
    let round = Round::assess(&config.general, devices, None, Vec::new(), NOW);
    hover::text(&round)
}

/// 当场读到、新鲜：名字一行，电量一行（连同它是哪一个百分比），来源与多久前一行。
#[test]
fn a_fresh_reading_names_the_device_its_level_and_where_and_when_it_came_from() {
    assert_eq!(
        hover_of(
            "dragonfly3",
            just_read(EndpointKind::Dongle24G, 62, NOW.minus_secs(12))
        ),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Dongle24G（12 秒前）"
    );
}

/// 这一次取数读到它正在充电：电量那一行后面说一句。没在充电不说——那一格要紧的只有"充电中"。
#[test]
fn a_device_charging_right_now_says_so_after_its_level() {
    assert_eq!(
        hover_of(
            "dragonfly3",
            charging(EndpointKind::Dongle24G, 62, NOW.minus_secs(3))
        ),
        "Dragonfly 3 Master+\n62%（Reported Level），充电中\n来自 Dongle24G（3 秒前）"
    );
}

/// `Ble` 读的是 Windows 攒的缓存，不是当场问的：来源那一行说出来，多久前是那份缓存的年龄。
#[test]
fn a_ble_reading_says_it_came_from_the_windows_cache() {
    assert_eq!(
        hover_of(
            "neon75",
            just_read(EndpointKind::Ble, 80, NOW.minus_secs(180))
        ),
        "VGN Neon75\n80%（Reported Level）\n来自 Ble（Windows 缓存，3 分钟前）"
    );
}

/// 当场读到、而已经过了陈旧阈值（`Ble` 缺省一小时）：来源那一行后面说一句"已陈旧"。
#[test]
fn a_stale_reading_is_marked_stale() {
    assert_eq!(
        hover_of(
            "neon75",
            just_read(EndpointKind::Ble, 80, NOW.minus_secs(2 * 3_600))
        ),
        "VGN Neon75\n80%（Reported Level）\n来自 Ble（Windows 缓存，2 小时前），已陈旧"
    );
}

/// 这一次取数没读到、拿出来顶上的上次已知值：一律陈旧，而且说清它是上次已知值——设备此刻不在，得先把
/// 它找出来，等一等不会自己变新。
#[test]
fn a_last_known_value_says_it_is_one() {
    assert_eq!(
        hover_of(
            "dragonfly3",
            last_known_value(EndpointKind::Dongle24G, 62, NOW.minus_secs(300))
        ),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Dongle24G（5 分钟前），已陈旧，上次已知值"
    );
}

/// 陈旧到不该再显示数字的那一档（`Ble` 缺省一天）：电量那一行换成它是哪一天的，末尾交代百分比去哪了。
#[test]
fn a_reading_too_stale_for_its_percentage_gives_only_its_date() {
    assert_eq!(
        hover_of(
            "neon75",
            just_read(EndpointKind::Ble, 80, NOW.minus_secs(10 * 86_400))
        ),
        "VGN Neon75\n2026-03-15 的读数\n来自 Ble（Windows 缓存，10 天前），已陈旧，不显示百分比"
    );
}

/// 这一次取数有 Endpoint 为厂商上位机让开过、退到了上次已知值：末尾另起一行点那个上位机的名——用户能
/// 动手的地方只有它。
#[test]
fn a_reading_behind_a_yielding_endpoint_names_the_vendor_hub() {
    let InHand::Reading(row) = last_known_value(EndpointKind::Dongle24G, 62, NOW.minus_secs(300))
    else {
        unreachable!()
    };
    let paused = InHand::Reading(RowReading {
        paused_by: Some(vendor_hub_running()),
        ..row
    });

    assert_eq!(
        hover_of("dragonfly3", paused),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Dongle24G（5 分钟前），已陈旧，上次已知值\n已暂停（VGN VHUB.exe 正在运行）"
    );
}

/// 取数失败、又没有上次已知值：电量那一行换成取数那一层交出的那句原因。
#[test]
fn a_failed_fetch_gives_its_reason() {
    assert_eq!(
        hover_of("dragonfly3", failed("读不到 —— 用例里的原因")),
        "Dragonfly 3 Master+\n读不到 —— 用例里的原因"
    );
}

/// 暂停、又没有上次已知值：那句话说的是暂停，不是取数失败。
#[test]
fn a_paused_device_without_a_value_says_it_is_paused() {
    let paused = InHand::NoReading(NoReading::Paused {
        hub: vendor_hub_running(),
        yielded: vec![EndpointKind::Dongle24G],
        failures: Vec::new(),
    });

    assert_eq!(
        hover_of("dragonfly3", paused),
        "Dragonfly 3 Master+\n暂停中 —— VGN VHUB.exe 正在运行，Dongle24G 让开（同时发命令会互相覆盖对方的应答），关掉它之后这几条才试得到"
    );
}

/// 一次取数都还没有结果、状态文件里也没有它：无已知值，照直说。
#[test]
fn a_device_with_nothing_read_yet_says_so() {
    assert_eq!(
        hover_of("dragonfly3", InHand::NoKnownValue),
        "Dragonfly 3 Master+\n无已知值 —— 还没读到过，也没有上次已知值"
    );
}

/// 电量字段采信不了：Unknown 那句话由产生它的规则说（`sources::level`），来源照写。
#[test]
fn an_unknown_level_says_why_it_cannot_be_trusted() {
    assert_eq!(
        hover_of(
            "neon75",
            just_read(EndpointKind::Ble, 0, NOW.minus_secs(180))
        ),
        "VGN Neon75\n电量 Unknown（固件报的是 0，采信不了）\n来自 Ble（Windows 缓存，3 分钟前）"
    );
}

/// 这一轮选不出 Primary Device：图标上没有哪一台，悬停提示说为什么。
#[test]
fn with_no_primary_device_it_says_why() {
    assert_eq!(
        hover_with_nothing_in_hand(MOUSE_AND_KEYBOARD),
        "选不出 Primary Device：没有一个新鲜且可信的读数，也没有上次的选择"
    );
}

/// 配置钉死的那个 id 不在登记的 Device 里：一次笔误，得说出来——用户以为钉住了。
#[test]
fn a_pinned_id_that_is_not_registered_says_so() {
    assert_eq!(
        hover_with_nothing_in_hand(&format!(
            "[general]\nprimary = \"retired\"\n{MOUSE_AND_KEYBOARD}"
        )),
        "配置里 primary 钉的 id \"retired\" 不在登记的 Device 里"
    );
}

/// 配置里一个 Device 都没有：那才是根上的原因，不再说选不出谁——接着指路，插上之后到"登记设备"里新建。用户先看到的
/// 往往是悬停提示，与菜单那一行说同一句（parking lot Q333 那一问，resident-tray 票 14 定）。
#[test]
fn with_no_device_registered_it_says_so_and_where_to_add_one() {
    assert_eq!(
        hover_with_nothing_in_hand("[general]\nprimary = \"retired\"\n"),
        "配置里一个 Device 都没有：插上设备或配对蓝牙之后，到\"登记设备\"里点\"新建一台 Device\""
    );
}

/// 托盘的悬停提示最多 127 个 UTF-16 单元（`NOTIFYICONDATAW::szTip` 128 格，末尾一格给 NUL）：放不下就截在
/// 那里，末尾一个省略号说明后面还有——完整原因在日志里。
#[test]
fn a_tooltip_too_long_for_the_tray_is_cut_with_an_ellipsis() {
    let text = hover_of("dragonfly3", failed(&"很长".repeat(100)));

    assert_eq!(text, format!("Dragonfly 3 Master+\n{}…", "很长".repeat(53)));
    assert_eq!(text.encode_utf16().count(), hover::MAX_UTF16);
}

// ---------------------------------------------------------------
// 从真的一次取数排出来（命令行 `status` 那批排版用例迁到这里，resident-tray 票 14）
//
// 取数本身（先试哪一条、怎么降级、什么时候算失联、暂停时让开哪几条）在 `tests/readout.rs` 断言完了；这里断言的是
// 那一次取数交出来的东西，悬停提示写成什么样。
// ---------------------------------------------------------------

/// `device` 是配置里唯一的一台、钉成 Primary Device，手上是 `in_hand`：这一轮的悬停提示。
///
/// 钉死的理由同 [`hover_of`]；另起一个，是因为下面这批用例的 Device 各配着不同的 Endpoint，不在 [`MOUSE_AND_KEYBOARD`] 里。
fn hover_of_device(device: &Device, in_hand: InHand) -> String {
    let general = General {
        primary: PrimaryRule::Pinned(device.id.clone()),
        ..default_general()
    };
    let round = Round::assess(&general, [(device, &in_hand)], None, Vec::new(), NOW);
    hover::text(&round)
}

/// 在 `at` 那一刻真去取一次数，交给这一轮的就是它：托盘取数线程走的那一条，读不到就退到 `last_known` 里的上次已知值。
fn fetch_at(
    device: &Device,
    endpoints: &FakeEndpoints,
    paused_by: Option<&VendorHub>,
    last_known: &mut LastKnown,
    at: Timestamp,
) -> InHand {
    InHand::from(readout::read_or_last_known_with_reason(
        device,
        endpoints,
        paused_by,
        last_known,
        &default_general(),
        at,
    ))
}

/// 在 [`NOW`] 真去取一次数：不暂停，状态文件里什么都没有。
fn fetch(device: &Device, endpoints: &FakeEndpoints) -> InHand {
    fetch_at(device, endpoints, None, &mut LastKnown::default(), NOW)
}

/// 只有 `endpoint` 这一条在场、回包是 `frame` 的假枚举。
fn only(endpoint: EndpointKind, frame: &[u8]) -> FakeEndpoints {
    FakeEndpoints::new(MOUSE_REPORT_ID, [(endpoint, vec![frame.to_vec()])])
}

/// 一条 HID Endpoint 都不在场，只有 Windows 缓存里的这台蓝牙设备：电量 `level`，Windows 说 `age_secs` 秒前更新过。
fn only_the_ble_cache(level: u8, age_secs: Option<u64>) -> FakeEndpoints {
    FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(level),
        age_secs,
    ))
}

/// 来源那一行说得出这份读数是从哪条 Endpoint 上来的：同一个 Device 的几条 Endpoint 在不同时刻各自可用，不写出来就看不出
/// 是插着线读的还是走 2.4G 读的——而"插着线"恰恰是最容易被误报成离线的时刻。插着线读到的正是充电中。
#[test]
fn a_reading_says_which_endpoint_it_came_from() {
    let device = mouse_with_both_endpoints();

    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only(EndpointKind::Wired, &MOUSE_CHARGING))
        ),
        "Dragonfly 3 Master+\n95%（Reported Level），充电中\n来自 Wired（0 秒前）"
    );
    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only(EndpointKind::Dongle24G, &MOUSE_RESTING_FULL))
        ),
        "Dragonfly 3 Master+\n100%（Reported Level）\n来自 Dongle24G（0 秒前）"
    );
}

/// `Ble` 与两条 HID 分得清：它不是当场问出来的，是 Windows 攒的缓存，"多久前"才是它的真实可信度（`battery-readout` spec
/// 用户故事 6）。Windows 的电量属性里没有充电这一项，说不上来就不写；当场往返的两条 HID 不说缓存。
#[test]
fn a_ble_reading_is_told_apart_from_the_two_hid_endpoints() {
    let device = mouse_with_all_three_endpoints();

    assert_eq!(
        hover_of_device(&device, fetch(&device, &only_the_ble_cache(62, Some(300)))),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，5 分钟前）"
    );
    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only(EndpointKind::Wired, &MOUSE_CHARGING))
        ),
        "Dragonfly 3 Master+\n95%（Reported Level），充电中\n来自 Wired（0 秒前）"
    );
}

/// 陈旧的缓存明确标出来，与新鲜的分得清：一个不带标注的 62% 和一个刚问出来的 62% 长得一模一样，而它们的可信度差着一个
/// 数量级。五分钟前的缓存还算现状（缺省 `stale_after` 一小时），两小时前的数字照写、标"已陈旧"。
#[test]
fn a_stale_ble_cache_is_marked_apart_from_a_fresh_one() {
    let device = mouse_with_all_three_endpoints();

    assert_eq!(
        hover_of_device(&device, fetch(&device, &only_the_ble_cache(62, Some(300)))),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，5 分钟前）"
    );
    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only_the_ble_cache(62, Some(7_200)))
        ),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，2 小时前），已陈旧"
    );
}

/// 上次已知值一律标成陈旧，哪怕它是十秒前取的：同一份读数当场读到时是新鲜的（Wired 缺省阈值 90 秒），设备不在了拿出来就
/// 不是现状。不标就正好在最危险的方向上说谎——十秒前的 95% 说的可能是一只已经收进抽屉的鼠标。它记着的"充电中"也不写。
#[test]
fn a_last_known_value_is_marked_stale_even_when_it_was_taken_seconds_ago() {
    let device = mouse_with_both_endpoints();
    let reading = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 95,
            charging: Some(true),
            voltage_mv: Some(4235),
        },
        NOW.minus_secs(10),
    );
    let taken = |provenance| {
        InHand::Reading(RowReading {
            reading,
            provenance,
            paused_by: None,
        })
    };

    assert_eq!(
        hover_of_device(&device, taken(Provenance::JustRead)),
        "Dragonfly 3 Master+\n95%（Reported Level），充电中\n来自 Wired（10 秒前）"
    );
    assert_eq!(
        hover_of_device(&device, taken(Provenance::LastKnown)),
        "Dragonfly 3 Master+\n95%（Reported Level）\n来自 Wired（10 秒前），已陈旧，上次已知值"
    );
}

/// 半小时前插着线真读到过一次（level 95 / 充电中 / 4235 mV），记进状态文件。
///
/// 下面两条都不手搓这一份：带着充电态的上次已知值只可能这么来（`Ble` 那一格恒为空，两条 HID 读的都是当场）。
fn charging_recorded_half_an_hour_ago(device: &Device) -> LastKnown {
    let mut last_known = LastKnown::default();
    let plugged_in = only(EndpointKind::Wired, &MOUSE_CHARGING);
    let live = fetch_at(
        device,
        &plugged_in,
        None,
        &mut last_known,
        NOW.minus_secs(1_800),
    );
    assert!(
        matches!(&live, InHand::Reading(row) if row.provenance == Provenance::JustRead),
        "插着线读得到"
    );
    last_known
}

/// 这一次取数时设备收进了抽屉，退到了上次已知值：电量照写，行尾照标"已陈旧，上次已知值"，但不说它此刻在充电——充电态
/// 是一个现在时的断言，它恰恰在收进抽屉、或者刚插上线的那一刻变掉。
#[test]
fn does_not_claim_a_device_is_charging_when_the_number_is_a_last_known_value() {
    let device = mouse_with_both_endpoints();
    let mut last_known = charging_recorded_half_an_hour_ago(&device);

    let put_away = FakeEndpoints::new(MOUSE_REPORT_ID, []);
    let in_hand = fetch_at(&device, &put_away, None, &mut last_known, NOW);

    assert_eq!(
        hover_of_device(&device, in_hand),
        "Dragonfly 3 Master+\n95%（Reported Level）\n来自 Wired（30 分钟前），已陈旧，上次已知值"
    );
}

/// 一台没有更新时间戳的蓝牙设备不该看着像刚问出来的：说不出这个数多旧的时候，最像真的那个说法恰恰是"刚取的"。
#[test]
fn a_ble_reading_without_a_timestamp_does_not_look_freshly_taken() {
    let device = mouse_with_all_three_endpoints();

    assert_eq!(
        hover_of_device(&device, fetch(&device, &only_the_ble_cache(62, None))),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，无时间戳），已陈旧"
    );
}

/// 刚好卡在阈值上的读数还不算陈旧，多一秒就算："超过才算"是配置注释的原话（"超过标灰"）。
#[test]
fn a_reading_exactly_at_the_stale_threshold_is_not_marked_stale() {
    let device = mouse_with_all_three_endpoints();

    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only_the_ble_cache(62, Some(3_600)))
        ),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，60 分钟前）"
    );
    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only_the_ble_cache(62, Some(3_601)))
        ),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，60 分钟前），已陈旧"
    );
}

/// 电量那一行说清这个百分比是两个来源里的哪一个：两个百分比可能不一致，混用过一次就导致过一个错误结论。4235 mV 越过查表的
/// 表顶，用固件自报的 95；3900 mV 落在查表区间，用查表算出的 53，不用固件报的 20。
#[test]
fn a_reading_says_which_of_the_two_percentages_it_is() {
    let device = mouse_with_both_endpoints();

    assert_eq!(
        hover_of_device(
            &device,
            fetch(&device, &only(EndpointKind::Wired, &MOUSE_CHARGING))
        ),
        "Dragonfly 3 Master+\n95%（Reported Level），充电中\n来自 Wired（0 秒前）"
    );
    assert_eq!(
        hover_of_device(
            &device,
            fetch(
                &device,
                &only(EndpointKind::Wired, &mouse_frame_with(20, 3900))
            )
        ),
        "Dragonfly 3 Master+\n53%（Derived Level）\n来自 Wired（0 秒前）"
    );
}

/// 键盘电量读到 0 时说 Unknown，绝不说 100%，也不说 0%：键盘的回包里没有电压，缺省的 auto 只剩固件自报值，而那个值是 0。
/// 上位机在这里显示 100%，偏偏发生在最该提醒充电的时刻。
#[test]
fn a_keyboard_that_reports_zero_is_unknown_not_full() {
    let keyboard = keyboard_with_dongle_endpoint();
    let mut empty = KEYBOARD_RESTING_FULL;
    empty[1] = 0;
    let endpoints = FakeEndpoints::new(
        KEYBOARD_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![empty.to_vec()])],
    );

    assert_eq!(
        hover_of_device(&keyboard, fetch(&keyboard, &endpoints)),
        "VGN Neon75\n电量 Unknown（固件报的是 0，采信不了）\n来自 Dongle24G（0 秒前）"
    );
}

/// 暂停期间保留上次已知值而不是清空，并且点名是哪个上位机——用户能动手的地方只有它。3950 mV 在 auto 下取的是 Derived
/// Level（63%），不是固件自报的 44。让开的两条 HID 一条都没被打开。
#[test]
fn keeps_the_last_known_value_while_paused_and_says_which_it_is() {
    let device = mouse_with_both_endpoints();
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );
    let taken_at = NOW.minus_secs(1_800);
    let previous = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 44,
            charging: Some(true),
            voltage_mv: Some(3_950),
        },
        taken_at,
    );
    let mut last_known = LastKnown::default();
    last_known.record(&device.id, &previous, taken_at);

    let in_hand = fetch_at(
        &device,
        &endpoints,
        Some(&vendor_hub_running()),
        &mut last_known,
        NOW,
    );

    assert_eq!(
        hover_of_device(&device, in_hand),
        "Dragonfly 3 Master+\n63%（Derived Level）\n来自 Wired（30 分钟前），已陈旧，上次已知值\n已暂停（VGN VHUB.exe 正在运行）"
    );
    assert!(endpoints.opens().is_empty(), "{:?}", endpoints.opens());
}

/// 暂停里退到上次已知值的那一台同样不写"充电中"：这一次取数没问过它在不在充电，而暂停时设备就在手边，用户可能一分钟前刚
/// 给它插上线。暂停那一行一个字不少。
#[test]
fn does_not_claim_a_paused_device_is_charging_either() {
    let device = mouse_with_both_endpoints();
    let mut last_known = charging_recorded_half_an_hour_ago(&device);
    let yielded = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_CHARGING.to_vec()]),
        ],
    );

    let in_hand = fetch_at(
        &device,
        &yielded,
        Some(&vendor_hub_running()),
        &mut last_known,
        NOW,
    );

    assert_eq!(
        hover_of_device(&device, in_hand),
        "Dragonfly 3 Master+\n95%（Reported Level）\n来自 Wired（30 分钟前），已陈旧，上次已知值\n已暂停（VGN VHUB.exe 正在运行）"
    );
}

/// `Ble` 顶上来的那一次照样说一句"已暂停"：没有任何东西读不到，可一个可能是几个月前的缓存顶掉了当场问出来的那个数，
/// 用户得知道那是因为上位机在跑。
#[test]
fn says_it_paused_even_when_the_ble_cache_answered() {
    let device = mouse_with_all_three_endpoints();
    let endpoints = only(EndpointKind::Wired, &MOUSE_RESTING_FULL).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));

    let in_hand = fetch_at(
        &device,
        &endpoints,
        Some(&vendor_hub_running()),
        &mut LastKnown::default(),
        NOW,
    );

    assert_eq!(
        hover_of_device(&device, in_hand),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Ble（Windows 缓存，5 分钟前）\n已暂停（VGN VHUB.exe 正在运行）"
    );
}

/// 只配了 `Ble` 的设备不因为上位机在跑就说"已暂停"：一副耳机压根没有让开的东西，对它说这一句是一句与它无关的话。
#[test]
fn does_not_mark_a_ble_only_device_as_paused() {
    let headset = Config::parse(
        r#"
        [[device]]
        id = "headset"
        name = "某副耳机"

          [device.bluetooth]
          address = "f4ee2553b27e"
        "#,
    )
    .expect("用例里的配置应当解析得动")
    .devices
    .remove(0);
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "某副耳机",
        "f4ee2553b27e",
        Some(80),
        Some(60),
    ));

    let in_hand = fetch_at(
        &headset,
        &endpoints,
        Some(&vendor_hub_running()),
        &mut LastKnown::default(),
        NOW,
    );

    assert_eq!(
        hover_of_device(&headset, in_hand),
        "某副耳机\n80%（Reported Level）\n来自 Ble（Windows 缓存，60 秒前）"
    );
}

/// 读不出来的那一台，名字下面那一行是取数那一层交出的真句子：一句标记，紧跟每条 Endpoint 自己的原因。"读不到"是一条都不在场
/// 那一支的话，试过了都失败的这一支不该出现它。措辞再动一次，这一条就响——手写的替身不会。
#[test]
fn composes_a_lost_device_from_the_real_failure_sentence() {
    let keyboard = keyboard_with_dongle_endpoint();
    // 在场却答不出：回包脚本空着，相当于超时。
    let endpoints = FakeEndpoints::new(KEYBOARD_REPORT_ID, [(EndpointKind::Dongle24G, vec![])]);

    let text = hover_of_device(&keyboard, fetch(&keyboard, &endpoints));

    assert!(
        text.starts_with("VGN Neon75\n没有可信的读数 —— Dongle24G: "),
        "{text}"
    );
    assert_eq!(text.matches("没有可信的读数").count(), 1, "{text}");
    assert!(!text.contains("读不到"), "{text}");
}
