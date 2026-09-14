//! 托盘内核里「告警」这一块（`juicebar::tray::warnings`，`CONTEXT.md`「告警」）：不属于任何一台 Device、
//! 而用户该知道的一件没办成的事，挂到那件事下一次办成为止，不随「一轮」来去。
//!
//! 全程只经内核的公开面：事件进（`Tray::handle`），此刻挂着的告警从 `Tray::warnings` 看；外壳由
//! `common::tray::Screen` 顶替，写进日志的每一行攒在 [`Screen::logs`]。一次取数的结果是罐装的，时钟是
//! 一串字面量。
//!
//! [`Screen::logs`]: common::tray::Screen::logs

mod common;

use common::NOW;
use common::tray::{
    MOUSE_AND_KEYBOARD, feed, fetched, fetched_with_warning, just_read, later, start,
    state_not_saved, state_saved,
};
use juicebar::clock::Timestamp;
use juicebar::endpoints::EndpointKind;
use juicebar::round::{InHand, Warning};
use juicebar::state::LastKnown;
use juicebar::tray::Event;

/// 用例里问本机进程没问出来时的完整原因。
const PROCESSES_ERROR: &str = "假接缝这一次故意枚举不动";

/// 用例里写不进状态文件时的完整原因。
const STATE_FILE_ERROR: &str = "写不进状态文件 C:/juicebar/state.toml";

/// 取数之前问本机进程、没问出来：那一次取数照常去试（按"没在跑"走），告警挂上。
#[test]
fn a_fetch_that_could_not_ask_about_processes_hangs_a_warning() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        could_not_ask("dragonfly3", NOW, PROCESSES_ERROR),
    );

    assert_eq!(tray.warnings(), [processes_unknown(PROCESSES_ERROR)]);
}

/// 之后任何一次问得出来就摘掉——不必是同一台：问的是本机，不是哪一台 Device。
#[test]
fn the_processes_warning_comes_down_once_any_later_fetch_asks_successfully() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        could_not_ask("dragonfly3", NOW, PROCESSES_ERROR),
    );

    feed(&mut tray, &mut screen, asked("neon75", NOW));

    assert_eq!(tray.warnings(), []);
}

/// 外壳写状态文件写不进：它告诉内核（一个事件），告警挂上。
#[test]
fn a_state_file_that_could_not_be_written_hangs_a_warning() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));

    assert_eq!(tray.warnings(), [state_file_warning(STATE_FILE_ERROR)]);
}

/// 之后某一次写成了就摘掉：外壳写成了也告诉内核，不然这一条摘不掉。
#[test]
fn the_state_file_warning_comes_down_once_a_later_write_succeeds() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));

    feed(&mut tray, &mut screen, state_saved());

    assert_eq!(tray.warnings(), []);
}

/// 每次写不进都把完整原因写进日志：菜单上那一行放不下整条错误链，查"为什么记不下"的人要的是它。
#[test]
fn a_state_file_that_could_not_be_written_logs_its_full_reason() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));

    assert_eq!(
        screen.logs,
        [
            "记不下这一轮的读数（下次启动就没有上次已知值了）—— 写不进状态文件 C:/juicebar/state.toml"
        ]
    );
}

/// 每次问不出来都把完整原因写进日志。
#[test]
fn a_fetch_that_could_not_ask_about_processes_logs_its_full_reason() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        could_not_ask("dragonfly3", NOW, PROCESSES_ERROR),
    );

    assert_eq!(
        screen.logs,
        ["认不出本机在跑哪些进程，这一轮不暂停 —— 假接缝这一次故意枚举不动"]
    );
}

