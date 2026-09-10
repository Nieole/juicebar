//! 托盘内核里「轮询节奏」这一块（`juicebar::tray::cadence`）：时钟走到了 → 该对哪几台做一次取数。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），动作出；外壳由 `common::tray::Screen` 顶替，它记下
//! 每一次要求取数的是哪一台。时钟是一串字面量，一秒都不必真等。
//!
//! 本票是**统一节奏**：每台都按 `poll_interval_24g`。各走各的节奏（按上一次拿到读数的那条 Endpoint）归
//! 票 05，那时这个文件里"统一"的那几条跟着换。

mod common;

use common::NOW;
use common::tray::{MOUSE_AND_KEYBOARD, feed, fetched, just_read, later, start};
use juicebar::endpoints::EndpointKind;
use juicebar::state::LastKnown;
use juicebar::tray::Event;

/// 托盘一启动，每台都立刻去取一次数，按配置里的书写顺序——不等第一个间隔过去：那之前图标上只有状态
/// 文件里的旧数，或者什么都没有。
#[test]
fn every_device_is_fetched_as_soon_as_the_tray_starts() {
    let (_tray, screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    assert_eq!(screen.fetches, ["dragonfly3", "neon75"]);
}

/// 一台的一次取数有了结果之后，隔 `poll_interval_24g`（缺省 60 秒）再问它一次，早一秒都不问：2.4G 取数走
/// 无线链路、会打扰设备（`docs/protocol.md` 第 0 节）。间隔从那一次取数的"当下"起算。
#[test]
fn a_device_is_fetched_again_poll_interval_24g_after_its_previous_fetch() {
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

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 59)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75"], "早一秒都不问");

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75", "dragonfly3"]);
}

/// 本票每台都按 `poll_interval_24g`：只配了 Ble 的键盘也一样，不按 `poll_interval_bluetooth`（缺省 10 秒）。
/// 各走各的节奏归票 05。
#[test]
fn in_this_ticket_a_ble_only_device_keeps_the_24g_cadence_too() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 80, NOW)),
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 10)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75"]);

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));
    assert_eq!(screen.fetches, ["dragonfly3", "neon75", "neon75"]);
}

/// 一次取数还没回来的那一台，到点了也不再问它：取数是一台一台排着做的，再排一次只会让它在后面堆起来。
#[test]
fn a_device_whose_fetch_has_not_come_back_is_not_asked_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 600)));

    assert_eq!(screen.fetches, ["dragonfly3", "neon75"]);
}
