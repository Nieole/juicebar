//! 字形选"系统字体"时的那一支：字交给 Windows 的文字渲染（GDI）去画。
//!
//! **这是渲染器里唯一不逐像素照设计稿的地方**（ADR-0005 那条的例外）：设计稿在这一项上用的是
//! 浏览器的文字光栅化，这边无论用什么实现都不会与它逐像素一致。所以它的用例只断言结构——字放得下、
//! 居中、没有被截断、确实画了东西（`tests/icon.rs` 末尾）。
//!
//! 怎么放照设计稿 `drawText` 里 `glyph === 'sys'` 那一支：Segoe UI 粗体，字号从方框高度的
//! 1.35 倍往下试，到 6 像素为止，第一个墨迹放得进方框的字号就是它，然后居中；都放不下就不画。
//! 与设计稿不同的两处，都是为了在 16 像素上不歪：
//! - 设计稿横向按字的前进宽度量、按前进宽度居中，这里横竖都按墨迹——字形两侧的留白不算数；
//! - 设计稿把字画在小数坐标上，这里画在整像素上。
//!
//! 它仍然是纯的：只在内存里的一块位图上画，画完就还给系统，不碰任何窗口。

use std::ffi::c_void;

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC,
    FW_BOLD, GdiFlush, HBITMAP, HDC, HGDIOBJ, OUT_TT_ONLY_PRECIS, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT, TextOutW,
};
use windows::core::{Owned, w};

use super::{Canvas, Rect, Rgba};

/// 在方框里居中画一串字；最小的字号也放不下就什么都不画（与设计稿一样）。
pub(super) fn draw(c: &mut Canvas, s: &str, rect: Rect, color: Rgba) {
    let largest = (f64::from(rect.h) * 1.35).floor() as i32;
    for px in (6..=largest).rev() {
        let Some(ink) = rasterize(s, px) else {
            // 交不出一个完整的字（GDI 不肯画，或草稿纸没装下）：这一块留空，别的照画。
            // 半个字比没有字更像是在说一个数。
            return;
        };
        if ink.w <= rect.w && ink.h <= rect.h {
            let (x0, y0) = rect.center(ink.w, ink.h);
            for y in 0..ink.h {
                for x in 0..ink.w {
                    let a = ink.coverage[(y * ink.w + x) as usize];
                    if a > 0 {
                        let a = (u16::from(a) * u16::from(color.a) / 255) as u8;
                        c.fill_rect(x0 + x, y0 + y, 1, 1, Rgba { a, ..color });
                    }
                }
            }
            return;
        }
    }
}

/// 一串字的墨迹：只含有笔画的那个外接矩形，每格是覆盖率 0–255。
struct Ink {
    w: i32,
    h: i32,
    coverage: Vec<u8>,
}

/// 用 `px` 像素的 Segoe UI 粗体把一串字画出来，裁到墨迹。一个笔画都没有（空串、全是空格）时墨迹
/// 是 0 × 0；GDI 哪一步不成就是 `None`。
///
/// 画在一块四周各留一个字号宽的草稿纸上，所以字不可能碰到纸边；万一碰到了，说明这张纸装不下它，
/// 裁出来的墨迹就是被截断过的——那时也返回 `None`，而不是交出半个字。
fn rasterize(s: &str, px: i32) -> Option<Ink> {
    let text: Vec<u16> = s.encode_utf16().collect();
    let n = i32::try_from(text.len()).ok()?;
    let (w, h) = ((n + 2) * px, 3 * px);
    let page = Page::new(w, h)?;
    // SAFETY: page.dc 是有效的内存 DC；字体在函数结束前选回原来的那个再删掉（Owned 的 drop）。
    unsafe {
        let font = Owned::new(CreateFontW(
            -px, // 负数：字号按字面高度（em）算，与 CSS 的 px 字号同一个意思
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_ONLY_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY, // 灰度抗锯齿：覆盖率只有一个通道，没有 ClearType 的彩边
            0,
            w!("Segoe UI"),
        ));
        if font.is_invalid() {
            return None;
        }
        let old = SelectObject(page.dc, HGDIOBJ::from(*font));
        SetBkMode(page.dc, TRANSPARENT);
        SetTextColor(page.dc, COLORREF(0x00FF_FFFF));
        let drawn = TextOutW(page.dc, px, px, &text).as_bool();
        let _ = GdiFlush();
        SelectObject(page.dc, old);
        if !drawn {
            return None;
        }
    }
    page.ink()
}

/// 一张草稿纸：一个内存 DC 加一块选进去的 32 位自顶向下的 DIB，整张涂黑。
struct Page {
    dc: HDC,
    _dib: Owned<HBITMAP>,
    old: HGDIOBJ,
    bits: *const u8,
    w: i32,
    h: i32,
}

impl Page {
    fn new(w: i32, h: i32) -> Option<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 负数：自顶向下，第 0 行在最上面
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: 标准的"内存 DC + DIB 节"用法；失败的每一步都把已拿到的还回去（Drop / Owned）。
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let mut bits: *mut c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                .ok()
                .map(|dib| Owned::new(dib)) // 先接管，下一步若丢掉它也会被删掉
                .filter(|_| !bits.is_null());
            let Some(dib) = dib else {
                let _ = DeleteDC(dc);
                return None;
            };
            std::ptr::write_bytes(bits.cast::<u8>(), 0, (w * h * 4) as usize);
            let old = SelectObject(dc, HGDIOBJ::from(*dib));
            Some(Self {
                dc,
                _dib: dib,
                old,
                bits: bits.cast(),
                w,
                h,
            })
        }
    }

    /// 纸上墨迹的外接矩形与每格的覆盖率。白字画在黑纸上，所以绿通道就是覆盖率。
    fn ink(&self) -> Option<Ink> {
        // SAFETY: bits 指向这块 DIB 的 w × h × 4 字节，DIB 活到 self 被 drop；画完已经 GdiFlush。
        let px = unsafe { std::slice::from_raw_parts(self.bits, (self.w * self.h * 4) as usize) };
        let at = |x: i32, y: i32| px[((y * self.w + x) * 4 + 1) as usize];
        let (mut l, mut t, mut r, mut b) = (self.w, self.h, -1, -1);
        for y in 0..self.h {
            for x in 0..self.w {
                if at(x, y) > 0 {
                    (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
                }
            }
        }
        if r < 0 {
            return Some(Ink {
                w: 0,
                h: 0,
                coverage: Vec::new(),
            });
        }
        if l == 0 || t == 0 || r == self.w - 1 || b == self.h - 1 {
            return None; // 碰到纸边：这张纸没装下，墨迹是截断过的
        }
        let (w, h) = (r - l + 1, b - t + 1);
        let coverage = (t..=b)
            .flat_map(|y| (l..=r).map(move |x| (x, y)))
            .map(|(x, y)| at(x, y));
        Some(Ink {
            w,
            h,
            coverage: coverage.collect(),
        })
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // SAFETY: 先把原来的位图选回去，DIB 才删得掉（Owned 在这之后 drop）；再删 DC。
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteDC(self.dc);
        }
    }
}
