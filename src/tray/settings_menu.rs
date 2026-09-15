//! "图标样式 ›"与"菜单显示 ›"：ADR-0005 的八项设置在右键菜单里的样子（`resident-tray` 票 07，`menu-as-designed`
//! spec「图标样式与菜单显示」），以及点了之后内核做什么。
//!
//! - **"图标样式 ›"**：一级那一行右边不写字。第二层六行，右边写当前值，末尾"恢复默认"（只恢复这六项）。最里一层每个
//!   选项前面挂一张预览（[`Preview`]），当前值那一项加粗（`MFS_DEFAULT`），**不画圆点**：模型照基准给它标上单选（`checked`），
//!   而挂着预览位图的项 Windows 不画圆点（圆点与预览位图普通菜单项画不到一起，
//!   调研 8.3，parking lot Q280），缺省值那一项右边写"默认"，B 电池右边写"16、20 像素不显示数字"。
//! - **预览**：只把这一项换成该选项，其余照用户此刻的设置；电量固定 57，"满电 100"那一组画 100；画哪个图标状态照设计稿
//!   （画法、字形、满电 → 正常；没有读数时 → 无已知值；充电标记 → 充电中；灰状态 → Stale、暂停、取数失败三张并排）；
//!   尺寸是托盘此刻那一档，调色是菜单此刻实际的深浅（[`Tray::menu_theme`]）——预览放在菜单上，就是它在托盘上的样子。
//! - **"菜单显示 ›"**：两个勾选项，勾着就是开着。点了交出的是**点完之后**的那个取值，不是"切换"：同一下送达两次不会
//!   切回去。
//!
//! **点了**：手上那份配置当场换成新的取值，图标照新的样式当场重画（不开新的一轮，[`super::round::Rounds::restyle`]），
//! 再让外壳格式保留地写回 `[tray]` 里那几个键（[`super::config::Action::WriteTray`]）——与"托盘上画哪一台"同一个道理
//! （`super::menu`，parking lot Q252）。菜单显示那两项不在图标上，下一次弹出的菜单照它排。

use crate::config::{PrimaryMark, TraySetting, TraySettings};
use crate::icon::{
    Charging, Full, Glyph, Gray, IconSettings, IconSize, IconState, NoLastKnown, Style, Theme,
};

use super::Tray;
use super::menu::{Command, Entry, Item, Kind, Preview};

/// 预览固定画的电量：预览说的是"这个选项长什么样"，不是"我现在多少电"。
const PREVIEW_PERCENT: u8 = 57;

/// "满电 100"那一组预览画的电量：别的电量下这一项看不出来。
const FULL_PERCENT: u8 = 100;

/// 缺省值那一项右边写的字。
const DEFAULT_MARK: &str = "默认";

/// B 电池那一项右边写的字：它在 16、20 像素上画不下数字（ADR-0005 保留的有争议选项），选之前看得到。
const BATTERY_NOTE: &str = "16、20 像素不显示数字";

/// "图标样式"里的一组：一项设置，连同它的全部选项与预览怎么画。
struct Group<T: 'static> {
    /// 第二层那一行的字。
    title: &'static str,
    /// 每个选项与它在菜单上的字，次序照设计稿。
    options: &'static [(T, &'static str)],
    /// 这项设置在一份图标设置里的取值。
    current: fn(&IconSettings) -> T,
    /// 这个选项点了交出、写回配置的那一项。
    setting: fn(T) -> TraySetting,
    /// 预览画哪几个图标状态，几个就并排几张。
    states: &'static [IconState],
    /// 预览画的电量。
    percent: u8,
}

/// 预览照什么尺寸、什么调色画：托盘此刻那一档，菜单此刻实际的深浅。
#[derive(Clone, Copy)]
struct PreviewLook {
    size: IconSize,
    theme: Theme,
}

