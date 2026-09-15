//! 托盘内核里「菜单」这一块（`juicebar::tray::menu`）：右键菜单上有什么，点了之后内核做什么。
//!
//! 菜单的结构与设计稿导出的基准逐项比对（`tests/menu_baselines/`，帮手在 `common::menu`）；设备行、告警行的
//! 确切字符串在 `tests/tray_device_row.rs`。

mod common;

use common::NOW;
use common::menu::{Depth, assert_matches_design, baseline, canned, submenu, which_device};
use common::tray::{
    MOUSE_AND_KEYBOARD, Screen, feed, fetched, fetched_with_warning, just_read, later, start,
    state_not_saved, state_saved,
};
use juicebar::config::Config;
use juicebar::endpoints::EndpointKind;
use juicebar::primary::PrimaryRule;
use juicebar::round::Warning;
use juicebar::state::LastKnown;
use juicebar::tray::menu::{self, Command, Item, Menu};
use juicebar::tray::{Event, Tray, config};

/// 一台 Device 都没登记的配置。
const NO_DEVICE: &str = "[general]\nprimary = \"lowest\"\n";

/// 用例里写不进状态文件时的完整原因。
const STATE_FILE_ERROR: &str = "写不进状态文件 C:/juicebar/state.toml";

/// 三件会挂告警的事都没办成：取数之前问本机进程没问出来、状态文件写不进、配置文件读不了。
fn hang_every_warning(tray: &mut Tray, screen: &mut Screen) {
    feed(
        tray,
        screen,
        fetched_with_warning(
            "dragonfly3",
            NOW,
            just_read(EndpointKind::Dongle24G, 57, NOW),
            Warning::ProcessesUnknown("假接缝这一次故意枚举不动".to_string()),
        ),
    );
    feed(tray, screen, state_not_saved(STATE_FILE_ERROR));
    feed(
        tray,
        screen,
        Event::Config(config::Event::Reloaded(Err(
            "配置解析失败: TOML parse error at line 3, column 9".to_string(),
        ))),
    );
}

/// 一级菜单里第一条分隔线之上的那几项：左边那段字、变没变灰、点了收到什么。
fn rows_above_the_first_separator(menu: &Menu) -> Vec<(String, bool, Option<Command>)> {
    menu.items
        .iter()
        .map_while(|item| match item {
            Item::Entry(entry) => Some((entry.text.clone(), entry.grayed, entry.command.clone())),
            Item::Separator => None,
        })
        .collect()
}

/// 一级菜单里归别的票、还没做出来的那几项：比对一级的基准时略去。那张票做出来了，就把它那一行拿掉；一个都不剩时，
/// 一级的整份比对归 `menu-as-designed` 07。
const NOT_BUILT_YET: &[&str] = &[
    // 票 07
    "子菜单 「图标样式」",
    "子菜单 「菜单显示」",
    // 票 11、12
    "子菜单 「登记设备」",
    // 票 13
    "勾选 「开机自启」",
];

