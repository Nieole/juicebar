//! 位图 → 交给 Windows 的字节。
//!
//! 渲染器交出来的是不预乘的 RGBA（[`Rgba`]），逐行从上往下、每行从左往右；Windows 要的是 32 位 BGRA。
//! 这一步原来写在外壳的 unsafe 块里，没有用例，所以搬到这里，外壳只剩把字节拷进 DIB 节的那几行。
//!
//! **不动行序**：第一行的像素在最前，配的 DIB 节 `biHeight` 取负数（自顶向下）。

use super::Rgba;

/// 托盘图标（`CreateIconIndirect` 的颜色位图）用的字节：不预乘的 BGRA，自顶向下。
///
/// `pixels` 逐行从上往下、每行从左往右（[`IconBitmap::pixels`](super::IconBitmap::pixels) 就是这样）；
/// 每个像素交出 B、G、R、A 四个字节，颜色原样照抄，不乘不透明度。
pub fn tray_icon_bytes(pixels: &[Rgba]) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|px| [px.b, px.g, px.r, px.a])
        .collect()
}

/// 菜单预览（`MENUITEMINFO::hbmpItem` 上的 32 位 DIB 节）用的字节：预乘过的 BGRA，自顶向下。
///
/// 主题菜单只对预乘过的 32 位 DIB 节做逐像素的透明混合（`docs/research/native-menu-research.md`）。
/// `pixels` 的排法与 [`tray_icon_bytes`] 一样；宽大于高的位图（几张预览并排）照样收，行里用不着补白。
///
/// **预乘的取整**：每个颜色通道乘上不透明度再除以 255，四舍五入到最近的整数——c × a / 255 的小数部分
/// 永远不会正好是 .5，所以没有平手要定。整数算法是 (c × a + 127) / 255：余数到 128 才进一。
/// 不透明度照抄，所以全透明的像素四个字节都是 0，不透明的像素颜色不变。
pub fn menu_preview_bytes(pixels: &[Rgba]) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|px| {
            let premultiply = |c: u8| ((u16::from(c) * u16::from(px.a) + 127) / 255) as u8;
            [
                premultiply(px.b),
                premultiply(px.g),
                premultiply(px.r),
                px.a,
            ]
        })
        .collect()
}
