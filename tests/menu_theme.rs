//! 外壳里「菜单深浅」那一个纯函数（`juicebar::shell::menu_theme`）：哪些 Windows 认得，认得的版本上 uxtheme
//! 序号 135 是哪一个函数（ADR-0006，`docs/research/native-menu-research.md` 第 3 节）。
//!
//! 期望值照 Notepad++ 的白名单（`DarkMode.cpp` 的 `CheckBuildNumber`）与它按 build 分两种调法的那一处，
//! 不是照这边的实现推出来的。

use juicebar::shell::menu_theme::{AppModeCall, app_mode_call};

/// 1809 之前的 Windows 10（1803，build 17134）认不得：一个未公开函数都不调，菜单是系统缺省的浅色。
#[test]
fn windows_10_before_1809_is_not_recognised() {
    assert_eq!(app_mode_call(17134), None);
}

/// 1809（build 17763）认得，但它的序号 135 是 `AllowDarkModeForApp(bool)`，不是 `SetPreferredAppMode`：
/// 照后者传"强制浅色"（3）进去，会被当成 `true`，反倒允许了深色。
#[test]
fn windows_10_1809_is_recognised_with_allow_dark_mode_for_app() {
    assert_eq!(app_mode_call(17763), Some(AppModeCall::AllowDarkModeForApp));
}

/// 1809 之后的 Windows 10 正式版认得，序号 135 已经换成了 `SetPreferredAppMode`：1903、1909（18362、18363），
/// 2004 到 22H2（19041–19045）。
#[test]
fn windows_10_releases_after_1809_are_recognised_with_set_preferred_app_mode() {
    for build in [18362, 18363, 19041, 19044, 19045] {
        assert_eq!(
            app_mode_call(build),
            Some(AppModeCall::SetPreferredAppMode),
            "build {build}"
        );
    }
}

/// 两次正式版之间的预览版 build 认不得：序号 135 就是在 1809 与 1903 之间的某个预览版里换了签名，谁也说不清
/// 其中哪一个是哪一种。18282 在 1809 与 1903 之间，18990 在 1909 与 2004 之间。
#[test]
fn insider_builds_between_windows_10_releases_are_not_recognised() {
    for build in [18282, 18990] {
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
