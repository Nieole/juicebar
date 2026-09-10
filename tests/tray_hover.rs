//! 悬停提示的确切文字（`juicebar::tray::hover`）：从"这一轮的结果"排出来，写的是 Primary Device 的名字、
//! 电量、来源和多久前（spec 用户故事 16）。
//!
//! 被测的是一个纯函数：一轮 → 一段字。这一轮由 `Round::assess` 从罐装的手上那一份合成，不走内核的
//! 事件——"什么时候更新悬停提示"在 `tests/tray_round.rs`，这里只管"它写成什么样"。

mod common;

use common::tray::{MOUSE_AND_KEYBOARD, failed, just_read, last_known_value, reading};
use common::{NOW, vendor_hub_running};
use juicebar::clock::Timestamp;
use juicebar::config::Config;
use juicebar::endpoints::EndpointKind;
use juicebar::readout::{NoReading, RowReading};
use juicebar::round::{InHand, Round};
use juicebar::state::Provenance;
use juicebar::tray::hover;

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

/// 配置里一个 Device 都没有：那才是根上的原因，不再说选不出谁。
#[test]
fn with_no_device_registered_it_says_so() {
    assert_eq!(
        hover_with_nothing_in_hand("[general]\nprimary = \"retired\"\n"),
        "配置里一个 Device 都没有"
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
