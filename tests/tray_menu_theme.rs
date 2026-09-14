//! 托盘内核里「菜单深浅」这一块（`juicebar::tray::menu_theme`）：哪些 Windows 让菜单的深浅跟得上任务栏，
//! 以及菜单此刻实际是深是浅（ADR-0006）。
//!
//! 白名单的期望值照 Notepad++（`DarkMode.cpp` 的 `CheckBuildNumber`，`docs/research/native-menu-research.md`
//! 第 3 节），1809 除外（parking lot Q260），不是照这边的实现推出来的。菜单实际的深浅经内核的
//! 公开面问（`Tray::menu_theme`），外壳由 `common::tray::Screen` 顶替。

mod common;

use common::NOW;
use common::tray::{LOOK, MOUSE_AND_KEYBOARD, start_with_look};
use juicebar::icon::Theme;
use juicebar::state::LastKnown;
use juicebar::tray::Look;
use juicebar::tray::menu_theme::{AppModeCall, MenuTheming, app_mode_call};

/// 按这个样子启动内核，问它菜单此刻实际是深是浅。
fn menu_theme_at_startup(look: Look) -> Theme {
    let (tray, _screen) = start_with_look(MOUSE_AND_KEYBOARD, &LastKnown::default(), look, NOW);
    tray.menu_theme()
}

/// 1809 之前的 Windows 10 认不得：一个未公开函数都不调，菜单是系统缺省的浅色。1803（build 17134），以及紧挨着
/// 1809 的前一个 build。
#[test]
fn windows_10_before_1809_is_not_recognised() {
    for build in [17134, 17762] {
        assert_eq!(app_mode_call(build), None, "build {build}");
    }
}

/// 1809（build 17763）认不得：Notepad++ 认它，这里不认。那一版的序号 135 是 `AllowDarkModeForApp(bool)`，只能
/// "允许深色"，菜单跟的是应用模式，做不到跟任务栏；认它只会让菜单是浅的、内核却说深（parking lot Q260，用户
/// 2026-09-14 定）。
#[test]
fn windows_10_1809_is_not_recognised_because_it_cannot_force_the_menu() {
    assert_eq!(app_mode_call(17763), None);
}

/// 1903 起的 Windows 10 正式版认得，序号 135 已经换成了 `SetPreferredAppMode`：1903、1909（18362、18363），
/// 2004 到 22H2（19041–19045）。
#[test]
fn windows_10_releases_from_1903_are_recognised_with_set_preferred_app_mode() {
    for build in [18362, 18363, 19041, 19044, 19045] {
        assert_eq!(
            app_mode_call(build),
            Some(AppModeCall::SetPreferredAppMode),
            "build {build}"
        );
    }
}

/// 两次正式版之间的预览版 build 认不得：序号 135 就是在 1809 与 1903 之间的某个预览版里换了签名，谁也说不清
/// 其中哪一个是哪一种。18282 在 1809 与 1903 之间，18990 在 1909 与 2004 之间；其余几个紧挨着正式版的两侧。
#[test]
fn insider_builds_between_windows_10_releases_are_not_recognised() {
    for build in [17764, 18282, 18361, 18364, 18990, 19040] {
        assert_eq!(app_mode_call(build), None, "build {build}");
    }
}

/// Windows 11 全部认得，序号 135 是 `SetPreferredAppMode`：第一版（build 22000），以及这台机器上的 25H2
/// （26200）。
#[test]
fn every_windows_11_is_recognised_with_set_preferred_app_mode() {
    for build in [22000, 26200] {
        assert_eq!(
            app_mode_call(build),
            Some(AppModeCall::SetPreferredAppMode),
            "build {build}"
        );
    }
}

/// 跟得上任务栏时，菜单就是任务栏的深浅：深色任务栏上是深的，浅色任务栏上是浅的。
#[test]
fn where_the_menu_can_follow_the_taskbar_it_is_as_dark_or_light_as_the_taskbar() {
    let dark_taskbar = Look {
        theme: Theme::Dark,
        menu_theming: MenuTheming::FollowsTaskbar,
        ..LOOK
    };
    let light_taskbar = Look {
        theme: Theme::Light,
        ..dark_taskbar
    };

    assert_eq!(menu_theme_at_startup(dark_taskbar), Theme::Dark);
    assert_eq!(menu_theme_at_startup(light_taskbar), Theme::Light);
}

/// 认不得这个版本的 Windows：一个未公开函数都不调，菜单是系统缺省的浅色——任务栏是深的也一样。
#[test]
fn on_a_windows_it_does_not_recognise_the_menu_is_light_even_on_a_dark_taskbar() {
    let look = Look {
        theme: Theme::Dark,
        menu_theming: MenuTheming::UnknownWindows,
        ..LOOK
    };

    assert_eq!(menu_theme_at_startup(look), Theme::Light);
}

/// 认得这个版本，却取不到那两个函数：同样退回浅色。
#[test]
fn when_the_two_functions_cannot_be_had_the_menu_falls_back_to_light() {
    let look = Look {
        theme: Theme::Dark,
        menu_theming: MenuTheming::FunctionsMissing { build: 26200 },
        ..LOOK
    };

    assert_eq!(menu_theme_at_startup(look), Theme::Light);
}

/// 取不到那两个函数时，启动那一刻记一条日志：菜单为什么是浅的，托盘上没有别处说。
#[test]
fn when_the_two_functions_cannot_be_had_startup_logs_why_the_menu_is_light() {
    let look = Look {
        menu_theming: MenuTheming::FunctionsMissing { build: 26200 },
        ..LOOK
    };

    let (_tray, screen) = start_with_look(MOUSE_AND_KEYBOARD, &LastKnown::default(), look, NOW);

    assert_eq!(
        screen.logs,
        [
            "菜单跟不上任务栏的深浅，退回浅色 —— 认得这个版本的 Windows（build 26200），却从 uxtheme.dll 取不到那两个函数（序号 135、136）"
        ]
    );
}
