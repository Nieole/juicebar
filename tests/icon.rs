//! 图标渲染器：设置、图标状态、电量、尺寸、调色 → 位图。
//!
//! **像素的唯一真相来源是定稿的设计稿** `.scratch/tray-design/icon-review.html`（ADR-0005）。
//! 这里的基准全部由它自己的渲染器画、从它的画布上读回来——`tests/icon_baselines/` 下每一节是一张
//! 图标：一个像素一个字符，字符对照同目录的 `palette.txt`。改了设计稿就重新导出：
//!
//! ```text
//! node tests/icon_baselines/export.mjs
//! ```
//!
//! **基准不许由 Rust 这边生成**：那样比对就成了自己比自己，永远是绿的。对不上的时候先查是谁画错了，
//! 而不是先改基准（ADR-0005）。
//!
//! 覆盖面照票面：缺省设置下 8 个图标状态 × 4 个尺寸 × 2 种调色；再对每一项设置，从缺省出发只改
//! 那一项、每个取值各一组；外加每个画法一组电量扫描。**不做全组合**。系统字体豁免逐像素，
//! 它的用例只断言结构，在本文件末尾。

use std::fmt::Write as _;

use juicebar::icon::{
    Charging, Full, Glyph, Gray, IconBitmap, IconSettings, IconSize, IconState, NoLastKnown, Rgba,
    Style, Theme, render,
};

/// 显示缩放落在四档之外时取最近的一档；正好落在两档中间时取小的那一档。
///
/// 这条规则照抄设计稿的 `autoSize()`：目标是 16 × 缩放，从 16 / 20 / 24 / 32 里挑差得最少的，
/// 平手时留住先比到的那个（小的）。175% 就是这样的平手：目标 28 像素，24 与 32 各差 4。
#[test]
fn picks_the_nearest_of_the_four_icon_sizes_for_the_display_scale() {
    // 四档本身：100% / 125% / 150% / 200%。
    assert_eq!(IconSize::for_dpi(96), IconSize::Px16);
    assert_eq!(IconSize::for_dpi(120), IconSize::Px20);
    assert_eq!(IconSize::for_dpi(144), IconSize::Px24);
    assert_eq!(IconSize::for_dpi(192), IconSize::Px32);

    // 落在两档之间：取近的那一档。
    assert_eq!(IconSize::for_dpi(156), IconSize::Px24); // 162.5%，目标 26
    assert_eq!(IconSize::for_dpi(180), IconSize::Px32); // 187.5%，目标 30

    // 正好在中间：取小的。
    assert_eq!(IconSize::for_dpi(108), IconSize::Px16); // 112.5%，目标 18
    assert_eq!(IconSize::for_dpi(132), IconSize::Px20); // 137.5%，目标 22
    assert_eq!(IconSize::for_dpi(168), IconSize::Px24); // 175%，目标 28

    // 出了两头：贴着最近的那一档。
    assert_eq!(IconSize::for_dpi(72), IconSize::Px16);
    assert_eq!(IconSize::for_dpi(288), IconSize::Px32);

    assert_eq!(IconSize::Px16.px(), 16);
    assert_eq!(IconSize::Px32.px(), 32);
}

/// 缺省样式（D 数字+底条、粗块字形）下，有数的五个状态：数字始终是最高对比色，状态交给底条。
#[test]
fn the_default_icon_draws_the_percentage_in_block_digits_over_a_bar() {
    assert_matches_design(
        baselines("default")
            .into_iter()
            .filter(|b| b.percent.is_some()),
    );
}

/// 缺省样式下，没有数可画的三个状态各画一个灰符号：取数失败 `!`、Unknown `?`、无已知值 `--`；
/// 底条只剩轨道。
#[test]
fn the_default_icon_draws_a_gray_symbol_where_there_is_no_number() {
    assert_matches_design(
        baselines("default")
            .into_iter()
            .filter(|b| b.percent.is_none()),
    );
}

/// A 纯数字：整个图标就是那个数，颜色就是状态。
#[test]
fn the_number_style_fills_the_icon_with_the_percentage_in_the_state_color() {
    assert_matches_design(baselines("style-number"));
}

