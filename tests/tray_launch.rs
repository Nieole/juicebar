//! 托盘内核里「启动」这一块（`juicebar::tray::launch`）：再启动一次时来敲门的第二个实例，与菜单上的"开机自启"。
//!
//! 全程只经内核的公开面。外壳由 `common::tray::Screen` 顶替：互斥体、找到老实例那扇窗口、计划任务建没建，都是外壳的事；
//! 内核收到的是"有人来敲门了"，交出的是一条通知。

mod common;

use common::NOW;
use common::tray::{MOUSE_AND_KEYBOARD, Screen, feed, start};
use juicebar::state::LastKnown;
use juicebar::tray::menu::{Command, Entry, Item, Kind, Menu};
use juicebar::tray::notify::Notice;
use juicebar::tray::{Event, Tray, launch};

/// 一级菜单里"开机自启"那一项。
fn autostart(menu: &Menu) -> &Entry {
    menu.items
        .iter()
        .find_map(|item| match item {
            Item::Entry(entry) if matches!(entry.kind, Kind::Check) && entry.text == "开机自启" => {
                Some(entry)
            }
            Item::Entry(_) | Item::Separator => None,
        })
        .expect("一级菜单里有「开机自启」这一项")
}

/// 外壳弹出菜单之前问过系统：那个计划任务在（`true`）还是不在（`false`）。
fn asked(found: Result<bool, String>) -> Event {
    Event::Launch(launch::Event::AutostartAsked(found))
}

/// 勾不勾只看系统此刻怎么答：外壳每次弹出菜单之前问一次那个计划任务在不在，答在就勾上，下一次答不在就不勾。
#[test]
fn the_autostart_check_follows_what_the_system_answers_each_time() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, asked(Ok(true)));
    let when_found = autostart(&tray.menu(NOW)).checked;
    feed(&mut tray, &mut screen, asked(Ok(false)));
    let when_gone = autostart(&tray.menu(NOW)).checked;

    assert!(when_found, "任务在就勾上");
    assert!(!when_gone, "任务不在就不勾");
}

/// 问不出来（任务计划程序服务没在跑）：照"不在"画——不勾，点了是"打开"；完整原因进日志，菜单上放不下它（parking lot Q322）。
#[test]
fn when_the_system_cannot_be_asked_autostart_is_unchecked_and_the_reason_is_logged() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, asked(Ok(true)));

    feed(
        &mut tray,
        &mut screen,
        asked(Err(
            "连不上本机的任务计划程序: 服务没有启动。 (0x80070426)".to_string()
        )),
    );

    let menu = tray.menu(NOW);
    let entry = autostart(&menu);
    assert!(!entry.checked, "问不出来就不勾");
    assert_eq!(entry.command, Some(Command::EnableAutostart));
    assert_eq!(
        screen.logs,
        [
            "问不出开机自启的计划任务在不在，菜单上照没开画 —— 连不上本机的任务计划程序: 服务没有启动。 (0x80070426)"
        ]
    );
}

/// 缺省不开：启动时内核一个计划任务都不碰；"开机自启"不勾，点它交出的是"打开"。
#[test]
fn autostart_is_off_by_default_and_starting_never_touches_the_task() {
    let (tray, screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    let menu = tray.menu(NOW);
    let entry = autostart(&menu);

    assert!(screen.launch.is_empty(), "启动时不建也不删");
    assert!(!entry.checked);
    assert_eq!(entry.command, Some(Command::EnableAutostart));
}

/// 勾上：系统说任务不在，点"开机自启"，外壳收到"建那个计划任务"。
#[test]
fn checking_autostart_asks_the_shell_to_create_the_task() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, asked(Ok(false)));

    click_autostart(&mut tray, &mut screen);

    assert_eq!(screen.launch, [launch::Action::EnableAutostart]);
}

/// 取消：系统说任务在，点"开机自启"，外壳收到"删那个计划任务"。
#[test]
fn unchecking_autostart_asks_the_shell_to_delete_the_task() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, asked(Ok(true)));

    click_autostart(&mut tray, &mut screen);

    assert_eq!(screen.launch, [launch::Action::DisableAutostart]);
}

/// 点一下此刻菜单上的"开机自启"：它交给内核的就是它说的那一条。
fn click_autostart(tray: &mut Tray, screen: &mut Screen) {
    let command = autostart(&tray.menu(NOW))
        .command
        .clone()
        .expect("「开机自启」点得到");
    feed(tray, screen, Event::Menu(command));
}

/// 再启动一次：第二个实例敲了老实例的门，老实例弹一条"已经在托盘里了"，别的什么都不做。
#[test]
fn a_second_instance_knocking_pops_already_in_the_tray() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        Event::Launch(launch::Event::Knocked),
    );

    assert_eq!(
        screen.notices,
        [Notice {
            title: "已经在托盘里了".to_string(),
            body: "juicebar 已经在运行，不用再启动一次：右键托盘图标就能用".to_string(),
        }]
    );
}
