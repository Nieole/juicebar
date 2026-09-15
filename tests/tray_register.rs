//! 托盘内核里「登记设备」这一块（`juicebar::tray::register`）：本机扫到、还没登记的蓝牙设备，"登记到"列谁不列谁，
//! "解除蓝牙登记"，点了之后交给外壳写回什么，以及什么时候扫一遍本机的蓝牙设备。
//!
//! 子菜单的结构与设计稿导出的基准逐项比对（`tests/menu_baselines/register*.txt`，帮手在 `common::menu`）；"新建一台
//! Device"那几行归票 12，比对时略去。

mod common;

use common::NOW;
use common::menu::{
    CANNED_DEVICES, Depth, assert_matches_design, baseline, baseline_text, canned, submenu,
};
use common::tray::{feed, later, start};
use juicebar::bluetooth::BleBattery;
use juicebar::config::Config;
use juicebar::state::LastKnown;
use juicebar::tray::menu::{Entry, Item, Kind, Menu};
use juicebar::tray::{Event, config, register};

/// 罐装：本机没扫到没登记的蓝牙设备，配置里也没有登记过蓝牙地址的 Device——子菜单里只有一行灰字。
#[test]
fn with_nothing_to_register_the_submenu_is_one_grayed_line() {
    let (tray, _screen) = canned("primary = \"lowest\"");

    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("register"),
        submenu(&menu, "登记设备"),
        Depth::Whole,
    );
}

/// 罐装，只改 neon75 登记了蓝牙地址：子菜单里是"解除蓝牙登记 ›"，里面是 neon75，右列写它登记的地址（照配置里的写法）。
#[test]
fn a_device_with_a_bluetooth_address_is_listed_under_unregister() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");

    feed(
        &mut tray,
        &mut screen,
        reloaded(&canned_with_neon75_address("f4ee2553b27e")),
    );
    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("register-ble_registered"),
        submenu(&menu, "登记设备"),
        Depth::Whole,
    );
}

/// 罐装那两台，neon75 另外登记了蓝牙地址 `address`。
fn canned_with_neon75_address(address: &str) -> String {
    format!(
        "[general]\nprimary = \"lowest\"\n{CANNED_DEVICES}\n  [device.bluetooth]\n  address = \"{address}\"\n"
    )
}

/// 外壳看到配置文件变了，重读了一遍，读好了，读出来的是 `text`。
fn reloaded(text: &str) -> Event {
    let config = Config::parse(text).expect("用例里的配置应当解析得动");
    Event::Config(config::Event::Reloaded(Ok(config)))
}

/// "登记设备 ›"里归票 12、还没做出来的那几行：比对时略去。
const NEW_DEVICE: &[&str] = &["    普通 「新建一台 Device」"];

/// 罐装，外加本机扫到一台没登记、带电量属性的蓝牙设备 WH-1000XM5：它一个子菜单，右列写扫到它的通路（Ble），里面是
/// "登记到 ›"，列出还没有蓝牙地址的罐装那两台。
#[test]
fn an_unregistered_ble_device_can_be_registered_to_each_device_without_an_address() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");

    feed(
        &mut tray,
        &mut screen,
        scanned(vec![found("WH-1000XM5", "38184c8f1a2b", Some(80))]),
    );
    let menu = tray.menu(NOW);

    assert_matches_design(
        baseline("register-unregistered_ble").without(NEW_DEVICE),
        submenu(&menu, "登记设备"),
        Depth::Whole,
    );
}

/// 本机扫到的一台蓝牙设备：名字 `name`、地址 `address`，电量属性 `level`（`None` 是没有电量属性）。
fn found(name: &str, address: &str, level: Option<u8>) -> BleBattery {
    BleBattery {
        instance_id: format!("用例里的蓝牙设备 {address}"),
        friendly_name: name.to_string(),
        address: address.to_string(),
        level,
        age_secs: Some(60),
        connected: true,
    }
}

