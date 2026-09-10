//! 托盘图标的画师：图标设置、图标状态、电量、尺寸、调色 → 一张位图（[`render`]）。
//!
//! **像素的唯一真相来源是定稿的设计稿** `.scratch/tray-design/icon-review.html`（ADR-0005）。
//! 像素字形、每个画法在每个尺寸上的几何、调色板，这里一律照它的 `drawIcon` 抄，连结构都照它排，
//! 好让两边能逐段对着读。`tests/icon.rs` 拿设计稿自己导出的文本网格逐像素比对；对不上时先查
//! 是谁画错了，而不是先改设计稿。
//!
//! 两处原本做不到逐像素，处理成了这样：
//! - **C 圆环**原来是浏览器的抗锯齿弧。设计稿已改成一个只用整数的逐像素判定，这边是同一个算法
//!   （`ring.rs`），于是照样逐像素比对。
//! - **系统字体**走 Windows 的文字渲染（`system_font.rs`），豁免逐像素，只断言结构。
//!
//! 纯函数：不碰 Win32 界面、不碰设备，同样的输入画出同样的位图（系统字体那一项取决于本机的字体）。
//! 用它的有两处：托盘图标本身（票 04），与"图标样式"子菜单里每个选项前面的预览（票 07）。
//! 交出去的是不预乘的 RGBA；做 32 位图标（`CreateIconIndirect`）时正是这个样子，塞进菜单的
//! 32 位位图（`MENUITEMINFO::hbmpItem`）要的是预乘过的，那一步归调用方。

mod glyphs;
mod ring;
mod system_font;

use glyphs::{BOLT, BOLT_CHAR, F35, F57, Font, LOGO, LOGO_CHAR};

/// ADR-0005 八项设置里管图标的那六项，也就是右键菜单"图标样式"子菜单里的六行。
///
/// 字段名与取值一一对应 `config.toml` 里 `[tray]` 表的键与取值（ADR-0005 的对外契约）。
/// 读配置、认不出的取值怎么处理归票 07；这里只收已经类型化的值。缺省值就是 ADR-0005 的缺省值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IconSettings {
    /// `style`：图标画法。
    pub style: Style,
    /// `glyph`：数字字形。
    pub glyph: Glyph,
    /// `full`：满电 100 怎么画。
    pub full: Full,
    /// `gray`：灰状态怎么区分。
    pub gray: Gray,
    /// `no_last_known`：一次读数都没有过时画什么。
    pub no_last_known: NoLastKnown,
    /// `charging`：充电中的标记。
    pub charging: Charging,
}

/// 图标画法（`style`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Style {
    /// `"number"`：A 纯数字。数字最大。
    Number,
    /// `"battery"`：B 电池。**16、20 像素不画数字**（ADR-0005 保留的有争议选项）。
    Battery,
    /// `"ring"`：C 圆环。一个量规，数字在环里。
    Ring,
    /// `"bar"`：D 数字+底条。数字始终是最高对比色，状态交给底下的条。
    #[default]
    Bar,
}

/// 数字字形（`glyph`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Glyph {
    /// `"block"`：粗块。3×5 的像素字，竖向可以比横向多放大一档。
    #[default]
    Block,
    /// `"fine"`：细体。5×7 的像素字，放不下就退到 3×5。
    Fine,
    /// `"system"`：系统字体。
    System,
}

/// 满电 100 怎么画（`full`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Full {
    /// `"digits"`：照画三位的 100。
    Digits,
    /// `"cap_99"`：画成 99（ADR-0005 保留的有争议选项）。
    Cap99,
    /// `"block"`：一个满格符号。
    #[default]
    Block,
}

/// 灰状态怎么区分（`gray`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Gray {
    /// `"one"`：全部一个灰。取数失败与 Unknown 也画成"没有读数时"那个符号。
    One,
    /// `"split"`：灰数字与灰符号两种。
    #[default]
    Split,
    /// `"split_pause"`：两种，另给暂停在右上角加一个琥珀点。
    SplitPause,
}