/// 一级，罐装：设备行在上（Primary Device 打勾，取数失败的那一行变灰），分隔线，"托盘上画哪一台 ›"，分隔线，
/// "打开配置文件"与"退出"。
#[test]
fn the_top_level_matches_the_design() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top").without(NOT_BUILT_YET),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一级，钉死 neon75：勾跟着这一轮的 Primary Device 挪到它那一行（它此刻取数失败，照样变灰），子菜单右列写它的名字。
#[test]
fn the_top_level_matches_the_design_when_a_failing_device_is_pinned() {
    let (tray, _screen) = canned("primary = \"neon75\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-primary-neon75").without(NOT_BUILT_YET),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一级，钉死 dragonfly3：这一轮的 Primary Device 本来就是它，一级与罐装只差子菜单右列。
#[test]
fn the_top_level_matches_the_design_when_the_lowest_device_is_pinned() {
    let (tray, _screen) = canned("primary = \"dragonfly3\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-primary-dragonfly3").without(NOT_BUILT_YET),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一级，罐装外加三条告警都挂着：告警在最顶上，一条一行，下面一条分隔线，再往下与罐装一样。
#[test]
fn the_top_level_matches_the_design_with_every_warning_hanging() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    hang_every_warning(&mut tray, &mut screen);

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-warnings").without(NOT_BUILT_YET),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一级，配置里一台 Device 都没有：设备行那一块是一句说明，"托盘上画哪一台"里只剩"自动（电量最低）"，其余照旧。
#[test]
fn the_top_level_matches_the_design_when_the_config_has_no_device() {
    let (tray, _screen) = start(NO_DEVICE, &LastKnown::default(), NOW);

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-no_devices").without(NOT_BUILT_YET),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一台 Device 都没有时那一句说明：与悬停提示同一句（命令行里"配置 … 里一个 Device 都没有。"那一行在托盘里的样子）；
/// 普通项、不变灰（一台都没有正是现状，parking lot Q285），点了什么都不做。
#[test]
fn with_no_device_the_device_rows_are_one_sentence_saying_so() {
    let (tray, _screen) = start(NO_DEVICE, &LastKnown::default(), NOW);

    assert_eq!(
        rows_above_the_first_separator(&tray.menu(NOW)),
        [("配置里一个 Device 都没有".to_string(), false, None)]
    );
}

/// 运行中往配置里加了 Device（重读读好了）：它第一次取数回来之前，一级照样有它那一行——无已知值、变灰。从一台都
/// 没有变成有两台时，说明句换成这两行，而不是整块空着（本票 Spec review 指出）。
#[test]
fn a_device_added_by_reloading_the_config_has_its_row_before_its_first_fetch() {
    let (mut tray, mut screen) = start(NO_DEVICE, &LastKnown::default(), NOW);

    feed(
        &mut tray,
        &mut screen,
        Event::Config(config::Event::Reloaded(Ok(Config::parse(
            MOUSE_AND_KEYBOARD,
        )
        .expect("用例里的配置应当解析得动")))),
    );

    assert_eq!(
        rows_above_the_first_separator(&tray.menu(NOW)),
        [
            ("Dragonfly 3 Master+".to_string(), true, None),
            ("VGN Neon75".to_string(), true, None),
        ]
    );
}

/// 告警行：一条一行，写的是哪件事没办成（不带完整原因——那在日志里）；普通项、不变灰，点了交出"打开日志"。
#[test]
fn a_warning_row_says_what_did_not_get_done_and_opens_the_log() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    hang_every_warning(&mut tray, &mut screen);

    let menu = tray.menu(NOW);

    assert_eq!(
        rows_above_the_first_separator(&menu),
        [
            (
                "认不出本机在跑哪些进程，这一轮不暂停".to_string(),
                false,
                Some(Command::OpenLog)
            ),
            (
                "记不下这一轮的读数（下次启动就没有上次已知值了）".to_string(),
                false,
                Some(Command::OpenLog)
            ),
            (
                "读不了配置文件，沿用上一份读好的".to_string(),
                false,
                Some(Command::OpenLog)
            ),
        ]
    );
}

/// 点了告警：外壳收到"打开日志"（它用系统默认程序打开配置文件旁边那份日志）。
#[test]
fn clicking_a_warning_asks_the_shell_to_open_the_log() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Menu(Command::OpenLog));

    assert_eq!(screen.menu, [menu::Action::OpenLog]);
}

/// 菜单画的是**弹出这一刻**挂着的告警（parking lot Q223）：写不进状态文件的结果进来时不开新的一轮，菜单照样当场
/// 看得见它；下一次写成了，它当场就不在了。
#[test]
fn the_menu_shows_the_warnings_hanging_at_the_moment_it_pops_up() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");

    feed(&mut tray, &mut screen, state_not_saved(STATE_FILE_ERROR));
    let hanging = rows_above_the_first_separator(&tray.menu(NOW));
    feed(&mut tray, &mut screen, state_saved());
    let cleared = tray.menu(NOW);

    assert_eq!(
        hanging,
        [(
            "记不下这一轮的读数（下次启动就没有上次已知值了）".to_string(),
            false,
            Some(Command::OpenLog)
        )]
    );
    assert!(
        !cleared.items.iter().any(|item| matches!(
            item,
            Item::Entry(entry) if entry.command == Some(Command::OpenLog)
        )),
        "写成了就摘掉"
    );
}

/// "托盘上画哪一台"，罐装（`primary = "lowest"`）：右列写用户的选择"自动（电量最低）"，圆点落在它上面，下面每台
/// Device 一个单选。
#[test]
fn the_which_device_submenu_matches_the_design_when_automatic() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("primary"),
        submenu(&menu, "托盘上画哪一台"),
        Depth::Whole,
    );
}

/// "托盘上画哪一台"，钉死 dragonfly3：右列写它的名字，圆点落在它上面。
#[test]
fn the_which_device_submenu_matches_the_design_when_the_mouse_is_pinned() {
    let (tray, _screen) = canned("primary = \"dragonfly3\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("primary-dragonfly3"),
        submenu(&menu, "托盘上画哪一台"),
        Depth::Whole,
    );
}

/// "托盘上画哪一台"，钉死 neon75：右列写的是用户的选择，哪怕那一台此刻取数失败。
#[test]
fn the_which_device_submenu_matches_the_design_when_the_keyboard_is_pinned() {
    let (tray, _screen) = canned("primary = \"neon75\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("primary-neon75"),
        submenu(&menu, "托盘上画哪一台"),
        Depth::Whole,
    );
}

