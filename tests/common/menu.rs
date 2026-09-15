//! 菜单那几份用例共用的：设计稿导出的菜单基准（`tests/menu_baselines/`）怎么读、菜单模型怎么排成同一种文本、
//! 对不上时怎么打印差异，以及基准用的那份罐装配置与这一轮（`tests/menu_baselines/canned.txt`）。
//!
//! 基准的语法写在每份基准的文件头里，这里照它排。一级那几份不展开子菜单，子菜单各有一份（parking lot Q281），
//! 所以排文本时选展开不展开（[`Depth`]）。设备行、告警行、"一台都没有"那一句的措辞归代码，基准里是一整个占位
//! （parking lot Q283）：模型里那一项带着它在基准里的占位（`Entry::placeholder`），排文本时用占位。
//!
//! 每张做菜单的票（06、07、12，`menu-as-designed` 07）都这样比：
//!
//! ```ignore
//! let (tray, _screen) = canned("primary = \"lowest\"");
//! let menu = tray.menu(NOW);
//! assert_matches_design(baseline("primary"), submenu(&menu, "托盘上画哪一台"), Depth::Whole);
//! assert_matches_design(baseline("top").without(NOT_BUILT_YET), &menu.items, Depth::TopLevel);
//! ```

use std::fmt::Write;

use juicebar::endpoints::EndpointKind;
use juicebar::icon::{Charging, Full, Glyph, Gray, IconSettings, NoLastKnown, Style, Theme};
use juicebar::readout::{FailureCause, NoReading};
use juicebar::round::InHand;
use juicebar::state::LastKnown;
use juicebar::tray::Tray;
use juicebar::tray::menu::{Entry, Item, Kind, Menu, Preview};

use super::NOW;
use super::tray::{Screen, feed, fetched, just_read, start};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/menu_baselines");

/// `canned.txt` 里的那两台：都没有蓝牙地址，各配一条 Dongle24G。
pub const CANNED_DEVICES: &str = r#"
[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0
"#;

/// 罐装：`[general]` 里写着 `general`（从罐装出发改的那一项，缺省就写 `primary = "lowest"`），两台照
/// `canned.txt` 各有了这一次取数的结果——dragonfly3 在 `NOW` 读到 57%、来自 Dongle24G；neon75 失联、没有
/// 上次已知值。没有挂着的告警。
pub fn canned(general: &str) -> (Tray, Screen) {
    let config = format!("[general]\n{general}\n{CANNED_DEVICES}");
    let (mut tray, mut screen) = start(&config, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 57, NOW),
        ),
    );
    feed(
        &mut tray,
        &mut screen,
        fetched(
            "neon75",
            NOW,
            InHand::NoReading(NoReading::failed(
                FailureCause::Unreachable,
                "读不到 —— 用例里的原因",
            )),
        ),
    );
    (tray, screen)
}

/// 同 [`canned`]（`primary = "lowest"`），外加 `[tray]` 表里写着 `tray`：从罐装出发改的那几行，一行一个 `键 = 取值`。
pub fn canned_with_tray(tray: &str) -> (Tray, Screen) {
    canned(&format!("primary = \"lowest\"\n\n[tray]\n{tray}"))
}

/// 菜单基准里"从罐装出发只改一项"的那几份：文件名是 `<prefix>-<键>-<取值>.txt`（设计稿导出的命名）。交出每一份的
/// 基准名与它在 `[tray]` 表里的那一行（布尔不加引号），按基准名排好。从目录里列而不是手抄名单：设计稿多导一份，
/// 这里就多比一份。
pub fn one_value_baselines(prefix: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = std::fs::read_dir(DIR)
        .unwrap_or_else(|e| panic!("读不到 {DIR}：{e}"))
        .filter_map(|entry| {
            let file = entry.ok()?.file_name().into_string().ok()?;
            let name = file.strip_suffix(".txt")?;
            let (key, value) = name
                .strip_prefix(prefix)?
                .strip_prefix('-')?
                .split_once('-')?;
            let value = match value {
                "true" | "false" => value.to_string(),
                word => format!("\"{word}\""),
            };
            Some((name.to_string(), format!("{key} = {value}")))
        })
        .collect();
    found.sort();
    found
}