impl Tray {
    /// "图标样式 ›"：六组，一条分隔线，"恢复默认"。
    pub(super) fn icon_style_menu(&self) -> Item {
        let settings = &self.config.tray;
        // 预览的尺寸是托盘此刻那一档，调色是菜单此刻实际的深浅（不一定是任务栏的深浅）。
        let drawn_at = PreviewLook {
            size: self.round.look().size,
            theme: self.menu_theme(),
        };
        let mut rows = vec![
            group(
                settings,
                drawn_at,
                &Group {
                    title: "画法",
                    options: &[
                        (Style::Number, "A 纯数字"),
                        (Style::Battery, "B 电池"),
                        (Style::Ring, "C 圆环"),
                        (Style::Bar, "D 数字+底条"),
                    ],
                    current: |icon| icon.style,
                    setting: TraySetting::Style,
                    states: &[IconState::Normal],
                    percent: PREVIEW_PERCENT,
                },
            ),
            group(
                settings,
                drawn_at,
                &Group {
                    title: "字形",
                    options: &[
                        (Glyph::Block, "粗块"),
                        (Glyph::Fine, "细体"),
                        (Glyph::System, "系统字体"),
                    ],
                    current: |icon| icon.glyph,
                    setting: TraySetting::Glyph,
                    states: &[IconState::Normal],
                    percent: PREVIEW_PERCENT,
                },
            ),
            group(
                settings,
                drawn_at,
                &Group {
                    title: "满电 100",
                    options: &[
                        (Full::Digits, "照画 100"),
                        (Full::Cap99, "画成 99"),
                        (Full::Block, "满格符号"),
                    ],
                    current: |icon| icon.full,
                    setting: TraySetting::Full,
                    states: &[IconState::Normal],
                    percent: FULL_PERCENT,
                },
            ),
            group(
                settings,
                drawn_at,
                &Group {
                    title: "灰状态",
                    options: &[
                        (Gray::One, "全部一个灰"),
                        (Gray::Split, "灰数字与灰符号两种"),
                        (Gray::SplitPause, "两种，另给暂停加琥珀点"),
                    ],
                    current: |icon| icon.gray,
                    setting: TraySetting::Gray,
                    states: &[IconState::Stale, IconState::Paused, IconState::FetchFailed],
                    percent: PREVIEW_PERCENT,
                },
            ),
            group(
                settings,
                drawn_at,
                &Group {
                    title: "没有读数时",
                    options: &[
                        (NoLastKnown::Dash, "两道横线 --"),
                        (NoLastKnown::Question, "问号"),
                        (NoLastKnown::Outline, "空的轮廓"),
                        (NoLastKnown::Logo, "程序图标"),
                    ],
                    current: |icon| icon.no_last_known,
                    setting: TraySetting::NoLastKnown,
                    states: &[IconState::NoKnownValue],
                    percent: PREVIEW_PERCENT,
                },
            ),
            group(
                settings,
                drawn_at,
                &Group {
                    title: "充电标记",
                    options: &[
                        (Charging::Color, "只靠绿色"),
                        (Charging::BoltLarge, "24 像素以上加闪电"),
                        (Charging::Bolt, "所有尺寸都加闪电"),
                    ],
                    current: |icon| icon.charging,
                    setting: TraySetting::Charging,
                    states: &[IconState::Charging],
                    percent: PREVIEW_PERCENT,
                },
            ),
        ];
        rows.push(Item::Separator);
        rows.push(Item::Entry(Entry {
            command: Some(Command::RestoreIconDefaults),
            ..Entry::new(Kind::Normal, "恢复默认")
        }));
        Item::Entry(Entry::new(Kind::Submenu(rows), "图标样式"))
    }

    /// "菜单显示 ›"：两个勾选项。
    pub(super) fn menu_display_menu(&self) -> Item {
        let settings = &self.config.tray;
        let source_and_age = settings.menu_source;
        let marks_primary = settings.marks_primary_device_row();
        let rows = vec![
            Item::Entry(Entry {
                checked: source_and_age,
                command: Some(Command::TraySetting(TraySetting::MenuSource(
                    !source_and_age,
                ))),
                ..Entry::new(Kind::Check, "写出来源和多久前")
            }),
            Item::Entry(Entry {
                checked: marks_primary,
                command: Some(Command::TraySetting(TraySetting::PrimaryMark(
                    if marks_primary {
                        PrimaryMark::Radio
                    } else {
                        PrimaryMark::Both
                    },
                ))),
                ..Entry::new(Kind::Check, "在设备列表里标出 Primary Device")
            }),
        ];
        Item::Entry(Entry::new(Kind::Submenu(rows), "菜单显示"))
    }

    /// 菜单里改了 `[tray]` 里的这几项：手上那份配置当场换成它们，图标照新的样式重画，外壳写回配置。
    pub(super) fn on_tray_settings(
        &mut self,
        settings: Vec<TraySetting>,
        out: &mut Vec<super::Action>,
    ) {
        for setting in &settings {
            setting.apply(&mut self.config.tray);
        }
        out.push(super::Action::Config(super::config::Action::WriteTray(
            settings,
        )));
        self.round.restyle(self.config.tray.icon, out);
    }
}

/// 第二层的一行（右边写当前值），连同它最里一层的全部选项。
fn group<T: Copy + PartialEq>(
    settings: &TraySettings,
    PreviewLook { size, theme }: PreviewLook,
    group: &Group<T>,
) -> Item {
    let current = (group.current)(&settings.icon);
    let default = (group.current)(&IconSettings::default());
    let options = group
        .options
        .iter()
        .map(|&(option, text)| {
            let setting = (group.setting)(option);
            let mut previewed = settings.clone();
            setting.apply(&mut previewed);
            let right = if setting == TraySetting::Style(Style::Battery) {
                Some(BATTERY_NOTE)
            } else if option == default {
                Some(DEFAULT_MARK)
            } else {
                None
            };
            Item::Entry(Entry {
                right: right.map(str::to_owned),
                checked: option == current,
                bold: option == current,
                preview: Some(Preview {
                    settings: previewed.icon,
                    states: group.states.to_vec(),
                    percent: group.percent,
                    size,
                    theme,
                }),
                command: Some(Command::TraySetting(setting)),
                ..Entry::new(Kind::Radio, text)
            })
        })
        .collect();
    let current_text = group
        .options
        .iter()
        .find(|(option, _)| *option == current)
        .map(|(_, text)| *text)
        .expect("每一组的选项穷举了这项设置的每一个取值");
    Item::Entry(Entry {
        right: Some(current_text.to_string()),
        ..Entry::new(Kind::Submenu(options), group.title)
    })
}