/// 配置里钉的 id 不在册（一次笔误）：右列照样写用户的选择——那个 id 原样——而子菜单里没有它那一项，圆点哪儿都不落
/// （parking lot Q293）。为什么选不出来，悬停提示在说。
#[test]
fn a_pinned_id_that_is_not_registered_is_written_as_is_and_no_choice_is_selected() {
    let (tray, _screen) = canned("primary = \"retired\"");

    let menu = tray.menu(NOW);
    let (which, choices) = which_device(&menu);

    assert_eq!(which.right.as_deref(), Some("retired"));
    assert!(
        choices
            .iter()
            .all(|item| matches!(item, Item::Entry(entry) if !entry.checked)),
        "圆点哪儿都不落"
    );
}

/// "托盘上画哪一台"里每个单选，点了交给内核的就是它说的那一条规则：自动，或者钉死那一台。
#[test]
fn each_which_device_choice_hands_the_kernel_the_rule_it_stands_for() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);
    let (_, choices) = which_device(&menu);
    let commands: Vec<(String, Option<Command>)> = choices
        .iter()
        .map(|item| match item {
            Item::Entry(entry) => (entry.text.clone(), entry.command.clone()),
            Item::Separator => panic!("这个子菜单里没有分隔线"),
        })
        .collect();

    assert_eq!(
        commands,
        [
            (
                "自动（电量最低）".to_string(),
                Some(Command::Primary(PrimaryRule::Lowest))
            ),
            (
                "Dragonfly 3 Master+".to_string(),
                Some(Command::Primary(pin_to("dragonfly3")))
            ),
            (
                "VGN Neon75".to_string(),
                Some(Command::Primary(pin_to("neon75")))
            ),
        ]
    );
}

/// 自动时托盘画电量最低的键盘；点"托盘上画哪一台 › Dragonfly 3 Master+"：图标**当场**换成鼠标（不等下一次取数），
/// 外壳收到"把 primary 写成 dragonfly3"，菜单上圆点与右列跟着换。
#[test]
fn pinning_a_device_redraws_the_icon_at_once_and_writes_it_back() {
    let (mut tray, mut screen) = both_read(MOUSE_AND_KEYBOARD);
    assert_eq!(screen.icon().percent, Some(30), "自动时画电量最低的键盘");

    feed(
        &mut tray,
        &mut screen,
        Event::Menu(Command::Primary(pin_to("dragonfly3"))),
    );

    assert_eq!(screen.icon().percent, Some(57), "钉死之后当场画鼠标");
    assert_eq!(
        screen.config,
        [config::Action::WritePrimary(pin_to("dragonfly3"))]
    );
    let menu = tray.menu(NOW);
    let (which, _) = which_device(&menu);
    assert_eq!(which.right.as_deref(), Some("Dragonfly 3 Master+"));
}

/// 钉死了鼠标，点"自动（电量最低）"：图标当场换回电量最低的键盘，外壳收到"把 primary 写成 lowest"。
#[test]
fn switching_back_to_automatic_redraws_the_icon_at_once_and_writes_lowest() {
    let pinned = format!("[general]\nprimary = \"dragonfly3\"\n{MOUSE_AND_KEYBOARD}");
    let (mut tray, mut screen) = both_read(&pinned);
    assert_eq!(screen.icon().percent, Some(57), "钉死时画鼠标");

    feed(
        &mut tray,
        &mut screen,
        Event::Menu(Command::Primary(PrimaryRule::Lowest)),
    );

    assert_eq!(screen.icon().percent, Some(30), "切回自动之后当场画键盘");
    assert_eq!(
        screen.config,
        [config::Action::WritePrimary(PrimaryRule::Lowest)]
    );
}

/// 按这份配置启动，两台在 `NOW` 都当场读到了：鼠标 57%（Dongle24G），键盘 30%（Ble）。
fn both_read(config: &str) -> (Tray, Screen) {
    let (mut tray, mut screen) = start(config, &LastKnown::default(), NOW);
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
        fetched("neon75", NOW, just_read(EndpointKind::Ble, 30, NOW)),
    );
    (tray, screen)
}

fn pin_to(id: &str) -> PrimaryRule {
    PrimaryRule::Pinned(id.to_string())
}

/// 点了"打开配置文件"：外壳收到"打开配置文件"（它交给系统默认程序打开）。
#[test]
fn open_config_file_asks_the_shell_to_open_it() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Menu(Command::OpenConfigFile));

    assert_eq!(screen.config, [config::Action::OpenFile]);
}

/// 点了"退出"就退出：外壳收到退出（它把图标拿掉、停下取数），之后时钟再怎么走，也不再问任何一台。
#[test]
fn quitting_stops_polling_for_good() {
    let (mut tray, mut screen) = start(MOUSE_AND_KEYBOARD, &LastKnown::default(), NOW);

    feed(&mut tray, &mut screen, Event::Menu(Command::Quit));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 3_600)));

    assert_eq!(screen.menu, [menu::Action::Quit]);
    assert_eq!(
        screen.fetches,
        ["dragonfly3", "neon75"],
        "退出之后一台都不再问"
    );
}
