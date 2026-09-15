//! 托盘内核的用例共用的东西：一个把动作攒成"此刻屏幕上是什么"的假外壳，以及罐装的一次取数结果。
//!
//! **断言看的是屏幕，不是动作清单**：内核只在图标或悬停提示真变了的时候才发动作，而用例要问的是
//! "这一刻托盘上画的是什么"。[`Screen`] 就是那个答案——它照外壳的样子把每一个动作落下去。
//!
//! 菜单、通知、配置三格的动作原样攒着（[`Screen::menu`]、[`Screen::notices`]、[`Screen::config`]）：
//! 往那几格里加动作的票（06、08、09……）用不着回来改这个文件。

use anyhow::anyhow;
use juicebar::clock::Timestamp;
use juicebar::config::Config;
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::icon::{IconSize, Theme};
use juicebar::readout::{FailureCause, NoReading, RowReading};
use juicebar::round::{InHand, Warning};
use juicebar::sources::Reading;
use juicebar::state::{LastKnown, Provenance};
use juicebar::tray::cadence::FetchRequest;
use juicebar::tray::launch;
use juicebar::tray::notify::Notice;
use juicebar::tray::register;
use juicebar::tray::round::{IconRequest, SaveState};
use juicebar::tray::{
    Action, Event, Fetched, Look, Tray, cadence, config, menu, menu_theme, round, warnings,
};

/// 用例里那两台：一只只配了 Dongle24G 的鼠标，一台只配了 Ble 的键盘。
///
/// 两台都只配一条，好让"这一份读数从哪条 Endpoint 来"在用例里只有一个答案。
pub const MOUSE_AND_KEYBOARD: &str = r#"
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

  [device.bluetooth]
  address = "e452430072a9"
"#;

/// 启动时任务栏的样子：用例里一律深色、100% 缩放、菜单跟得上任务栏，除非用例自己关心它。
pub const LOOK: Look = Look {
    theme: Theme::Dark,
    size: IconSize::Px16,
    menu_theming: menu_theme::MenuTheming::FollowsTaskbar,
};

/// 外壳顶替品：把内核交出来的动作落成"此刻屏幕上是什么"。
#[derive(Default)]
pub struct Screen {
    /// 此刻托盘上画的图标。
    pub icon: Option<IconRequest>,
    /// 此刻的悬停提示。
    pub tooltip: Option<String>,
    /// 至今要求存了几次状态文件，每次存时说了什么。
    pub saves: Vec<SaveState>,
    /// 至今写进日志的每一行，按顺序。
    pub logs: Vec<String>,
    /// 至今要求取数的每一台 Device 的 id，按顺序。
    pub fetches: Vec<String>,
    /// 至今每一次要求取数时带着的东西（那台 Device 与此刻的 `[general]`），按顺序。
    pub requests: Vec<FetchRequest>,
    /// 菜单那一格至今交出的动作，按顺序。
    pub menu: Vec<menu::Action>,
    /// 至今弹过的通知，按顺序。
    pub notices: Vec<Notice>,
    /// 配置那一格至今交出的动作，按顺序。扫一遍本机、写回配置文件这两样不在这里，在下面两格。
    pub config: Vec<config::Action>,
    /// 至今要求扫了几遍本机（[`config::Action::Scan`]）。
    pub scans: usize,
    /// 至今写回配置文件的每一份全文，按顺序（[`config::Action::Write`]）。
    pub written: Vec<String>,
    /// 启动那一格至今交出的动作（建、删开机自启的计划任务），按顺序。
    pub launch: Vec<launch::Action>,
    /// 至今要求扫了几遍本机的蓝牙设备（[`register::Action::Scan`]）。
    pub ble_scans: usize,
}

impl Screen {
    /// 照外壳的样子把这一批动作落下去。
    pub fn apply(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Cadence(cadence::Action::Fetch(request)) => {
                    self.fetches.push(request.device.id.clone());
                    self.requests.push(request);
                }
                Action::Round(round::Action::DrawIcon(icon)) => self.icon = Some(icon),
                Action::Round(round::Action::Tooltip(text)) => self.tooltip = Some(text),
                Action::Round(round::Action::SaveState(save)) => self.saves.push(save),
                Action::Menu(action) => self.menu.push(action),
                Action::Notify(notice) => self.notices.push(notice),
                Action::Config(config::Action::Scan) => self.scans += 1,
                Action::Config(config::Action::Write(text)) => self.written.push(text),
                Action::Config(action) => self.config.push(action),
                Action::Log(line) => self.logs.push(line),
                Action::Launch(action) => self.launch.push(action),
                Action::Register(register::Action::Scan) => self.ble_scans += 1,
            }
        }
    }

    /// 此刻托盘上画的图标。一次都还没画过就是用例写错了。
    pub fn icon(&self) -> IconRequest {
        self.icon.expect("托盘上还没有画过图标")
    }

    /// 此刻的悬停提示。一次都还没设过就是用例写错了。
    pub fn tooltip(&self) -> &str {
        self.tooltip.as_deref().expect("还没有设过悬停提示")
    }
}

/// 按这份配置、这份状态文件、这个"当下"启动内核，连同启动那一刻屏幕上是什么。
pub fn start(config: &str, last_known: &LastKnown, now: Timestamp) -> (Tray, Screen) {
    start_with_look(config, last_known, LOOK, now)
}