/// 一次读数都没有过时画什么（`no_last_known`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NoLastKnown {
    /// `"dash"`：两道横线 `--`。
    #[default]
    Dash,
    /// `"question"`：问号——会和 Unknown 的 `?` 撞在一起，ADR-0005 接受了这个代价。
    Question,
    /// `"outline"`：空的轮廓（只在 A 纯数字里画出一个框，别的画法里那一块留空）。
    Outline,
    /// `"logo"`：程序图标。
    Logo,
}

/// 充电中的标记（`charging`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Charging {
    /// `"color"`：只靠绿色。
    #[default]
    Color,
    /// `"bolt_large"`：24 像素以上加闪电。
    BoltLarge,
    /// `"bolt"`：所有尺寸都加闪电。
    Bolt,
}

/// 托盘图标为 Primary Device 画出的那一种状态，八选一（`CONTEXT.md`「图标状态」）。
///
/// 状态本身只是一个名字。它画成什么符号、颜色落在数字上还是底条上、要不要闪电与琥珀点，由
/// [`IconSettings`] 决定；每个状态用调色板里的哪种颜色则照设计稿的 `STATES`，由渲染器私下对照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconState {
    /// 正常。
    Normal,
    /// 低电：低于低电量阈值。
    Low,
    /// 充电中。
    Charging,
    /// Stale：这份 Reading 的取得时刻距今已久，不该再当作现状。
    Stale,
    /// 暂停：厂商上位机在跑，我们没去问。
    Paused,
    /// 取数失败。
    FetchFailed,
    /// Unknown：设备答了，但电量字段不可采信。
    Unknown,
    /// 无已知值：从没取到过一份 Reading，也没有上次已知值。
    NoKnownValue,
}

/// 托盘图标的四档尺寸，对应 100% / 125% / 150% / 200% 显示缩放。
///
/// 只有这四档：设计稿逐档审过每一笔落在哪个像素上，别的尺寸没有人看过。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconSize {
    /// 16 像素，100% 缩放。
    Px16,
    /// 20 像素，125% 缩放。
    Px20,
    /// 24 像素，150% 缩放。
    Px24,
    /// 32 像素，200% 缩放。
    Px32,
}

impl IconSize {
    /// 边长，像素。
    pub const fn px(self) -> u32 {
        match self {
            Self::Px16 => 16,
            Self::Px20 => 20,
            Self::Px24 => 24,
            Self::Px32 => 32,
        }
    }

    /// 这个显示缩放（每英寸点数，96 = 100%）该用哪一档。
    ///
    /// 照设计稿的 `autoSize()`：目标是 16 × 缩放，取四档里差得最少的；**正好在两档中间时取小的**
    /// （175% 的目标是 28，落在 24 与 32 正中，取 24）。整数比较，没有浮点：目标 = dpi / 6，
    /// 于是比的是 |6 × 边长 − dpi|。
    pub fn for_dpi(dpi: u32) -> Self {
        let off = |size: Self| (6 * size.px()).abs_diff(dpi);
        [Self::Px16, Self::Px20, Self::Px24, Self::Px32]
            .into_iter()
            .reduce(|best, next| if off(next) < off(best) { next } else { best })
            .expect("四档不是空的")
    }
}

/// 调色：跟着任务栏的深浅色走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Theme {
    /// 深色任务栏。
    Dark,
    /// 浅色任务栏。
    Light,
}

/// 一个像素：红绿蓝与不透明度，各 8 位，**颜色不预乘**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgba {
    /// 红。
    pub r: u8,
    /// 绿。
    pub g: u8,
    /// 蓝。
    pub b: u8,
    /// 不透明度，0 是全透明。
    pub a: u8,
}

/// 画好的图标：边长 × 边长个像素，逐行从上往下、每行从左往右。底是全透明的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconBitmap {
    size: u32,
    pixels: Vec<Rgba>,
}

impl IconBitmap {
    /// 边长，像素。
    pub fn size(&self) -> u32 {
        self.size
    }

    /// (x, y) 那个像素，左上角是 (0, 0)。
    ///
    /// # Panics
    ///
    /// 坐标出了图标。
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        assert!(
            x < self.size && y < self.size,
            "({x}, {y}) 不在 {0}×{0} 的图标里",
            self.size
        );
        self.pixels[(y * self.size + x) as usize]
    }

    /// 全部像素，逐行从上往下、每行从左往右。
    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }
}

