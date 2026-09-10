//! 托盘内核里「轮询节奏」这一块（`juicebar::tray::cadence`）：时钟走到了 → 该对哪几台做一次取数。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），动作出；外壳由 `common::tray::Screen` 顶替，它记下
//! 每一次要求取数的是哪一台。时钟是一串字面量，一秒都不必真等。
//!
//! 每台 Device 各走各的节奏（spec「轮询节奏」）：按它上一次取数拿到读数的那条 Endpoint 的间隔。

mod common;

use common::NOW;
use common::tray::{
    MOUSE_AND_KEYBOARD, Screen, failed, feed, fell_back, fetched, just_read, last_known_value,
    later, start,
};
use juicebar::clock::Timestamp;
use juicebar::endpoints::EndpointKind;
use juicebar::round::InHand;
use juicebar::state::LastKnown;
use juicebar::tray::{Event, Tray};

/// 一只配了 Dongle24G 与 Ble 两条的鼠标：这一次取数从哪条读到，由用例罐装的结果说了算。
const MOUSE_ON_24G_AND_BLE: &str = r#"
[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

  [device.bluetooth]
  address = "e452430072a9"
"#;

/// 一只三条 Endpoint 都配了的鼠标（与 `config.example.toml` 里那一只一样），外加一台只配了 Ble 的键盘。
const EVERY_LINK_MOUSE_AND_KEYBOARD: &str = r#"
[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"

  [device.wired]
  vid = 0x391D
  pid = 0x1005
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

  [device.bluetooth]
  address = "e452430072a9"

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.bluetooth]
  address = "d5a1c2b3e4f6"
"#;

/// 托盘一启动，每台都立刻去取一次数，按配置里的书写顺序——不等第一个间隔过去：那之前图标上只有状态
/// 文件里的旧数，或者什么都没有。
#[test]
fn every_device_is_fetched_as_soon_as_the_tray_starts() {
    let (_tray, screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    assert_eq!(screen.fetches, ["dragonfly3", "neon75"]);
}

/// 每台按它上一次取数拿到读数的那条 Endpoint 的间隔再问：鼠标走 2.4G 就隔 `poll_interval_24g`（缺省
/// 60 秒），早一秒都不问——2.4G 取数走无线链路、会打扰设备（`docs/protocol.md` 第 0 节）；退到 Ble 就
/// 隔 `poll_interval_bluetooth`（缺省 10 秒）；回到 2.4G，又是 60 秒。间隔从那一次取数的"当下"起算。
#[test]
fn a_device_is_asked_again_at_the_interval_of_the_endpoint_its_previous_fetch_read_from() {
    let (mut tray, mut screen) = start(MOUSE_ON_24G_AND_BLE, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 62, NOW),
        ),
    );
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 59)));
    assert_eq!(screen.fetches.len(), 1, "走 2.4G：早一秒都不问");
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));
    assert_eq!(screen.fetches.len(), 2, "走 2.4G：60 秒");

    // 这一次 2.4G 没答话，Ble 顶上了。
    let on_ble = later(NOW, 60);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            on_ble,
            just_read(EndpointKind::Ble, 61, on_ble),
        ),
    );
    feed(&mut tray, &mut screen, Event::Tick(later(on_ble, 9)));
    assert_eq!(screen.fetches.len(), 2, "退到 Ble：早一秒都不问");
    feed(&mut tray, &mut screen, Event::Tick(later(on_ble, 10)));
    assert_eq!(screen.fetches.len(), 3, "退到 Ble：10 秒");

    // 2.4G 又答话了。
    let back_on_24g = later(on_ble, 10);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            back_on_24g,
            just_read(EndpointKind::Dongle24G, 61, back_on_24g),
        ),
    );
    feed(&mut tray, &mut screen, Event::Tick(later(back_on_24g, 59)));
    assert_eq!(screen.fetches.len(), 3, "回到 2.4G：早一秒都不问");
    feed(&mut tray, &mut screen, Event::Tick(later(back_on_24g, 60)));
    assert_eq!(screen.fetches.len(), 4, "回到 2.4G：又是 60 秒");
}

