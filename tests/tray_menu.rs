//! 托盘内核里「菜单」这一块（`juicebar::tray::menu`）：右键菜单上有什么，点了之后内核做什么。
//!
//! 菜单的结构与设计稿导出的基准逐项比对（`tests/menu_baselines/`，帮手在 `common::menu`）；设备行、告警行的
//! 确切字符串在 `tests/tray_device_row.rs`。

mod common;

use std::collections::BTreeSet;

use common::NOW;
use common::menu::{Depth, assert_matches_design, baseline, canned, submenu, which_device};
use common::menu::{canned_with_tray, device_row, exported_baselines, one_value_baselines};
use common::tray::{
    MOUSE_AND_KEYBOARD, Screen, feed, fetched, fetched_with_warning, just_read, later, start,
    state_not_saved, state_saved,
};
use juicebar::config::Config;
use juicebar::config::{PrimaryMark, TraySetting};
use juicebar::endpoints::EndpointKind;
use juicebar::icon::{Charging, Full, Glyph, Gray, IconSettings, NoLastKnown, Style};
use juicebar::primary::PrimaryRule;
use juicebar::round::Warning;
use juicebar::state::LastKnown;
use juicebar::tray::menu::{self, Command, Entry, Item, Kind, Menu};
use juicebar::tray::round::{self, IconRequest};
use juicebar::tray::{Action, Event, Tray, config};

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

/// 设计稿导出到 `tests/menu_baselines/` 的每一份基准，都有用例在比：设计稿多导出一份、没有用例去比它，这里就红。
///
/// "有用例在比"看的是测试目标（`tests/*.rs`，Cargo 自己找出来的就是这几个；`tests/common/` 不是）的源码里有没有这三种
/// 调用：`baseline("名字")`——一份 `Baseline` 除了交给 `assert_matches_design` 什么都做不了；`one_value_baselines("前缀")`
/// ——从目录里列出"从罐装出发只改一项"的那几份逐份比，列出来的都算；`preview_baselines("名字")`——`tests/icon.rs` 里逐像素
/// 比并排预览的那份网格。注释行不算。调用折成了几行、名字不是字面量的，这里认不出，照"没人比"红：宁可错红，不可错绿
/// （parking lot Q350）。
#[test]
fn every_baseline_the_design_exports_is_compared_by_some_test() {
    let compared = compared_baselines();

    let not_compared: Vec<String> = exported_baselines()
        .into_iter()
        .filter(|name| !compared.contains(name))
        .collect();

    assert!(
        not_compared.is_empty(),
        "设计稿导出了、却没有用例在比的基准（tests/menu_baselines/<名字>.txt）：{not_compared:?}"
    );
}

/// 测试目标的源码里比了哪几份基准（怎么认，见 [`every_baseline_the_design_exports_is_compared_by_some_test`]）。
fn compared_baselines() -> BTreeSet<String> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests");
    let mut compared = BTreeSet::new();
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("读不到 {dir}：{e}")) {
        let path = entry.expect("列得出 tests/ 里的一项").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读不到 {}：{e}", path.display()));
        for line in source
            .lines()
            .map(str::trim_start)
            .filter(|line| !line.starts_with("//"))
        {
            compared.extend(
                literal_arguments(line, "baseline")
                    .into_iter()
                    .map(str::to_owned),
            );
            compared.extend(
                literal_arguments(line, "preview_baselines")
                    .into_iter()
                    .map(str::to_owned),
            );
            for prefix in literal_arguments(line, "one_value_baselines") {
                compared.extend(
                    one_value_baselines(prefix)
                        .into_iter()
                        .map(|(name, _)| name),
                );
            }
        }
    }
    compared
}

/// 这一行里 `function("…")` 每一处括号里的那段字面量。紧挨在前面的若还是标识符的一部分（`one_value_baselines` 里的
/// `baselines`），那是另一个函数，不算。
fn literal_arguments<'a>(line: &'a str, function: &str) -> Vec<&'a str> {
    let call = format!("{function}(\"");
    line.match_indices(call.as_str())
        .filter(|(at, _)| !line[..*at].ends_with(|c: char| c == '_' || c.is_alphanumeric()))
        .filter_map(|(at, _)| {
            line[at + call.len()..]
                .split_once('"')
                .map(|(argument, _)| argument)
        })
        .collect()
}