/// 画一个图标。
///
/// `percent` 是要画的那个电量百分比（0–100，超出按 100 画）：`sources::level` 已经在 Reported Level
/// 与 Derived Level 之间选定的那一个。图标上只画数，不画它取自哪里——16 像素里放不下，来源由菜单行
/// 与悬停提示写出来（`sources/level.rs`「两个来源必须一路带到界面上」说的是那一处）。取数失败、
/// Unknown、无已知值三个状态没有电量可画，给了也不看。
pub fn render(
    settings: IconSettings,
    state: IconState,
    percent: Option<u8>,
    size: IconSize,
    theme: Theme,
) -> IconBitmap {
    let mut canvas = Canvas::new(size.px() as i32);
    draw_icon(&mut canvas, settings, state, percent, theme.palette());
    canvas.into_bitmap()
}

// ---------- 颜色与状态（设计稿的 PAL 与 STATES） ----------

/// 一种调色下的全部颜色。设计稿 `PAL` 里的 `bg` 与 `grid` 只给放大镜用，不画进图标。
struct Palette {
    fg: Rgba,
    red: Rgba,
    green: Rgba,
    gray: Rgba,
    amber: Rgba,
    /// 底条与圆环没被电量盖住的那一段。设计稿写的是 `rgba(…, .22)` / `rgba(…, .17)`，
    /// 不透明度折成 8 位是 round(.22 × 255) = 56、round(.17 × 255) = 43。
    track: Rgba,
}

/// B 电池在 24 像素以上要把数字压在电量那一格上，那一格于是淡到这个不透明度：
/// 设计稿的 `DIM = 0.45`，折成 8 位是 round(0.45 × 255) = 115。
const DIM_ALPHA: u8 = 115;

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba { r, g, b, a }
}

const DARK: Palette = Palette {
    fg: rgba(0xFF, 0xFF, 0xFF, 255),
    red: rgba(0xFF, 0x5F, 0x57, 255),
    green: rgba(0x4C, 0xD9, 0x8A, 255),
    gray: rgba(0x8E, 0x94, 0x9B, 255),
    amber: rgba(0xF2, 0xB2, 0x33, 255),
    track: rgba(255, 255, 255, 56),
};

const LIGHT: Palette = Palette {
    fg: rgba(0x1C, 0x1C, 0x1C, 255),
    red: rgba(0xD1, 0x24, 0x2F, 255),
    green: rgba(0x1B, 0x8A, 0x3F, 255),
    gray: rgba(0x8A, 0x8F, 0x96, 255),
    amber: rgba(0xB8, 0x7E, 0x00, 255),
    track: rgba(0, 0, 0, 43),
};

impl Theme {
    fn palette(self) -> &'static Palette {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
        }
    }
}

impl IconState {
    /// 这个状态的颜色（设计稿 `STATES[].color`）。
    fn color(self, p: &Palette) -> Rgba {
        match self {
            Self::Normal => p.fg,
            Self::Low => p.red,
            Self::Charging => p.green,
            Self::Stale | Self::Paused | Self::FetchFailed | Self::Unknown | Self::NoKnownValue => {
                p.gray
            }
        }
    }

    /// 是不是灰的那几个：灰状态的数字画成灰色，其余的数字画成前景色。
    ///
    /// "灰状态"是 ADR-0005 给这一组起的名字（`gray` 那一项设置就叫"灰状态怎么区分"）。它是按
    /// 设计稿里的颜色归的组，不是按含义：低电也可能是一个 Stale 的上次已知值，却不在这一组里。
    fn is_gray(self) -> bool {
        !matches!(self, Self::Normal | Self::Low | Self::Charging)
    }
}

/// 画在中间那一块的东西（设计稿 `drawSym` 收的 `txt`）。
#[derive(Debug, Clone)]
enum Mark {
    /// 一串字：电量数字，或 `!` `?` `--`。
    Text(String),
    /// 满格符号：100 画成满格时。
    FullBlock,
    /// 什么都不画——"空的轮廓"。只有 A 纯数字会在这时画一个框。
    Empty,
    /// 程序图标。
    Logo,
}