/// 排成文本时子菜单展不展开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// 只排这一层：子菜单那一行下面不跟内容（一级那几份基准，parking lot Q281）。
    TopLevel,
    /// 连同子菜单的内容一层层排下去，每深一层多缩进两个空格。
    Whole,
}

/// 一份菜单基准：去掉文件头的注释与空行之后，一项一行。
pub struct Baseline {
    name: String,
    lines: Vec<String>,
}

/// 读 `tests/menu_baselines/<name>.txt`。
pub fn baseline(name: &str) -> Baseline {
    let path = format!("{DIR}/{name}.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {path}：{e}"));
    let lines = text
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    Baseline {
        name: name.to_string(),
        lines,
    }
}

impl Baseline {
    /// 比对时略去这几行（各略去第一次出现的那一行）：一级菜单里归别的票、还没做出来的那几项。那张票做出来了，
    /// 就把它那一行从名单里拿掉；一个都不略去的整份比对归 `menu-as-designed` 07。
    ///
    /// 每一行都得真在基准里：略去一行基准里没有的，多半是设计稿改了而名单没跟上，那就当场说出来。
    pub fn without(mut self, lines: &[&str]) -> Self {
        for line in lines {
            let at = self
                .lines
                .iter()
                .position(|kept| kept == line)
                .unwrap_or_else(|| {
                    panic!(
                        "tests/menu_baselines/{}.txt 里没有要略去的这一行：{line}",
                        self.name
                    )
                });
            self.lines.remove(at);
        }
        self
    }
}

/// 这几项排成基准的文本格式，一项一行。
pub fn baseline_text(items: &[Item], depth: Depth) -> Vec<String> {
    let mut lines = Vec::new();
    push_lines(items, depth, 0, &mut lines);
    lines
}

/// 菜单模型与基准逐项比对；对不上就把两份文本的差异打印出来。
pub fn assert_matches_design(baseline: Baseline, items: &[Item], depth: Depth) {
    let got = baseline_text(items, depth);
    if got == baseline.lines {
        return;
    }
    panic!(
        "菜单模型与设计稿导出的基准对不上：tests/menu_baselines/{}.txt\n行首 - 是基准里有、菜单模型里没有的，+ 是菜单模型里多出来的：\n\n{}\n先查是菜单模型错了，还是设计稿该改（改设计稿，再跑 node tests/menu_baselines/export.mjs，别手改基准）。\n",
        baseline.name,
        diff(&baseline.lines, &got)
    );
}

/// 一级菜单里文字是 `text` 的那个子菜单，当成只有它一项的一张菜单交出来：子菜单那几份基准的第一行就是它在一级
/// 里的那一行。
pub fn submenu<'a>(menu: &'a Menu, text: &str) -> &'a [Item] {
    let at = menu
        .items
        .iter()
        .position(|item| {
            matches!(item, Item::Entry(entry) if matches!(entry.kind, Kind::Submenu(_)) && entry.text == text)
        })
        .unwrap_or_else(|| panic!("一级菜单里没有「{text}」这个子菜单"));
    std::slice::from_ref(&menu.items[at])
}

/// 一级菜单里"托盘上画哪一台"那一项，连同它子菜单里的几项。
pub fn which_device(menu: &Menu) -> (&Entry, &[Item]) {
    let [Item::Entry(which)] = submenu(menu, "托盘上画哪一台") else {
        panic!("「托盘上画哪一台」是一个 Entry");
    };
    let Kind::Submenu(choices) = &which.kind else {
        panic!("「托盘上画哪一台」带着子菜单");
    };
    (which, choices)
}

/// 一级菜单里名字是 `name` 的那一行设备行（左边那段字就是名字，或者名字隔两个空格接着中段）。
pub fn device_row<'a>(menu: &'a Menu, name: &str) -> &'a Entry {
    let with_middle = format!("{name}  ");
    menu.items
        .iter()
        .find_map(|item| match item {
            Item::Entry(entry)
                if matches!(entry.kind, Kind::Normal)
                    && (entry.text == name || entry.text.starts_with(&with_middle)) =>
            {
                Some(entry)
            }
            Item::Entry(_) | Item::Separator => None,
        })
        .unwrap_or_else(|| panic!("一级菜单里没有「{name}」那一行设备行"))
}