/// 两种各自挂上，各自摘掉：写成了状态文件摘不掉进程那一条，问得出进程也摘不掉状态文件那一条。
#[test]
fn each_warning_hangs_and_comes_down_on_its_own_matter() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        could_not_ask("dragonfly3", NOW, PROCESSES_ERROR),
    );
    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));
    assert_eq!(
        tray.warnings(),
        [
            processes_unknown(PROCESSES_ERROR),
            state_file_warning(STATE_FILE_ERROR),
        ]
    );

    feed(&mut tray, &mut screen, state_saved());
    assert_eq!(
        tray.warnings(),
        [processes_unknown(PROCESSES_ERROR)],
        "写成了状态文件，只摘状态文件那一条"
    );

    feed(&mut tray, &mut screen, asked("neon75", NOW));
    assert_eq!(tray.warnings(), [], "问得出进程，摘掉进程那一条");
}

/// 挂着的告警跨轮仍在：下一轮由另一台 Device 的取数开头，状态文件那一条照样挂着——一轮来去的是各台
/// Device 手上的东西，不是告警（parking lot Q162）。
#[test]
fn a_hanging_warning_outlives_a_round_started_by_another_device() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, asked("dragonfly3", NOW));
    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));

    feed(&mut tray, &mut screen, asked("neon75", later(NOW, 60)));

    assert_eq!(tray.warnings(), [state_file_warning(STATE_FILE_ERROR)]);
}

/// 进程那一条跨轮时，由另一台的取数又问不出来一次：还是一条，原因换成这一次的；日志里两次各一行。
#[test]
fn asking_about_processes_failing_again_from_another_device_keeps_one_warning_with_the_latest_reason()
 {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        could_not_ask("dragonfly3", NOW, "第一次枚举不动"),
    );

    feed(
        &mut tray,
        &mut screen,
        could_not_ask("neon75", later(NOW, 60), "第二次枚举不动"),
    );

    assert_eq!(tray.warnings(), [processes_unknown("第二次枚举不动")]);
    assert_eq!(
        screen.logs,
        [
            "认不出本机在跑哪些进程，这一轮不暂停 —— 第一次枚举不动",
            "认不出本机在跑哪些进程，这一轮不暂停 —— 第二次枚举不动",
        ]
    );
}

/// 日志记的是每一次发生，不是每一轮挂着：写不进状态文件那一次记一行，之后几轮它挂着，不再记。
#[test]
fn a_hanging_warning_is_logged_when_it_happens_not_again_with_every_round() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);
    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));

    for secs in [60, 120] {
        feed(&mut tray, &mut screen, asked("neon75", later(NOW, secs)));
    }

    assert_eq!(
        screen.logs,
        [
            "记不下这一轮的读数（下次启动就没有上次已知值了）—— 写不进状态文件 C:/juicebar/state.toml"
        ]
    );
}

/// 某台 Device 在 `at` 那一刻的一次取数：读到了，取数之前问本机进程也问出来了。
fn asked(device: &str, at: Timestamp) -> Event {
    fetched(device, at, read_now(device, at))
}

/// 某台 Device 在 `at` 那一刻的一次取数：读到了，而取数之前问本机进程没问出来，完整原因是 `reason`。
fn could_not_ask(device: &str, at: Timestamp, reason: &str) -> Event {
    fetched_with_warning(device, at, read_now(device, at), processes_unknown(reason))
}

/// 这台 Device 在 `at` 那一刻当场读到的一份：鼠标走 Dongle24G，键盘走 Ble（`MOUSE_AND_KEYBOARD` 各只配了
/// 这一条）。电量是多少这里的用例不关心。
fn read_now(device: &str, at: Timestamp) -> InHand {
    match device {
        "dragonfly3" => just_read(EndpointKind::Dongle24G, 62, at),
        "neon75" => just_read(EndpointKind::Ble, 80, at),
        other => panic!("用例里没有这台 Device：{other}"),
    }
}

/// 问不出本机进程的那一条告警。
fn processes_unknown(reason: &str) -> Warning {
    Warning::ProcessesUnknown(reason.to_string())
}

/// 写不进状态文件的那一条告警。
fn state_file_warning(reason: &str) -> Warning {
    Warning::StateNotSaved(reason.to_string())
}
