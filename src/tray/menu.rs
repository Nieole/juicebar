//! 菜单：右键菜单上有什么，点了之后内核做什么。
//!
//! **菜单模型在内核里**（ADR-0006），外壳只把它画成普通原生菜单（`shell/menu.rs`），不 owner-draw、不做决定：它在
//! 弹出之前来问 [`Tray::menu`]，拿到的是此刻的样子。模型描述的是**内容**——每一项的种类、文字、勾或圆点、右列、
//! 变不变灰、加不加粗、挂什么预览、点了收到什么（[`Entry`]）；字体、行高、高亮归 Windows。内容由设计稿导出的菜单
//! 基准锁住（`tests/menu_baselines/`）：用例把模型排成同一种文本逐项比对。
//!
//! 一级自上而下（`menu-as-designed` spec「菜单自上而下」）：告警（有才出现）→ 设备行（`super::device_row`）→
//! "托盘上画哪一台 ›" → "图标样式 ›" → "菜单显示 ›"（这两个在 `super::settings_menu`）→ "打开配置文件" → "退出"，
//! 分隔线照设计稿。"登记设备 ›"归票 11、12，"开机自启"归票 13。

use crate::clock::Timestamp;
use crate::config::Config;
use crate::config::TraySetting;
use crate::icon::{IconSettings, IconSize, IconState, Theme};
use crate::primary::PrimaryRule;
use crate::round::Warning;

use super::device_row::{self, RowDisplay};
use super::{Tray, hover};

/// "打开配置文件"那一项写着的字。首次运行的通知叫用户去点它（`super::config`），两处说的得是同一个名字。
pub(super) const OPEN_CONFIG_FILE: &str = "打开配置文件";

/// "托盘上画哪一台"子菜单那一行的字。
const WHICH_DEVICE: &str = "托盘上画哪一台";

/// `primary = "lowest"` 在菜单上的说法：子菜单里那个单选，也是子菜单右列写着的用户的选择。
const AUTOMATIC: &str = "自动（电量最低）";

/// 右键菜单，自上而下。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<Item>,
}

/// 菜单上的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// 分隔线。
    Separator,
    /// 除分隔线以外的每一项。
    Entry(Entry),
}

/// 除分隔线以外的一项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 普通、勾选、单选，还是子菜单（连同子菜单里的几项）。
    pub kind: Kind,
    /// 左边那段字。
    pub text: String,
    /// 右列：普通项自带的那一列，贴右对齐；带子菜单的项紧挨着箭头（调研第 8 节）。一项只有一个右列。
    pub right: Option<String>,
    /// 标记：勾选项与普通项上是勾，单选项上是圆点。普通项上的勾只标不点（设备行上 Primary Device 那个）。
    pub checked: bool,
    /// 加粗：最里一层的当前项（票 07，`MFS_DEFAULT`）。
    pub bold: bool,
    /// 变灰（禁用的菜单项）。
    pub grayed: bool,
    /// 挂在这一项前面的预览（票 07）。
    pub preview: Option<Preview>,
    /// 点了它，内核收到什么。`None` = 点了什么都不收（带子菜单的项、设备行、说明）。
    pub command: Option<Command>,
    /// 措辞归代码的这一项，在设计稿基准里写成的占位；设计稿写死了字的项是 `None`。
    pub placeholder: Option<Placeholder>,
}

/// 一项的种类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// 普通项。
    Normal,
    /// 勾选项：点一下切换勾。
    Check,
    /// 单选项：同一张子菜单里圆点落在一项上。
    Radio,
    /// 带子菜单的项，连同子菜单里的几项。
    Submenu(Vec<Item>),
}

/// 措辞归代码的一项在设计稿基准里的样子（parking lot Q283）：设备行、告警行、"一台 Device 都没有"那一句，设计稿
/// 只写它是哪一种情形（`〔…〕`），确切的字由内核的用例断言。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// 左边那段字的占位。
    pub text: String,
    /// 右列的占位；没有右列是 `None`。
    pub right: Option<String>,
}

/// 挂在菜单项前面的一张预览（票 07）：按这组参数画，几张并排时 `states` 不止一个。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub settings: IconSettings,
    pub states: Vec<IconState>,
    pub percent: u8,
    pub size: IconSize,
    pub theme: Theme,
}

