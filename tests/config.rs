//! 配置解析：从 `config.toml` 读出有哪些 Device、每个 Device 有哪几条 Endpoint。

use juicebar::config::Config;

/// 只有一个 Device、只有 Dongle24G 一条 Endpoint —— 最小链路要的就这么多。
const ONE_DEVICE: &str = r#"
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
"#;

#[test]
fn reads_a_device_and_its_dongle_24g_endpoint() {
    let config = Config::parse(ONE_DEVICE).unwrap();

    let device = config.devices.first().expect("配置里应当有一个 Device");
    assert_eq!(device.id, "dragonfly3");
    assert_eq!(device.name, "Dragonfly 3 Master+");
    assert_eq!(device.driver, "vgn_mouse");

    let endpoint = device.dongle_24g.as_ref().expect("应当有 Dongle24G");
    assert_eq!(endpoint.vid, 0x391D);
    assert_eq!(endpoint.pid, 0x1A05);
    assert_eq!(endpoint.usage_page, 0xFF02);
    assert_eq!(endpoint.usage, 0x0002);
    assert_eq!(endpoint.report_id, 8);
}

/// 没有 Dongle24G 的 Device 照样能读出来 —— 键盘的有线块在自举时就是缺席的，
/// 缺一条 Endpoint 不该让整份配置读不动。
#[test]
fn a_device_may_have_no_endpoint_at_all() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "neon75"
        name = "VGN Neon75"
        driver = "vgn_keyboard"
        "#,
    )
    .unwrap();

    assert!(config.devices[0].dongle_24g.is_none());
}

/// 仓库里那份样例配置是给用户抄的，它必须真的能被解析。
/// 这条用例守的是文档与代码之间的漂移：schema 改了而样例没跟上，这里就会红。
#[test]
fn the_shipped_example_config_parses() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.toml");
    let text = std::fs::read_to_string(path).unwrap();

    let config = Config::parse(&text).unwrap();

    let ids: Vec<&str> = config.devices.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["dragonfly3", "neon75"]);
    assert_eq!(config.devices[1].driver, "vgn_keyboard");
}

/// 插线时才出现的那条 Endpoint 也要读得出来。
///
/// `pid` 与 Dongle24G 的 `0x1A05` 只差一个字符，而它们是**两个并存的设备**而不是
/// 替换关系（见 docs/adr/0001）——这条用例顺带把两个 pid 钉在一起对照。
#[test]
fn reads_a_device_wired_endpoint() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"

          [device.wired]
          vid = 0x391D
          pid = 0x1005
          usage_page = 0xFF02
          usage = 0x0002
          report_id = 8

          [device.wireless_24g]
          vid = 0x391D
          pid = 0x1A05
          usage_page = 0xFF02
          usage = 0x0002
          report_id = 8
        "#,
    )
    .unwrap();

    let device = &config.devices[0];
    let wired = device.wired.as_ref().expect("应当有 Wired");
    assert_eq!(wired.vid, 0x391D);
    assert_eq!(wired.pid, 0x1005);
    assert_eq!(wired.report_id, 8);
    assert_eq!(device.dongle_24g.as_ref().unwrap().pid, 0x1A05);
}

/// 没插线时 `[device.wired]` 是缺席的——自举扫不到它，缺一条 Endpoint 不该让整份
/// 配置读不动。
#[test]
fn a_device_may_have_no_wired_endpoint() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"
        "#,
    )
    .unwrap();

    assert!(config.devices[0].wired.is_none());
}

/// 蓝牙那条 Endpoint 配的是一个 MAC，不是一组 VID/PID —— 它不经过 HID，也不经过驱动。
#[test]
fn reads_a_device_bluetooth_endpoint() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"

          [device.bluetooth]
          address = "e452430072a9"
        "#,
    )
    .unwrap();

    let bluetooth = config.devices[0].bluetooth.as_ref().expect("应当有 Ble");
    assert_eq!(bluetooth.address, "e452430072a9");
}

