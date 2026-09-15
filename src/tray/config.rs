//! 配置：配置文件读到了、读坏了，首次运行，打开配置文件（spec「配置的读与写」「首次运行」），以及自动补空块
//! （spec「自动补空块」）。往配置里写一处改动的动作（钉死 Primary Device、`[tray]` 的一个键、登记一台设备，票 06、
//! 10、11 起）也住这里（[`Action`]），外壳那一段在 `shell/config.rs`。
//!
//! **配置归内核**：启动时外壳读好一份交给 [`Tray::new`]；之后外壳每一格时钟之前看一眼配置文件变没变，变了
//! 就重读，把读的结果交进来（[`Event::Reloaded`]）。读好了就换掉手上那一份，下一次取数带的就是它（轮询节奏
//! 每次都拿此刻的配置排取数）；读不了就沿用上一份读好的，挂上"配置读不了"（[`Warning::ConfigUnreadable`]，
//! 完整原因随之进日志），挂到下一次读好为止（`super::warnings`）。
//!
//! **写回之后不必另外告诉内核**：程序自己往配置文件里写了一处，外壳下一格照样看得见它变了，重读的结果走
//! 同一条路进来——用户手改的与程序写回的，内核分不出、也不必分。
//!
//! 换掉配置**不重开一轮**：图标与悬停提示照新配置排，要等下一次取数有结果（parking lot Q252）；各台下一次
//! 什么时候问，也还是上一次取数有结果时按旧配置算定的那一刻（parking lot Q172）。**例外是菜单里换了"托盘上画
//! 哪一台"**：那是内核自己做的改动，当场改手上那一份的 `primary`、重算这一轮（`super::menu`），写回照旧交给外壳
//! （[`Action::WritePrimary`]），之后重读进来的是同一个选择。
//!
//! **`[tray]` 的图标样式不等下一次取数**：换掉配置时照新样式把画着的那一张当场重画（不是新的一轮，parking lot Q300），
//! 认不出的取值记进日志。菜单里改的那几项同样当场生效，写回交给外壳（[`Action::WriteTray`]，`super::settings_menu`）。

use crate::config::TraySetting;
use crate::config::{Config, Fill, refresh};
use crate::hid::HidInfo;
use crate::primary::PrimaryRule;
use crate::round::Warning;

use super::Tray;
use super::menu::OPEN_CONFIG_FILE;
use super::notify::Notice;
use super::warnings::Matter;

/// 配置这一块收的事件。
#[derive(Debug)]
pub enum Event {
    /// 外壳看到配置文件变了，重读了一遍：读好了是那一份，读不了是完整原因（`{:#}` 排好的错误链）。
    ///
    /// 文件不见了也算读不了：运行中配置文件被删掉，多半是编辑器存盘的那一瞬，不是首次运行（parking lot Q253）。
    Reloaded(Result<Config, String>),
    /// 首次运行：启动时没有配置文件，外壳照现有的草稿生成写了一份，交给 [`Tray::new`] 的就是它。
    DraftWritten,
    /// 本机设备变了：插上或拔掉了一个 HID 设备（外壳收到了系统的设备变化消息）。
    DevicesChanged,
    /// 外壳照内核的吩咐扫了一遍本机（[`Action::Scan`]）：扫到的与配置文件的全文；扫不了、读不到配置文件时是
    /// 完整原因。
    Scanned(Result<Scan, String>),
}

/// 外壳扫一遍本机交回来的东西（[`Action::Scan`]）。
#[derive(Debug)]
pub struct Scan {
    /// 此刻在场的 HID collection（一次 [`crate::hid::enumerate`]）。
    pub collections: Vec<HidInfo>,
    /// 同一刻配置文件的全文。
    ///
    /// 补空块照它补、写回的是它补过之后的样子，**不是内核手上那一份 [`Config`]**：那一份要等外壳下一次重读才跟得上
    /// 文件，照它写回会把用户刚存下、还没重读的改动盖掉。
    pub text: String,
}

/// 配置这一块的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 用系统默认程序打开配置文件（菜单上的"打开配置文件"）。
    OpenFile,
    /// 扫一遍本机：枚举此刻在场的 HID collection、读一遍配置文件的全文，一起交回来（[`Event::Scanned`]）。
    Scan,
    /// 把配置文件整个换成这份全文。今天只有自动补空块写它，新文本由配置那道缝生成（[`crate::config::refresh`]）。
    Write(String),
    /// 把 `primary` 写成这一条（菜单"托盘上画哪一台"里点的）：外壳读此刻文件的全文、照配置那道缝格式保留地只改那一格
    /// （[`crate::config::pin_primary`]），本来就是这一条就不写。读全文而不是照内核手上那一份写，理由同 [`Scan::text`]。
    WritePrimary(PrimaryRule),
    /// 把 `[tray]` 里这几个键写成这几个取值（菜单"图标样式""菜单显示"里点的一项；"恢复默认"是图标样式那六项）：外壳读
    /// 此刻文件的全文、照配置那道缝格式保留地只改这几个键（[`crate::config::write_tray`]），本来就这样写着就不写。读全文
    /// 而不是照内核手上那一份写，理由同 [`Scan::text`]。
    WriteTray(Vec<TraySetting>),
}