/// 同 [`start`]，但任务栏是给定的样子。
pub fn start_with_look(
    config: &str,
    last_known: &LastKnown,
    look: Look,
    now: Timestamp,
) -> (Tray, Screen) {
    let config = Config::parse(config).expect("用例里的配置应当解析得动");
    let (tray, actions) = Tray::new(config, last_known, look, now);
    let mut screen = Screen::default();
    screen.apply(actions);
    (tray, screen)
}

/// `now` 之后 `secs` 秒。
pub fn later(now: Timestamp, secs: u64) -> Timestamp {
    Timestamp::from_unix_secs(now.as_unix_secs() + secs)
}

/// 喂一个事件，把交出来的动作落到屏幕上。
pub fn feed(tray: &mut Tray, screen: &mut Screen, event: Event) {
    screen.apply(tray.handle(event));
}

/// 这一次取数当场读到的一份：`endpoint` 上的电量 `level`，取得时刻 `at`。
///
/// 电压给 4200 mV，越过查表的表顶，所以缺省的 `auto` 采信的就是这个 `level`（`docs/adr/0002`）；
/// Ble 那一级本来就没有电压，采信的也是它。用例里写下的那个数就是图标上画的那个数。
pub fn just_read(endpoint: EndpointKind, level: u8, at: Timestamp) -> InHand {
    InHand::Reading(RowReading {
        reading: reading(endpoint, level, at),
        provenance: Provenance::JustRead,
        paused_by: None,
    })
}

/// `endpoint` 上的一份读数：电量 `level`，取得时刻 `at`（电压与充电态见 [`just_read`]）。
pub fn reading(endpoint: EndpointKind, level: u8, at: Timestamp) -> EndpointReading {
    let (charging, voltage_mv) = match endpoint {
        EndpointKind::Wired | EndpointKind::Dongle24G => (Some(false), Some(4_200)),
        EndpointKind::Ble => (None, None),
    };
    EndpointReading {
        endpoint,
        reading: Reading {
            reported_level: level,
            charging,
            voltage_mv,
        },
        cache_age_secs: None,
        taken_at: Some(at),
    }
}

/// 这一次取数没读到、退到的上次已知值：`endpoint` 上的电量 `level`，取得时刻 `taken_at`。
pub fn last_known_value(endpoint: EndpointKind, level: u8, taken_at: Timestamp) -> InHand {
    InHand::Reading(RowReading {
        reading: reading(endpoint, level, taken_at),
        provenance: Provenance::LastKnown,
        paused_by: None,
    })
}

/// 这一次取数交不出可以印的数、又退不到上次已知值：取数失败，原因是 `reason`。
pub fn failed(reason: &str) -> InHand {
    InHand::NoReading(NoReading::Failed(anyhow!("{reason}")))
}

/// 同 [`failed`]，但说得出来路（失联、读取异常、一条 Endpoint 都没配），完整原因是 `full_reason`。
pub fn failed_because(cause: FailureCause, full_reason: &str) -> InHand {
    InHand::NoReading(NoReading::failed(cause, full_reason))
}

/// 某台 Device 的一次取数在 `at` 那一刻有了结果，手上是 `in_hand`；没有别的要交代。
pub fn fetched(device: &str, at: Timestamp, in_hand: InHand) -> Event {
    Event::Fetched(Box::new(Fetched {
        device_id: device.to_string(),
        at,
        in_hand,
        warning: None,
    }))
}

/// 同 [`fetched`]，但取数之前问本机进程时交出了一条告警。
pub fn fetched_with_warning(
    device: &str,
    at: Timestamp,
    in_hand: InHand,
    warning: Warning,
) -> Event {
    Event::Fetched(Box::new(Fetched {
        device_id: device.to_string(),
        at,
        in_hand,
        warning: Some(warning),
    }))
}

/// 某台 Device 的一次取数在 `at` 那一刻没读到（原因 `reason`，按失联算），退到了上次已知值 `last_known`。
pub fn fell_back(device: &str, at: Timestamp, last_known: InHand, reason: &str) -> Event {
    fell_back_because(
        device,
        at,
        last_known,
        NoReading::Failed(anyhow!("{reason}")),
    )
}

/// 同 [`fell_back`]，但这一次没读到的原因是给定的 `because`（取数失败的某一种来路，或者暂停）。`last_known` 得是
/// 一份读数（[`last_known_value`] 造的那种）。
pub fn fell_back_because(
    device: &str,
    at: Timestamp,
    last_known: InHand,
    because: NoReading,
) -> Event {
    let InHand::Reading(last_known) = last_known else {
        panic!("退到的上次已知值得是一份读数");
    };
    Event::Fetched(Box::new(Fetched {
        device_id: device.to_string(),
        at,
        in_hand: InHand::FellBack {
            last_known,
            because,
        },
        warning: None,
    }))
}

/// 外壳照内核的吩咐写了一次状态文件（[`round::Action::SaveState`]），写成了。
pub fn state_saved() -> Event {
    Event::Warnings(warnings::Event::StateSaved(Ok(())))
}

/// 外壳照内核的吩咐写了一次状态文件，写不进，完整原因是 `reason`。
pub fn state_not_saved(reason: &str) -> Event {
    Event::Warnings(warnings::Event::StateSaved(Err(reason.to_string())))
}