/// 一级，罐装：设备行在上（Primary Device 打勾，取数失败的那一行变灰），分隔线，"托盘上画哪一台 ›""图标样式 ›""菜单显示 ›"
/// "登记设备 ›"与"开机自启"，分隔线，"打开配置文件"与"退出"。一级的每一份基准都整份比，一行都不略。
#[test]
fn the_top_level_matches_the_design() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);

    assert_matches_design(baseline("top"), &menu.items, Depth::TopLevel);
}

/// 一级，钉死 neon75：勾跟着这一轮的 Primary Device 挪到它那一行（它此刻取数失败，照样变灰），子菜单右列写它的名字。
#[test]
fn the_top_level_matches_the_design_when_a_failing_device_is_pinned() {
    let (tray, _screen) = canned("primary = \"neon75\"");

    let menu = tray.menu(NOW);

    assert_matches_design(baseline("top-primary-neon75"), &menu.items, Depth::TopLevel);
}

/// 一级，钉死 dragonfly3：这一轮的 Primary Device 本来就是它，一级与罐装只差子菜单右列。
#[test]
fn the_top_level_matches_the_design_when_the_lowest_device_is_pinned() {
    let (tray, _screen) = canned("primary = \"dragonfly3\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-primary-dragonfly3"),
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

    assert_matches_design(baseline("top-warnings"), &menu.items, Depth::TopLevel);
}

/// 一级，配置里一台 Device 都没有：设备行那一块是一句说明，"托盘上画哪一台"里只剩"自动（电量最低）"，其余照旧。
#[test]
fn the_top_level_matches_the_design_when_the_config_has_no_device() {
    let (tray, _screen) = start(NO_DEVICE, &LastKnown::default(), NOW);

    let menu = tray.menu(NOW);

    assert_matches_design(baseline("top-no_devices"), &menu.items, Depth::TopLevel);
}

/// 一级，从罐装出发关掉"写出来源和多久前"（`menu_source = false`）：有读数的那一行中段空着，取数失败的那一行照写短原因。
#[test]
fn the_top_level_matches_the_design_without_source_and_age() {
    let (tray, _screen) = canned_with_tray("menu_source = false");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-menu_source-false"),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一级，从罐装出发只留单选（`primary_mark = "radio"`）：设备行上不再打勾，这一轮是谁只看"托盘上画哪一台"之外的地方。
#[test]
fn the_top_level_matches_the_design_when_only_the_radio_marks_the_primary_device() {
    let (tray, _screen) = canned_with_tray("primary_mark = \"radio\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("top-primary_mark-radio"),
        &menu.items,
        Depth::TopLevel,
    );
}

/// 一台 Device 都没有时那一句说明：悬停提示那一句，接着指向"登记设备"里的"新建一台 Device"——首次运行一台设备都没插，
/// 插上之后从那里加进来（resident-tray 票 12）；普通项、不变灰（一台都没有正是现状，parking lot Q285），点了什么都不做。
#[test]
fn with_no_device_the_device_rows_are_one_sentence_saying_so() {
    let (tray, _screen) = start(NO_DEVICE, &LastKnown::default(), NOW);

    assert_eq!(
        rows_above_the_first_separator(&tray.menu(NOW)),
        [(
            "配置里一个 Device 都没有：插上设备或配对蓝牙之后，到\"登记设备\"里点\"新建一台 Device\"".to_string(),
            false,
            None
        )]
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

/// "图标样式 ›"，罐装（六项都是缺省值）：一级那一行右边不写字；第二层六行各写当前值，末尾一条分隔线与"恢复默认"；最里一层
/// 每个选项挂一张预览（固定画 57，"满电 100"那一组画 100；"灰状态"那一组三张并排），当前值那一项落圆点、加粗，缺省值
/// 那一项右边写"默认"，B 电池右边写"16、20 像素不显示数字"。罐装的托盘是 16 像素、菜单是深色，预览照它画。
#[test]
fn the_icon_style_submenu_matches_the_design() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("icon_style"),
        submenu(&menu, "图标样式"),
        Depth::Whole,
    );
}

/// "图标样式 ›"，从罐装出发只改一项，每个取值各一份：第二层那一行的右列换成新的取值，圆点与加粗挪过去；每一张预览
/// 都照新的设置画——只把它那一项换成它的选项，其余照用户此刻的设置（所以改了充电标记，别的几组预览也带着它）。
#[test]
fn the_icon_style_submenu_matches_the_design_after_changing_any_one_value() {
    let changed = one_value_baselines("icon_style");
    assert_eq!(
        changed.len(),
        14,
        "ADR-0005 管图标的六个键里，不是缺省值的取值一共 14 个（3 + 2 + 2 + 2 + 3 + 2）：{changed:?}"
    );

    for (name, line) in changed {
        let (tray, _screen) = canned_with_tray(&line);

        let menu = tray.menu(NOW);

        assert_matches_design(baseline(&name), submenu(&menu, "图标样式"), Depth::Whole);
    }
}

/// "菜单显示 ›"，罐装：两个勾选项，勾照 `menu_source` 与 `primary_mark` 打——罐装两项都勾着。
#[test]
fn the_menu_display_submenu_matches_the_design() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("menu_display"),
        submenu(&menu, "菜单显示"),
        Depth::Whole,
    );
}

