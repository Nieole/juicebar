//! 托盘图标本身：加进任务栏通知区、改它、拿掉它、资源管理器重启后加回来，以及把渲染器画好的位图
//! 做成一个图标。
//!
//! 画什么不在这里（`crate::tray::round` 定参数，`crate::icon::render` 画像素）；这里只把像素交给 Windows。

use std::collections::VecDeque;

use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};
use windows::core::Owned;

use crate::icon::{IconBitmap, render, tray_icon_bytes};
use crate::tray::notify::Notice;
use crate::tray::round::IconRequest;

/// 托盘里的那一个图标。
pub(super) struct TrayIcon {
    hwnd: HWND,
    callback: u32,
    icon: Option<Owned<HICON>>,
    tip: String,
    /// 通知区里此刻有没有它。资源管理器还没起来、或者刚重启过，就没有。
    added: bool,
    /// 还没弹出去的通知，按先后排着：图标不在通知区时弹不出来，等它加进去再弹（[`TrayIcon::balloon`]）。
    pending: VecDeque<Notice>,
}

impl TrayIcon {
    pub(super) fn new(hwnd: HWND, callback: u32) -> Self {
        Self {
            hwnd,
            callback,
            icon: None,
            tip: String::new(),
            added: false,
            pending: VecDeque::new(),
        }
    }

    /// 按内核给的参数重画。
    pub(super) fn draw(&mut self, request: &IconRequest) -> Result<()> {
        let bitmap = render(
            request.settings,
            request.state,
            request.percent,
            request.size,
            request.theme,
        );
        self.icon = Some(to_icon(&bitmap)?);
        self.show();
        Ok(())
    }

    /// 换悬停提示。
    pub(super) fn set_tip(&mut self, tip: &str) {
        tip.clone_into(&mut self.tip);
        self.show();
    }

    /// 资源管理器重启了：通知区是新的，里面没有我们，照此刻的样子再加一次。
    pub(super) fn add_again(&mut self) {
        self.added = false;
        self.show();
    }

    /// 拿掉。
    pub(super) fn remove(&mut self) {
        if self.added {
            let data = self.data();
            // SAFETY: data 是一份完整填好的 NOTIFYICONDATAW。
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = false;
        }
    }

    /// 弹一条通知（从这个图标上弹出来）。
    ///
    /// **图标不在通知区时不丢**（parking lot Q204）：资源管理器还没起来（开机自启正是这种时候）、或者正在重启的那几秒，
    /// 这条通知先排着，等图标加进去——下一次重画、换悬停提示，或者资源管理器起来时的那条广播——再弹。内核交出它的那一刻
    /// 已经记下"提醒过"，这里丢了，这一次跌破就再也不会弹。
    pub(super) fn balloon(&mut self, notice: &Notice) {
        self.pending.push_back(notice.clone());
        self.show();
    }

    /// 把此刻的图标与悬停提示交给通知区：已经在了就改，还没在（或者改不动了——通知区换过一个）就加；在了，就把排着的通知
    /// 弹出去。
    ///
    /// 加不进去（资源管理器还没起来）不算错：下一次重画、或者资源管理器起来时的那条广播，会再加一次。**加说没加进去时
    /// 再试一次改**：资源管理器正忙（开机时常见）时加可能超时、报失败，而图标其实已经加进去了；照"不在"记着，之后每一次
    /// 都只会再加、再失败，排着的通知就一直弹不出去。
    fn show(&mut self) {
        let data = self.data();
        // SAFETY: 同上。
        unsafe {
            if !(self.added && Shell_NotifyIconW(NIM_MODIFY, &data).as_bool()) {
                self.added = Shell_NotifyIconW(NIM_ADD, &data).as_bool()
                    || Shell_NotifyIconW(NIM_MODIFY, &data).as_bool();
            }
        }
        self.pop_pending();
    }

    /// 图标在通知区里时，把排着的通知按先后弹出去。弹不动（通知区刚换过一个、还没收到那条广播）就照"不在"记着，剩下的
    /// 接着排。弹的那一下超时报失败、而其实已经弹出去了时，这一条之后会再弹一次：宁可重一次，不丢（parking lot Q321）。
    fn pop_pending(&mut self) {
        while self.added {
            let Some(notice) = self.pending.front() else {
                return;
            };
            let mut data = self.data();
            data.uFlags |= NIF_INFO;
            data.dwInfoFlags = NIIF_INFO;
            copy_wide(&notice.title, &mut data.szInfoTitle);
            copy_wide(&notice.body, &mut data.szInfo);
            // SAFETY: 同上。
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
                self.pending.pop_front();
            } else {
                self.added = false;
            }
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: self.callback,
            ..Default::default()
        };
        if let Some(icon) = &self.icon {
            data.uFlags |= NIF_ICON;
            data.hIcon = **icon;
        }
        copy_wide(&self.tip, &mut data.szTip);
        data
    }
}

/// 把一段字写进一个定长的宽字符格子，末尾留一格 NUL；放不下就截断（悬停提示在内核里已经截好了）。
fn copy_wide(text: &str, field: &mut [u16]) {
    let room = field.len() - 1;
    let mut end = 0;
    for (slot, unit) in field.iter_mut().zip(text.encode_utf16().take(room)) {
        *slot = unit;
        end += 1;
    }
    field[end] = 0;
}

/// 渲染器画好的位图 → 一个 32 位图标。
///
/// 颜色位图的字节由 [`tray_icon_bytes`] 换好（不预乘的 BGRA，自顶向下），这里只把它拷进 DIB 节；掩码位图全 0，
/// 透明与否全看颜色位图里的不透明度。两块位图交给 `CreateIconIndirect` 之后它各自拷一份，这两块随即删掉。
fn to_icon(bitmap: &IconBitmap) -> Result<Owned<HICON>> {
    let size = bitmap.size() as i32;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // 负数：自顶向下，第 0 行在最上面，与渲染器同序
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let color_bytes = tray_icon_bytes(bitmap.pixels());
    // 单色位图每行按 16 位对齐。
    let mask_bits = vec![0u8; (size as usize).div_ceil(16) * 2 * size as usize];
    // SAFETY: 标准的"DIB 节 + 单色掩码 → 图标"用法；bits 指向 size × size × 4 字节，只在这里写。
    unsafe {
        let mut bits = std::ptr::null_mut();
        let color = Owned::new(
            CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .context("建不了图标的颜色位图")?,
        );
        // 长度按 DIB 节自己的大小取：字节与它对不上就 panic，不会写出界。
        std::slice::from_raw_parts_mut(bits.cast::<u8>(), (size * size * 4) as usize)
            .copy_from_slice(&color_bytes);
        let mask = Owned::new(CreateBitmap(
            size,
            size,
            1,
            1,
            Some(mask_bits.as_ptr().cast()),
        ));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: *mask,
            hbmColor: *color,
        };
        Ok(Owned::new(CreateIconIndirect(&info).context("做不出图标")?))
    }
}
