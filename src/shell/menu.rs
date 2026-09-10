//! 右键菜单：把内核的菜单模型（[`crate::tray::menu::Menu`]）画成原生菜单，点了哪一项交回内核；以及菜单
//! 那一格的动作。

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, GetCursorPos, MF_STRING, PostMessageW, SetForegroundWindow,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{HSTRING, Owned};

use crate::tray::menu::{Action, Command};

use super::{App, with_app};

/// 在鼠标那里弹出此刻的右键菜单，交回点了哪一项；没点（点到菜单外面、按了 Esc）就是 `None`。
///
/// 弹出期间 Windows 在菜单自己的循环里照样派发消息（计时器、取数结果），所以问完菜单模型就把外壳还回去，
/// 不借着它弹。
pub(super) fn popup(hwnd: HWND) -> Option<Command> {
    let model = with_app(|app| app.tray.menu())?;
    // SAFETY: 标准的托盘菜单弹法；菜单句柄由 Owned 收回。
    unsafe {
        let menu = Owned::new(CreatePopupMenu().ok()?);
        // 菜单项的编号是它在模型里的位置加一：0 留给"什么都没点"。
        for (index, entry) in model.entries.iter().enumerate() {
            AppendMenuW(*menu, MF_STRING, index + 1, &HSTRING::from(&entry.text)).ok()?;
        }
        let mut cursor = POINT::default();
        GetCursorPos(&mut cursor).ok()?;
        // 先把自己的窗口放到前台，否则点到菜单外面时菜单不收起（Windows 对托盘菜单的老规矩）。
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            *menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let index = usize::try_from(chosen.0).ok()?.checked_sub(1)?;
        model.entries.get(index).map(|entry| entry.command)
    }
}

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::Quit => app.quit(),
    }
}