/// "菜单显示 ›"，从罐装出发各关一项各一份：关掉的那一项不再打勾。
#[test]
fn the_menu_display_submenu_matches_the_design_after_switching_off_either_item() {
    let changed = one_value_baselines("menu_display");
    assert_eq!(changed.len(), 2, "两项各关一份：{changed:?}");

    for (name, line) in changed {
        let (tray, _screen) = canned_with_tray(&line);

        let menu = tray.menu(NOW);

        assert_matches_design(baseline(&name), submenu(&menu, "菜单显示"), Depth::Whole);
    }
}

/// 一级菜单里沿着这几段字一层层往里走到的那一项：`["图标样式", "画法"]` 是第二层"画法"那一行。
fn entry_at<'a>(menu: &'a Menu, path: &[&str]) -> &'a Entry {
    let mut items = menu.items.as_slice();
    let mut found = None;
    for text in path {
        let entry = items
            .iter()
            .find_map(|item| match item {
                Item::Entry(entry) if entry.text == *text => Some(entry),
                Item::Entry(_) | Item::Separator => None,
            })
            .unwrap_or_else(|| panic!("菜单里沿着 {path:?} 走不到「{text}」"));
        if let Kind::Submenu(children) = &entry.kind {
            items = children;
        }
        found = Some(entry);
    }
    found.expect("路径不是空的")
}

/// 这一项子菜单里每一项的字与点了交给内核的东西（分隔线跳过）。
fn choices(entry: &Entry) -> Vec<(String, Option<Command>)> {
    let Kind::Submenu(children) = &entry.kind else {
        panic!("「{}」不是子菜单", entry.text);
    };
    children
        .iter()
        .filter_map(|item| match item {
            Item::Entry(entry) => Some((entry.text.clone(), entry.command.clone())),
            Item::Separator => None,
        })
        .collect()
}

fn tray_setting(setting: TraySetting) -> Option<Command> {
    Some(Command::TraySetting(setting))
}

