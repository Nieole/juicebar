//! 让菜单的深浅跟任务栏（ADR-0006）：从 uxtheme.dll 按序号取那两个未公开函数，每次弹出菜单之前调它们。
//!
//! 取不取、序号 135 按哪一种签名装，由内核的版本白名单定（[`app_mode_call`]）：认不得的版本上一个都不取。
//! 启动时取一次（`UxTheme::load`），取没取到交给内核（[`MenuTheming`]）；每次弹出菜单之前按任务栏此刻的深浅
//! 调一次（`UxTheme::follow`）。菜单此刻实际是深是浅由内核答（[`crate::tray::Tray::menu_theme`]）。

use std::mem::transmute;
use std::ptr::without_provenance;

use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
use windows::core::{PCSTR, w};

use crate::icon::Theme;
use crate::tray::menu_theme::{AppModeCall, MenuTheming, app_mode_call};

/// `SetPreferredAppMode` 的"强制深色"。
const FORCE_DARK: i32 = 2;
/// `SetPreferredAppMode` 的"强制浅色"。
const FORCE_LIGHT: i32 = 3;

/// `GetProcAddress` 交出来的函数指针的样子；用之前要换成它真正的签名。
type AnyFunction = unsafe extern "system" fn() -> isize;

/// 启动时从 uxtheme.dll 取到的那两个函数。
pub(super) struct UxTheme {
    /// 跟不上任务栏时是 `None`：一个都没取到手，也就一个都不调。
    calls: Option<Calls>,
}

/// 取到手的那两个函数。
struct Calls {
    app_mode: AppMode,
    flush_menu_themes: unsafe extern "system" fn(),
}

/// 序号 135，按这个 build 上它的真签名装好。
enum AppMode {
    AllowDarkModeForApp(unsafe extern "system" fn(bool) -> bool),
    SetPreferredAppMode(unsafe extern "system" fn(i32) -> i32),
}

impl UxTheme {
    /// 启动时问一次：这个版本认不认得，认得就去取那两个函数。连同交给内核的那一份：菜单跟不跟得上任务栏。
    pub(super) fn load() -> (Self, MenuTheming) {
        let build = windows_build();
        let Some(call) = app_mode_call(build) else {
            return (Self { calls: None }, MenuTheming::UnknownWindows);
        };
        match take_calls(call) {
            Some(calls) => (Self { calls: Some(calls) }, MenuTheming::FollowsTaskbar),
            None => (
                Self { calls: None },
                MenuTheming::FunctionsMissing { build },
            ),
        }
    }

    /// 弹出菜单之前：按任务栏此刻的深浅强制菜单深或浅，再刷新菜单主题——不刷新，这一次弹出的还是上一种。
    /// 跟不上任务栏时什么都不调。
    pub(super) fn follow(&self, taskbar: Theme) {
        let Some(calls) = &self.calls else {
            return;
        };
        let dark = taskbar == Theme::Dark;
        // SAFETY: 两个函数是按这个 build 上的真签名装好的（`take_calls`），uxtheme.dll 装进来之后从不卸载。
        unsafe {
            match calls.app_mode {
                AppMode::AllowDarkModeForApp(allow) => {
                    allow(dark);
                }
                AppMode::SetPreferredAppMode(set) => {
                    set(if dark { FORCE_DARK } else { FORCE_LIGHT });
                }
            }
            (calls.flush_menu_themes)();
        }
    }
}

/// 这台机器的 Windows build 号。
///
/// 用 `GetVersionExW`：它只对在程序清单里声明了支持 Windows 10 的程序说真话，`juicebar.exe.manifest` 声明了。
/// 清单要是丢了，它交出 Windows 8 的 9200，白名单认不得，菜单是浅色——错也错在不调的那一边（parking lot
/// Q262）。问不出来当 0，同样认不得。
fn windows_build() -> u32 {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: 结构体的大小照实填在它自己的第一格里。
    if unsafe { GetVersionExW(&mut info) }.is_ok() {
        info.dwBuildNumber
    } else {
        0
    }
}

/// 从 System32 的 uxtheme.dll 按序号取那两个函数，装成这个 build 上的真签名；哪一个取不到都是 `None`。
fn take_calls(call: AppModeCall) -> Option<Calls> {
    // SAFETY: 只从 System32 加载，不给别处的同名 DLL 顶替的机会。不释放：取出来的函数指针要一直有效。
    let uxtheme =
        unsafe { LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
    // SAFETY: 按序号取函数：序号放在名字指针的位置上传（`MAKEINTRESOURCEA` 的写法），模块句柄是上面刚拿到的。
    let by_ordinal =
        |ordinal: usize| unsafe { GetProcAddress(uxtheme, PCSTR(without_provenance(ordinal))) };
    let app_mode = by_ordinal(135)?;
    let flush_menu_themes = by_ordinal(136)?;
    // SAFETY: 两个序号在认得的版本上各是什么签名，照调研第 3 节的序号表；序号 135 的两种签名由
    // `app_mode_call` 按 build 分好了。
    unsafe {
        Some(Calls {
            app_mode: match call {
                AppModeCall::AllowDarkModeForApp => {
                    AppMode::AllowDarkModeForApp(transmute::<
                        AnyFunction,
                        unsafe extern "system" fn(bool) -> bool,
                    >(app_mode))
                }
                AppModeCall::SetPreferredAppMode => {
                    AppMode::SetPreferredAppMode(transmute::<
                        AnyFunction,
                        unsafe extern "system" fn(i32) -> i32,
                    >(app_mode))
                }
            },
            flush_menu_themes: transmute::<AnyFunction, unsafe extern "system" fn()>(
                flush_menu_themes,
            ),
        })
    }
}
