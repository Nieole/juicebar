//! 启动时任务栏的样子：深浅色与显示缩放。内核拿它选调色与尺寸（[`crate::tray::Look`]）。
//!
//! 给内核的这一份只在启动时问一次；它们变化时重画归票 07（那时这里多一个"变了"的事件）。任务栏的深浅另在
//! 每次弹出菜单之前再读一次（`menu.rs`），菜单按那一刻的深浅强制深或浅。

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::core::w;

use crate::icon::{IconSize, Theme};
use crate::tray::{Look, MenuTheming};

/// 问一次 Windows。菜单跟不跟得上任务栏（`menus`）是外壳启动时从 uxtheme 那里问来的（`menu_theme.rs`）。
pub(super) fn detect(menus: MenuTheming) -> Look {
    // SAFETY: 读系统的显示缩放。程序清单声明了按显示器感知缩放，所以这里拿到的是真的缩放，不是 96。
    let dpi = unsafe { GetDpiForSystem() };
    Look {
        theme: taskbar_theme(),
        size: IconSize::for_dpi(dpi),
        menus,
    }
}

/// 任务栏此刻是深色还是浅色。托盘图标在启动时读它，菜单在每次弹出之前读它。
///
/// Windows 没给这件事一个 API，公开的答案是 `Personalize` 下的 `SystemUsesLightTheme`：1 是浅色。读不到
/// （没有这个值的老系统）按深色：那是这个值出现之前任务栏的样子。
pub(super) fn taskbar_theme() -> Theme {
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
