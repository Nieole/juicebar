//! 像素字形与小位图：每个字是一张位图，按整数倍放大，落在整像素上（设计稿的 `F35`、`F57`、
//! `BOLT`、`LOGO` 与 `fitBitmap` / `drawBitmap`）。位图逐字照抄设计稿。

use super::{Canvas, Rgba};

/// 一套像素字：每个字 `w` × `h` 格，一行一段、段间空格隔开，`1` 是有笔画的格子。
/// 闪电与程序图标也是这样一张位图。
pub(super) struct Font {
    w: i32,
    h: i32,
    glyphs: &'static [(char, &'static str)],
}

/// 一串字放进方框时的放大倍数，与放大后的宽高。
pub(super) struct Fit {
    pub sx: i32,
    pub sy: i32,
    pub w: i32,
    pub h: i32,
}

impl Font {
    /// 一个字的宽，格。
    pub(super) const fn w(&self) -> i32 {
        self.w
    }

    /// 一个字的高，格。
    pub(super) const fn h(&self) -> i32 {
        self.h
    }

    /// 找能塞进 `w` × `h` 的最大整数倍（设计稿的 `fitBitmap`）。字与字之间空一个放大后的格子。
    ///
    /// `tall` 时竖向可以比横向多放大一档——粗块用，数字更高更好认；否则竖向不许比横向小。
    pub(super) fn fit(&self, s: &str, w: i32, h: i32, tall: bool) -> Option<Fit> {
        let n = s.chars().count() as i32;
        for sx in (1..=8).rev() {
            let fw = n * self.w * sx + (n - 1) * sx;
            if fw > w {
                continue;
            }
            let sy = h.div_euclid(self.h).min(if tall { sx + 1 } else { sx });
            if sy < 1 || (!tall && sy < sx) {
                continue;
            }
            return Some(Fit {
                sx,
                sy,
                w: fw,
                h: self.h * sy,
            });
        }
        None
    }

    /// 从 (x, y) 起按 sx × sy 倍画一串字（设计稿的 `drawBitmap`）。字体里没有的字跳过，位置照留。
    pub(super) fn draw(
        &self,
        c: &mut Canvas,
        s: &str,
        (x, y): (i32, i32),
        (sx, sy): (i32, i32),
        color: Rgba,
    ) {
        for (i, ch) in (0..).zip(s.chars()) {
            let Some(&(_, rows)) = self.glyphs.iter().find(|(g, _)| *g == ch) else {
                continue;
            };
            let ox = x + i * (self.w + 1) * sx;
            for (r, row) in (0..).zip(rows.split(' ')) {
                for (col, bit) in (0..).zip(row.bytes()) {
                    if bit == b'1' {
                        c.fill_rect(ox + col * sx, y + r * sy, sx, sy, color);
                    }
                }
            }
        }
    }
}

/// 粗块：3×5。
pub(super) const F35: Font = Font {
    w: 3,
    h: 5,
    glyphs: &[
        ('0', "111 101 101 101 111"),
        ('1', "010 110 010 010 111"),
        ('2', "111 001 111 100 111"),
        ('3', "111 001 111 001 111"),
        ('4', "101 101 111 001 001"),
        ('5', "111 100 111 001 111"),
        ('6', "111 100 111 101 111"),
        ('7', "111 001 001 001 001"),
        ('8', "111 101 111 101 111"),
        ('9', "111 101 111 001 111"),
        ('?', "111 001 010 000 010"),
        ('-', "000 000 111 000 000"),
        ('!', "010 010 010 000 010"),
    ],
};

/// 细体：5×7。
pub(super) const F57: Font = Font {
    w: 5,
    h: 7,
    glyphs: &[
        ('0', "01110 10001 10011 10101 11001 10001 01110"),
        ('1', "00100 01100 00100 00100 00100 00100 01110"),
        ('2', "01110 10001 00001 00010 00100 01000 11111"),
        ('3', "11111 00010 00100 00010 00001 10001 01110"),
        ('4', "00010 00110 01010 10010 11111 00010 00010"),
        ('5', "11111 10000 11110 00001 00001 10001 01110"),
        ('6', "00110 01000 10000 11110 10001 10001 01110"),
        ('7', "11111 00001 00010 00100 01000 01000 01000"),
        ('8', "01110 10001 10001 01110 10001 10001 01110"),
        ('9', "01110 10001 10001 01111 00001 00010 01100"),
        ('?', "01110 10001 00001 00010 00100 00000 00100"),
        ('-', "00000 00000 00000 11111 00000 00000 00000"),
        ('!', "00100 00100 00100 00100 00100 00000 00100"),
    ],
};

/// 程序图标：7×7 的一张位图，当作一个字（[`LOGO_CHAR`]）来放、来画。
pub(super) const LOGO: Font = Font {
    w: 7,
    h: 7,
    glyphs: &[(
        'j',
        "0000011 0000110 1111111 0111110 0111110 0111110 0011100",
    )],
};

/// [`LOGO`] 里唯一的那个字。
pub(super) const LOGO_CHAR: &str = "j";

/// 充电闪电：4×7。
pub(super) const BOLT: Font = Font {
    w: 4,
    h: 7,
    glyphs: &[('b', "0011 0110 1100 1111 0011 0110 1100")],
};

/// [`BOLT`] 里唯一的那个字。
pub(super) const BOLT_CHAR: &str = "b";
