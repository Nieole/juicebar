//! 右键菜单：把内核的菜单模型（[`crate::tray::menu::Menu`]）画成普通原生菜单（ADR-0006），点了哪一项交回内核；
//! 以及菜单那一格的动作。
//!
//! 只画、不决定：每一项的种类、字、勾或圆点、右列、灰不灰、粗不粗都照模型原样落到 `MENUITEMINFOW` 上。

use std::iter::once;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, InsertMenuItemW, MENU_ITEM_MASK,
    MENU_ITEM_STATE, MENU_ITEM_TYPE, MENUITEMINFOW, MFS_CHECKED, MFS_DEFAULT, MFS_GRAYED,
    MFT_RADIOCHECK, MFT_SEPARATOR, MFT_STRING, MIIM_FTYPE, MIIM_ID, MIIM_STATE, MIIM_STRING,
    MIIM_SUBMENU, PostMessageW, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, WM_NULL,
};
use windows::core::{Owned, PWSTR};

use crate::clock::{Clock, SystemClock};
use crate::tray::menu::{Action, Command, Entry, Item, Kind};

use super::{App, look, with_app};

/// 在鼠标那里弹出此刻的右键菜单，交回点了哪一项；没点（点到菜单外面、按了 Esc）就是 `None`。
///
/// 弹出期间 Windows 在菜单自己的循环里照样派发消息（计时器、取数结果），所以问完菜单模型就把外壳还回去，
/// 不借着它弹。
pub(super) fn popup(hwnd: HWND) -> Option<Command> {
    let model = with_app(|app| app.tray.menu(SystemClock.now()))?;
    // 按任务栏此刻的深浅强制菜单深或浅（ADR-0006）。每次都重读：运行中切了深浅，这一次弹出就跟上。
    with_app(|app| app.uxtheme.follow(look::taskbar_theme()));
    // SAFETY: 标准的托盘菜单弹法；菜单句柄由 Owned 收回，子菜单挂在它上面、随它一起销毁。
    unsafe {
        let menu = Owned::new(CreatePopupMenu().ok()?);
        let mut commands = Vec::new();
        fill(*menu, &model.items, &mut commands).ok()?;
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
        commands.into_iter().nth(index).flatten()
    }
}

/// 把这几项依次插进 `menu`，子菜单另建一张弹出菜单挂上去。
///
/// 每一项（连同子菜单里的）的编号是它在 `commands` 里的位置加一，0 留给"什么都没点"；点了什么都不收的项在那里
/// 占一个 `None`。预览位图（[`Entry::preview`]）归票 07。
fn fill(
    menu: HMENU,
    items: &[Item],
    commands: &mut Vec<Option<Command>>,
) -> windows::core::Result<()> {
    for (position, item) in (0u32..).zip(items) {
        let Item::Entry(entry) = item else {
            let info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE,
                fType: MFT_SEPARATOR,
                ..Default::default()
            };
            // SAFETY: 一张自己刚建的菜单，一个只有类型的分隔线。
            unsafe { InsertMenuItemW(menu, position, true, &info)? };
            continue;
        };
        commands.push(entry.command.clone());
        let mut text: Vec<u16> = label(entry).encode_utf16().chain(once(0)).collect();
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MENU_ITEM_MASK(MIIM_FTYPE.0 | MIIM_STATE.0 | MIIM_ID.0 | MIIM_STRING.0),
            fType: MENU_ITEM_TYPE(match entry.kind {
                Kind::Radio => MFT_STRING.0 | MFT_RADIOCHECK.0,
                Kind::Normal | Kind::Check | Kind::Submenu(_) => MFT_STRING.0,
            }),
            fState: MENU_ITEM_STATE(state(entry)),
            wID: commands.len() as u32,
            dwTypeData: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        let submenu = match &entry.kind {
            Kind::Submenu(children) => {
                // SAFETY: 建一张空的弹出菜单；插不进父菜单时下面把它销毁。
                let sub = unsafe { CreatePopupMenu()? };
                if let Err(e) = fill(sub, children, commands) {
                    // SAFETY: 还没挂到父菜单上的那一张，只有这里握着它。
                    let _ = unsafe { DestroyMenu(sub) };
                    return Err(e);
                }
                info.fMask = MENU_ITEM_MASK(info.fMask.0 | MIIM_SUBMENU.0);
                info.hSubMenu = sub;
                Some(sub)
            }
            Kind::Normal | Kind::Check | Kind::Radio => None,
        };
        // SAFETY: 字符串在调用期间活着，InsertMenuItemW 自己拷一份。
        if let Err(e) = unsafe { InsertMenuItemW(menu, position, true, &info) } {
            if let Some(sub) = submenu {
                // SAFETY: 同上，没挂上去的子菜单只有这里握着。
                let _ = unsafe { DestroyMenu(sub) };
            }
            return Err(e);
        }
    }
    Ok(())
}

/// 一项上写的字：有右列就用 `\t` 接上——普通项自带的那一列，贴右对齐（`docs/research/native-menu-research.md` 8.1）。
/// `&` 写成 `&&`：不然 Windows 把它当成助记键的标记吃掉，设备名里的 `&` 就不见了。
fn label(entry: &Entry) -> String {
    let text = entry.text.replace('&', "&&");
    match &entry.right {
        Some(right) => format!("{text}\t{}", right.replace('&', "&&")),
        None => text,
    }
}

/// 勾或圆点（单选项的类型已经带着 `MFT_RADIOCHECK`，勾上就画成圆点）、变灰、加粗。
fn state(entry: &Entry) -> u32 {
    let mut state = 0;
    if entry.checked {
        state |= MFS_CHECKED.0;
    }
    if entry.grayed {
        state |= MFS_GRAYED.0;
    }
    if entry.bold {
        state |= MFS_DEFAULT.0;
    }
    state
}

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::OpenLog => {
            if let Err(e) = super::open_in_default_program(app.hwnd, app.log.path()) {
                app.log.write(&format!("{e:#}"));
            }
        }
        Action::Quit => app.quit(),
    }
}