/// 菜单上点得到的一项——菜单这一块收的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 打开配置文件。
    OpenConfigFile,
    /// 打开日志（点了一条告警）。
    OpenLog,
    /// "托盘上画哪一台"里点了一项：自动（电量最低），或者钉死某一台。
    Primary(PrimaryRule),
    /// 退出。
    Quit,
    /// 点了没勾着的"开机自启"：打开它（`super::launch`）。
    EnableAutostart,
    /// 点了勾着的"开机自启"：关掉它。
    DisableAutostart,
    /// "图标样式""菜单显示"里点了一项：`[tray]` 里那一个键换成这个取值（`super::settings_menu`）。
    TraySetting(TraySetting),
    /// "图标样式"里点了"恢复默认"：图标样式那六项换回缺省值，"菜单显示"那两项不动。
    RestoreIconDefaults,
}

/// 菜单这一块的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 用系统默认程序打开日志文件（配置文件旁边那一份）：告警行放不下完整原因，完整原因在那里。
    OpenLog,
    /// 退出：外壳拿掉图标、停下取数线程、结束消息循环。
    Quit,
}

impl Entry {
    /// 一项最朴素的样子：没有标记、没有右列与预览、点了什么都不收。
    pub(super) fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            right: None,
            checked: false,
            bold: false,
            grayed: false,
            preview: None,
            command: None,
            placeholder: None,
        }
    }
}

impl Tray {
    /// 此刻的右键菜单。外壳在弹出菜单之前来问，带着那一刻的"当下"：设备行上的"多久前"照它算。
    pub fn menu(&self, now: Timestamp) -> Menu {
        // 告警读的是此刻挂着的（parking lot Q223）：写状态文件、重读配置的结果进来时不开新的一轮，读存下来的那一轮
        // 会晚一拍。
        let mut items: Vec<Item> = self.warnings.hanging().iter().map(warning_row).collect();
        if !items.is_empty() {
            items.push(Item::Separator);
        }
        // 一台都没有：设备行那一块写一句说明（命令行里"配置 … 里一个 Device 都没有。"那一行在托盘里的样子）。普通项、
        // 不变灰——变灰说的是"这不是现状"，而一台都没有正是现状（parking lot Q285）。"登记设备 ›"落地之后（票 11、
        // 12），这句说明指向它。
        if self.config.devices.is_empty() {
            items.push(Item::Entry(Entry {
                placeholder: Some(Placeholder {
                    text: "〔一台 Device 都没有的说明〕".to_string(),
                    right: None,
                }),
                ..Entry::new(Kind::Normal, hover::NO_DEVICE)
            }));
        }
        let display = RowDisplay::of(&self.config.tray);
        let primary = self.round.primary();
        items.extend(
            self.round
                .device_states(&self.config, now)
                .iter()
                .map(|state| {
                    let is_primary = primary == Some(state.device.id.as_str());
                    Item::Entry(device_row::entry(state, is_primary, display))
                }),
        );
        items.push(Item::Separator);
        items.push(which_device(&self.config));
        // "图标样式 ›""菜单显示 ›"（票 07）、"登记设备 ›"（票 11、12）、"开机自启"（票 13）依次排在这里。
        items.push(self.icon_style_menu());
        items.push(self.menu_display_menu());
        items.push(self.launch.autostart_entry());
        items.push(Item::Separator);
        items.push(Item::Entry(Entry {
            command: Some(Command::OpenConfigFile),
            ..Entry::new(Kind::Normal, OPEN_CONFIG_FILE)
        }));
        items.push(Item::Entry(Entry {
            command: Some(Command::Quit),
            ..Entry::new(Kind::Normal, "退出")
        }));
        Menu { items }
    }

