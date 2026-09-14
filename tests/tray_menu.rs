//! 托盘内核里「菜单」这一块（`juicebar::tray::menu`）：右键菜单上有什么，点了之后内核做什么。
//!
//! 今天的菜单只有"打开配置文件"与"退出"。设备行与切换 Primary Device 归票 06，图标样式归票 07。

mod common;

use common::NOW;
use common::tray::{MOUSE_AND_KEYBOARD, feed, later, start};
use juicebar::state::LastKnown;
use juicebar::tray::menu::{self, Command};
use juicebar::tray::{Event, config};

/// 右键菜单自上而下两项："打开配置文件"，"退出"。
#[test]
fn the_menu_has_open_config_file_then_quit() {
    let (tray, _screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    let entries: Vec<(String, Command)> = tray
        .menu()
        .entries
        .into_iter()
        .map(|entry| (entry.text, entry.command))
        .collect();

    assert_eq!(
        entries,
        [
            ("打开配置文件".to_string(), Command::OpenConfigFile),
            ("退出".to_string(), Command::Quit),
        ]
    );
}

/// 点了"打开配置文件"：外壳收到"打开配置文件"（它交给系统默认程序打开）。
#[test]
fn open_config_file_asks_the_shell_to_open_it() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Menu(Command::OpenConfigFile));

    assert_eq!(screen.config, [config::Action::OpenFile]);
}

/// 点了"退出"就退出：外壳收到退出（它把图标拿掉、停下取数），之后时钟再怎么走，也不再问任何一台。
#[test]
fn quitting_stops_polling_for_good() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Menu(Command::Quit));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 3_600)));

    assert_eq!(screen.menu, [menu::Action::Quit]);
    assert_eq!(
        screen.fetches,
        ["dragonfly3", "neon75"],
        "退出之后一台都不再问"
    );
}