/// 中间那一块画符号还是画电量。两者互斥：有符号的状态没有电量可画
/// （设计稿：`L = st.text ? null : …`）。
enum Middle {
    Symbol(Mark),
    Percent(i32),
}

fn middle(o: IconSettings, state: IconState, percent: Option<u8>) -> Middle {
    let text = |s: &str| Middle::Symbol(Mark::Text(s.to_string()));
    let no_last_known = || Middle::Symbol(no_last_known_mark(o.no_last_known));
    match state {
        IconState::NoKnownValue => no_last_known(),
        // "全部一个灰"时，取数失败与 Unknown 也用无已知值那个符号。
        IconState::FetchFailed | IconState::Unknown if o.gray == Gray::One => no_last_known(),
        IconState::FetchFailed => text("!"),
        IconState::Unknown => text("?"),
        IconState::Normal
        | IconState::Low
        | IconState::Charging
        | IconState::Stale
        | IconState::Paused => match percent {
            Some(l) => Middle::Percent(i32::from(l.min(100))),
            // 有数的状态手上却没有数。走得到这里的正路只有一条：暂停，而且从没读到过——
            // `CONTEXT.md` 说暂停期间显示上次已知值，可这时没有。设计稿没画过这种情况（它的暂停
            // 永远带着一个数），这里画"没有读数时"那个符号：它说的正是"手上没有数"
            // （parking lot Q120）。其余四个状态按优先顺序本来就意味着手上有数。
            None => no_last_known(),
        },
    }
}

/// "一次读数都没有过时画什么"那个符号。
fn no_last_known_mark(setting: NoLastKnown) -> Mark {
    match setting {
        NoLastKnown::Dash => Mark::Text("--".to_string()),
        NoLastKnown::Question => Mark::Text("?".to_string()),
        NoLastKnown::Outline => Mark::Empty,
        NoLastKnown::Logo => Mark::Logo,
    }
}

/// 电量画成什么：100 按"满电 100 怎么画"的设置，其余照写数字（设计稿的 `levelText`）。
fn percent_text(l: i32, full: Full) -> Mark {
    match full {
        Full::Cap99 if l >= 100 => Mark::Text("99".to_string()),
        Full::Block if l >= 100 => Mark::FullBlock,
        Full::Digits | Full::Cap99 | Full::Block => Mark::Text(l.to_string()),
    }
}

// ---------- 四个画法（设计稿的 drawIcon） ----------

/// Math.round。只用在非负数上，那里它与 `f64::round` 一致（都是四舍五入到远离零的一侧）。
fn round(v: f64) -> i32 {
    v.round() as i32
}

/// 一个放字的方框。
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    /// 一块 `w` × `h` 的东西居中放进方框时的左上角。放不居中的那一个像素给右边、下边
    /// （设计稿的 `box.x + Math.floor((box.w - f.w) / 2)`）。
    fn center(self, w: i32, h: i32) -> (i32, i32) {
        (
            self.x + (self.w - w).div_euclid(2),
            self.y + (self.h - h).div_euclid(2),
        )
    }
}

