//! 托盘内核：纯粹的"事件进、动作出"（spec「托盘内核」）。不碰 Win32 界面、不碰真硬件、不问时钟。
//!
//! 外壳（`crate::shell`）把事件喂进来（[`Tray::handle`]），把交出来的动作执行出去，它自己不做决定。
//! 内核因此整个测得到：用例拿一串字面量当时钟、拿罐装的一次取数结果当硬件（`tests/tray_*.rs`）。
//!
//! **一次取数本身不在这里**：内核只决定"去读哪一台"（[`cadence`]），读由取数那一层
//! （`crate::readout`）在外壳的取数线程上做，暂停检测也照旧在那一层、每次取数之前问一次。状态文件
//! 同理：什么时候存、记下哪一台是 Primary Device 由内核定（[`round`]），外壳去写。
//!
//! # 按关注点分块
//!
//! 事件与动作不是一张大清单，而是按关注点各住各的文件，外壳那一侧也照这几格分派：
//!
//! | 关注点 | 内核 | 外壳 | 它收的事件 | 它发的动作 |
//! |---|---|---|---|---|
//! | 轮询节奏 | [`cadence`] | `shell/cadence.rs` | 看 [`Event::Tick`]、[`Event::Fetched`] | [`cadence::Action`] |
//! | 一轮 | [`round`]，文字在 [`hover`] | `shell/round.rs` | 看 [`Event::Tick`]、[`Event::Fetched`] | [`round::Action`] |
//! | 菜单 | [`menu`] | `shell/menu.rs` | [`menu::Command`] | [`menu::Action`] |
//! | 通知 | [`notify`] | `shell/notify.rs` | 看 [`Event::Fetched`] | [`notify::Notice`] |
//! | 配置 | [`config`] | `shell/config.rs` | [`config::Event`] | [`config::Action`] |
//! | 告警 | [`warnings`] | 没有（写状态文件的结果由 `shell/worker.rs` 交回） | 看 [`Event::Fetched`]；[`warnings::Event`] | 只写日志 |
//! | 菜单深浅 | [`menu_theme`] | `shell/menu_theme.rs` | 没有（外壳启动时问过 Windows，随 [`Look`] 交进来） | 只写日志 |
//!
//! 这一层（[`Event`]、[`Action`]、[`Tray::handle`]）只做路由：每个关注点在这里占一格，往自己那一格里
//! 加事件、加动作、加状态，只改自己的那两个文件（内核一个、外壳一个）。三样是大家共用的，所以住在
//! 这里：时钟（[`Event::Tick`]）与一次取数的结果（[`Event::Fetched`]）谁关心谁看，日志
//! （[`Action::Log`]）谁都能写。此刻还挂着的告警也是几块共用的（「一轮」合成时带上，配置那一块往里挂"配置
//! 读不了"），但它有自己挂与摘的规矩，所以住自己的一格（[`warnings`]），由这一层交给要读它的那一格。
//!
//! **为什么非这样不可**：票 05（各走各的节奏）、06（菜单：设备行与切换 Primary Device）、08（低电通知）、
//! 09（配置重读与首次运行）会同时落在这一层上。写成一个大 `match`、一个大枚举，四张票就会在同几行上
//! 撞四次。通知与配置两格起初空着，就是给 08、09 留的位置；通知那一格的动作（[`notify::Notice`]）与外壳里
//! 弹通知的那一段在票 04 就先写好了，因为 08 与 09 都要弹通知，谁先写都会跟另一张撞在同一处。

pub mod cadence;
pub mod config;
pub mod hover;
pub mod menu;
pub mod menu_theme;
pub mod notify;
pub mod round;
pub mod warnings;

use crate::clock::Timestamp;
use crate::config::Config;
use crate::icon::{IconSize, Theme};
use crate::readout::NoReading;
use crate::round::{InHand, Warning};
use crate::state::LastKnown;

/// 托盘内核。
pub struct Tray {
    /// 此刻手上那一份配置：启动时读的那一份，之后配置文件变了、读好了就换成新的，读不了就沿用（[`config`]）。
    config: Config,
    cadence: cadence::Cadence,
    round: round::Rounds,
    notify: notify::Notify,
    /// 此刻还挂着的告警。几块共用：「一轮」合成时带上它，配置那一块往里挂"配置读不了"。
    warnings: warnings::Warnings,
    /// 点过"退出"了：之后什么事件都不再有动作——不再轮询，也不再画。
    quit: bool,
}

