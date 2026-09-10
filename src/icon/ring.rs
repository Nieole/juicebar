//! C 圆环的像素：哪些像素属于圆环，按什么顺序被电量填上。
//!
//! **与设计稿的 `ringPixels` 是同一个算法，一步不差。**圆环原来是浏览器用 `ctx.arc` 画的抗锯齿弧，
//! 那种弧没法逐像素对照，这边也照抄不出来；所以两边一起换成这个只用整数的逐像素判定——坐标一律
//! 乘 2，像素中心 (x+½, y+½) 就落在整数上，没有一处浮点或三角函数，两边算出来必然一样。

/// 圆环上的一个像素，连同排序要用的量（都已乘 2）。
struct RingPixel {
    x: i32,
    y: i32,
    /// 像素中心相对圆心的位移，向右、向下为正。
    dx: i32,
    dy: i32,
    /// 到圆心距离的平方。
    d: i32,
    /// 右半圈（含正上方）是 0，左半圈（含正下方）是 1。
    half: i32,
}

/// 圆环占的像素，已按"从 12 点起顺时针"排好：调用方拿前 k 个画电量色，其余画轨道色。
///
/// - 圆环占 `[ax, s)` 那一块，圆心在它正中，外半径是它宽度 `aw` 的一半，线宽 `sw`。
/// - 一个像素属于圆环：它的中心到圆心的距离落在 [外半径 − 线宽, 外半径) 里。
/// - 排序先分左右两半，同一半里比叉积（不求角度），同一条射线上里圈在前。这是一个全序，
///   所以用什么排序算法排出来都一样——设计稿那边是 `Array.prototype.sort`。
pub(super) fn ring_pixels(ax: i32, aw: i32, s: i32, sw: i32) -> Vec<(i32, i32)> {
    let (cx2, cy2) = (2 * ax + aw, s); // 圆心，乘了 2
    let (ro, ri) = (aw, aw - 2 * sw); // 外、内半径，乘了 2
    let mut px = Vec::new();
    for y in 0..s {
        for x in ax..s {
            let (dx, dy) = (2 * x + 1 - cx2, 2 * y + 1 - cy2);
            let d = dx * dx + dy * dy;
            if d >= ri * ri && d < ro * ro {
                let half = if dx > 0 || (dx == 0 && dy < 0) { 0 } else { 1 };
                px.push(RingPixel {
                    x,
                    y,
                    dx,
                    dy,
                    d,
                    half,
                });
            }
        }
    }
    px.sort_by(|a, b| {
        a.half
            .cmp(&b.half)
            .then((a.dy * b.dx - a.dx * b.dy).cmp(&0))
            .then(a.d.cmp(&b.d))
    });
    px.into_iter().map(|p| (p.x, p.y)).collect()
}

/// 电量 `l` 该填上前多少个像素：round(像素数 × max(2, l) / 100)，最低 2%——和原来的弧一样。
/// 整数写法 ⌊(n × m + 50) / 100⌋ 与设计稿逐字相同。
pub(super) fn filled(n: usize, l: i32) -> usize {
    (n * l.max(2) as usize + 50) / 100
}