    /// 点了菜单上的某一项。
    pub(super) fn on_menu(&mut self, command: Command, out: &mut Vec<super::Action>) {
        match command {
            // 打开是配置那一格的动作：文件在哪只有外壳知道。用户改完存下，外壳下一格看得见它变了（`super::config`）。
            Command::OpenConfigFile => {
                out.push(super::Action::Config(super::config::Action::OpenFile));
            }
            // 日志在哪也只有外壳知道（配置文件旁边）。
            Command::OpenLog => out.push(super::Action::Menu(Action::OpenLog)),
            // 自己写的配置自己知道：手上那一份当场换成新的选择、照它重算这一轮（图标立刻换人，不等下一次取数，也不等
            // 外壳重读，parking lot Q252），再让外壳格式保留地写回。重读进来的是同一个选择。
            Command::Primary(rule) => {
                self.config.general.primary = rule.clone();
                out.push(super::Action::Config(super::config::Action::WritePrimary(
                    rule,
                )));
                self.round
                    .on_primary_changed(&self.config, self.warnings.hanging(), out);
            }
            // 退出之后内核什么都不再做（`Tray::handle` 的第一句）：外壳停计时器、停取数线程之间还可能
            // 有一两个事件落进来，它们不该再排出一次取数。
            Command::Quit => {
                self.quit = true;
                out.push(super::Action::Menu(Action::Quit));
            }
            // 建、删计划任务是外壳的事。勾不勾不跟着改：外壳下一次弹出菜单之前再问系统，建或删没办成，菜单照实不变，而不是
            // 勾着一个系统里不存在的任务（`super::launch`）。
            Command::EnableAutostart => {
                out.push(super::Action::Launch(
                    super::launch::Action::EnableAutostart,
                ));
            }
            Command::DisableAutostart => {
                out.push(super::Action::Launch(
                    super::launch::Action::DisableAutostart,
                ));
            }
            // "图标样式""菜单显示"里改了一项：当场生效、重画、写回（`super::settings_menu`）。
            Command::TraySetting(setting) => self.on_tray_settings(vec![setting], out),
            Command::RestoreIconDefaults => {
                self.on_tray_settings(TraySetting::icon_defaults().to_vec(), out);
            }
        }
    }
}

/// 一条告警一行（`menu-as-designed` spec「告警」）：写那件事没办成的那半句（[`Warning::headline`]），完整原因在
/// 日志里；普通项、不变灰，点了打开日志。设计稿基准里它是一整个占位，占位名说是哪一种。
fn warning_row(warning: &Warning) -> Item {
    let placeholder = match warning {
        Warning::ProcessesUnknown(_) => "认不出本机在跑哪些进程",
        Warning::StateNotSaved(_) => "读数记不进状态文件",
        Warning::ConfigUnreadable(_) => "配置文件读不了",
    };
    Item::Entry(Entry {
        command: Some(Command::OpenLog),
        placeholder: Some(Placeholder {
            text: format!("〔告警：{placeholder}〕"),
            right: None,
        }),
        ..Entry::new(Kind::Normal, warning.headline())
    })
}

/// "托盘上画哪一台 ›"：右列写**用户的选择**（配置里的 `primary`），不是这一轮的 Primary Device——这一轮是谁看设备
/// 行上的勾。里面是"自动（电量最低）"与每台 Device 的单选，圆点落在用户的选择上。
fn which_device(config: &Config) -> Item {
    let rule = &config.general.primary;
    let right = match rule {
        PrimaryRule::Lowest => AUTOMATIC.to_string(),
        PrimaryRule::Pinned(id) => config
            .devices
            .iter()
            .find(|device| &device.id == id)
            .map_or_else(|| id.clone(), |device| device.name.clone()),
    };
    let mut choices = vec![Item::Entry(Entry {
        checked: *rule == PrimaryRule::Lowest,
        command: Some(Command::Primary(PrimaryRule::Lowest)),
        ..Entry::new(Kind::Radio, AUTOMATIC)
    })];
    choices.extend(config.devices.iter().map(|device| {
        Item::Entry(Entry {
            checked: matches!(rule, PrimaryRule::Pinned(id) if *id == device.id),
            command: Some(Command::Primary(PrimaryRule::Pinned(device.id.clone()))),
            ..Entry::new(Kind::Radio, device.name.clone())
        })
    }));
    Item::Entry(Entry {
        right: Some(right),
        ..Entry::new(Kind::Submenu(choices), WHICH_DEVICE)
    })
}