/// "图标样式"里每一项点了交给内核的，就是它说的那一项：六组里每个选项是 `[tray]` 里那个键的那个取值（键与取值照
/// ADR-0005，菜单上的字照设计稿），末尾"恢复默认"是恢复默认。
#[test]
fn each_icon_style_choice_hands_the_kernel_the_setting_it_stands_for() {
    let (tray, _screen) = canned("primary = \"lowest\"");
    let menu = tray.menu(NOW);

    let group = |title: &str| choices(entry_at(&menu, &["图标样式", title]));

    assert_eq!(
        group("画法"),
        [
            (
                "A 纯数字".to_string(),
                tray_setting(TraySetting::Style(Style::Number))
            ),
            (
                "B 电池".to_string(),
                tray_setting(TraySetting::Style(Style::Battery))
            ),
            (
                "C 圆环".to_string(),
                tray_setting(TraySetting::Style(Style::Ring))
            ),
            (
                "D 数字+底条".to_string(),
                tray_setting(TraySetting::Style(Style::Bar))
            ),
        ]
    );
    assert_eq!(
        group("字形"),
        [
            (
                "粗块".to_string(),
                tray_setting(TraySetting::Glyph(Glyph::Block))
            ),
            (
                "细体".to_string(),
                tray_setting(TraySetting::Glyph(Glyph::Fine))
            ),
            (
                "系统字体".to_string(),
                tray_setting(TraySetting::Glyph(Glyph::System))
            ),
        ]
    );
    assert_eq!(
        group("满电 100"),
        [
            (
                "照画 100".to_string(),
                tray_setting(TraySetting::Full(Full::Digits))
            ),
            (
                "画成 99".to_string(),
                tray_setting(TraySetting::Full(Full::Cap99))
            ),
            (
                "满格符号".to_string(),
                tray_setting(TraySetting::Full(Full::Block))
            ),
        ]
    );
    assert_eq!(
        group("灰状态"),
        [
            (
                "全部一个灰".to_string(),
                tray_setting(TraySetting::Gray(Gray::One))
            ),
            (
                "灰数字与灰符号两种".to_string(),
                tray_setting(TraySetting::Gray(Gray::Split))
            ),
            (
                "两种，另给暂停加琥珀点".to_string(),
                tray_setting(TraySetting::Gray(Gray::SplitPause))
            ),
        ]
    );
    assert_eq!(
        group("没有读数时"),
        [
            (
                "两道横线 --".to_string(),
                tray_setting(TraySetting::NoLastKnown(NoLastKnown::Dash))
            ),
            (
                "问号".to_string(),
                tray_setting(TraySetting::NoLastKnown(NoLastKnown::Question))
            ),
            (
                "空的轮廓".to_string(),
                tray_setting(TraySetting::NoLastKnown(NoLastKnown::Outline))
            ),
            (
                "程序图标".to_string(),
                tray_setting(TraySetting::NoLastKnown(NoLastKnown::Logo))
            ),
        ]
    );
    assert_eq!(
        group("充电标记"),
        [
            (
                "只靠绿色".to_string(),
                tray_setting(TraySetting::Charging(Charging::Color))
            ),
            (
                "24 像素以上加闪电".to_string(),
                tray_setting(TraySetting::Charging(Charging::BoltLarge))
            ),
            (
                "所有尺寸都加闪电".to_string(),
                tray_setting(TraySetting::Charging(Charging::Bolt))
            ),
        ]
    );
    assert_eq!(
        entry_at(&menu, &["图标样式", "恢复默认"]).command,
        Some(Command::RestoreIconDefaults)
    );
}

/// 点"图标样式 › 画法 › A 纯数字"：图标**当场**照新的样式重画（不等下一次取数），画的还是那一台、那个数；外壳收到"把
/// `[tray]` 的 style 写成 number"；再弹出的菜单里，"画法"那一行右边写"A 纯数字"。
#[test]
fn choosing_an_icon_style_redraws_the_icon_at_once_and_writes_that_key_back() {
    let (mut tray, mut screen) = both_read(MOUSE_AND_KEYBOARD);
    let before = screen.icon();

    feed(
        &mut tray,
        &mut screen,
        Event::Menu(Command::TraySetting(TraySetting::Style(Style::Number))),
    );

    assert_eq!(
        screen.icon(),
        IconRequest {
            settings: IconSettings {
                style: Style::Number,
                ..IconSettings::default()
            },
            ..before
        }
    );
    assert_eq!(
        screen.config,
        [config::Action::WriteTray(vec![TraySetting::Style(
            Style::Number
        )])]
    );
    assert_eq!(
        entry_at(&tray.menu(NOW), &["图标样式", "画法"])
            .right
            .as_deref(),
        Some("A 纯数字")
    );
}