/// B 电池：电量是电池里那一格。**16、20 像素不画数字**（设计稿与 ADR-0005 都点明的已知行为）；
/// 24 像素以上画数字，电量那一格于是淡下去，好让数字压在上面看得清。
#[test]
fn the_battery_style_fills_a_cell_and_shows_the_number_only_from_24_pixels() {
    assert_matches_design(baselines("style-battery"));
}

/// C 圆环：从 12 点起顺时针走过电量那一段，数字在环里。
///
/// 圆环原来是浏览器的抗锯齿弧，谁都没法逐像素照抄；设计稿改成了一个只用整数的逐像素判定，
/// Rust 这边是同一个算法——这组基准就是两边真的一致的证据。
#[test]
fn the_ring_style_sweeps_clockwise_from_twelve_with_the_number_inside() {
    assert_matches_design(baselines("style-ring"));
}

/// 细体：5×7 的像素字，横竖同一个倍数。缺省的 D 在四档里都放得下 5×7；放不下时退到 3×5 的
/// 那一支，在方框小的 C 圆环与 B 电池里才走得到，守在电量扫描那一组。
#[test]
fn the_fine_glyph_draws_five_by_seven_digits() {
    assert_matches_design(baselines("glyph-fine"));
}

/// 满电 100：照画三位、画成 99、或者一个满格符号。三组都按电量 100 画——别的电量下这一项
/// 看不出来（设计稿"图标样式"里那张预览也是这么做的）。
#[test]
fn a_full_battery_is_drawn_as_digits_as_99_or_as_a_solid_block() {
    for group in ["full-digits", "full-cap_99", "full-block"] {
        assert_matches_design(baselines(group));
    }
}

/// 灰状态怎么区分："全部一个灰"让取数失败与 Unknown 也画成"没有读数时"那个符号；
/// "两种，另给暂停加琥珀点"在暂停的右上角点一个琥珀色的方块。
#[test]
fn gray_states_collapse_into_one_symbol_or_mark_pause_with_an_amber_dot() {
    for group in ["gray-one", "gray-split_pause"] {
        assert_matches_design(baselines(group));
    }
}

/// 一次读数都没有过时画什么：问号、空的轮廓（A 纯数字里是一个框，别的画法里那一块留空）、
/// 或者程序图标。
#[test]
fn no_known_value_is_drawn_as_a_question_mark_an_outline_or_the_logo() {
    for group in [
        "no_last_known-question",
        "no_last_known-outline",
        "no_last_known-logo",
    ] {
        assert_matches_design(baselines(group));
    }
}

/// 充电中的闪电占掉图标左边一条，画法在剩下的那一块里画——数字因此变窄，这是它的代价。
/// "24 像素以上加闪电"在 16、20 像素上只靠绿色。**D 里闪电只占数字那一块，不压到底条上**
/// （设计稿里已知的行为，照搬）。
#[test]
fn a_charging_bolt_takes_a_strip_on_the_left_and_stays_off_the_bar() {
    for group in ["charging-bolt_large", "charging-bolt"] {
        assert_matches_design(baselines(group));
    }
}

/// 电量扫描：每个画法走一遍电量的两头与中间（0、1、50、99、100）——底条、电池那一格、圆环的
/// 最短一截与满圈、一位数怎么居中。每个画法走四遍，各守一类"从缺省只改一项"碰不到的交叉：
/// 正常；充电中（C 圆环里的满格用状态色，别的画法用数字色）；充电中加闪电（A、B、C 让出左边
/// 一条，C 在 16、20 像素上宽度变成奇数）；细体（C、B 的方框放不下 5×7，退到 3×5）。
#[test]
fn every_style_follows_the_design_from_empty_to_full() {
    assert_matches_design(baselines("sweep"));
}

