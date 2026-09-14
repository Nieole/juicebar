//! 位图交给 Windows 之前的那一步：像素 → 字节。
//!
//! 渲染器画对了，像素还可能在这一步被弄错——通道顺序、预乘与否、行序，哪一处弄反，托盘或菜单上的
//! 颜色与边缘就与设计稿不一样，而 `tests/icon.rs` 照样全绿。外壳（`src/shell/`）不做自动测试，所以
//! 这一步是两个纯函数，用例在这里：托盘图标要不预乘的 BGRA，菜单预览要预乘过的 BGRA，都自顶向下。
//!
//! 期望的字节一律手算好写成字面量，不照实现的算法再算一遍。

use juicebar::icon::{Rgba, menu_preview_bytes, tray_icon_bytes};

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba { r, g, b, a }
}

/// 托盘图标：不透明的像素只换通道顺序，RGBA → BGRA。
#[test]
fn tray_icon_bytes_put_an_opaque_pixel_in_bgra_order() {
    assert_eq!(tray_icon_bytes(&[rgba(10, 20, 30, 255)]), [30, 20, 10, 255]);
}

/// 托盘图标：半透明的像素颜色原样照抄，不乘不透明度（预乘过的话会是 `[1, 50, 100, 128]`）。
#[test]
fn tray_icon_bytes_leave_a_translucent_pixel_unpremultiplied() {
    assert_eq!(
        tray_icon_bytes(&[rgba(200, 100, 1, 128)]),
        [1, 100, 200, 128]
    );
}

/// 托盘图标：全透明的像素也照抄，颜色不清零——不透明度 0 就够 Windows 不画它了。
#[test]
fn tray_icon_bytes_copy_a_fully_transparent_pixel_as_is() {
    assert_eq!(tray_icon_bytes(&[rgba(200, 100, 1, 0)]), [1, 100, 200, 0]);
}

/// 托盘图标：两行两列的位图，第一行在最前，每行从左往右——配自顶向下的 DIB 节。
#[test]
fn tray_icon_bytes_start_with_the_top_row() {
    let pixels = [
        rgba(1, 2, 3, 255),
        rgba(4, 5, 6, 255), // 第一行
        rgba(7, 8, 9, 255),
        rgba(10, 11, 12, 255), // 第二行
    ];
    assert_eq!(
        tray_icon_bytes(&pixels),
        [3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]
    );
}

/// 菜单预览：不透明的像素乘上 255/255 还是自己，只换通道顺序。
#[test]
fn menu_preview_bytes_put_an_opaque_pixel_in_bgra_order() {
    assert_eq!(
        menu_preview_bytes(&[rgba(10, 20, 30, 255)]),
        [30, 20, 10, 255]
    );
}

/// 菜单预览：半透明的像素每个颜色乘上不透明度 / 255，四舍五入到最近的整数。
///
/// - 红 200 × 128 / 255 = 100.39 → 100
/// - 绿 100 × 128 / 255 = 50.20 → 50（向上取整会是 51）
/// - 蓝 1 × 128 / 255 = 0.502 → 1（截断会是 0）
#[test]
fn menu_preview_bytes_premultiply_a_translucent_pixel_rounding_to_nearest() {
    assert_eq!(
        menu_preview_bytes(&[rgba(200, 100, 1, 128)]),
        [1, 50, 100, 128]
    );
}

/// 菜单预览：全透明的像素乘上 0，四个字节都是 0——不管颜色原来是什么。
#[test]
fn menu_preview_bytes_zero_a_fully_transparent_pixel() {
    assert_eq!(menu_preview_bytes(&[rgba(200, 100, 1, 0)]), [0, 0, 0, 0]);
}

/// 菜单预览：三列两行、宽大于高的位图（几张预览并排就是这个形状），第一行在最前，每行从左往右。
#[test]
fn menu_preview_bytes_start_with_the_top_row_of_a_wide_bitmap() {
    let pixels = [
        rgba(1, 2, 3, 255),
        rgba(4, 5, 6, 255),
        rgba(7, 8, 9, 255), // 第一行
        rgba(10, 11, 12, 255),
        rgba(13, 14, 15, 255),
        rgba(16, 17, 18, 255), // 第二行
    ];
    assert_eq!(
        menu_preview_bytes(&pixels),
        [
            3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, // 第一行
            12, 11, 10, 255, 15, 14, 13, 255, 18, 17, 16, 255, // 第二行
        ]
    );
}