/// 设计稿的 `drawIcon`，一段一段对着抄：先定中间画什么，再画充电闪电，再画四个画法之一，
/// 最后是暂停的琥珀点。
fn draw_icon(c: &mut Canvas, o: IconSettings, state: IconState, percent: Option<u8>, p: &Palette) {
    let s = c.size;
    let col = state.color(p);
    let u = 1.max(round(f64::from(s) / 16.0)); // 一个“笔画单位”，16 像素时是 1
    let (l, txt) = match middle(o, state, percent) {
        Middle::Symbol(m) => (None, m),
        Middle::Percent(l) => (Some(l), percent_text(l, o.full)),
    };

    // 充电闪电占掉左边一条，画法在剩下的 [ax, s) 里画——数字因此变窄，这是它的代价
    let bh = 2.max(round(f64::from(s) / 8.0)); // D 底条的高度
    let mut ax = 0;
    let bolt = match o.charging {
        Charging::Color => false,
        Charging::BoltLarge => s >= 24,
        Charging::Bolt => true,
    };
    if state == IconState::Charging && bolt {
        let hb = if o.style == Style::Bar { s - bh - u } else { s }; // D 里闪电只占数字那一块，别压到底条上
        let (sx, sy) = (
            1.max(round(f64::from(s) / 16.0)),
            1.max(hb.div_euclid(BOLT.h())),
        );
        BOLT.draw(
            c,
            BOLT_CHAR,
            (0, (hb - BOLT.h() * sy).div_euclid(2)),
            (sx, sy),
            p.green,
        );
        ax = BOLT.w() * sx + u;
    }
    let aw = s - ax;

    match o.style {
        Style::Number => {
            if let Mark::Empty = txt {
                // 空的轮廓
                c.fill_rect(ax + u, u, aw - 2 * u, u, col);
                c.fill_rect(ax + u, s - 2 * u, aw - 2 * u, u, col);
                c.fill_rect(ax + u, u, u, s - 2 * u, col);
                c.fill_rect(s - 2 * u, u, u, s - 2 * u, col);
            } else {
                let rect = Rect {
                    x: ax,
                    y: 0,
                    w: aw,
                    h: s,
                };
                draw_sym(c, &txt, rect, col, o.glyph);
            }
        }
        Style::Bar => {
            let rect = Rect {
                x: ax,
                y: 0,
                w: aw,
                h: s - bh - u,
            };
            let num_col = if state.is_gray() { p.gray } else { p.fg }; // 数字始终最高对比，状态交给底条
            draw_sym(c, &txt, rect, num_col, o.glyph);
            c.fill_rect(0, s - bh, s, bh, p.track);
            if let Some(l) = l {
                let w = u.max(round(f64::from(s) * f64::from(l) / 100.0));
                c.fill_rect(0, s - bh, w, bh, col);
            }
        }
        Style::Battery => {
            let (nub, bw) = (u, u);
            let (bx, by) = (ax, round(f64::from(s) * 0.22));
            let (bwid, bht) = (aw - nub, s - 2 * by);
            let edge = col; // 设计稿写的是 `st.color === 'fg' ? P.fg : col`，两支是同一个颜色
            c.fill_rect(bx, by, bwid, bw, edge);
            c.fill_rect(bx, by + bht - bw, bwid, bw, edge);
            c.fill_rect(bx, by, bw, bht, edge);
            c.fill_rect(bx + bwid - bw, by, bw, bht, edge);
            let nh = 2.max(round(f64::from(bht) / 2.0));
            c.fill_rect(
                bx + bwid,
                by + round(f64::from(bht - nh) / 2.0),
                nub,
                nh,
                edge,
            );
            let inner = Rect {
                x: bx + bw + u,
                y: by + bw + u,
                w: bwid - 2 * bw - 2 * u,
                h: bht - 2 * bw - 2 * u,
            };
            let show_num = s >= 24;
            if let Some(l) = l {
                let fill = if show_num {
                    Rgba {
                        a: DIM_ALPHA,
                        ..col
                    }
                } else {
                    col
                };
                let w = u.max(round(f64::from(inner.w) * f64::from(l) / 100.0));
                c.fill_rect(inner.x, inner.y, w, inner.h, fill);
            }
            match l {
                None => draw_sym(c, &txt, inner, col, o.glyph),
                Some(_) if show_num => draw_sym(c, &txt, inner, p.fg, o.glyph),
                Some(_) => {}
            }
        }
        Style::Ring => {
            let sw = 2.max(round(f64::from(s) / 8.0));
            let pixels = ring::ring_pixels(ax, aw, s, sw); // 已按顺时针排好
            let k = l.map_or(0, |l| ring::filled(pixels.len(), l));
            for (i, &(x, y)) in pixels.iter().enumerate() {
                c.fill_rect(x, y, 1, 1, if i < k { col } else { p.track });
            }
            let pad = sw + u;
            let rect = Rect {
                x: ax + pad,
                y: round(f64::from(s - aw) / 2.0) + pad,
                w: aw - 2 * pad,
                h: aw - 2 * pad,
            };
            let num_col = if state.is_gray() { p.gray } else { p.fg };
            // 环里的满格用状态色，数字与符号用数字色
            let color = if let Mark::FullBlock = txt {
                col
            } else {
                num_col
            };
            draw_sym(c, &txt, rect, color, o.glyph);
        }
    }

    if state == IconState::Paused && o.gray == Gray::SplitPause {
        // 暂停：右上角一个琥珀点
        let d = 2.max(round(f64::from(s) / 7.0));
        c.fill_rect(s - d, 0, d, d, p.amber);
    }
}

