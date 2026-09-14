//! 托盘内核里「配置」这一块（`juicebar::tray::config`）：配置文件读到了、读坏了，首次运行，自动补空块。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），动作出，此刻挂着的告警从 `Tray::warnings` 看；外壳由
//! `common::tray::Screen` 顶替，它记下每一次要求取数时带着的那一份配置、弹过的通知、要求扫了几遍本机、写回配置
//! 文件的全文。配置文件本身不碰：外壳看一眼它变没变、变了重读一遍，交给内核的是读的结果，在这里罐装；首次运行
//! 写草稿也是外壳的事，内核收到的是"写好了"；扫一遍本机同理，内核收到的是扫到的与配置文件的全文。

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

// ---------------------------------------------------------------
// 自动补空块：本机出现了认得的 Endpoint、而配置里它那块空着，就补上
// ---------------------------------------------------------------

/// 启动时扫一遍本机：托盘没开着时插上的线，启动那一刻就查得到。
#[test]
fn startup_scans_the_machine_once() {
    let (_tray, screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    assert_eq!(screen.scans, 1);
}

/// 托盘开着时插上鼠标的线：本机设备变了，再扫一遍；这一遍看到 Wired 在场、配置里那块空着，补上并弹通知。
#[test]
fn plugging_in_while_running_scans_again_and_fills_the_blank_block() {
    let text = draft(&[mouse_dongle()]);
    let (mut tray, mut screen) = start(&text, &LastKnown::default(), NOW);
    // 启动那一遍：线还没插，没有要补的。
    feed(&mut tray, &mut screen, scanned(&text, &[mouse_dongle()]));

    feed(&mut tray, &mut screen, devices_changed());
    assert_eq!(screen.scans, 2, "本机设备变了，再扫一遍");
    feed(
        &mut tray,
        &mut screen,
        scanned(&text, &[mouse_dongle(), mouse_wired()]),
    );

    assert_eq!(screen.written.len(), 1);
    assert_eq!(
        screen.notices,
        [Notice {
            title: "配置已自动更新".to_string(),
            body: "补上了 Dragonfly 3 Master+ 的 Wired".to_string(),
        }]
    );
}

/// 插上一根线，系统为它冒出来的每一条 collection 各报一次"设备变了"，一口气好几次。一遍扫描还没交回来时再来的
/// 只记一笔，交回来之后**再扫一遍**：那一遍开始之后的变化都有一遍扫描看得见，一串变化也不排出一串扫描。
#[test]
fn changes_while_a_scan_is_out_add_up_to_one_more_scan() {
    let text = draft(&[mouse_dongle()]);
    // 启动那一遍扫描还没交回来。
    let (mut tray, mut screen) = start(&text, &LastKnown::default(), NOW);

    for _ in 0..4 {
        feed(&mut tray, &mut screen, devices_changed());
    }
    assert_eq!(screen.scans, 1, "那一遍还没交回来，不另排");

    feed(&mut tray, &mut screen, scanned(&text, &[mouse_dongle()]));
    assert_eq!(screen.scans, 2, "交回来之后再扫一遍，只一遍");

    feed(
        &mut tray,
        &mut screen,
        scanned(&text, &[mouse_dongle(), mouse_wired()]),
    );
    assert_eq!(screen.scans, 2, "这一遍开始之后本机没再变过，不再扫");
    assert_eq!(screen.written.len(), 1, "后面那一遍看到了插上的线，补上了");
}

/// 扫不了（枚举不了本机、读不到配置文件）：完整原因进日志，什么都不写；之后本机再变，照样再扫。
#[test]
fn a_scan_that_failed_is_logged_and_the_next_change_scans_again() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        Event::Config(config::Event::Scanned(Err(
            "读不到配置 C:/juicebar/config.toml：另一个程序正在使用此文件，进程无法访问。 (os error 32)".to_string(),
        ))),
    );
    feed(&mut tray, &mut screen, devices_changed());

    assert_eq!(
        screen.logs,
        [
            "没能检查配置里有没有空块可补 —— 读不到配置 C:/juicebar/config.toml：另一个程序正在使用此文件，进程无法访问。 (os error 32)"
        ]
    );
    assert_eq!(screen.written, Vec::<String>::new());
    assert_eq!(screen.scans, 2);
}

/// 配置文件此刻写坏了：补不了，原因进日志，一个字节都不写——照一份读不动的文件补，写回去的只会更坏。
#[test]
fn a_config_that_does_not_parse_is_not_filled() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        scanned("[[device]\nid = ", &[mouse_dongle(), mouse_wired()]),
    );

    let [line] = screen.logs.as_slice() else {
        panic!("该记一行日志：{:?}", screen.logs);
    };
    assert!(
        line.starts_with("没能检查配置里有没有空块可补 —— 配置解析失败"),
        "{line}"
    );
    assert_eq!(screen.written, Vec::<String>::new());
    assert_eq!(screen.notices, []);
}

/// 插上了鼠标的线，而配置里它那块 Wired 空着（草稿留下的注释占位）：补上、写回配置文件；弹一条通知说补了哪台
/// 的哪条；日志里记下补的是哪组身份。
#[test]
fn a_blank_block_that_is_now_present_is_filled_notified_and_logged() {
    let text = draft(&[mouse_dongle()]);
    let (mut tray, mut screen) = start(&text, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        scanned(&text, &[mouse_dongle(), mouse_wired()]),
    );

    let [written] = screen.written.as_slice() else {
        panic!("该写回配置文件一次，写了 {} 次", screen.written.len());
    };
    let filled = Config::parse(written).expect("写回去的配置得解析得动");
    assert_eq!(
        filled.devices[0].wired.as_ref().expect("补上了 Wired").pid,
        0x1005
    );
    assert_eq!(
        screen.notices,
        [Notice {
            title: "配置已自动更新".to_string(),
            body: "补上了 Dragonfly 3 Master+ 的 Wired".to_string(),
        }]
    );
    assert_eq!(
        screen.logs,
        ["自动补空块 —— dragonfly3：补上了 Wired（VID 391D PID 1005 UP FF02 U 0002）。"]
    );
}

/// 没有要补的（鼠标的线没插，Wired 那块空着也补不上）：一声不响——不写文件、不弹通知、不记日志。
#[test]
fn nothing_to_fill_writes_nothing_and_says_nothing() {
    let text = draft(&[mouse_dongle()]);
    let (mut tray, mut screen) = start(&text, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, scanned(&text, &[mouse_dongle()]));

    assert_eq!(screen.written, Vec::<String>::new());
    assert_eq!(screen.notices, []);
    assert_eq!(screen.logs, Vec::<String>::new());
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

/// 插上线之后才冒出来的鼠标本体，实测身份 `391D:1005`，与接收器只差 `pid`（与 `tests/config.rs` 里那一条相同）。
fn mouse_wired() -> HidInfo {
    HidInfo {
        path: "\\\\?\\hid#vid_391d&pid_1005&mi_01&col05#7&5678abcd&0&0000".to_string(),
        pid: 0x1005,
        ..mouse_dongle()
    }
}

/// 外壳收到了系统的设备变化消息：本机插上或拔掉了一个 HID 设备。
fn devices_changed() -> Event {
    Event::Config(config::Event::DevicesChanged)
}

/// 外壳照内核的吩咐扫了一遍本机：此刻在场的是 `collections`，配置文件此刻写着 `text`。
fn scanned(text: &str, collections: &[HidInfo]) -> Event {
    Event::Config(config::Event::Scanned(Ok(config::Scan {
        collections: collections.to_vec(),
        text: text.to_string(),
    })))
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