/// 暂停而且从没读到过：`CONTEXT.md` 说暂停期间显示上次已知值，可这时没有上次已知值。设计稿
/// 没画过这种情况（它的暂停永远带着一个数）；这里画"没有读数时"那个符号，和无已知值一模一样，
/// "另给暂停加琥珀点"时照样点上那个点（parking lot Q120）。
#[test]
fn a_pause_with_no_known_value_draws_the_no_value_symbol_and_keeps_its_dot() {
    // 缺省设置：就是设计稿的无已知值那一张。
    let plain = baselines("default")
        .into_iter()
        .filter(|b| b.state == IconState::NoKnownValue);
    assert_matches_design(plain.map(|b| Baseline {
        origin: format!("{} 当作暂停、不给电量来画", b.origin),
        state: IconState::Paused,
        ..b
    }));

    // "另给暂停加琥珀点"：无已知值那一张，加上设计稿里暂停那一张的琥珀点。
    let split = baselines("gray-split_pause");
    let dotted = split
        .iter()
        .filter(|b| b.state == IconState::NoKnownValue)
        .map(|none| {
            let paused = split
                .iter()
                .find(|b| {
                    b.state == IconState::Paused && (b.size, b.theme) == (none.size, none.theme)
                })
                .expect("同尺寸同调色的暂停那一张");
            let grid = none
                .grid
                .iter()
                .zip(&paused.grid)
                .map(|(n, p)| {
                    n.chars()
                        .zip(p.chars())
                        .map(|(n, p)| if p == 'A' { p } else { n })
                        .collect()
                })
                .collect();
            Baseline {
                origin: format!(
                    "{} 当作暂停、不给电量来画，加上暂停那张的琥珀点",
                    none.origin
                ),
                settings: none.settings,
                state: IconState::Paused,
                percent: None,
                size: none.size,
                theme: none.theme,
                grid,
            }
        });
    assert_matches_design(dotted.collect::<Vec<_>>());
}

/// 电量超过 100 按 100 画，不画出三位以上的数，也不让底条、电池那一格、圆环溢出去。
#[test]
fn a_percentage_above_100_is_drawn_as_100() {
    let full = baselines("sweep")
        .into_iter()
        .filter(|b| b.percent == Some(100));
    assert_matches_design(full.map(|b| Baseline {
        origin: format!("{} 电量给 255 来画", b.origin),
        percent: Some(255),
        ..b
    }));
}

/// 取数失败、Unknown、无已知值没有电量可画：调用方顺手给了一个数，图标也不因此变样。
#[test]
fn the_states_without_a_number_ignore_a_percentage_if_one_is_given() {
    let symbols = baselines("default")
        .into_iter()
        .filter(|b| b.percent.is_none());
    assert_matches_design(symbols.map(|b| Baseline {
        origin: format!("{} 电量给 57 来画", b.origin),
        percent: Some(57),
        ..b
    }));
}

/// 基准的覆盖面就是票面那一句：缺省设置下 8 个图标状态 × 4 个尺寸 × 2 种调色；再对每一项设置，
/// 从缺省出发只改那一项、每个取值各一组。守着它，是因为导出脚本少导一组、少导一种调色，
/// 上面那些用例照样全绿——只是比到的少了。
#[test]
fn the_baselines_cover_every_value_of_every_setting_in_all_states_sizes_and_themes() {
    let mut groups = vec![("default".to_string(), IconSettings::default())];
    for &(key, value, set) in VALUES {
        // 系统字体豁免逐像素（ADR-0005）；"100 怎么画"连缺省值也单独一组，因为缺省那组的电量
        // 不是 100，看不出这一项。
        let is_default = only(set) == IconSettings::default();
        if (key, value) == ("glyph", "system") || (is_default && key != "full") {
            continue;
        }
        groups.push((format!("{key}-{value}"), only(set)));
    }
    for (group, want) in groups {
        let found = baselines(&group);
        for b in &found {
            assert_eq!(b.settings, want, "{} 用的设置不是这一组该用的", b.origin);
        }
        let cells: std::collections::HashSet<_> =
            found.iter().map(|b| (b.state, b.size, b.theme)).collect();
        assert_eq!(
            cells.len(),
            8 * 4 * 2,
            "{group}：该有 8 个状态 × 4 个尺寸 × 2 种调色"
        );
    }
}

// ---------- 基准：读、画、比 ----------