/// 画中间那一块。
fn draw_sym(c: &mut Canvas, txt: &Mark, rect: Rect, color: Rgba, glyph: Glyph) {
    match txt {
        Mark::Text(s) => draw_text(c, s, rect, color, glyph),
        Mark::FullBlock => {
            let i = 1.max(round(f64::from(rect.w.min(rect.h)) * 0.12));
            c.fill_rect(
                rect.x + i,
                rect.y + i,
                rect.w - 2 * i,
                rect.h - 2 * i,
                color,
            );
        }
        Mark::Empty => {}
        Mark::Logo => {
            if let Some(f) = LOGO.fit(LOGO_CHAR, rect.w, rect.h, false) {
                LOGO.draw(c, LOGO_CHAR, rect.center(f.w, f.h), (f.sx, f.sy), color);
            }
        }
    }
}

/// 在方框里居中画一串字；按字形选项挑字体，放不下就退到更小的字体。
fn draw_text(c: &mut Canvas, s: &str, rect: Rect, color: Rgba, glyph: Glyph) {
    let order: &[(&Font, bool)] = match glyph {
        Glyph::Fine => &[(&F57, false), (&F35, false)],
        Glyph::Block => &[(&F35, true)],
        Glyph::System => return system_font::draw(c, s, rect, color),
    };
    for &(font, tall) in order {
        if let Some(f) = font.fit(s, rect.w, rect.h, tall) {
            font.draw(c, s, rect.center(f.w, f.h), (f.sx, f.sy), color);
            return;
        }
    }
}

// ---------- 画布 ----------

/// 一块正方形画布，行为照 canvas 2D 的 `fillRect`：整数坐标、不抗锯齿、盖上去按不透明度叠。
struct Canvas {
    size: i32,
    pixels: Vec<Rgba>,
}

impl Canvas {
    fn new(size: i32) -> Self {
        Self {
            size,
            pixels: vec![Rgba::default(); (size * size) as usize],
        }
    }

    /// 填一个矩形。宽高是负数时照 canvas 的规矩往反方向铺；出了画布的部分剪掉。
    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgba) {
        let (x0, x1) = if w < 0 { (x + w, x) } else { (x, x + w) };
        let (y0, y1) = if h < 0 { (y + h, y) } else { (y, y + h) };
        for py in y0.max(0)..y1.min(self.size) {
            for px in x0.max(0)..x1.min(self.size) {
                let i = (py * self.size + px) as usize;
                self.pixels[i] = over(color, self.pixels[i]);
            }
        }
    }

    fn into_bitmap(self) -> IconBitmap {
        IconBitmap {
            size: self.size as u32,
            pixels: self.pixels,
        }
    }
}

/// 把 `src` 盖在 `dst` 上（canvas 的 source-over），颜色不预乘。
///
/// 两种最常见的情形——不透明的盖上去、盖在全透明上——结果就是 `src` 本身，一位不差：
/// 基准逐像素比的颜色全落在这两种情形里。
fn over(src: Rgba, dst: Rgba) -> Rgba {
    if src.a == 255 || dst.a == 0 {
        return src;
    }
    if src.a == 0 {
        return dst;
    }
    let sa = f64::from(src.a) / 255.0;
    let da = f64::from(dst.a) / 255.0 * (1.0 - sa);
    let oa = sa + da;
    let mix = |s: u8, d: u8| ((f64::from(s) * sa + f64::from(d) * da) / oa).round() as u8;
    Rgba {
        r: mix(src.r, dst.r),
        g: mix(src.g, dst.g),
        b: mix(src.b, dst.b),
        a: (oa * 255.0).round() as u8,
    }
}
