//! 右键菜单：把内核的菜单模型（[`crate::tray::menu::Menu`]）画成普通原生菜单（ADR-0006），点了哪一项交回内核；
//! 以及菜单那一格的动作。
//!
//! 只画、不决定：每一项的种类、字、勾或圆点、右列、灰不灰、粗不粗、挂什么预览都照模型原样落到 `MENUITEMINFOW` 上。

use std::iter::once;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, InsertMenuItemW, MENU_ITEM_MASK,
    MENU_ITEM_STATE, MENU_ITEM_TYPE, MENUITEMINFOW, MFS_CHECKED, MFS_DEFAULT, MFS_GRAYED,
    MFT_RADIOCHECK, MFT_SEPARATOR, MFT_STRING, MIIM_BITMAP, MIIM_FTYPE, MIIM_ID, MIIM_STATE,
    MIIM_STRING, MIIM_SUBMENU, PostMessageW, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{Owned, PWSTR};

use crate::clock::{Clock, SystemClock};
use crate::icon::{menu_preview_bytes, render_preview};
use crate::tray::menu::{Action, Command, Entry, Item, Kind, Preview};

use super::{App, look, with_app};

/// 在鼠标那里弹出此刻的右键菜单，交回点了哪一项；没点（点到菜单外面、按了 Esc）就是 `None`。
///
/// 弹出期间 Windows 在菜单自己的循环里照样派发消息（计时器、取数结果），所以问完菜单模型就把外壳还回去，
/// 不借着它弹。
pub(super) fn popup(hwnd: HWND) -> Option<Command> {
    // 先把任务栏此刻的样子交给内核，再问菜单、再照同一份强制菜单深浅（ADR-0006）：运行中切了深浅、改了缩放，这一次
    // 弹出的菜单、菜单里预览的调色与尺寸就都跟上，而且是同一份（parking lot Q261）。
    let look = look::changed()?;
    let model = with_app(|app| app.tray.menu(SystemClock.now()))?;
    with_app(|app| app.uxtheme.follow(look.theme));
    // 预览位图要活到菜单销毁之后：菜单项只借用 `hbmpItem`，不接管它。先声明的后销毁，所以它在菜单之前声明。
    let mut previews = Vec::new();
    // SAFETY: 标准的托盘菜单弹法；菜单句柄由 Owned 收回，子菜单挂在它上面、随它一起销毁。
    unsafe {
        let menu = Owned::new(CreatePopupMenu().ok()?);
        let mut commands = Vec::new();
        fill(*menu, &model.items, &mut commands, &mut previews).ok()?;
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
/// 占一个 `None`。挂了预览的项，位图做好之后交给 `previews` 攒着，菜单销毁之后才删。
fn fill(
    menu: HMENU,
    items: &[Item],
    commands: &mut Vec<Option<Command>>,
    previews: &mut Vec<Owned<HBITMAP>>,
) -> windows::core::Result<()> {
    for (position, item) in (0u32..).zip(items) {
        let entry = match item {
            Item::Separator => {
                let info = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE,
                    fType: MFT_SEPARATOR,
                    ..Default::default()
                };
                // SAFETY: 一张自己刚建的菜单，一个只有类型的分隔线。
                unsafe { InsertMenuItemW(menu, position, true, &info)? };
                continue;
            }
            Item::Entry(entry) => entry,
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
        // 预览挂在勾那一格上（不设 `MNS_CHECKORBMP`，调研 8.3）：当前项靠加粗标出来，不画圆点。做不出位图就不挂、记一行
        // 日志，菜单照样弹——少一张预览比整张菜单弹不出来好。
        if let Some(preview) = &entry.preview {
            match preview_bitmap(preview) {
                Ok(bitmap) => {
                    info.fMask = MENU_ITEM_MASK(info.fMask.0 | MIIM_BITMAP.0);
                    info.hbmpItem = *bitmap;
                    previews.push(bitmap);
                }
                Err(e) => {
                    with_app(|app| {
                        app.log
                            .write(&format!("菜单里「{}」的预览画不出来：{e}", entry.text));
                    });
                }
            }
        }
        let submenu = match &entry.kind {
            Kind::Submenu(children) => {
                // SAFETY: 建一张空的弹出菜单；插不进父菜单时下面把它销毁。
                let sub = unsafe { CreatePopupMenu()? };
                if let Err(e) = fill(sub, children, commands, previews) {
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
        // SAFETY: 字符串在调用期间活着，InsertMenuItemW 自己拷一份；位图由 `previews` 攒着，活过菜单。
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

/// 一张预览 → 挂在菜单项上的 32 位位图。
///
/// 像素照模型给的参数画（[`render_preview`]，几张并排），字节由 [`menu_preview_bytes`] 换好（预乘过的 BGRA，自顶向下），
/// 这里只把它拷进 DIB 节。宽高取自画出来的那张：`biHeight` 取负数，第 0 行在最上面，与字节同序——正数的话预览上下颠倒。
fn preview_bitmap(preview: &Preview) -> windows::core::Result<Owned<HBITMAP>> {
    let bitmap = render_preview(
        preview.settings,
        &preview.states,
        Some(preview.percent),
        preview.size,
        preview.theme,
    );
    let (width, height) = (bitmap.width() as i32, bitmap.height() as i32);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // 负数：自顶向下
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let bytes = menu_preview_bytes(bitmap.pixels());
    // SAFETY: 标准的 32 位 DIB 节；bits 指向宽 × 高 × 4 字节（32 位的行本来就按 4 字节对齐，没有补白），只在这里写。
    unsafe {
        let mut bits = std::ptr::null_mut();
        let section = Owned::new(CreateDIBSection(
            None,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )?);
        // 长度按 DIB 节自己的大小取：字节与它对不上就 panic，不会写出界。
        std::slice::from_raw_parts_mut(bits.cast::<u8>(), (width * height * 4) as usize)
            .copy_from_slice(&bytes);
        Ok(section)
    }
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