/// 设计稿导出的一张图标，连同画它用的全部输入。
struct Baseline {
    /// 出自哪个文件、哪一节，出错时照原样印出来，好回头去找。
    origin: String,
    settings: IconSettings,
    state: IconState,
    percent: Option<u8>,
    size: IconSize,
    theme: Theme,
    grid: Vec<String>,
}

const BASELINE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/icon_baselines");

/// 读一个基准文件里的每一节。节头是 `= 状态 电量 尺寸 底色 [键=取值 …]`，没写的设置取缺省值。
fn baselines(group: &str) -> Vec<Baseline> {
    let path = format!("{BASELINE_DIR}/{group}.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {path}：{e}"));
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(head) = line.strip_prefix("= ") else {
            continue;
        };
        let tokens: Vec<&str> = head.split_whitespace().collect();
        let [state, percent, size, theme, overrides @ ..] = tokens.as_slice() else {
            panic!("{path}：认不出这一节的节头：{line}");
        };
        let mut settings = IconSettings::default();
        for kv in overrides {
            apply(&mut settings, kv);
        }
        let mut grid = Vec::new();
        while let Some(row) = lines.next_if(|l| !l.is_empty()) {
            grid.push(row.to_string());
        }
        out.push(Baseline {
            origin: format!("tests/icon_baselines/{group}.txt「{head}」"),
            settings,
            state: icon_state(state),
            percent: (*percent != "-").then(|| percent.parse().expect("电量是 0–100 的整数")),
            size: match *size {
                "16" => IconSize::Px16,
                "20" => IconSize::Px20,
                "24" => IconSize::Px24,
                "32" => IconSize::Px32,
                other => panic!("{path}：没有 {other} 像素这一档"),
            },
            theme: match *theme {
                "dark" => Theme::Dark,
                "light" => Theme::Light,
                other => panic!("{path}：没有 {other} 这种调色"),
            },
            grid,
        });
    }
    assert!(!out.is_empty(), "{path} 里一张网格都没有");
    out
}

/// 节头里的状态名就是 `CONTEXT.md`「图标状态」那八个词（设计稿的 `STATES[].name`）。
fn icon_state(name: &str) -> IconState {
    match name {
        "正常" => IconState::Normal,
        "低电" => IconState::Low,
        "充电中" => IconState::Charging,
        "Stale" => IconState::Stale,
        "暂停" => IconState::Paused,
        "取数失败" => IconState::FetchFailed,
        "Unknown" => IconState::Unknown,
        "无已知值" => IconState::NoKnownValue,
        other => panic!("「{other}」不是八个图标状态之一"),
    }
}

/// 一项设置的一个取值：`[tray]` 表里的键与取值（ADR-0005 的对外契约），与它落到类型化设置上的样子。
type Value = (&'static str, &'static str, fn(&mut IconSettings));

/// ADR-0005 管图标的六个键的全部取值。哪个是缺省值不另写：套在缺省设置上、设置不变的那个就是。
const VALUES: &[Value] = &[
    ("style", "number", |s| s.style = Style::Number),
    ("style", "battery", |s| s.style = Style::Battery),
    ("style", "ring", |s| s.style = Style::Ring),
    ("style", "bar", |s| s.style = Style::Bar),
    ("glyph", "block", |s| s.glyph = Glyph::Block),
    ("glyph", "fine", |s| s.glyph = Glyph::Fine),
    ("glyph", "system", |s| s.glyph = Glyph::System),
    ("full", "digits", |s| s.full = Full::Digits),
    ("full", "cap_99", |s| s.full = Full::Cap99),
    ("full", "block", |s| s.full = Full::Block),
    ("gray", "one", |s| s.gray = Gray::One),
    ("gray", "split", |s| s.gray = Gray::Split),
    ("gray", "split_pause", |s| s.gray = Gray::SplitPause),
    ("no_last_known", "dash", |s| {
        s.no_last_known = NoLastKnown::Dash
    }),
    ("no_last_known", "question", |s| {
        s.no_last_known = NoLastKnown::Question
    }),
    ("no_last_known", "outline", |s| {
        s.no_last_known = NoLastKnown::Outline
    }),
    ("no_last_known", "logo", |s| {
        s.no_last_known = NoLastKnown::Logo
    }),
    ("charging", "color", |s| s.charging = Charging::Color),
    ("charging", "bolt_large", |s| {
        s.charging = Charging::BoltLarge
    }),
    ("charging", "bolt", |s| s.charging = Charging::Bolt),
];

