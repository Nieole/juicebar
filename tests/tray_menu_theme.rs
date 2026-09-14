//! 托盘内核里「菜单深浅」这一块（`juicebar::tray::Look::menu_theme`）：菜单此刻实际是深是浅（ADR-0006）。
//!
//! 菜单跟的是任务栏，不是应用模式；跟不上（认不得这个版本的 Windows，或者取不到那两个函数）就是系统缺省的
//! 浅色。图标样式子菜单里的预览按这个答案选调色（`resident-tray` 票 07）。

mod common;

use common::NOW;
use common::tray::{MOUSE_AND_KEYBOARD, start_with_look};
use juicebar::icon::{IconSize, Theme};
use juicebar::state::LastKnown;
use juicebar::tray::{Look, MenuTheming};

/// 跟得上任务栏时，菜单就是任务栏的深浅：深色任务栏上是深的，浅色任务栏上是浅的。
#[test]
fn where_the_menu_can_follow_the_taskbar_it_is_as_dark_or_light_as_the_taskbar() {
    let dark_taskbar = Look {
        theme: Theme::Dark,
        size: IconSize::Px16,
        menus: MenuTheming::FollowsTaskbar,
    };
    let light_taskbar = Look {
        theme: Theme::Light,
        ..dark_taskbar
    };

    assert_eq!(dark_taskbar.menu_theme(), Theme::Dark);
    assert_eq!(light_taskbar.menu_theme(), Theme::Light);
}

/// 认不得这个版本的 Windows：一个未公开函数都不调，菜单是系统缺省的浅色——任务栏是深的也一样。
#[test]
fn on_a_windows_it_does_not_recognise_the_menu_is_light_even_on_a_dark_taskbar() {
    let look = Look {
        theme: Theme::Dark,
        size: IconSize::Px16,
        menus: MenuTheming::UnknownWindows,
    };

    assert_eq!(look.menu_theme(), Theme::Light);
}

/// 认得这个版本，却取不到那两个函数：同样退回浅色。
#[test]
fn when_the_two_functions_cannot_be_had_the_menu_falls_back_to_light() {
    let look = Look {
        theme: Theme::Dark,
        size: IconSize::Px16,
        menus: MenuTheming::FunctionsMissing { build: 26200 },
    };

    assert_eq!(look.menu_theme(), Theme::Light);
}

/// 取不到那两个函数时，启动那一刻记一条日志：菜单为什么是浅的，托盘上没有别处说。
#[test]
fn when_the_two_functions_cannot_be_had_startup_logs_why_the_menu_is_light() {
    let look = Look {
        theme: Theme::Dark,
        size: IconSize::Px16,
        menus: MenuTheming::FunctionsMissing { build: 26200 },
    };

    let (_tray, screen) = start_with_look(MOUSE_AND_KEYBOARD, &LastKnown::default(), look, NOW);

    assert_eq!(
        screen.logs,
        [
            "菜单跟不上任务栏的深浅，退回浅色 —— 认得这个版本的 Windows（build 26200），却从 uxtheme.dll 取不到那两个函数（序号 135、136）"
        ]
    );
}