/// 外壳照内核的吩咐扫了一遍本机的蓝牙设备，扫到的是 `found`。
fn scanned(found: Vec<BleBattery>) -> Event {
    Event::Register(register::Event::Scanned(Ok(found)))
}

/// "登记到"只列还没有蓝牙地址的 Device：neon75 已经登记着一个地址，就不在里面——一点就把旧地址悄悄覆盖掉的那一项
/// 不存在。
#[test]
fn register_to_lists_only_the_devices_that_have_no_bluetooth_address() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(
        &mut tray,
        &mut screen,
        reloaded(&canned_with_neon75_address("f4ee2553b27e")),
    );
    feed(
        &mut tray,
        &mut screen,
        scanned(vec![found("WH-1000XM5", "38184c8f1a2b", Some(80))]),
    );

    let menu = tray.menu(NOW);

    assert_eq!(
        texts(register_to(&menu, "WH-1000XM5")),
        ["Dragonfly 3 Master+"]
    );
}

/// "登记设备 › `name` › 登记到"那一项。
fn register_to<'a>(menu: &'a Menu, name: &str) -> &'a Entry {
    let [Item::Entry(register)] = submenu(menu, "登记设备") else {
        panic!("「登记设备」是一个 Entry");
    };
    let found = child(register, name);
    child(found, "登记到")
}

/// `parent` 子菜单里文字是 `text` 的那一项。
fn child<'a>(parent: &'a Entry, text: &str) -> &'a Entry {
    children(parent)
        .iter()
        .find_map(|item| match item {
            Item::Entry(entry) if entry.text == text => Some(entry),
            Item::Entry(_) | Item::Separator => None,
        })
        .unwrap_or_else(|| panic!("「{}」里没有「{text}」", parent.text))
}

/// `entry` 子菜单里的几项；不带子菜单就是空的。
fn children(entry: &Entry) -> &[Item] {
    match &entry.kind {
        Kind::Submenu(items) => items,
        Kind::Normal | Kind::Check | Kind::Radio => &[],
    }
}

/// `entry` 子菜单里每一项的文字（分隔线不算）。
fn texts(entry: &Entry) -> Vec<&str> {
    children(entry)
        .iter()
        .filter_map(|item| match item {
            Item::Entry(entry) => Some(entry.text.as_str()),
            Item::Separator => None,
        })
        .collect()
}

/// 两台都已经登记着蓝牙地址。
const BOTH_REGISTERED: &str = r#"
[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"

  [device.bluetooth]
  address = "e452430072a9"

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.bluetooth]
  address = "f4ee2553b27e"
"#;

/// 每台 Device 都已经有蓝牙地址："登记到"变灰、不带子菜单——没有一台登记得上（设计稿，parking lot Q284）。
#[test]
fn register_to_is_grayed_when_every_device_already_has_an_address() {
    let (mut tray, mut screen) = start(BOTH_REGISTERED, &LastKnown::default(), NOW);
    feed(
        &mut tray,
        &mut screen,
        scanned(vec![found("WH-1000XM5", "38184c8f1a2b", Some(80))]),
    );

    let menu = tray.menu(NOW);
    let to = register_to(&menu, "WH-1000XM5");

    assert!(to.grayed, "没得登记就变灰");
    assert_eq!(to.kind, Kind::Normal, "变灰的那一项不带子菜单");
}

/// 扫到的那一台，地址已经登记在某台 Device 上（写法不同也算：大小写、分隔符）：它不是"未登记"，不列——它已经是一级里的
/// 那一行设备行了。
#[test]
fn a_ble_device_whose_address_is_already_registered_is_not_listed() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(
        &mut tray,
        &mut screen,
        reloaded(&canned_with_neon75_address("f4ee2553b27e")),
    );
    feed(
        &mut tray,
        &mut screen,
        scanned(vec![
            found("VGN Neon75", "F4:EE:25:53:B2:7E", Some(64)),
            found("WH-1000XM5", "38184c8f1a2b", Some(80)),
        ]),
    );

    let menu = tray.menu(NOW);
    let [Item::Entry(register)] = submenu(&menu, "登记设备") else {
        panic!("「登记设备」是一个 Entry");
    };

    assert_eq!(texts(register), ["WH-1000XM5", "解除蓝牙登记"]);
}