/// 节头里的一个 `键=取值`，套到设置上。
fn apply(settings: &mut IconSettings, kv: &str) {
    let (key, value) = kv.split_once('=').expect("设置写成 键=取值");
    let &(_, _, set) = VALUES
        .iter()
        .find(|&&(k, v, _)| (k, v) == (key, value))
        .unwrap_or_else(|| panic!("{kv} 不是 ADR-0005 里的一个键与取值"));
    set(settings);
}

/// 从缺省设置出发只改这一项，得到的设置。
fn only(set: fn(&mut IconSettings)) -> IconSettings {
    let mut settings = IconSettings::default();
    set(&mut settings);
    settings
}

/// `palette.txt`：这种调色下，每个字符是哪个 RGBA（颜色不预乘）。
fn palette(theme: Theme) -> Vec<(char, Rgba)> {
    let path = format!("{BASELINE_DIR}/palette.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {path}：{e}"));
    let want = match theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
    };
    let mut out = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [t, ch, r, g, b, a] = fields.as_slice() else {
            panic!("{path}：认不出这一行：{line}");
        };
        if *t != want {
            continue;
        }
        let n = |s: &str| s.parse::<u8>().expect("颜色分量是 0–255");
        let ch = ch.chars().next().expect("一个字符");
        out.push((
            ch,
            Rgba {
                r: n(r),
                g: n(g),
                b: n(b),
                a: n(a),
            },
        ));
    }
    out
}

/// 把 Rust 画的位图写成与基准同一种文本网格：全透明是 `.`，调色板里没有的颜色是 `?`。
fn grid_of(icon: &IconBitmap, palette: &[(char, Rgba)]) -> Vec<String> {
    let s = icon.size();
    (0..s)
        .map(|y| {
            (0..s)
                .map(|x| {
                    let p = icon.pixel(x, y);
                    if p.a == 0 {
                        return '.';
                    }
                    palette
                        .iter()
                        .find(|(_, c)| *c == p)
                        .map_or('?', |(ch, _)| *ch)
                })
                .collect()
        })
        .collect()
}

/// 每一张都画出来和设计稿比；对不上的，印出两张网格与差异，读得出是哪几个像素。
fn assert_matches_design(cases: impl IntoIterator<Item = Baseline>) {
    const SHOWN: usize = 4;
    let dark = palette(Theme::Dark);
    let light = palette(Theme::Light);
    let mut total = 0;
    let mut failures = Vec::new();
    for b in cases {
        total += 1;
        let icon = render(b.settings, b.state, b.percent, b.size, b.theme);
        let pal = match b.theme {
            Theme::Dark => &dark,
            Theme::Light => &light,
        };
        let got = grid_of(&icon, pal);
        if got != b.grid {
            failures.push(describe(&b, &got, &icon));
        }
    }
    assert!(total > 0, "没有一张基准被比到");
    if failures.is_empty() {
        return;
    }
    let mut msg = format!("{} / {total} 张图标与设计稿对不上。\n\n", failures.len());
    for f in failures.iter().take(SHOWN) {
        msg.push_str(f);
        msg.push('\n');
    }
    if failures.len() > SHOWN {
        let _ = writeln!(msg, "其余 {} 张只列出处：", failures.len() - SHOWN);
        for f in &failures[SHOWN..] {
            let _ = writeln!(msg, "  {}", f.lines().next().unwrap_or_default());
        }
    }
    msg.push_str(
        "\n字符对照 tests/icon_baselines/palette.txt：F 前景 R 红 G 绿 N 灰 A 琥珀 T 轨道，\
         小写是 B 电池里淡了的那一格，. 全透明，? 是调色板里没有的颜色。\n",
    );
    panic!("{msg}");
}