/// 还没读到过的（一次取数什么都没交出来，也没有上次已知值），按它配置里优先级最高的那条 Endpoint 的间隔：
/// 三条都配了的鼠标按 Wired 的 `poll_interval_wired`（缺省 30 秒），只配了 Ble 的键盘按
/// `poll_interval_bluetooth`（缺省 10 秒）。
#[test]
fn a_device_never_read_is_asked_again_at_the_interval_of_its_highest_priority_endpoint() {
    let (mut tray, mut screen) = start(EVERY_LINK_MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched("dragonfly3", NOW, failed("三条都没读到")),
    );
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, failed("Ble 不在场")),
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 9)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75"]);
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 10)));
    assert_eq!(
        screen.fetches,
        ["dragonfly3", "neon75", "neon75"],
        "只配了 Ble 的键盘：10 秒"
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 29)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75", "neon75"]);
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 30)));
    assert_eq!(
        screen.fetches,
        ["dragonfly3", "neon75", "neon75", "dragonfly3"],
        "三条都配了的鼠标：Wired 的 30 秒"
    );
}

/// 一次取数没读到、退到了上次已知值，按那份上次已知值来自的那条 Endpoint 走：那就是这台上一次拿到读数的
/// 那条，哪怕它是上次启动时读到、从状态文件里带过来的。鼠标上次是从 Ble 读到的，这一次两条都没答话：还是
/// 隔 Ble 的 10 秒再问，不当它还没读到过、去按优先级最高的 2.4G 的 60 秒。
#[test]
fn falling_back_to_the_last_known_value_keeps_the_interval_of_the_endpoint_it_came_from() {
    let (mut tray, mut screen) = start(MOUSE_ON_24G_AND_BLE, &LastKnown::default(), NOW);
    let an_hour_ago = NOW.minus_secs(3_600);
    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "dragonfly3",
            NOW,
            last_known_value(EndpointKind::Ble, 61, an_hour_ago),
            "两条都没答话",
        ),
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 9)));
    assert_eq!(screen.fetches, ["dragonfly3"]);
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 10)));
    assert_eq!(screen.fetches, ["dragonfly3", "dragonfly3"]);
}

/// 陈旧判定的"3 倍间隔"与节奏用的是同一个间隔：`poll_interval_wired` 调成 20 秒，从 Wired 读到的那一台就
/// 隔 20 秒再问，而它手上那一份也正好在 3 × 20 = 60 秒之后才算陈旧——不是一边按 20、一边按别的数。这里
/// 那一次再问没有回来，悬停提示上的"多久前"就一直往上走，越过 60 秒的那一刻标上"已陈旧"。
#[test]
fn staleness_goes_by_three_times_the_same_interval_the_cadence_asks_at() {
    let config = r#"
[general]
poll_interval_wired = 20

[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"

  [device.wired]
  vid = 0x391D
  pid = 0x1005
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8
"#;
    let (mut tray, mut screen) = start(config, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched("dragonfly3", NOW, just_read(EndpointKind::Wired, 62, NOW)),
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 19)));
    assert_eq!(screen.fetches, ["dragonfly3"]);
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 20)));
    assert_eq!(screen.fetches, ["dragonfly3", "dragonfly3"], "隔 20 秒再问");

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));
    assert!(
        screen.tooltip().ends_with("来自 Wired（60 秒前）"),
        "{}",
        screen.tooltip()
    );
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 61)));
    assert!(
        screen.tooltip().ends_with("来自 Wired（61 秒前），已陈旧"),
        "3 × 20 秒之后才陈旧：{}",
        screen.tooltip()
    );
}

