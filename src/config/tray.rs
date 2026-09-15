//! `[tray]` 表：ADR-0005 的八项设置——托盘图标怎么画（六项，[`IconSettings`]）、菜单里写什么（两项）。
//!
//! **键与取值是对外契约**（ADR-0005）：发出去之后改名或删掉一个取值，写过它的配置就读不动了。所以键名只在
//! [`TraySetting::key`] 写一次，每个键的取值与它落到类型上的样子在这里各写一张表，读与写（`super::edit::write_tray`）
//! 共用这两样。
//!
//! **认不出的取值按缺省值处理，不让整份配置读不动**，与 `primary` 写错时的处置一致（`crate::primary`）：一个画法
//! 写错了，托盘照样该画、设备照样该读。每一处认不出记一句（[`Unrecognised`]），托盘把它写进日志——不说出来，
//! 用户会以为自己选的画法就长这样。
//!
//! 读不走 serde 的派生：派生遇到认不出的取值只会让整份配置报错。这里先收成一个任意的 TOML 值，再逐键认。

use std::fmt;

use serde::{Deserialize, Deserializer};

use crate::icon::{Charging, Full, Glyph, Gray, IconSettings, NoLastKnown, Style};

/// `[tray]` 表的八项设置。整节缺席时全取缺省值（ADR-0005）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraySettings {
    /// 管图标的六项：`style`、`glyph`、`full`、`gray`、`no_last_known`、`charging`。
    pub icon: IconSettings,
    /// `menu_source`：设备行写不写来源和多久前。缺省 `true`。
    pub menu_source: bool,
    /// `primary_mark`：Primary Device 在菜单里标在哪。
    pub primary_mark: PrimaryMark,
    /// 读这张表时认不出的每一处，按表里的次序。读好了是空的。
    pub unrecognised: Vec<Unrecognised>,
}

/// Primary Device 在菜单里标在哪（`primary_mark`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrimaryMark {
    /// `"both"`：设备行上打勾（这一轮托盘画的是谁），"托盘上画哪一台"里落圆点（你的设置）。
    #[default]
    Both,
    /// `"radio"`：只留"托盘上画哪一台"里的圆点，设备行不打勾。
    Radio,
}

/// `[tray]` 里认不出的一处：一整句话，说清是哪个键、写的是什么、按什么处理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecognised(String);

impl Unrecognised {
    /// `raw` 认不出，按 `fallback`（那个键的缺省值）处理。
    fn value(raw: &toml::Value, fallback: TraySetting) -> Self {
        Self(format!(
            "[tray] 里的 {} = {raw} 认不出，按缺省值 {} 处理",
            fallback.key(),
            fallback.literal()
        ))
    }
}

impl fmt::Display for Unrecognised {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for TraySettings {
    fn default() -> Self {
        Self {
            icon: IconSettings::default(),
            menu_source: true,
            primary_mark: PrimaryMark::default(),
            unrecognised: Vec::new(),
        }
    }
}

/// `style` 的取值。
pub(super) const STYLE: &[(&str, Style)] = &[
    ("number", Style::Number),
    ("battery", Style::Battery),
    ("ring", Style::Ring),
    ("bar", Style::Bar),
];

/// `glyph` 的取值。
pub(super) const GLYPH: &[(&str, Glyph)] = &[
    ("block", Glyph::Block),
    ("fine", Glyph::Fine),
    ("system", Glyph::System),
];

/// `full` 的取值。
pub(super) const FULL: &[(&str, Full)] = &[
    ("digits", Full::Digits),
    ("cap_99", Full::Cap99),
    ("block", Full::Block),
];

/// `gray` 的取值。
pub(super) const GRAY: &[(&str, Gray)] = &[
    ("one", Gray::One),
    ("split", Gray::Split),
    ("split_pause", Gray::SplitPause),
];

/// `no_last_known` 的取值。
pub(super) const NO_LAST_KNOWN: &[(&str, NoLastKnown)] = &[
    ("dash", NoLastKnown::Dash),
    ("question", NoLastKnown::Question),
    ("outline", NoLastKnown::Outline),
    ("logo", NoLastKnown::Logo),
];

/// `charging` 的取值。
pub(super) const CHARGING: &[(&str, Charging)] = &[
    ("color", Charging::Color),
    ("bolt_large", Charging::BoltLarge),
    ("bolt", Charging::Bolt),
];

/// `primary_mark` 的取值。
pub(super) const PRIMARY_MARK: &[(&str, PrimaryMark)] =
    &[("both", PrimaryMark::Both), ("radio", PrimaryMark::Radio)];

/// 这个取值在配置里写成什么。表是穷举的，查不到是这个文件自己的错。
pub(super) fn name_of<T: PartialEq>(values: &[(&'static str, T)], value: &T) -> &'static str {
    values
        .iter()
        .find(|(_, v)| v == value)
        .map(|(name, _)| *name)
        .expect("取值表穷举了这个类型的每一个值")
}

impl TraySettings {
    /// 设备行上打不打 Primary Device 的勾（`primary_mark = "both"`）。
    pub fn marks_primary_device_row(&self) -> bool {
        self.primary_mark == PrimaryMark::Both
    }
}

impl<'de> Deserialize<'de> for TraySettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_toml(&toml::Value::deserialize(deserializer)?))
    }
}

impl TraySettings {
    /// 从 `tray` 那个键底下写着的东西认出八项；认不出的按缺省值，记一句。
    fn from_toml(raw: &toml::Value) -> Self {
        let mut settings = Self::default();
        let Some(table) = raw.as_table() else {
            settings.unrecognised.push(Unrecognised(format!(
                "[tray] 不是一张表（写成了 {raw}），八项都按缺省值处理"
            )));
            return settings;
        };
        let notes = &mut settings.unrecognised;
        let icon = &mut settings.icon;
        icon.style = pick(table, TraySetting::Style, STYLE, icon.style, notes);
        icon.glyph = pick(table, TraySetting::Glyph, GLYPH, icon.glyph, notes);
        icon.full = pick(table, TraySetting::Full, FULL, icon.full, notes);
        icon.gray = pick(table, TraySetting::Gray, GRAY, icon.gray, notes);
        icon.no_last_known = pick(
            table,
            TraySetting::NoLastKnown,
            NO_LAST_KNOWN,
            icon.no_last_known,
            notes,
        );
        icon.charging = pick(table, TraySetting::Charging, CHARGING, icon.charging, notes);
        let menu_source = TraySetting::MenuSource(settings.menu_source);
        settings.menu_source = match table.get(menu_source.key()) {
            None => settings.menu_source,
            Some(toml::Value::Boolean(on)) => *on,
            Some(other) => {
                notes.push(Unrecognised::value(other, menu_source));
                settings.menu_source
            }
        };
        settings.primary_mark = pick(
            table,
            TraySetting::PrimaryMark,
            PRIMARY_MARK,
            settings.primary_mark,
            notes,
        );
        settings
    }
}

/// 表里 `setting` 那个键：没写是缺省值；写了表里有的取值就是它；别的（笔误、类型不对）按缺省值，记一句。
fn pick<T: Copy + PartialEq>(
    table: &toml::Table,
    setting: fn(T) -> TraySetting,
    values: &[(&'static str, T)],
    default: T,
    notes: &mut Vec<Unrecognised>,
) -> T {
    let Some(raw) = table.get(setting(default).key()) else {
        return default;
    };
    let known = raw
        .as_str()
        .and_then(|written| values.iter().find(|(name, _)| *name == written));
    if let Some(&(_, value)) = known {
        return value;
    }
    notes.push(Unrecognised::value(raw, setting(default)));
    default
}

/// `[tray]` 里一个键的一个取值：菜单里点了一项、回写配置里的一个键，说的都是它。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraySetting {
    /// `style`。
    Style(Style),
    /// `glyph`。
    Glyph(Glyph),
    /// `full`。
    Full(Full),
    /// `gray`。
    Gray(Gray),
    /// `no_last_known`。
    NoLastKnown(NoLastKnown),
    /// `charging`。
    Charging(Charging),
    /// `menu_source`。
    MenuSource(bool),
    /// `primary_mark`。
    PrimaryMark(PrimaryMark),
}

/// 一个取值在配置里写成的样子：一个字符串，或者一个布尔。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Literal {
    Word(&'static str),
    Switch(bool),
}

impl fmt::Display for Literal {
    /// 照 TOML 的写法：字符串带引号，布尔不带。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Word(word) => write!(f, "\"{word}\""),
            Self::Switch(on) => write!(f, "{on}"),
        }
    }
}