/// 一张对不上的图标：出处、并排的两张网格加一张差异图，再逐个列出不对的像素。
fn describe(b: &Baseline, got: &[String], icon: &IconBitmap) -> String {
    const LISTED: usize = 12;
    let want = &b.grid;
    let rows = want.len().max(got.len());
    let width = want
        .iter()
        .chain(got)
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(0);
    let row = |g: &[String], y: usize| -> Vec<char> {
        g.get(y).map(|r| r.chars().collect()).unwrap_or_default()
    };
    let mut bad = Vec::new();
    for y in 0..rows {
        let (w, g) = (row(want, y), row(got, y));
        for x in 0..width {
            if w.get(x) != g.get(x) {
                bad.push((x, y, w.get(x).copied(), g.get(x).copied()));
            }
        }
    }

    let mut out = String::new();
    let _ = writeln!(out, "{}：{} 个像素不对", b.origin, bad.len());
    let pad = |s: &str| format!("{s:<width$}");
    // “设计稿”三个字在终端里占六列，按字数补空格会歪，所以按列数补。
    let design = format!("设计稿{}", " ".repeat(width.saturating_sub(6)));
    let _ = writeln!(out, "    {design}   {}   差异", pad("Rust"));
    for y in 0..rows {
        let marks: String = (0..width)
            .map(|x| {
                if bad.iter().any(|&(bx, by, ..)| (bx, by) == (x, y)) {
                    'X'
                } else {
                    '.'
                }
            })
            .collect();
        let w = want.get(y).map_or("", String::as_str);
        let g = got.get(y).map_or("", String::as_str);
        let _ = writeln!(out, "    {}   {}   {marks}", pad(w), pad(g));
    }
    let show = |c: Option<char>| c.map_or_else(|| "（没有）".to_string(), String::from);
    for &(x, y, w, g) in bad.iter().take(LISTED) {
        let _ = write!(
            out,
            "    (x={x}, y={y}) 设计稿 {}，Rust {}",
            show(w),
            show(g)
        );
        if g == Some('?') {
            let p = icon.pixel(x as u32, y as u32);
            let _ = write!(out, "（RGBA {} {} {} {}）", p.r, p.g, p.b, p.a);
        }
        out.push('\n');
    }
    if bad.len() > LISTED {
        let _ = writeln!(out, "    ……还有 {} 个", bad.len() - LISTED);
    }
    out
}

// ---------- 系统字体：豁免逐像素，只断言结构 ----------
//
// "系统字体"走 Windows 的文字渲染，设计稿走浏览器的，两边不会逐像素一致（ADR-0005 那条的例外）。
// 所以这几条只问结构：字放得下（只画在数字那个方框里）、居中、没有被截断、确实画了东西。
// 对照物是同一组输入换成粗块字形画出来的那张——它已经与设计稿逐像素对上了。

/// 图标上的一块矩形区域：左上角 (x, y)，宽 w、高 h，像素。
#[derive(Debug, Clone, Copy)]
struct Area {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

const fn area(x: u32, y: u32, w: u32, h: u32) -> Area {
    Area { x, y, w, h }
}

impl Area {
    fn contains(self, x: u32, y: u32) -> bool {
        (self.x..self.x + self.w).contains(&x) && (self.y..self.y + self.h).contains(&y)
    }

    /// 区域里的每个像素，逐行。
    fn cells(self) -> impl Iterator<Item = (u32, u32)> {
        (self.y..self.y + self.h).flat_map(move |y| (self.x..self.x + self.w).map(move |x| (x, y)))
    }

