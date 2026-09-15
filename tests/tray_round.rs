//! 托盘内核里「一轮」这一块（`juicebar::tray::round`）：某台 Device 的一次取数有了结果 → 新的一轮
//! （`CONTEXT.md`「一轮」）→ 重画图标、更新悬停提示、存状态文件、写日志。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），动作出；外壳由 `common::tray::Screen` 顶替。一次
//! 取数的结果是罐装的（`InHand` 直接造，一次取数本身怎么走在 `tests/readout.rs`），时钟是一串字面量。

mod common;

use common::NOW;
use common::tray::{
    LOOK, MOUSE_AND_KEYBOARD, failed, feed, fell_back, fetched, just_read, last_known_value, later,
    reading, start, start_with_look,
};
use juicebar::endpoints::EndpointKind;
use juicebar::icon::{IconSettings, IconSize, IconState, Theme};
use juicebar::state::LastKnown;
use juicebar::tray::menu_theme::MenuTheming;
use juicebar::tray::round::{IconRequest, SaveState};
use juicebar::tray::{Event, Look};

/// 任一台的一次取数有了结果就是新的一轮：图标画这一轮的 Primary Device（缺省 `lowest`，此刻手上只有
/// 鼠标这一份新鲜读数，就是它）。
#[test]
fn a_fetch_result_starts_a_new_round_and_the_icon_draws_the_primary_devices_state() {
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

    assert_eq!(screen.icon().state, IconState::Normal);
    assert_eq!(screen.icon().percent, Some(62));
}

/// 一次取数都还没回来的时候，图标就已经在了：状态文件里什么都没有，画无已知值——调色与尺寸按启动
/// 时任务栏的深浅色与显示缩放（运行中变了见下一条）。配置里没写 `[tray]`，样式是缺省的那一套。
#[test]
fn at_startup_with_nothing_on_record_the_icon_shows_no_known_value_in_the_taskbars_theme_and_size()
{
    let look = Look {
        theme: Theme::Light,
        size: IconSize::Px24,
        menu_theming: MenuTheming::FollowsTaskbar,
    };

    let (_tray, screen) = start_with_look(MOUSE_AND_KEYBOARD, &LastKnown::default(), look, NOW);

    assert_eq!(
        screen.icon(),
        IconRequest {
            settings: IconSettings::default(),
            state: IconState::NoKnownValue,
            percent: None,
            size: IconSize::Px24,
            theme: Theme::Light,
        }
    );
}

/// 运行中任务栏切到浅色、显示缩放改成 150%（外壳收到 Windows 的消息，把此刻的样子喂进来）：图标**当场**照新的调色与
/// 尺寸重画，画的还是那一台、那个数、那个样式；这不是新的一轮，没有东西要存。
#[test]
fn a_changed_taskbar_theme_or_display_scale_redraws_the_icon_at_once() {
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
    let before = screen.icon();
    let saves = screen.saves.len();

    feed(
        &mut tray,
        &mut screen,
        Event::LookChanged(Look {
            theme: Theme::Light,
            size: IconSize::Px24,
            ..LOOK
        }),
    );

    assert_eq!(
        screen.icon(),
        IconRequest {
            theme: Theme::Light,
            size: IconSize::Px24,
            ..before
        }
    );
    assert_eq!(screen.saves.len(), saves, "不是新的一轮，没有东西要存");
}

/// Windows 为别的设置广播了一次、任务栏的样子其实没变：什么都不做。
#[test]
fn a_look_that_did_not_change_redraws_nothing() {
    let (mut tray, _screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    let actions = tray.handle(Event::LookChanged(LOOK));

    assert!(actions.is_empty(), "{actions:?}");
}

/// 状态文件里有上次已知值、也记着上一轮的 Primary Device：一次取数都还没回来，图标就画那台的上次已知值
/// —— Stale（上次已知值一律如此），数字照画。那不是无已知值：`CONTEXT.md` 的无已知值是"也没有上次
/// 已知值"。
///
/// 两台的上次已知值都才半分钟，另一台的还更低：上次已知值不参与 `lowest`（parking lot Q154），哪怕它还在陈旧阈值以内；
/// 于是这一轮保持上次的选择，而"上次"只能来自状态文件。
#[test]
fn at_startup_the_icon_shows_the_last_known_value_of_the_primary_device_on_record() {
    let taken_at = NOW.minus_secs(30);
    let mut last_known = LastKnown::default();
    last_known.record(
        "dragonfly3",
        &reading(EndpointKind::Dongle24G, 62, taken_at),
        taken_at,
    );
    last_known.record(
        "neon75",
        &reading(EndpointKind::Ble, 40, taken_at),
        taken_at,
    );
    last_known.remember_primary("dragonfly3");

    let (_tray, screen) = start(MOUSE_AND_KEYBOARD, &last_known, NOW);

    assert_eq!(screen.icon().state, IconState::Stale);
    assert_eq!(screen.icon().percent, Some(62));
}

/// 这一轮选出了 Primary Device，就记进状态文件——下次启动时"保持上次的选择"靠的就是这一格
/// （`CONTEXT.md`「保持上次的选择」）。读到的那一份也跟着这一次写盘落下去。
#[test]
fn a_round_that_selects_a_primary_device_remembers_it_in_the_state_file() {
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

    assert_eq!(
        screen.saves,
        [SaveState {
            remember_primary: Some("dragonfly3".to_string())
        }]
    );
}

/// 这一轮选不出 Primary Device，状态文件里记着"上一轮是谁"的那一格就不动：选不出来的那一轮正是最
/// 需要上次那个值的时候。读到的那一份照样落盘——它是一份新读数，哪怕电量采信不了。
///
/// 状态文件里记着的"上一轮"是一台已经从配置里删掉的设备，保持不了它；键盘这一份是 Unknown，参与不了
/// `lowest`。于是这一轮选不出来。
#[test]
fn a_round_that_selects_no_primary_device_leaves_that_cell_of_the_state_file_alone() {
    let mut last_known = LastKnown::default();
    last_known.remember_primary("retired");
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &last_known, NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 0, NOW)),
    );

    assert_eq!(
        screen.saves,
        [SaveState {
            remember_primary: None
        }]
    );
}

