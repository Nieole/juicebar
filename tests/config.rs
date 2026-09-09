//! 配置解析：从 `config.toml` 读出有哪些 Device、每个 Device 有哪几条 Endpoint。

use juicebar::config::Config;
use juicebar::sources::level::LevelSource;

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

/// 每个 Device 自己的 `level_source`，不写就是 `auto`。
///
/// **没有全局默认是有意的**（spec「电量数值」）：想切的理由是"某型号固件 level 不准"，
/// 那本质上是设备相关的，不该有一个能连带影响其它设备的全局值。
#[test]
fn reads_the_level_source_of_each_device_and_defaults_to_auto() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"

        [[device]]
        id = "neon75"
        name = "VGN Neon75"
        driver = "vgn_keyboard"
        level_source = "reported"
        "#,
    )
    .unwrap();

    assert_eq!(config.devices[0].level_source, LevelSource::Auto);
    assert_eq!(config.devices[1].level_source, LevelSource::Reported);
}

/// 写错的 `level_source` 当场报错，**不静默当成 auto**。
///
/// 认不出的 `driver` 是留到取数时才报的（配置里可以出现本次编译还没实现的驱动名），
/// 这一项相反：它只有两个合法值，而"没认出来所以按缺省走"意味着用户以为自己钉住了
/// 数值来源、实际没有——而钉住它的理由恰恰是"这台设备的另一个来源不可信"。
#[test]
fn refuses_a_level_source_it_does_not_recognise() {
    // `{:#}` 而不是 `to_string()`：外层那句 context 只说"配置解析失败"，真正指出哪一项
    // 不认得的是错误链里面那一截，而 `status` 印给用户的也正是 `{e:#}`。
    let error = format!(
        "{:#}",
        Config::parse(
            r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"
        level_source = "derived"
        "#,
        )
        .unwrap_err()
    );

    assert!(error.contains("配置解析失败"), "{error}");
    // 光断言"解析失败"太松——缺个字段、TOML 语法写错都能让它变绿。这句话必须指出是
    // 哪一项、哪个值不认得，否则用户拿着它没法改。
    assert!(
        error.contains("level_source"),
        "要说清是哪一项不认得：{error}"
    );
    assert!(error.contains("derived"), "要说清读到的是什么：{error}");
    assert!(
        error.contains("auto") && error.contains("reported"),
        "要说清合法值有哪些：{error}"
    );
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