    /// `inner` 在这块区域里左右、上下各留了多少白。
    fn margins(self, inner: Area) -> ((u32, u32), (u32, u32)) {
        let horizontal = (inner.x - self.x, self.x + self.w - (inner.x + inner.w));
        let vertical = (inner.y - self.y, self.y + self.h - (inner.y + inner.h));
        (horizontal, vertical)
    }
}

/// 一张要检查的系统字体图标：输入，数字方框，以及方框里"没有字时"的不透明度。
struct TextCase {
    settings: IconSettings,
    state: IconState,
    percent: u8,
    size: IconSize,
    theme: Theme,
    /// 数字那个方框，照设计稿的几何逐档算出来的。
    rect: Area,
    /// 方框里没有字的地方是什么不透明度：大多是全透明；B 电池里是电量那一格（满电时铺满方框）。
    bg_alpha: u8,
}

const SIZES: [IconSize; 4] = [
    IconSize::Px16,
    IconSize::Px20,
    IconSize::Px24,
    IconSize::Px32,
];
const THEMES: [Theme; 2] = [Theme::Dark, Theme::Light];

fn system(style: Style) -> IconSettings {
    IconSettings {
        style,
        glyph: Glyph::System,
        ..IconSettings::default()
    }
}

/// 数字方框里没有别的东西的那几种：A 纯数字（整个图标）、D 数字+底条（底条以上）、
/// B 电池在 24 像素以上画满电的"100"（方框里铺满淡了的那一格）。C 圆环另算，见 `ring_cases`。
fn text_cases() -> Vec<TextCase> {
    let mut out = Vec::new();
    for size in SIZES {
        let s = size.px();
        let bar_h = match s {
            16 => 13,
            20 => 16,
            24 => 19,
            _ => 26,
        };
        for theme in THEMES {
            let case = |settings, percent, rect, bg_alpha| TextCase {
                settings,
                state: IconState::Normal,
                percent,
                size,
                theme,
                rect,
                bg_alpha,
            };
            out.push(case(system(Style::Number), 57, area(0, 0, s, s), 0));
            out.push(case(system(Style::Bar), 57, area(0, 0, s, bar_h), 0));
            if s >= 24 {
                let battery = IconSettings {
                    full: Full::Digits,
                    ..system(Style::Battery)
                };
                let rect = if s == 24 {
                    area(4, 9, 14, 6)
                } else {
                    area(4, 11, 22, 10)
                };
                out.push(case(battery, 100, rect, 115));
            }
        }
    }
    out
}

/// C 圆环：数字方框的四个角会压到圆环上，所以用低电（圆环是红的、数字是前景色）好把两者分开。
fn ring_cases() -> Vec<TextCase> {
    let mut out = Vec::new();
    for size in SIZES {
        let rect = match size.px() {
            16 => area(3, 3, 10, 10),
            20 => area(4, 4, 12, 12),
            24 => area(5, 5, 14, 14),
            _ => area(6, 6, 20, 20),
        };
        for theme in THEMES {
            out.push(TextCase {
                settings: system(Style::Ring),
                state: IconState::Low,
                percent: 12,
                size,
                theme,
                rect,
                bg_alpha: 0,
            });
        }
    }
    out
}

impl TextCase {
    fn render(&self, glyph: Glyph) -> IconBitmap {
        let settings = IconSettings {
            glyph,
            ..self.settings
        };
        render(
            settings,
            self.state,
            Some(self.percent),
            self.size,
            self.theme,
        )
    }

    fn describe(&self) -> String {
        format!(
            "{:?} {:?} {} {}px {:?}",
            self.settings.style,
            self.state,
            self.percent,
            self.size.px(),
            self.theme
        )
    }