/// 一次取数失败了、又没有选出谁：没有一样新东西要记，状态文件整个不碰。
#[test]
fn a_failed_fetch_that_selects_no_primary_device_writes_nothing() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched("neon75", NOW, failed("读不到 —— 用例里的原因")),
    );

    assert_eq!(screen.saves, []);
}

/// 选出过一台之后，这一轮什么可信的都没有，就保持上次的选择：图标照旧画那一台的上次已知值。
///
/// 真实的形状（parking lot Q166）：两台都刚失联，各自退到半分钟前的上次已知值，另一台的还更低。上次已知值不参与 `lowest`
/// （Q154），所以抢不走；要是"选出了谁"没被记下来，这一轮就选不出来，图标会掉成无已知值。
#[test]
fn once_selected_the_primary_device_is_held_over_when_nothing_can_be_trusted() {
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

    let lost = later(NOW, 30);
    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "dragonfly3",
            lost,
            last_known_value(EndpointKind::Dongle24G, 62, NOW),
            "读不到 —— 用例里的原因",
        ),
    );
    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "neon75",
            lost,
            last_known_value(EndpointKind::Ble, 40, NOW),
            "读不到 —— 用例里的原因",
        ),
    );

    assert_eq!(screen.icon().state, IconState::Stale);
    assert_eq!(
        screen.icon().percent,
        Some(62),
        "上次选的那一台，不是更低的那一台"
    );
}

/// 每一次取数失败都把完整原因写进日志：图标上只有一个"!"，悬停提示也放不下整句，而查"为什么读不到"
/// 的人要的是整条错误链（`docs/gaps.md`：权限不足的症状和硬件故障分不开，那时唯一的线索就在这里）。
#[test]
fn a_failed_fetch_logs_its_full_reason() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            failed("没有可信的读数 —— Dongle24G: 读超时（3000ms 内没有回包）"),
        ),
    );

    assert_eq!(
        screen.logs,
        ["Dragonfly 3 Master+ 取数失败：没有可信的读数 —— Dongle24G: 读超时（3000ms 内没有回包）"]
    );
}

/// 没读到、退到了上次已知值的那一次取数，一样把没读到的原因写进日志：图标上它只是一个灰的旧数，而这
/// 正是最常见的那一种失败——设备收进了抽屉、接收器拔了，或者没有管理员权限。
#[test]
fn a_fetch_that_falls_back_to_the_last_known_value_still_logs_why_it_did_not_read() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        fell_back(
            "dragonfly3",
            NOW,
            last_known_value(EndpointKind::Dongle24G, 62, NOW.minus_secs(600)),
            "没有可信的读数 —— Dongle24G: 读超时（3000ms 内没有回包）",
        ),
    );

    assert_eq!(
        screen.logs,
        [
            "Dragonfly 3 Master+ 取数失败，退到上次已知值：没有可信的读数 —— Dongle24G: 读超时（3000ms 内没有回包）"
        ]
    );
}

/// 陈旧到只该说日期的那一档（只有 `Ble` 到得了：Windows 那份缓存超过了 `very_stale_after`）：图标
/// 照样按 Stale 画，但不画数字——十天前的一个 62 画在托盘上，就是一句假话（悬停提示与菜单那一行在这一档
/// 同样只写日期）。键盘钉成了 Primary Device，好让这一份一定画在图标上。
#[test]
fn a_reading_too_stale_to_show_its_percentage_draws_no_digits() {
    let config = format!("[general]\nprimary = \"neon75\"\n{MOUSE_AND_KEYBOARD}");
    let (mut tray, mut screen) = start(&config, &LastKnown::default(), NOW);

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
        (screen.icon().state, screen.icon().percent),
        (IconState::Stale, None)
    );
}

/// 每一轮都把悬停提示换成这一轮的（写成什么样见 `tests/tray_hover.rs`）。两轮之间时钟每走一格，"多久前"
/// 跟着走：悬停提示是一段死字，不跟着走，它就一直说"0 秒前"。
#[test]
fn the_tooltip_shows_each_round_and_keeps_its_age_current_between_rounds() {
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
    assert_eq!(
        screen.tooltip(),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Dongle24G（0 秒前）"
    );

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 30)));

    assert_eq!(
        screen.tooltip(),
        "Dragonfly 3 Master+\n62%（Reported Level）\n来自 Dongle24G（30 秒前）"
    );
}