/// 扫到的那一台没有电量属性（Windows 手上没有它的电量）：不列——登记上了，Ble 那一级也读不出它的电量。
#[test]
fn a_ble_device_without_a_battery_level_is_not_listed() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(
        &mut tray,
        &mut screen,
        scanned(vec![
            found("手环", "a1b2c3d4e5f6", None),
            found("WH-1000XM5", "38184c8f1a2b", Some(80)),
        ]),
    );

    let menu = tray.menu(NOW);
    let [Item::Entry(register)] = submenu(&menu, "登记设备") else {
        panic!("「登记设备」是一个 Entry");
    };

    assert_eq!(texts(register), ["WH-1000XM5"]);
}

/// 一级菜单不列未登记的蓝牙设备：它们的去处就是"登记设备 ›"，一级里再列一遍就是同一台说在两处（用户 2026-09-15 定，
/// parking lot Q286）。扫到了一台，一级一个字都不变。
#[test]
fn the_top_level_does_not_list_unregistered_ble_devices() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    let before = baseline_text(&tray.menu(NOW).items, Depth::TopLevel);

    feed(
        &mut tray,
        &mut screen,
        scanned(vec![found("WH-1000XM5", "38184c8f1a2b", Some(80))]),
    );

    assert_eq!(
        baseline_text(&tray.menu(NOW).items, Depth::TopLevel),
        before
    );
}

/// 点"登记设备 › WH-1000XM5 › 登记到 › VGN Neon75"：外壳收到"把这个蓝牙地址登记到 neon75"（它读此刻文件的全文、照配置
/// 那道缝写回），日志里记下点了什么。写回之后靠重读生效（parking lot Q312）。
#[test]
fn clicking_a_device_under_register_to_asks_the_shell_to_write_that_address_into_it() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(
        &mut tray,
        &mut screen,
        scanned(vec![found("WH-1000XM5", "38184c8f1a2b", Some(80))]),
    );
    let menu = tray.menu(NOW);
    let command = child(register_to(&menu, "WH-1000XM5"), "VGN Neon75")
        .command
        .clone()
        .expect("登记到里的每一台都点得到");

    feed(&mut tray, &mut screen, Event::Menu(command));

    assert_eq!(
        screen.config,
        [config::Action::RegisterBle {
            device_id: "neon75".to_string(),
            address: "38184c8f1a2b".to_string(),
        }]
    );
    assert_eq!(
        screen.logs.last().map(String::as_str),
        Some("登记设备 —— 把蓝牙地址 38184c8f1a2b 登记到 neon75")
    );
}

/// 点"登记设备 › 解除蓝牙登记 › VGN Neon75"：外壳收到"删掉 neon75 的蓝牙登记"，日志里记下删的是哪个地址——登记错了，照着
/// 日志还登记得回去。
#[test]
fn clicking_a_device_under_unregister_asks_the_shell_to_remove_its_address() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(
        &mut tray,
        &mut screen,
        reloaded(&canned_with_neon75_address("f4ee2553b27e")),
    );
    let menu = tray.menu(NOW);
    let [Item::Entry(register)] = submenu(&menu, "登记设备") else {
        panic!("「登记设备」是一个 Entry");
    };
    let command = child(child(register, "解除蓝牙登记"), "VGN Neon75")
        .command
        .clone()
        .expect("解除蓝牙登记里的每一台都点得到");

    feed(&mut tray, &mut screen, Event::Menu(command));

    assert_eq!(
        screen.config,
        [config::Action::UnregisterBle {
            device_id: "neon75".to_string(),
        }]
    );
    assert_eq!(
        screen.logs.last().map(String::as_str),
        Some("登记设备 —— 解除 neon75 的蓝牙登记（它登记的是 f4ee2553b27e）")
    );
}

