//! 托盘内核里「配置」这一块（`juicebar::tray::config`）：配置文件读到了、读坏了，首次运行。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），动作出，此刻挂着的告警从 `Tray::warnings` 看；外壳由
//! `common::tray::Screen` 顶替，它记下每一次要求取数时带着的那一份配置、弹过的通知。配置文件本身不碰：外壳
//! 看一眼它变没变、变了重读一遍，交给内核的是读的结果，在这里罐装；首次运行写草稿也是外壳的事，内核收到的
//! 是"写好了"。

mod common;

use common::NOW;
use common::tray::{MOUSE_AND_KEYBOARD, feed, fetched, just_read, later, start};
use juicebar::clock::Timestamp;
use juicebar::config::{Config, draft};
use juicebar::endpoints::EndpointKind;
use juicebar::hid::HidInfo;
use juicebar::round::Warning;
use juicebar::state::LastKnown;
use juicebar::tray::notify::Notice;
use juicebar::tray::{Event, config};

/// 用例里配置读不了时的完整原因。
const BROKEN: &str =
    "配置 C:/juicebar/config.toml 有问题: 配置解析失败: TOML parse error at line 3, column 9";

/// 改了一个键、重读好了：下一次取数带的就是新值，不必重启。
#[test]
fn a_changed_key_is_used_by_the_next_fetch() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, mouse_read(NOW));

    feed(&mut tray, &mut screen, reloaded(&another_vendor_hub()));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));

    let (first, next) = (&screen.requests[0], screen.requests.last().unwrap());
    assert_eq!(first.general.vendor_hub_processes, ["VGN VHUB.exe"]);
    assert_eq!(next.device.id, "dragonfly3");
    assert_eq!(next.general.vendor_hub_processes, ["VGN HUB 2.exe"]);
}

/// 写坏了一处、重读读不了：照旧运行，取数用的是上一份读好的；"配置读不了"的告警挂上。
#[test]
fn a_broken_config_keeps_the_last_good_one_and_hangs_a_warning() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, mouse_read(NOW));
    feed(&mut tray, &mut screen, reloaded(&another_vendor_hub()));

    feed(&mut tray, &mut screen, unreadable(BROKEN));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));

    let next = screen.requests.last().unwrap();
    assert_eq!(next.device.id, "dragonfly3");
    assert_eq!(
        next.general.vendor_hub_processes,
        ["VGN HUB 2.exe"],
        "沿用的是上一份读好的，不是启动时那一份"
    );
    assert_eq!(tray.warnings(), [config_unreadable(BROKEN)]);
}

/// 读不了时把完整原因写进日志：菜单顶上那一行放不下整条错误链，查"哪儿写坏了"的人要的是它。
#[test]
fn a_broken_config_logs_its_full_reason() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, unreadable(BROKEN));

    assert_eq!(
        screen.logs,
        [
            "读不了配置文件，沿用上一份读好的 —— 配置 C:/juicebar/config.toml 有问题: 配置解析失败: TOML parse error at line 3, column 9"
        ]
    );
}

/// 修好了、下一次读好了：告警摘掉，取数用上修好的那一份。
#[test]
fn the_config_warning_comes_down_once_the_config_reads_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, mouse_read(NOW));
    feed(&mut tray, &mut screen, unreadable(BROKEN));

    feed(&mut tray, &mut screen, reloaded(&another_vendor_hub()));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 60)));

    assert_eq!(tray.warnings(), []);
    assert_eq!(
        screen.requests.last().unwrap().general.vendor_hub_processes,
        ["VGN HUB 2.exe"]
    );
}

/// 首次运行：外壳照现有的草稿生成写好了一份，托盘拿它启动、照常开始轮询，并弹一条通知叫用户去看一眼。
#[test]
fn a_first_run_starts_polling_the_draft_and_says_where_to_look() {
    let (mut tray, mut screen) = start(&draft(&[mouse_dongle()]), &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        Event::Config(config::Event::DraftWritten),
    );

    assert_eq!(
        screen.notices,
        [Notice {
            title: "已生成配置".to_string(),
            body: "右键托盘图标，点\"打开配置文件\"看一眼".to_string(),
        }]
    );
    assert_eq!(screen.fetches, ["dragonfly3"], "照常轮询草稿里的那一台");
}

/// 与 `MOUSE_AND_KEYBOARD` 同两台，只改了一个键：厂商上位机换了个进程名。
fn another_vendor_hub() -> String {
    format!("[general]\nvendor_hub_processes = [\"VGN HUB 2.exe\"]\n{MOUSE_AND_KEYBOARD}")
}

/// 鼠标在 `at` 那一刻的一次取数：从 Dongle24G 读到了（`MOUSE_AND_KEYBOARD` 里它只配了这一条），下一次隔
/// `poll_interval_24g`（缺省 60 秒）。
fn mouse_read(at: Timestamp) -> Event {
    fetched("dragonfly3", at, just_read(EndpointKind::Dongle24G, 62, at))
}

/// 首次运行那一刻本机插着的鼠标 2.4G 接收器，实测身份 `391D:1A05`（与 `tests/config.rs` 里那一条相同）。
fn mouse_dongle() -> HidInfo {
    HidInfo {
        path: "\\\\?\\hid#vid_391d&pid_1a05&mi_01&col05#7&1234abcd&0&0000".to_string(),
        vid: 0x391D,
        pid: 0x1A05,
        version: 0x0303,
        usage_page: 0xFF02,
        usage: 0x0002,
        input_len: 17,
        output_len: 17,
        feature_len: 0,
        product: String::new(),
        manufacturer: String::new(),
    }
}

/// 外壳看到配置文件变了，重读了一遍，读好了，读出来的是 `text`。
fn reloaded(text: &str) -> Event {
    let config = Config::parse(text).expect("用例里的配置应当解析得动");
    Event::Config(config::Event::Reloaded(Ok(config)))
}

/// 外壳看到配置文件变了，重读了一遍，读不了，完整原因是 `reason`。
fn unreadable(reason: &str) -> Event {
    Event::Config(config::Event::Reloaded(Err(reason.to_string())))
}

/// 配置读不了的那一条告警。
fn config_unreadable(reason: &str) -> Warning {
    Warning::ConfigUnreadable(reason.to_string())
}