/// "恢复默认"：起手照配置画的 C 圆环、所有尺寸加闪电，当场换回缺省的样式重画；外壳收到图标样式那六个键的缺省值；
/// "菜单显示"那两项不动。
#[test]
fn restoring_the_icon_defaults_redraws_the_icon_and_leaves_the_menu_display_alone() {
    let configured = format!(
        "[tray]\nstyle = \"ring\"\ncharging = \"bolt\"\nmenu_source = false\n{MOUSE_AND_KEYBOARD}"
    );
    let (mut tray, mut screen) = both_read(&configured);
    assert_eq!(
        screen.icon().settings,
        IconSettings {
            style: Style::Ring,
            charging: Charging::Bolt,
            ..IconSettings::default()
        },
        "起手就照配置里的 [tray] 画"
    );

    feed(
        &mut tray,
        &mut screen,
        Event::Menu(Command::RestoreIconDefaults),
    );

    assert_eq!(screen.icon().settings, IconSettings::default());
    assert_eq!(
        screen.config,
        [config::Action::WriteTray(vec![
            TraySetting::Style(Style::Bar),
            TraySetting::Glyph(Glyph::Block),
            TraySetting::Full(Full::Block),
            TraySetting::Gray(Gray::Split),
            TraySetting::NoLastKnown(NoLastKnown::Dash),
            TraySetting::Charging(Charging::Color),
        ])]
    );
    assert!(
        !entry_at(&tray.menu(NOW), &["菜单显示", "写出来源和多久前"]).checked,
        "菜单显示那两项不归恢复默认管"
    );
}

/// "菜单显示 › 写出来源和多久前"：开着时点它交出的是"关掉"（不是"切换"）；喂进去之后，下一次弹出的菜单里有读数的那一行
/// 只写名字，勾也落了，再点交出的是"打开"；外壳收到"把 menu_source 写成 false"；图标上没有来源，不重画。
#[test]
fn switching_off_source_and_age_shows_in_the_next_menu_and_is_written_back() {
    let (mut tray, mut screen) = both_read(MOUSE_AND_KEYBOARD);
    let click = entry_at(&tray.menu(NOW), &["菜单显示", "写出来源和多久前"])
        .command
        .clone()
        .expect("点得到");
    assert_eq!(click, Command::TraySetting(TraySetting::MenuSource(false)));

    let actions = tray.handle(Event::Menu(click));

    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::Round(round::Action::DrawIcon(_)))),
        "图标上没有来源，不重画：{actions:?}"
    );
    screen.apply(actions);
    assert_eq!(
        screen.config,
        [config::Action::WriteTray(vec![TraySetting::MenuSource(
            false
        )])]
    );
    let menu = tray.menu(NOW);
    assert_eq!(
        device_row(&menu, "Dragonfly 3 Master+").text,
        "Dragonfly 3 Master+"
    );
    let switch = entry_at(&menu, &["菜单显示", "写出来源和多久前"]);
    assert!(!switch.checked);
    assert_eq!(
        switch.command,
        Some(Command::TraySetting(TraySetting::MenuSource(true)))
    );
}

/// "菜单显示 › 在设备列表里标出 Primary Device"：开着时点它交出的是"只留单选"；喂进去之后设备行上的勾当场没了，再点交出
/// 的是"两处都标"；外壳收到"把 primary_mark 写成 radio"。
#[test]
fn switching_off_the_primary_device_mark_shows_in_the_next_menu_and_is_written_back() {
    let (mut tray, mut screen) = both_read(MOUSE_AND_KEYBOARD);
    let click = entry_at(
        &tray.menu(NOW),
        &["菜单显示", "在设备列表里标出 Primary Device"],
    )
    .command
    .clone()
    .expect("点得到");
    assert_eq!(
        click,
        Command::TraySetting(TraySetting::PrimaryMark(PrimaryMark::Radio))
    );

    feed(&mut tray, &mut screen, Event::Menu(click));

    assert_eq!(
        screen.config,
        [config::Action::WriteTray(vec![TraySetting::PrimaryMark(
            PrimaryMark::Radio
        )])]
    );
    let menu = tray.menu(NOW);
    assert!(
        !device_row(&menu, "VGN Neon75").checked,
        "这一轮的 Primary Device 是电量最低的键盘"
    );
    let switch = entry_at(&menu, &["菜单显示", "在设备列表里标出 Primary Device"]);
    assert!(!switch.checked);
    assert_eq!(
        switch.command,
        Some(Command::TraySetting(TraySetting::PrimaryMark(
            PrimaryMark::Both
        )))
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
