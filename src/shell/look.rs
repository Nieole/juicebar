//! 任务栏的样子：深浅色与显示缩放。内核拿它选调色与尺寸（[`crate::tray::Look`]）。
//!
//! 启动时问一次（[`detect`]）；之后 Windows 说设置变了（深浅色切换、显示缩放、显示器）、或者要弹出菜单了，再问一次
//! 交给内核（[`changed`]），内核看出没变就什么都不做。弹出之前那一次让预览的调色与菜单被强制成的深浅出自同一份
//! （parking lot Q261），也兜住万一漏收的那条消息。

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForSystem, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{WM_DISPLAYCHANGE, WM_DPICHANGED, WM_SETTINGCHANGE};
use windows::core::w;

use crate::icon::{IconSize, Theme};
use crate::tray::menu_theme::MenuTheming;
use crate::tray::{Event, Look};

/// 问一次 Windows。菜单跟不跟得上任务栏（`menu_theming`）是外壳启动时从 uxtheme 那里问来的（`menu_theme.rs`）。
pub(super) fn detect(menu_theming: MenuTheming) -> Look {
    Look {
        theme: taskbar_theme(),
        size: IconSize::for_dpi(primary_monitor_dpi()),
        menu_theming,
    }
}

/// 这条窗口消息说的是不是"任务栏的样子可能变了"：设置变了（切深浅色时 Windows 广播的就是它）、显示缩放变了、显示器
/// 变了。
pub(super) fn is_change(message: u32) -> bool {
    matches!(message, WM_SETTINGCHANGE | WM_DPICHANGED | WM_DISPLAYCHANGE)
}

/// 再问一次 Windows，把此刻的样子交给内核，交回问到的那一份；借不到外壳（窗口过程被重入）时是 `None`。
pub(super) fn changed() -> Option<Look> {
    let menu_theming = super::with_app(|app| app.uxtheme.menu_theming())?;
    let look = detect(menu_theming);
    super::feed(Event::LookChanged(look));
    Some(look)
}

/// 任务栏此刻是深色还是浅色。托盘图标、预览与菜单都照它。
///
/// Windows 没给这件事一个 API，公开的答案是 `Personalize` 下的 `SystemUsesLightTheme`：1 是浅色。读不到
/// （没有这个值的老系统）按深色：那是这个值出现之前任务栏的样子。
fn taskbar_theme() -> Theme {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: 读一个 DWORD 进一个 u32，大小照实给。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    if status.is_ok() && value != 0 {
        Theme::Light
    } else {
        Theme::Dark
    }
}

/// 主显示器此刻的有效 DPI：通知区在主显示器的任务栏上。
///
/// 不用 `GetDpiForSystem`：那是登录那一刻的系统 DPI，运行中改了缩放它不变（parking lot Q301）。程序清单声明了按显示器
/// 感知缩放，这里拿到的是真的缩放，不是 96。问不出来才退回系统 DPI。
fn primary_monitor_dpi() -> u32 {
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: (0, 0) 总在主显示器上，取不到也按主显示器；两个出参是本地变量。
    let asked = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y)
    };
    if asked.is_ok() {
        dpi_x
    } else {
        // SAFETY: 读系统的显示缩放。
        unsafe { GetDpiForSystem() }
    }
}