impl TraySetting {
    /// "恢复默认"：图标样式那六项各自的缺省值（ADR-0005）。"菜单显示"那两项不归它管。
    pub fn icon_defaults() -> [Self; 6] {
        let icon = IconSettings::default();
        [
            Self::Style(icon.style),
            Self::Glyph(icon.glyph),
            Self::Full(icon.full),
            Self::Gray(icon.gray),
            Self::NoLastKnown(icon.no_last_known),
            Self::Charging(icon.charging),
        ]
    }

    /// 它在 `[tray]` 里的键。
    pub fn key(self) -> &'static str {
        match self {
            Self::Style(_) => "style",
            Self::Glyph(_) => "glyph",
            Self::Full(_) => "full",
            Self::Gray(_) => "gray",
            Self::NoLastKnown(_) => "no_last_known",
            Self::Charging(_) => "charging",
            Self::MenuSource(_) => "menu_source",
            Self::PrimaryMark(_) => "primary_mark",
        }
    }

    /// 它在配置里写成什么（读的时候认的是同一张表）。
    pub(super) fn literal(self) -> Literal {
        match self {
            Self::Style(v) => Literal::Word(name_of(STYLE, &v)),
            Self::Glyph(v) => Literal::Word(name_of(GLYPH, &v)),
            Self::Full(v) => Literal::Word(name_of(FULL, &v)),
            Self::Gray(v) => Literal::Word(name_of(GRAY, &v)),
            Self::NoLastKnown(v) => Literal::Word(name_of(NO_LAST_KNOWN, &v)),
            Self::Charging(v) => Literal::Word(name_of(CHARGING, &v)),
            Self::MenuSource(on) => Literal::Switch(on),
            Self::PrimaryMark(v) => Literal::Word(name_of(PRIMARY_MARK, &v)),
        }
    }

    /// 把这一项落到一份设置上，别的项不动。
    pub fn apply(self, settings: &mut TraySettings) {
        match self {
            Self::Style(v) => settings.icon.style = v,
            Self::Glyph(v) => settings.icon.glyph = v,
            Self::Full(v) => settings.icon.full = v,
            Self::Gray(v) => settings.icon.gray = v,
            Self::NoLastKnown(v) => settings.icon.no_last_known = v,
            Self::Charging(v) => settings.icon.charging = v,
            Self::MenuSource(on) => settings.menu_source = on,
            Self::PrimaryMark(v) => settings.primary_mark = v,
        }
    }

    /// 这份设置里这一项是不是正好是它。
    pub(super) fn is_in(self, settings: &TraySettings) -> bool {
        let mut applied = settings.clone();
        self.apply(&mut applied);
        applied == *settings
    }

    /// 它是不是这一项的缺省值：不写这个键时生效的就是它。
    pub(super) fn is_default(self) -> bool {
        self.is_in(&TraySettings::default())
    }
}