/// 一条 Endpoint 都没配的那一台，没有"优先级最高的那条"可按：问它什么都碰不到，在配置变之前每一次都是同一句
/// 取数失败，所以按三个间隔里最长的那个问。这里把 `poll_interval_wired` 调成 90 秒，最长的就是它。
#[test]
fn a_device_with_no_endpoint_configured_is_asked_again_at_the_longest_interval() {
    let config = r#"
[general]
poll_interval_wired = 90

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"
"#;
    let (mut tray, mut screen) = start(config, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, failed("配置里一条 Endpoint 都没有")),
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 89)));
    assert_eq!(screen.fetches, ["neon75"]);
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 90)));
    assert_eq!(screen.fetches, ["neon75", "neon75"]);
}

/// 两台各走各的，交错着问：鼠标走 2.4G 六十秒一次，键盘走 Ble 十秒一次。头一分钟里键盘被问了五次，鼠标
/// 一次都没有，第 60 秒两台都到点，按配置里的书写顺序。
#[test]
fn two_devices_on_different_cadences_are_asked_interleaved() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    let answer = |device: &str, at| match device {
        "dragonfly3" => just_read(EndpointKind::Dongle24G, 62, at),
        _ => just_read(EndpointKind::Ble, 80, at),
    };
    for device in ["dragonfly3", "neon75"] {
        feed(
            &mut tray,
            &mut screen,
            fetched(device, NOW, answer(device, NOW)),
        );
    }

    let asked = run_clock(&mut tray, &mut screen, NOW, 60, answer);

    let asked: Vec<(u64, &str)> = asked.iter().map(|(sec, id)| (*sec, id.as_str())).collect();
    assert_eq!(
        asked,
        [
            (10, "neon75"),
            (20, "neon75"),
            (30, "neon75"),
            (40, "neon75"),
            (50, "neon75"),
            (60, "dragonfly3"),
            (60, "neon75"),
        ]
    );
}

/// 任一台有了新结果就开始新的一轮（`CONTEXT.md`「一轮」）：键盘十秒后的那一次读到 15%，图标当场换成画它，
/// 不等鼠标——而鼠标不因为这一轮被多问一次。
#[test]
fn a_new_result_from_one_device_starts_a_new_round_without_asking_the_other_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 62, NOW),
        ),
    );
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 80, NOW)),
    );
    assert_eq!(screen.icon().percent, Some(62));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 10)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75", "neon75"]);

    let ten_secs_later = later(NOW, 10);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "neon75",
            ten_secs_later,
            just_read(EndpointKind::Ble, 15, ten_secs_later),
        ),
    );

    assert_eq!(screen.icon().percent, Some(15), "新的一轮，画键盘");
    assert_eq!(
        screen.fetches,
        ["dragonfly3", "neon75", "neon75"],
        "鼠标不因为这一轮被多问一次"
    );
}

/// 一次取数还没回来的那一台，到点了也不再问它：取数是一台一台排着做的，再排一次只会让它在后面堆起来。
#[test]
fn a_device_whose_fetch_has_not_come_back_is_not_asked_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 600)));

    assert_eq!(screen.fetches, ["dragonfly3", "neon75"]);
}

/// 时钟从 `start` 起一秒一格走 `secs` 秒，每一次要求的取数都当场有结果（`answer` 按 Device 的 id 与那一刻
/// 交出这一次手上的那一份）。交回这一路上的每一次取数：第几秒、问的哪一台。
fn run_clock(
    tray: &mut Tray,
    screen: &mut Screen,
    start: Timestamp,
    secs: u64,
    answer: impl Fn(&str, Timestamp) -> InHand,
) -> Vec<(u64, String)> {
    let mut asked = Vec::new();
    for sec in 1..=secs {
        let now = later(start, sec);
        let before = screen.fetches.len();
        feed(tray, screen, Event::Tick(now));
        let asked_now: Vec<String> = screen.fetches.iter().skip(before).cloned().collect();
        for device in asked_now {
            feed(tray, screen, fetched(&device, now, answer(&device, now)));
            asked.push((sec, device));
        }
    }
    asked
}