fn push_lines(items: &[Item], depth: Depth, level: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(level);
    for item in items {
        match item {
            Item::Separator => lines.push(format!("{indent}分隔线")),
            Item::Entry(entry) => {
                lines.push(format!("{indent}{}", entry_line(entry)));
                if let (Kind::Submenu(children), Depth::Whole) = (&entry.kind, depth) {
                    push_lines(children, depth, level + 1, lines);
                }
            }
        }
    }
}

/// 一项：种类；标记（勾或圆点、粗、灰）；「文字」；右列；预览。措辞归代码的那几项写占位。
fn entry_line(entry: &Entry) -> String {
    let mut line = match entry.kind {
        Kind::Normal => "普通",
        Kind::Check => "勾选",
        Kind::Radio => "单选",
        Kind::Submenu(_) => "子菜单",
    }
    .to_string();
    if entry.checked {
        line.push_str(match entry.kind {
            Kind::Radio => " 圆点",
            Kind::Normal | Kind::Check | Kind::Submenu(_) => " 勾",
        });
    }
    if entry.bold {
        line.push_str(" 粗");
    }
    if entry.grayed {
        line.push_str(" 灰");
    }
    let (text, right) = match &entry.placeholder {
        Some(placeholder) => (placeholder.text.as_str(), placeholder.right.as_deref()),
        None => (entry.text.as_str(), entry.right.as_deref()),
    };
    let _ = write!(line, " 「{text}」");
    if let Some(right) = right {
        let _ = write!(line, " 右列「{right}」");
    }
    if let Some(preview) = &entry.preview {
        let _ = write!(line, " {}", preview_text(preview));
    }
    line
}

/// `预览(状态 电量 尺寸 底色 [键=取值 …])`：几张并排时状态用 + 连起来；键=取值只写与缺省不同的，次序照 `[tray]` 表。
fn preview_text(preview: &Preview) -> String {
    let states: Vec<String> = preview.states.iter().map(ToString::to_string).collect();
    let theme = match preview.theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
    };
    let mut text = format!(
        "预览({} {} {} {theme}",
        states.join("+"),
        preview.percent,
        preview.size.px()
    );
    let (settings, default) = (preview.settings, IconSettings::default());
    let overrides = [
        (
            settings.style != default.style,
            "style",
            match settings.style {
                Style::Number => "number",
                Style::Battery => "battery",
                Style::Ring => "ring",
                Style::Bar => "bar",
            },
        ),
        (
            settings.glyph != default.glyph,
            "glyph",
            match settings.glyph {
                Glyph::Block => "block",
                Glyph::Fine => "fine",
                Glyph::System => "system",
            },
        ),
        (
            settings.full != default.full,
            "full",
            match settings.full {
                Full::Digits => "digits",
                Full::Cap99 => "cap_99",
                Full::Block => "block",
            },
        ),
        (
            settings.gray != default.gray,
            "gray",
            match settings.gray {
                Gray::One => "one",
                Gray::Split => "split",
                Gray::SplitPause => "split_pause",
            },
        ),
        (
            settings.no_last_known != default.no_last_known,
            "no_last_known",
            match settings.no_last_known {
                NoLastKnown::Dash => "dash",
                NoLastKnown::Question => "question",
                NoLastKnown::Outline => "outline",
                NoLastKnown::Logo => "logo",
            },
        ),
        (
            settings.charging != default.charging,
            "charging",
            match settings.charging {
                Charging::Color => "color",
                Charging::BoltLarge => "bolt_large",
                Charging::Bolt => "bolt",
            },
        ),
    ];
    for (changed, key, value) in overrides {
        if changed {
            let _ = write!(text, " {key}={value}");
        }
    }
    text.push(')');
    text
}

/// 两份文本逐行的差异（最长公共子序列）：相同的行前面两个空格，基准独有的 `- `，菜单模型独有的 `+ `。
fn diff(want: &[String], got: &[String]) -> String {
    let (n, m) = (want.len(), got.len());
    let mut common = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if want[i] == got[j] {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = String::new();
    while i < n || j < m {
        if i < n && j < m && want[i] == got[j] {
            let _ = writeln!(out, "  {}", want[i]);
            i += 1;
            j += 1;
        } else if j < m && (i == n || common[i][j + 1] >= common[i + 1][j]) {
            let _ = writeln!(out, "+ {}", got[j]);
            j += 1;
        } else {
            let _ = writeln!(out, "- {}", want[i]);
            i += 1;
        }
    }
    out
}