/// 启动时就扫一遍本机的蓝牙设备：第一次右键弹菜单时，"登记设备 ›"里已经有东西可列。
#[test]
fn startup_scans_the_machine_bluetooth_devices_once() {
    let (_tray, screen) = canned("primary = \"lowest\"");

    assert_eq!(screen.ble_scans, 1);
}

/// 扫回来之后，隔 `poll_interval_bluetooth`（缺省 10 秒）再扫一遍：刚配对的蓝牙设备十来秒内就出现在"登记设备 ›"里。没到
/// 那一刻，时钟走多少格都不扫（parking lot Q310）。
#[test]
fn the_bluetooth_devices_are_scanned_again_every_bluetooth_poll_interval() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(&mut tray, &mut screen, scanned(Vec::new()));

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 9)));
    let before_the_interval = screen.ble_scans;
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 10)));

    assert_eq!(before_the_interval, 1, "没到间隔不扫");
    assert_eq!(screen.ble_scans, 2, "到了间隔再扫一遍");
}

/// 一遍扫描还没交回来（它与取数排在同一条队上，一次 HID 超时就是三秒）：到了间隔也不再要，免得队里排出一串扫描；交回来
/// 之后，到了间隔就再要。
#[test]
fn no_second_scan_is_asked_for_while_one_is_still_out() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");

    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 30)));
    let while_out = screen.ble_scans;
    feed(&mut tray, &mut screen, scanned(Vec::new()));
    feed(&mut tray, &mut screen, Event::Tick(later(NOW, 31)));

    assert_eq!(while_out, 1, "启动时那一遍还没交回来，不再要");
    assert_eq!(screen.ble_scans, 2, "交回来之后，到了间隔就再要");
}

/// 扫不了（设备树正在变、没有蓝牙硬件……）：完整原因记进日志，同一个原因接连扫不了只记一次——每 10 秒一行同样的话是噪音；
/// 扫成过一次之后再扫不了，再记。"登记设备 ›"照上一遍扫到的列，不因为一次扫不了就变空（parking lot Q310）。
#[test]
fn a_scan_that_failed_is_logged_once_per_reason_and_keeps_the_last_list() {
    let (mut tray, mut screen) = canned("primary = \"lowest\"");
    feed(
        &mut tray,
        &mut screen,
        scanned(vec![found("WH-1000XM5", "38184c8f1a2b", Some(80))]),
    );
    let logged_before = screen.logs.len();

    feed(
        &mut tray,
        &mut screen,
        scan_failed("设备列表反复变化，放弃枚举"),
    );
    feed(
        &mut tray,
        &mut screen,
        scan_failed("设备列表反复变化，放弃枚举"),
    );
    let menu = tray.menu(NOW);
    feed(&mut tray, &mut screen, scanned(Vec::new()));
    feed(
        &mut tray,
        &mut screen,
        scan_failed("设备列表反复变化，放弃枚举"),
    );

    assert_eq!(
        screen.logs[logged_before..],
        [
            "扫不了本机的蓝牙设备，登记设备里照上一遍扫到的列 —— 设备列表反复变化，放弃枚举",
            "扫不了本机的蓝牙设备，登记设备里照上一遍扫到的列 —— 设备列表反复变化，放弃枚举",
        ],
        "同一个原因接连两次只记一次，扫成过之后再记"
    );
    let [Item::Entry(register)] = submenu(&menu, "登记设备") else {
        panic!("「登记设备」是一个 Entry");
    };
    assert_eq!(texts(register), ["WH-1000XM5"], "照上一遍扫到的列");
}

/// 外壳照内核的吩咐扫了一遍本机的蓝牙设备，扫不了，完整原因是 `reason`。
fn scan_failed(reason: &str) -> Event {
    Event::Register(register::Event::Scanned(Err(reason.to_string())))
}