impl Tray {
    /// 启动：画出手上已有的（状态文件里的上次已知值，没有就是无已知值），并对每台发出第一次取数。
    ///
    /// `last_known` 是外壳从状态文件里读回来的那一份，内核只拿它当起手：每台 Device 的上次已知值、
    /// 上一轮的 Primary Device。之后记读数、写盘都在取数线程那一侧（`crate::shell`），内核只说什么
    /// 时候存、记下哪一台（[`round::SaveState`]）。
    ///
    /// `look` 是启动时任务栏的深浅色与显示缩放，本票之内不变（变了之后重画归票 07），外加菜单跟不跟得上任务栏
    /// （[`menu_theme::MenuTheming`]，取不到那两个函数时启动这一刻记一条日志）。
    pub fn new(
        config: Config,
        last_known: &LastKnown,
        look: Look,
        now: Timestamp,
    ) -> (Self, Vec<Action>) {
        let mut out = Vec::new();
        menu_theme::on_start(look, &mut out);
        let round = round::Rounds::new(&config, last_known, look, now, &mut out);
        let mut cadence = cadence::Cadence::default();
        cadence.on_tick(&config, now, &mut out);
        let tray = Self {
            config,
            cadence,
            round,
            notify: notify::Notify::default(),
            warnings: warnings::Warnings::default(),
            quit: false,
        };
        (tray, out)
    }

    /// 喂一个事件，交出要执行的动作。
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let mut out = Vec::new();
        if self.quit {
            return out;
        }
        match event {
            Event::Tick(now) => {
                self.cadence.on_tick(&self.config, now, &mut out);
                self.round.on_tick(&self.config, now, &mut out);
            }
            Event::Fetched(fetched) => {
                let fetched = *fetched;
                self.cadence.on_fetched(&self.config, &fetched);
                self.notify.on_fetched(&self.config, &fetched, &mut out);
                // 告警在「一轮」之前：这一次取数问进程的结果，要进它开始的这一轮。
                self.warnings.on_fetched(&fetched, &mut out);
                self.round
                    .on_fetched(&self.config, self.warnings.hanging(), fetched, &mut out);
            }
            Event::Menu(command) => self.on_menu(command, &mut out),
            Event::Config(event) => self.on_config(event, &mut out),
            Event::Warnings(event) => self.warnings.on_event(event, &mut out),
        }
        out
    }
}

/// 启动时任务栏的样子：深浅色与显示缩放，以及菜单的深浅跟不跟得上它。外壳去问 Windows，内核只收答案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// 任务栏是深色还是浅色，图标的调色跟着它。
    pub theme: Theme,
    /// 按显示缩放选出的那一档尺寸（[`IconSize::for_dpi`]）。
    pub size: IconSize,
    /// 菜单的深浅跟不跟得上任务栏。启动时问一次，运行中不变；菜单此刻实际的深浅由它与 `theme` 答
    /// （[`Tray::menu_theme`]）。
    pub menu_theming: menu_theme::MenuTheming,
}

/// 喂进内核的事件。
pub enum Event {
    /// 时钟走到了，带着那一刻的"当下"。外壳每秒喂一次。
    Tick(Timestamp),
    /// 某台 Device 的一次取数有了结果。装箱只是因为它比别的事件大得多（一次取数就一个，不在乎那一次分配）。
    Fetched(Box<Fetched>),
    /// 用户点了菜单里的某一项。
    Menu(menu::Command),
    /// 配置那一侧的事：配置文件变了、重读的结果，首次运行写好了草稿。
    Config(config::Event),
    /// 告警那一侧的事：一件会挂告警的事办没办成，而它不跟着一次取数来。
    Warnings(warnings::Event),
}

/// 某台 Device 的一次取数有了结果：取数线程交回来的全部东西。
pub struct Fetched {
    /// 那台 Device 的 id。
    pub device_id: String,
    /// 这一次取数的"当下"：取数与陈旧判定共用的那一个（`crate::clock` 上写了为什么只问一次）。
    pub at: Timestamp,
    /// 交给这一轮的那一份（[`crate::readout::read_or_last_known`] 交的就是它）。
    pub in_hand: InHand,
    /// 退到了上次已知值时，这一次取数自己为什么没读到（[`crate::readout::Outcome`]）。
    pub fell_back_because: Option<NoReading>,
    /// 取数之前问本机进程的结果：问不出来时的那一条告警，问出来了是 `None`
    /// （[`crate::round::PauseCheck::warning`]）。有就挂上、没有就摘掉（[`warnings`]）。
    pub warning: Option<Warning>,
}

/// 内核交出来、要外壳执行的动作。按关注点分格，外壳也按这几格分派（`crate::shell`）。
#[derive(Debug)]
pub enum Action {
    /// 轮询节奏：对某台 Device 做一次取数。
    Cadence(cadence::Action),
    /// 一轮：重画图标、更新悬停提示、存状态文件。
    Round(round::Action),
    /// 菜单：本票只有退出。
    Menu(menu::Action),
    /// 弹一条通知。
    Notify(notify::Notice),
    /// 配置：打开配置文件。
    Config(config::Action),
    /// 写一条日志。日志写在配置文件旁边，大小与滚动由外壳管。
    Log(String),
}