/// 没配蓝牙的 Device 照样读得动 —— 三条 Endpoint 每一条都是可选的。
#[test]
fn a_device_may_have_no_bluetooth_endpoint() {
    let config = Config::parse(ONE_DEVICE).unwrap();

    assert!(config.devices[0].bluetooth.is_none());
}

/// MAC 有太多种写法：配置里写 `E4:52:43:00:72:A9`，Windows 报的是 `e452430072a9`，
/// 说的是同一台设备。
///
/// 容忍大小写和分隔符不是宽松，是因为认不出来的症状是"蓝牙那一级永远不在场"，
/// 而它和"设备没配对"长得一模一样——没有任何提示指向那个多写的冒号。
#[test]
fn recognizes_a_bluetooth_address_written_with_separators_or_capitals() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"

          [device.bluetooth]
          address = "E4:52:43:00:72:A9"
        "#,
    )
    .unwrap();

    let bluetooth = config.devices[0].bluetooth.as_ref().unwrap();
    assert!(bluetooth.matches("e452430072a9"));
    assert!(bluetooth.matches("E4-52-43-00-72-A9"));
    assert!(!bluetooth.matches("f4ee2553b27e"));
}

/// `show_unknown_ble` 缺省是关的：本机扫得到的 BLE 设备里，多数跟键鼠无关
/// （耳机、手机、手环），默认全列出来只会把两行有用的埋掉。
#[test]
fn show_unknown_ble_is_off_unless_the_config_asks_for_it() {
    let without_general = Config::parse(ONE_DEVICE).unwrap();
    assert!(!without_general.general.show_unknown_ble);

    let asked_for = Config::parse(
        r#"
        [general]
        show_unknown_ble = true
        "#,
    )
    .unwrap();
    assert!(asked_for.general.show_unknown_ble);
}

/// 轮询间隔与陈旧阈值读得出来。
///
/// 这五项以前是被静默忽略的键，现在陈旧判定要用它们了：两条 HID 的阈值由各自的轮询
/// 间隔推出来（3 倍），`Ble` 用那两个阈值。
#[test]
fn reads_the_polling_intervals_and_the_stale_thresholds() {
    let config = Config::parse(
        r#"
        [general]
        poll_interval_wired = 15
        poll_interval_24g = 45
        poll_interval_bluetooth = 5
        stale_after = 600
        very_stale_after = 7200
        "#,
    )
    .unwrap();

    let general = &config.general;
    assert_eq!(general.poll_interval_wired, 15);
    assert_eq!(general.poll_interval_24g, 45);
    assert_eq!(general.poll_interval_bluetooth, 5);
    assert_eq!(general.stale_after, 600);
    assert_eq!(general.very_stale_after, 7200);
}

/// 缺省值必须是 `config.example.toml` 注释里写的那几个数。
///
/// 缺省不能靠 `#[derive(Default)]` 给出的零：一个 0 秒的轮询间隔会让 3 倍推导出
/// 0 秒的陈旧阈值，于是**每一份读数在取到的同一刻就是陈旧的**——一份没写 `[general]`
/// 的配置会让整个工具把所有读数都标成不可信。这条用例守的就是那个塌陷。
#[test]
fn falls_back_to_the_documented_intervals_when_general_is_absent() {
    let general = Config::parse(ONE_DEVICE).unwrap().general;

    assert_eq!(general.poll_interval_wired, 30);
    assert_eq!(general.poll_interval_24g, 60);
    assert_eq!(general.poll_interval_bluetooth, 10);
    assert_eq!(general.stale_after, 3_600);
    assert_eq!(general.very_stale_after, 86_400);
}

/// `[general]` 里只写了一项的时候，其余各项仍取各自的缺省值。
///
/// 逐字段的缺省而不是整节的缺省：用户为了打开 `show_unknown_ble` 写一行
/// `[general]`，不该因此把三个轮询间隔全归零。
#[test]
fn a_partial_general_section_keeps_the_other_defaults() {
    let general = Config::parse(
        r#"
        [general]
        stale_after = 60
        "#,
    )
    .unwrap()
    .general;

    assert_eq!(general.stale_after, 60);
    assert_eq!(general.very_stale_after, 86_400);
    assert_eq!(general.poll_interval_wired, 30);
}