/// 扫一遍本机这件事走到哪了（[`Action::Scan`]）。
///
/// 插上一根线，系统会为它冒出来的每一条 HID collection 各报一次"设备变了"。一遍扫描还没交回来时再来的只记一笔，
/// 交回来之后再扫**一遍**：那一遍排出之后的每一次变化都有一遍扫描看得见，而一串变化不排出一串扫描。
#[derive(Default)]
pub(super) enum Scans {
    /// 外头没有扫描。
    #[default]
    Idle,
    /// 排出了一遍，还没交回来。
    Out,
    /// 排出了一遍、还没交回来，而那之后本机又变过：交回来之后要再扫一遍。
    OutAndChanged,
}

impl Scans {
    /// 要扫一遍本机：外头没有扫描就排一遍，有就记一笔。
    pub(super) fn request(&mut self, out: &mut Vec<super::Action>) {
        *self = match self {
            Self::Idle => {
                out.push(super::Action::Config(Action::Scan));
                Self::Out
            }
            Self::Out | Self::OutAndChanged => Self::OutAndChanged,
        };
    }

    /// 扫描交回来了：那之间本机又变过，就再排一遍。
    fn answered(&mut self, out: &mut Vec<super::Action>) {
        let changed = matches!(self, Self::OutAndChanged);
        *self = Self::Idle;
        if changed {
            self.request(out);
        }
    }
}

impl Tray {
    /// 配置那一侧的事。
    pub(super) fn on_config(&mut self, event: Event, out: &mut Vec<super::Action>) {
        match event {
            Event::Reloaded(Ok(config)) => {
                log_unrecognised(&self.config.tray.unrecognised, &config, out);
                self.round.track_devices(&config);
                self.config = config;
                self.warnings.clear(Matter::ConfigFile);
                // `[tray]` 手改了就当场照它重画：不是新的一轮，只是把画着的那一张换个样式（parking lot Q300）。
                self.round.restyle(self.config.tray.icon, out);
            }
            Event::Reloaded(Err(reason)) => {
                self.warnings.raise(Warning::ConfigUnreadable(reason), out);
            }
            Event::DraftWritten => out.push(super::Action::Notify(draft_notice())),
            Event::DevicesChanged => self.scans.request(out),
            Event::Scanned(scan) => {
                fill_blank_blocks(scan, out);
                self.scans.answered(out);
            }
        }
    }
}

/// 启动：配置里 `[tray]` 认不出的每一处记一行日志。
pub(super) fn on_start(config: &Config, out: &mut Vec<super::Action>) {
    log_unrecognised(&[], config, out);
}

/// 这一份配置的 `[tray]` 里认不出的每一处（按缺省值处理了），上一份里没有的记一行日志——不说出来，用户会以为自己选的
/// 画法就长这样。上一份里就在的不再重记：程序自己写回配置之后也会重读一遍，点一下菜单不该把别处那句笔误再记一遍。
fn log_unrecognised(
    before: &[crate::config::Unrecognised],
    config: &Config,
    out: &mut Vec<super::Action>,
) {
    for note in &config.tray.unrecognised {
        if !before.contains(note) {
            out.push(super::Action::Log(note.to_string()));
        }
    }
}

/// 首次运行生成了草稿之后弹的那一条：草稿里注释掉的块是程序猜不动、或者此刻扫不到的东西，用户该看一眼。
fn draft_notice() -> Notice {
    Notice {
        title: "已生成配置".to_string(),
        body: format!("右键托盘图标，点\"{OPEN_CONFIG_FILE}\"看一眼"),
    }
}

/// 扫了一遍本机：配置里有空块、而它那条 Endpoint 此刻在场，就补上——写回配置文件，每补一块记一行日志，弹一条
/// 通知说补了哪台的哪条。
///
/// 没有要补的，一声不响：不写文件（没改动却重写一遍，会白白改掉修改时间、再招来一次重读），不弹通知，不记日志
/// ——插一次线就说一句"什么都没补"，是噪音。
///
/// 补不了（扫不了、读不到配置文件、配置此刻写坏了）就把完整原因记进日志，一个字节都不写：用户插上了线、却没见到
/// 那条通知，查"为什么没补上"的人要的是这一行。
fn fill_blank_blocks(scan: Result<Scan, String>, out: &mut Vec<super::Action>) {
    let refreshed =
        scan.and_then(|scan| refresh(&scan.text, &scan.collections).map_err(|e| format!("{e:#}")));
    let refreshed = match refreshed {
        Ok(refreshed) => refreshed,
        Err(reason) => {
            out.push(super::Action::Log(format!(
                "没能检查配置里有没有空块可补 —— {reason}"
            )));
            return;
        }
    };
    if refreshed.filled.is_empty() {
        return;
    }
    out.push(super::Action::Config(Action::Write(refreshed.text)));
    for fill in &refreshed.filled {
        out.push(super::Action::Log(format!("自动补空块 —— {fill}")));
    }
    out.push(super::Action::Notify(filled_notice(&refreshed.filled)));
}

/// 补了配置之后弹的那一条：配置变了，用户该知道补的是哪台的哪条。
fn filled_notice(filled: &[Fill]) -> Notice {
    let blocks: Vec<String> = filled
        .iter()
        .map(|fill| format!("{} 的 {}", fill.device_name, fill.endpoint))
        .collect();
    Notice {
        title: "配置已自动更新".to_string(),
        body: format!("补上了 {}", blocks.join("、")),
    }
}
