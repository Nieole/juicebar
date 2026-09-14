//! 菜单的深浅跟任务栏（ADR-0006）：uxtheme 里未公开的那两个函数。
//!
//! Windows 没有让弹出菜单变深的公开接口，只有 uxtheme.dll 里按序号取的两个函数：序号 135 设首选模式（强制
//! 深色或强制浅色），序号 136 刷新菜单主题（`docs/research/native-menu-research.md` 第 3 节）。微软随时可以
//! 改掉它们，所以只在认得的版本上取（[`app_mode_call`]），认不得就一个都不调，菜单是系统缺省的浅色。
//!
//! 版本白名单是纯函数，公开出去是为了用例（`tests/menu_theme.rs`）；其余是外壳，只给 `shell` 自己用：启动时
//! 取一次（`MenuTheme::load`），每次弹出菜单之前按任务栏此刻的深浅调一次（`MenuTheme::follow`）。菜单此刻
//! 实际是深是浅由内核答（[`crate::tray::Look::menu_theme`]）。

use std::mem::transmute;
use std::ptr::without_provenance;

use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
use windows::core::{PCSTR, w};

use crate::icon::Theme;
use crate::tray::MenuTheming;

/// 认得的版本上，uxtheme 序号 135 是哪一个函数。两种签名不一样，调错了不会报错，只会做错事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppModeCall {
    /// Windows 10 1809：`AllowDarkModeForApp(bool)`。它只能"允许深色"——允许了，菜单跟的是应用模式——
    /// 强制不了深色，也强制不了浅色（parking lot Q260）。
    AllowDarkModeForApp,
    /// 1903 起：`SetPreferredAppMode(mode)`，强制深色是 2、强制浅色是 3。
    SetPreferredAppMode,
}

/// 这个 build 认不认得；认得时序号 135 是哪一个函数。认不得（`None`）就一个未公开函数都不调。
///
/// 照 Notepad++ 的白名单（`DarkMode.cpp` 的 `CheckBuildNumber`，调研第 3 节）：1809（17763）、1903 与 1909
/// （18362、18363）、2004 起（19041 及以后，含全部 Windows 11）。之间的预览版 build 不认：序号 135 就是在
/// 那几个预览版里换的签名。
///
/// 收的是 build 号，不是版本名：注册表里的 `ProductName` 在 Windows 11 上照样可能写着"Windows 10"。
pub fn app_mode_call(build: u32) -> Option<AppModeCall> {
    match build {
        17763 => Some(AppModeCall::AllowDarkModeForApp),
        18362 | 18363 | 19041.. => Some(AppModeCall::SetPreferredAppMode),
        _ => None,
    }
}

/// `SetPreferredAppMode` 的"强制深色"。
const FORCE_DARK: i32 = 2;
/// `SetPreferredAppMode` 的"强制浅色"。
const FORCE_LIGHT: i32 = 3;

/// `GetProcAddress` 交出来的函数指针的样子；用之前要换成它真正的签名。
type AnyFunction = unsafe extern "system" fn() -> isize;

/// 启动时取到的那两个函数，连同交给内核的那一份答案。
pub(super) struct MenuTheme {
    theming: MenuTheming,
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

impl MenuTheme {
    /// 启动时问一次：这个版本认不认得，认得就去取那两个函数。
    pub(super) fn load() -> Self {
        let build = windows_build();
        let Some(call) = app_mode_call(build) else {
            return Self {
                theming: MenuTheming::UnknownWindows,
                calls: None,
            };
        };
        match take_calls(call) {
            Some(calls) => Self {
                theming: MenuTheming::FollowsTaskbar,
                calls: Some(calls),
            },
            None => Self {
                theming: MenuTheming::FunctionsMissing { build },
                calls: None,
            },
        }
    }

    /// 交给内核的那一份：菜单跟不跟得上任务栏。
    pub(super) fn theming(&self) -> MenuTheming {
        self.theming
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