    /// 方框里被字盖到的那一块的外接矩形；方框里一个字都没有就是 `None`。
    ///
    /// "被字盖到"：比方框里的底更不透明。C 圆环的方框角上压着圆环——粗块那张上不是前景色、又不
    /// 透明的格子——那几格跳过。
    fn ink(&self, text: &IconBitmap, block: &IconBitmap) -> Option<Area> {
        let fg = palette(self.theme)
            .into_iter()
            .find_map(|(ch, color)| (ch == 'F').then_some(color))
            .expect("palette.txt 里有前景色 F");
        let covered = self.rect.cells().filter(|&(x, y)| {
            let under = block.pixel(x, y);
            let ring = self.settings.style == Style::Ring && under.a > 0 && under != fg;
            !ring && text.pixel(x, y).a > self.bg_alpha
        });
        let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
        let mut any = false;
        for (x, y) in covered {
            (l, t, r, b) = (l.min(x), t.min(y), r.max(x), b.max(y));
            any = true;
        }
        any.then(|| area(l, t, r - l + 1, b - t + 1))
    }
}

/// 系统字体那一项真的走了 Windows 的文字渲染，而不是换个名字的像素字：方框里的字与粗块那张
/// 不一样，而且字的边上有抗锯齿留下的半透明像素——像素字只有全不透明的笔画。
#[test]
fn the_system_font_is_rendered_text_not_a_pixel_font() {
    for case in text_cases().into_iter().chain(ring_cases()) {
        let text = case.render(Glyph::System);
        let block = case.render(Glyph::Block);
        assert!(
            case.rect
                .cells()
                .any(|(x, y)| text.pixel(x, y) != block.pixel(x, y)),
            "{}：系统字体画出来和粗块一模一样",
            case.describe()
        );
        assert!(
            case.rect.cells().any(|(x, y)| {
                let a = text.pixel(x, y).a;
                a > case.bg_alpha && a < 255 && block.pixel(x, y).a == case.bg_alpha
            }),
            "{}：字的边上没有一个半透明的像素，不像是文字渲染画的",
            case.describe()
        );
    }
}

/// 字放得下：换成系统字体，变的只有数字那个方框里的像素，方框外面——底条、圆环、电池的壳、
/// 闪电——一个像素都不动。
#[test]
fn the_system_font_paints_only_inside_the_number_box() {
    for case in text_cases().into_iter().chain(ring_cases()) {
        let text = case.render(Glyph::System);
        let block = case.render(Glyph::Block);
        let s = case.size.px();
        for (x, y) in area(0, 0, s, s).cells() {
            if !case.rect.contains(x, y) {
                assert_eq!(
                    text.pixel(x, y),
                    block.pixel(x, y),
                    "{}：系统字体画到了数字方框 {:?} 外面的 ({x}, {y})",
                    case.describe(),
                    case.rect
                );
            }
        }
    }
}

/// 确实画了东西，而且居中：方框里有字，字的左右留白、上下留白各自最多差一个像素。
#[test]
fn the_system_font_draws_the_number_centered_in_its_box() {
    for case in text_cases().into_iter().chain(ring_cases()) {
        let text = case.render(Glyph::System);
        let block = case.render(Glyph::Block);
        let Some(ink) = case.ink(&text, &block) else {
            panic!("{}：数字方框里一个字都没画", case.describe());
        };
        let ((left, right), (top, bottom)) = case.rect.margins(ink);
        assert!(
            left.abs_diff(right) <= 1 && top.abs_diff(bottom) <= 1,
            "{}：字没有居中——左右留白 {left} / {right}，上下留白 {top} / {bottom}",
            case.describe()
        );
        assert!(
            ink.w * ink.h >= 12,
            "{}：字只占了 {}×{} 的地方，不像一个数",
            case.describe(),
            ink.w,
            ink.h
        );
    }
}

/// 没有被截断：放不下的数换小一号的字，而不是照原样画、把出界的部分切掉——所以三位的"100"
/// 比两位的"88"矮。截断的实现会把两者画成一样高，再把"100"的两头切掉。
#[test]
fn the_system_font_shrinks_a_number_that_does_not_fit_instead_of_cutting_it_off() {
    for size in SIZES {
        for theme in THEMES {
            let s = size.px();
            let base = TextCase {
                settings: IconSettings {
                    full: Full::Digits,
                    ..system(Style::Number)
                },
                state: IconState::Normal,
                percent: 88,
                size,
                theme,
                rect: area(0, 0, s, s),
                bg_alpha: 0,
            };
            let hundred = TextCase {
                percent: 100,
                ..base
            };
            let height = |case: &TextCase| {
                case.ink(&case.render(Glyph::System), &case.render(Glyph::Block))
                    .unwrap_or_else(|| panic!("{}：一个字都没画", case.describe()))
                    .h
            };
            let (h88, h100) = (height(&base), height(&hundred));
            assert!(
                h100 < h88,
                "{s}px {theme:?}：\"100\" 高 {h100}、\"88\" 高 {h88}——三位数没有换小一号的字"
            );
        }
    }
}
