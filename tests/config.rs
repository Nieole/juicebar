//! 配置文件这一头：解析（连同 `[tray]` 表）、自举草稿、自动补空块、`primary` 与 `[tray]` 的回写。
//!
//! 这几样东西各住一个文件（`src/config/`），而公开面仍然是一处，所以用例也在同一个文件里。

use juicebar::config::{self, Config};
use juicebar::config::{PrimaryMark, TraySetting, TraySettings};
use juicebar::endpoints::EndpointKind;
use juicebar::hid::HidInfo;
use juicebar::icon::{Charging, Full, Glyph, Gray, IconSettings, NoLastKnown, Style};
use juicebar::primary::PrimaryRule;
use juicebar::sources::level::LevelSource;

/// 造一条"本机枚举得到的 HID collection"。
///
/// 只有 VID/PID/usage page/usage 参与身份判断，报文长度参与"这条通路能不能发命令"，
/// 其余字段填得像真的即可。
fn collection(vid: u16, pid: u16, usage_page: u16, usage: u16, out: u16, feat: u16) -> HidInfo {
    HidInfo {
        path: format!("\\\\?\\hid#vid_{vid:04x}&pid_{pid:04x}&mi_01&col05#7&1234abcd&0&0000"),
        vid,
        pid,
        version: 0x0303,
        usage_page,
        usage,
        input_len: out,
        output_len: out,
        feature_len: feat,
        product: String::new(),
        manufacturer: String::new(),
    }
}

/// 鼠标的 2.4G 接收器，实测身份 `391D:1A05`。
fn mouse_dongle() -> HidInfo {
    collection(0x391D, 0x1A05, 0xFF02, 0x0002, 17, 0)
}

/// 插线时才出现的鼠标本体，实测身份 `391D:1005`。`pid` 与上面只差一个字符。
fn mouse_wired() -> HidInfo {
    collection(0x391D, 0x1005, 0xFF02, 0x0002, 17, 0)
}

/// 键盘的 2.4G 接收器，实测身份 `3151:5038`。
///
/// 它的 vendor collection 实测是 `in:0 out:0 feat:65` —— 一条**只有 feature 报文**的通路。
fn keyboard_dongle() -> HidInfo {
    collection(0x3151, 0x5038, 0xFFFF, 0x0002, 0, 65)
}

/// 拨到有线档并插线之后才出现的键盘本体，`3151:502F`。同样只有 feature 报文。
fn keyboard_wired() -> HidInfo {
    collection(0x3151, 0x502F, 0xFFFF, 0x0002, 0, 65)
}

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
    assert_eq!(device.driver.as_deref(), Some("vgn_mouse"));

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

/// 只走蓝牙的 Device 不写 `driver`：驱动只作用于 HID Endpoint，给它编一个驱动名等于在配置里说假话（spec「Device 的
/// schema」）。
#[test]
fn a_bluetooth_only_device_needs_no_driver() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "wh-1000xm5"
        name = "WH-1000XM5"

          [device.bluetooth]
          address = "38184c8f1a2b"
        "#,
    )
    .expect("只走蓝牙的 Device 不写 driver 也读得动");

    assert_eq!(config.devices[0].driver, None);
}

/// 配了 Wired 或 Dongle24G 却没写 `driver`，照旧是配置错误：那两条要靠驱动才读得出电量，读不动的配置当场说，不留到取数时。
#[test]
fn a_device_with_a_hid_endpoint_still_needs_a_driver() {
    for block in ["wired", "wireless_24g"] {
        let text = format!(
            "[[device]]\nid = \"dragonfly3\"\nname = \"Dragonfly 3 Master+\"\n\n  [device.{block}]\n  vid = 0x391D\n  pid = 0x1A05\n  usage_page = 0xFF02\n  usage = 0x0002\n  report_id = 8\n"
        );

        let error = Config::parse(&text).expect_err("配了 HID Endpoint 却没写 driver，读不动");

        let reason = format!("{error:#}");
        assert!(
            reason.contains("dragonfly3") && reason.contains("driver"),
            "要说出是哪一台缺了 driver：{reason}"
        );
    }
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
    assert_eq!(config.devices[1].driver.as_deref(), Some("vgn_keyboard"));
}

/// 样例配置写着 `[tray]` 整张表：八个键一个不少、次序照 ADR-0005，每一个都认得，写的正是缺省值（设计稿页底按缺省值
/// 生成的那一份）。样例里写一个认不出的取值、漏一个键，这里就红。
#[test]
fn the_example_config_writes_every_tray_key_with_its_default() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.toml");
    let text = std::fs::read_to_string(path).unwrap();

    let config = Config::parse(&text).unwrap();

    assert_eq!(config.tray, TraySettings::default());
    let keys: Vec<&str> = text
        .lines()
        .skip_while(|line| line.trim() != "[tray]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split('=').next())
        .map(str::trim)
        .collect();
    assert_eq!(
        keys,
        [
            "style",
            "glyph",
            "full",
            "gray",
            "no_last_known",
            "charging",
            "menu_source",
            "primary_mark"
        ]
    );
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
    // 不认得的是错误链里面那一截，而托盘起不来时说给用户的也正是 `{e:#}`。
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

/// 删掉的 `show_unknown_ble`（未登记的蓝牙设备的去处是菜单"登记设备"，用户 2026-09-15 定，parking lot Q286）还写在用户
/// 真在用的配置里：那份配置照样读得动，同一节里别的键照读——`[general]` 里认不出的键静默忽略，不让整份配置读不动。
#[test]
fn a_config_that_still_has_the_removed_show_unknown_ble_key_reads_as_before() {
    for written in ["false", "true"] {
        let config = Config::parse(&format!(
            "[general]\nprimary = \"lowest\"\nshow_unknown_ble = {written}\nlow_battery = 15\n\n\
             [[device]]\nid = \"dragonfly3\"\nname = \"Dragonfly 3 Master+\"\ndriver = \"vgn_mouse\"\n"
        ))
        .unwrap_or_else(|e| panic!("写着 show_unknown_ble = {written} 的旧配置该照样读得动：{e:#}"));

        assert_eq!(config.general.low_battery, 15, "同一节里别的键照读");
        assert_eq!(
            config.general.poll_interval_wired, 30,
            "没写的键照旧取缺省值"
        );
        assert_eq!(config.devices.len(), 1);
    }
}

// ---------------------------------------------------------------
// 自举草稿
// ---------------------------------------------------------------

/// 认得出来的设备要写成一条**直接能用**的 `[[device]]`：id、name、driver、以及扫到的
/// 那条 Endpoint 的真实身份，用户一个字都不用补。
#[test]
fn the_draft_writes_a_recognised_device_ready_to_use() {
    let text = config::draft(&[mouse_dongle()]);

    let config = Config::parse(&text).expect("草稿本身必须是一份解析得动的配置");
    let device = config
        .devices
        .first()
        .expect("扫到的鼠标应当写成一个 Device");
    assert_eq!(device.id, "dragonfly3");
    assert_eq!(device.name, "Dragonfly 3 Master+");
    assert_eq!(device.driver.as_deref(), Some("vgn_mouse"));

    let dongle = device.dongle_24g.as_ref().expect("应当有 Dongle24G");
    assert_eq!(dongle.vid, 0x391D);
    assert_eq!(dongle.pid, 0x1A05);
    assert_eq!(dongle.usage_page, 0xFF02);
    assert_eq!(dongle.usage, 0x0002);
    assert_eq!(dongle.report_id, 8);
}

/// 扫不到的 Endpoint 不许就这么消失：它要以**注释掉的占位**写进草稿，并写明怎么才能
/// 让它出现。用户不该在不知情的情况下缺一整条 Endpoint。
#[test]
fn the_draft_leaves_an_absent_endpoint_as_a_commented_placeholder() {
    // 只插着 dongle，鼠标本体没插 —— 自举那一刻的常态。
    let text = config::draft(&[mouse_dongle()]);

    // 占位是注释，不是一份猜出来的配置：解析出来仍然没有 Wired。
    let config = Config::parse(&text).expect("草稿必须解析得动");
    assert!(
        config.devices[0].wired.is_none(),
        "占位必须是注释，不能被解析成真的 Wired 块"
    );

    // 但文本里看得见它：块名、实测记下来的身份、以及补全办法。
    assert!(text.contains("# [device.wired]"), "缺的块要写出块名");
    assert!(
        text.contains("# pid = 0x1005"),
        "占位里要有实测记下来的 pid"
    );
    // 补全办法：插上之后程序会自动补上，不再叫人去跑命令行。占位那一段与开头那段话都这么说。
    let wired = comment_run_with_block_name(&text, "wired").join("\n");
    assert!(
        wired.contains("插上之后程序会自动补上"),
        "要写明补全办法：{wired}"
    );
    assert!(
        !wired.contains("config-refresh"),
        "不再叫人去跑命令行：{wired}"
    );
    let preamble = &text[..text.find("[general]").unwrap()];
    assert!(
        preamble.contains("插上之后程序会自动补上"),
        "开头那段也这么说：{preamble}"
    );
    assert!(!preamble.contains("config-refresh"), "{preamble}");
}

/// 补不上的那几条 Endpoint 的占位出自**同一套逻辑**：同样的开头、同样的"怎么让它出现"、
/// 同样的 `  # ` 缩进，只有"为什么补不上"和注释掉的那几行逐种类不同。
///
/// 守的是"两条生成、一条手写"这件事不再回来：蓝牙那一段曾经是个硬写的常量，措辞和排布
/// 与另两条各说各话——而用户读到的是同一份文件里三条本该长得一样的 Endpoint。
#[test]
fn the_draft_writes_the_endpoint_placeholders_from_one_shape() {
    // 只插着 dongle：Dongle24G 是真块，Wired 与 Ble 都只能是占位。
    let text = config::draft(&[mouse_dongle()]);

    // 占位**始终**只是注释：两块都不许解析成真配置。`address` 那一行是本次改动第一次往
    // 草稿里写，那个 `#` 一旦掉了，用户会拿到一台地址是"12 位十六进制的蓝牙地址…"的蓝牙设备。
    let config = Config::parse(&text).expect("草稿必须解析得动");
    assert!(config.devices[0].wired.is_none(), "Wired 那一块只能是注释");
    assert!(
        config.devices[0].bluetooth.is_none(),
        "Ble 那一块只能是注释 —— 那个地址是叫用户去抄的占位，不是一个真地址"
    );

    for key in ["wired", "bluetooth"] {
        let lines = comment_run_with_block_name(&text, key);
        assert!(
            lines.iter().all(|line| line.starts_with("  # ")),
            "占位整段都该是缩进两格的注释：{key} 那一段是 {lines:#?}"
        );
        let opening = lines.first().expect("占位不该是空的");
        assert!(
            opening.ends_with("所以下面这一块是**注释**，不是配置。"),
            "开头那句要说清这一块是注释、不是配置：{key} 那一段开头是 {opening:?}"
        );
        let how = lines.get(1).expect("占位该有第二句");
        assert!(
            how.starts_with("  # 怎么让它出现："),
            "紧接着要说清怎么才能让它出现：{key} 那一段第二句是 {how:?}"
        );
    }

    // 一样的形状不等于一样的话：蓝牙那一条补不上的理由和另两条根本不同 —— 它不是
    // "这次扫不到"，是程序永远不该猜，所以那一段要说清为什么，并指向去哪儿抄。
    let ble = comment_run_with_block_name(&text, "bluetooth").join("\n");
    assert!(
        ble.contains("另一颗芯片"),
        "蓝牙那一段要说清地址为什么猜不出来：{ble}"
    );
    assert!(
        ble.contains("登记设备"),
        "还要说清去托盘菜单的哪儿登记它：{ble}"
    );
    assert!(!ble.contains("scan"), "不再叫人去跑命令行抄地址：{ble}");

    // 在场的那一条仍然是真块，所以上面那两段的形状不是"整份草稿都被注释掉了"。
    assert!(
        text.lines()
            .any(|line| line.trim() == "[device.wireless_24g]"),
        "在场的那一条该写成真块，不是占位"
    );
}

/// 含注释掉的 `[device.<key>]` 块名的那一串**连续注释行**。名字说的就是它切的东西：
/// 它不认识"占位"，只认识注释行连成的一段。
///
/// 按连续注释行切而不是按固定行数切，是为了让上面那条用例守的是**形状**，不是某一版
/// 措辞占了几行。代价是一条隐式契约：草稿里每段占位前面**恰好有一个空行**（`draft` 的
/// 循环每轮先 `push('\n')`）。那个空行要是没了，这里会把上一段注释（`OPTIONAL_LOW_BATTERY`
/// 或 `OPTIONAL_ADDRESS`）一起吞进来，而断言会在一个看不出所以然的地方失败——所以下面
/// 把这条契约**当场核一遍**，让它响在自己的名下。
fn comment_run_with_block_name<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    let name = format!("# [device.{key}]");
    let lines: Vec<&str> = text.lines().collect();
    let commented = |line: &str| line.trim_start().starts_with('#');
    let at = lines
        .iter()
        .position(|line| line.trim() == name)
        .unwrap_or_else(|| panic!("草稿里没有注释掉的 [device.{key}] 块"));
    let start = lines[..at]
        .iter()
        .rposition(|line| !commented(line))
        .map_or(0, |index| index + 1);
    assert!(
        start > 0 && lines[start - 1].trim().is_empty(),
        "这个夹具假定每段占位前面有一个空行，而 {key} 那一段前面是 {:?} —— \
         假定破了就别让它静默切错，先来改这里",
        lines.get(start.wrapping_sub(1))
    );
    let end = at + 1 + lines[at + 1..].iter().take_while(|l| commented(l)).count();
    lines[start..end].to_vec()
}

/// 怎样才能让 Wired 出现是**逐台设备**不同的，草稿里那句提示因此不能是一句通用套话：
/// 鼠标插上线就行，键盘还得拨机身上的模式开关。
#[test]
fn the_draft_says_how_to_make_a_wired_endpoint_appear_per_device() {
    let mouse = config::draft(&[mouse_dongle()]);
    let keyboard = config::draft(&[keyboard_dongle()]);

    assert!(mouse.contains("插上 USB 线"), "鼠标的补全办法是插线");
    assert!(
        keyboard.contains("模式开关"),
        "键盘的补全办法是先拨模式开关"
    );
    assert!(
        !keyboard.contains("插上 USB 线它就会"),
        "键盘不该抄鼠标那句话 —— 仅插线对它不够"
    );
}

/// `level_source` 要在草稿里**显式**写出来。它的取值语义是另一张票的活，但用户得先看见
/// 这个开关存在——否则某型号固件的 level 不准时，他不会想到自己有退路。
#[test]
fn the_draft_spells_out_level_source() {
    let text = config::draft(&[mouse_dongle()]);

    assert!(
        text.lines()
            .any(|line| !line.trim_start().starts_with('#')
                && line.contains("level_source = \"auto\"")),
        "level_source 要是真的一行配置，不能只在注释里提一句、也不能靠缺省"
    );
    assert!(
        text.contains("reported"),
        "写出开关还不够，另一个取值也要写出来，否则看不出它能切到哪里去"
    );
}

/// 表里没有的 vendor collection 也要出现在草稿里，写成**注释掉的 `[[device]]` 骨架**并
/// 附上扫到的身份。程序不猜它是谁、更不猜该用哪个驱动——但既然扫到了，就不该不声不响地
/// 丢掉：那正是"用户不该在不知情的情况下缺一整条 Endpoint"。
#[test]
fn the_draft_lists_an_unrecognised_vendor_collection_as_a_commented_skeleton() {
    // 同一件不认识的硬件暴露两条 vendor collection，外加一条普通鼠标 collection。
    let text = config::draft(&[
        mouse_dongle(),
        collection(0x1234, 0x5678, 0xFF01, 0x0001, 32, 0),
        collection(0x1234, 0x5678, 0xFF00, 0x0001, 8, 0),
        collection(0x046D, 0xC52B, 0x0001, 0x0002, 0, 0),
    ]);

    let config = Config::parse(&text).expect("草稿必须解析得动");
    let ids: Vec<&str> = config.devices.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids,
        ["dragonfly3"],
        "认不出来的硬件不能被当成一个真的 Device 写进去"
    );

    assert_eq!(
        text.matches("VID_1234&PID_5678").count(),
        1,
        "一件硬件列一次 —— 它暴露几条 collection 是它自己的事"
    );
    for listed in ["UP:FF01 U:0001", "UP:FF00 U:0001"] {
        assert!(
            text.contains(listed),
            "它的每条 collection 都要列出来，用户得据此挑一条：{listed}"
        );
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.trim() == "# [[device]]")
            .count(),
        1,
        "照着填的骨架整节只写一份，不必逐条重复"
    );
    assert!(
        !text.contains("C52B"),
        "普通鼠标/键盘 collection 不该混进来：私有协议一定落在 vendor 页"
    );
}

/// 草稿要向 `config.example.toml` 看齐，把 `[general]` 那几个开关也写出来：用户看不见的
/// 开关等于不存在，而草稿是他唯一真会读到的那份文件。
#[test]
fn the_draft_carries_the_general_switches() {
    let text = config::draft(&[mouse_dongle()]);

    assert!(text.starts_with('#'), "开头要有一段说明这是什么文件");
    for key in [
        "primary",
        "poll_interval_wired",
        "poll_interval_24g",
        "low_battery",
        "pause_when_vendor_hub_running",
    ] {
        assert!(
            text.lines()
                .any(|line| line.starts_with(&format!("{key} = "))),
            "{key} 要写成真的一行配置"
        );
    }
    // TOML 的坑：顶层键写在一个表后面就落进那个表里了。[general] 必须在第一条
    // [[device]] 之前，否则这几个开关会静默变成某个 Device 的字段。
    assert!(
        text.find("[general]") < text.find("\n[[device]]"),
        "[general] 必须在所有 [[device]] 之前"
    );
}

/// 一台设备都没扫到时，草稿也得是一份**说得清话**的文件：解析得动，而且说明它为什么是空的。
#[test]
fn the_draft_says_so_when_nothing_was_found() {
    let text = config::draft(&[]);

    let config = Config::parse(&text).expect("空草稿也必须解析得动");
    assert!(config.devices.is_empty());
    assert!(text.contains("都没扫到"), "要说清楚为什么一台设备都没有");
    // 插上之后到托盘菜单"登记设备"里点"新建一台 Device"就加得进来（resident-tray 票 12）：不必删掉文件、重新启动。
    assert!(
        text.contains("登记设备") && text.contains("新建一台 Device"),
        "要说清楚一台都没有时怎么办：{text}"
    );
    assert!(!text.contains("重新启动"), "不再叫人删掉文件重来：{text}");
    assert!(
        !text.contains("config-refresh"),
        "不再叫人去跑命令行：{text}"
    );
    assert!(
        !["status", "config-refresh", "scan", "caps", "probe"]
            .iter()
            .any(|command| text.contains(&format!("juicebar {command}"))),
        "成品没有命令行，不叫人跑 juicebar 的任何一条子命令：{text}"
    );
}

// ---------------------------------------------------------------
// 自动补空块：只填空缺（`config::refresh`）
// ---------------------------------------------------------------

/// 插好线之后跑一次，空着的 `[device.wired]` 就被填上真身份——用户不必手抄 VID/PID。
#[test]
fn config_refresh_fills_an_empty_endpoint_block_that_is_now_present() {
    // ONE_DEVICE 正是自举时的常态：dongle 有了，Wired 那一块还空着。
    let refreshed = config::refresh(ONE_DEVICE, &[mouse_dongle(), mouse_wired()]).unwrap();

    let config = Config::parse(&refreshed.text).expect("补全之后必须还解析得动");
    let wired = config.devices[0].wired.as_ref().expect("Wired 应当被填上");
    assert_eq!(wired.vid, 0x391D);
    assert_eq!(wired.pid, 0x1005);
    assert_eq!(wired.usage_page, 0xFF02);
    assert_eq!(wired.usage, 0x0002);
    assert_eq!(wired.report_id, 8);

    assert!(
        refreshed
            .filled
            .iter()
            .any(|fill| fill.to_string().contains("Wired")),
        "填了什么要说出来，不能悄悄改用户的文件"
    );
}

/// 草稿里注释掉的占位也是空块：插上线之后补上的就是那一块，**别的一个字节都不动**——占位那几行注释、
/// 用户没碰过的每一行、行与行之间的空行，原样按原次序留着。托盘每插上一次设备就可能跑一遍它，所以这里
/// 逐行守着"只多了这几行"。
#[test]
fn config_refresh_fills_a_drafted_placeholder_and_adds_nothing_but_that_block() {
    // 首次运行时插着两个接收器：两台都写进了草稿，两台的 Wired 都是注释掉的占位。
    let before = config::draft(&[mouse_dongle(), keyboard_dongle()]);
    // 之后插上了鼠标的线。
    let refreshed =
        config::refresh(&before, &[mouse_dongle(), keyboard_dongle(), mouse_wired()]).unwrap();

    let (at, inserted) = inserted_lines(&before, &refreshed.text);
    assert_eq!(
        inserted,
        [
            "\n",
            "  [device.wired]\n",
            "  vid = 0x391D\n",
            "  pid = 0x1005\n",
            "  usage_page = 0xFF02\n",
            "  usage = 0x0002\n",
            "  report_id = 8\n",
        ],
        "多出来的只该是鼠标那一块 Wired（插在第 {at} 行）"
    );
    let config = Config::parse(&refreshed.text).unwrap();
    assert_eq!(
        config.devices[0].wired.as_ref().expect("补在鼠标底下").pid,
        0x1005
    );
    assert!(
        config.devices[1].wired.is_none(),
        "键盘的线没插，它那块照旧空着"
    );
}

/// **不新增 `[[device]]`**：本机插着一台认得的设备、而配置里没有它，也一个字节都不写。自动的事分不清"用户删掉的"
/// 和"从没加过的"，新增只能由用户在菜单里点（resident-tray 票 12）。
#[test]
fn config_refresh_never_adds_a_device() {
    // ONE_DEVICE 里只有鼠标；键盘的接收器与本体都插着。
    let refreshed = config::refresh(
        ONE_DEVICE,
        &[mouse_dongle(), keyboard_dongle(), keyboard_wired()],
    )
    .unwrap();

    assert_eq!(refreshed.text, ONE_DEVICE);
    assert!(refreshed.filled.is_empty());
}

/// 没写 `driver` 的 Device 说的是"我只走蓝牙"：它不是身份表里哪一台 HID 设备，id 恰好叫 `neon75` 也不是。键盘的接收器插着，
/// 补空块也不往它里面补 HID 块（补了就是一份缺 `driver` 的配置，读不动），一句提醒都不说；菜单"登记设备"照样把 VGN Neon75
/// 列成认得但没登记的那一台。
#[test]
fn config_refresh_leaves_a_bluetooth_only_device_alone() {
    let bluetooth_only = r#"[[device]]
id = "neon75"
name = "我的蓝牙键盘"

  [device.bluetooth]
  address = "f4ee2553b27e"
"#;

    let refreshed =
        config::refresh(bluetooth_only, &[keyboard_dongle(), keyboard_wired()]).unwrap();

    assert_eq!(refreshed.text, bluetooth_only, "一个字节都不补");
    assert!(
        refreshed.notes.is_empty(),
        "只走蓝牙的没有缺什么：{:?}",
        refreshed.notes
    );
    let unregistered = Config::parse(bluetooth_only)
        .unwrap()
        .unregistered_known_devices(&[keyboard_dongle()]);
    let ids: Vec<&str> = unregistered.iter().map(|hid| hid.known.id).collect();
    assert_eq!(ids, ["neon75"], "VGN Neon75 还不在配置里");
}

/// `after` 比 `before` 多出来的那一段连续的行（每行带着自己的换行），以及它插在第几行（从 0 数）。
///
/// **`before` 的每一行都得原样、按原次序留在 `after` 里**，否则当场失败：它守的正是"只多了这几行，别的
/// 一个字节没动"。行按 `\n` 切、切出来的行带着换行，所以连空行与行尾也逐字节比。
fn inserted_lines<'a>(before: &str, after: &'a str) -> (usize, Vec<&'a str>) {
    let old: Vec<&str> = before.split_inclusive('\n').collect();
    let new: Vec<&str> = after.split_inclusive('\n').collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    assert_eq!(
        head + tail,
        old.len(),
        "原有的行被改了或删了：从第 {head} 行起，原来是 {:?}，现在是 {:?}",
        &old[head..old.len() - tail],
        &new[head..new.len() - tail]
    );
    (head, new[head..new.len() - tail].to_vec())
}

/// ADR-0003 划死的边界：只补空缺。用户写过的值和他写的注释，一个字都不许动。
#[test]
fn config_refresh_touches_nothing_the_user_wrote() {
    let before = "\
# 顶上这句是我自己写的，一个字都不许动
[[device]]
id = \"dragonfly3\"
name = \"我给它起的名字\"
driver = \"vgn_mouse\"

  # 这一块我按自己的坑改过 report_id
  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 9
";
    let refreshed = config::refresh(before, &[mouse_dongle(), mouse_wired()]).unwrap();

    for kept in [
        "# 顶上这句是我自己写的，一个字都不许动",
        "# 这一块我按自己的坑改过 report_id",
        "name = \"我给它起的名字\"",
        "report_id = 9",
    ] {
        assert!(refreshed.text.contains(kept), "不该动的东西被动了：{kept}");
    }

    let config = Config::parse(&refreshed.text).unwrap();
    assert_eq!(
        config.devices[0].dongle_24g.as_ref().unwrap().report_id,
        9,
        "程序知道的是 8，但用户写的是 9 —— 用户赢"
    );
    assert!(config.devices[0].wired.is_some(), "空着的那一块还是要补上");
}

/// 不在场的 Endpoint 不许凭表里的记录填进去：填了等于替用户断言"你这条线插着"，
/// 而下一个取数周期会因为一条根本不在的通路白试一次。什么都没补时文件**逐字节**不变。
#[test]
fn config_refresh_fills_nothing_when_the_endpoint_is_absent() {
    let refreshed = config::refresh(ONE_DEVICE, &[mouse_dongle()]).unwrap();

    assert_eq!(
        refreshed.text, ONE_DEVICE,
        "什么都没补的时候不该把文件重写一遍"
    );
    assert!(refreshed.filled.is_empty());
    assert!(
        refreshed.notes.iter().any(|note| matches!(
            note,
            config::Note::NotFilled(text) if text.contains("Wired") && text.contains("不在场")
        )),
        "要说明为什么没填"
    );
    assert!(
        refreshed
            .notes
            .iter()
            .any(|note| note.to_string().contains("插上 USB 线")),
        "还要说清楚怎么才能让它出现"
    );
}

/// 键盘那条最麻烦的 Endpoint 也要补得上：它的 vendor collection 实测是 `in:0 out:0 feat:65`
/// ——一条**发不出输出报文**的通路。取数那一步按"能发输出报文"筛通路，而往配置里写一条身份
/// 不该受那道筛子约束，否则最需要自动补空块的那一条永远补不上。
#[test]
fn config_refresh_fills_a_feature_only_endpoint_too() {
    let before = "\
[[device]]
id = \"neon75\"
name = \"VGN Neon75\"
driver = \"vgn_keyboard\"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0
";
    let refreshed = config::refresh(before, &[keyboard_dongle(), keyboard_wired()]).unwrap();

    let config = Config::parse(&refreshed.text).unwrap();
    let wired = config.devices[0]
        .wired
        .as_ref()
        .expect("键盘的 Wired 也该补得上，它只有 feature 报文不是理由");
    assert_eq!(wired.pid, 0x502F);
}

/// 补的块要落在**对的那一个** `[[device]]` 底下。这不是排版讲究：TOML 里
/// `[device.wired]` 属于它前面最近的那个 `[[device]]`，落错位置就是把一台设备的有线身份
/// 挂到另一台头上——而文件照样解析得动，错误只会在取数时以一句"读不到"的面目出现。
#[test]
fn config_refresh_puts_the_block_under_the_right_device() {
    let before = "\
[[device]]
id = \"dragonfly3\"
name = \"Dragonfly 3 Master+\"
driver = \"vgn_mouse\"

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

[[device]]
id = \"neon75\"
name = \"VGN Neon75\"
driver = \"vgn_keyboard\"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0
";
    // 键盘拨到了有线档并插着线，鼠标没插线。
    let refreshed = config::refresh(
        before,
        &[mouse_dongle(), keyboard_dongle(), keyboard_wired()],
    )
    .unwrap();

    let config = Config::parse(&refreshed.text).unwrap();
    assert!(
        config.devices[0].wired.is_none(),
        "鼠标没插线，它不该凭空多出一块"
    );
    assert_eq!(
        config.devices[1]
            .wired
            .as_ref()
            .expect("键盘的 Wired 该补在键盘底下")
            .pid,
        0x502F
    );
}

/// 认不出来的设备一律不猜：程序说"我认不出来，你得手填"，而不是拿本机凑巧扫到的别人的
/// 身份填上去。
#[test]
fn config_refresh_refuses_to_guess_for_an_unrecognised_device() {
    let before = "\
[[device]]
id = \"我自己接的一台设备\"
name = \"某某\"
driver = \"vgn_mouse\"

  [device.wireless_24g]
  vid = 0x1234
  pid = 0x5678
  usage_page = 0xFF01
  usage = 0x0001
  report_id = 0
";
    // 本机还插着鼠标本体 —— 它绝不该被安到这台认不出来的设备头上。
    let refreshed = config::refresh(
        before,
        &[
            collection(0x1234, 0x5678, 0xFF01, 0x0001, 32, 0),
            mouse_wired(),
        ],
    )
    .unwrap();

    assert_eq!(refreshed.text, before, "认不出来就什么都别写");
    assert!(
        refreshed
            .notes
            .iter()
            .any(|note| matches!(note, config::Note::NotFilled(text) if text.contains("认不出"))),
        "但要说出来自己认不出，而不是一声不吭"
    );
}

/// 一块都没配的 Device 也要认得出来 —— 靠 `id` 兜底。
///
/// 这不是假想的情形：用户手写一条 `[[device]]` 骨架、或者删掉某一块想让程序重新补上，
/// 都会落到这里，而"看它已配好的身份"那条路在一块都没有时问不出任何东西。
#[test]
fn config_refresh_recognises_a_device_by_id_when_no_block_is_configured() {
    let before = "\
[[device]]
id = \"neon75\"
name = \"VGN Neon75\"
driver = \"vgn_keyboard\"
";
    // 键盘的 dongle 插着，本体没有（开关不在有线档）。
    let refreshed = config::refresh(before, &[keyboard_dongle()]).unwrap();

    let config = Config::parse(&refreshed.text).unwrap();
    assert_eq!(
        config.devices[0]
            .dongle_24g
            .as_ref()
            .expect("Dongle24G 在场，该补上")
            .pid,
        0x5038
    );
    assert!(
        config.devices[0].wired.is_none(),
        "不在场的那一条仍然不许填"
    );
}

/// Ble 也是一条 Endpoint，草稿不该让用户不知道它存在。
///
/// 它的地址程序猜不出来——同一只鼠标的 BLE 射频是另一颗芯片，连 VID 都和 dongle 不同，
/// 几套身份之间没有能缝合的字段——所以它是注释掉的占位，并写明去托盘菜单的哪儿登记它的地址。
#[test]
fn the_draft_leaves_room_for_the_ble_endpoint() {
    let text = config::draft(&[mouse_dongle()]);

    assert!(text.contains("# [device.bluetooth]"), "Ble 那一块要写出来");
    assert!(text.contains("登记设备"), "要写明去托盘菜单的哪儿登记 MAC");
    assert!(
        !text.lines().any(|line| line.trim() == "[device.bluetooth]"),
        "地址猜不出来，所以这一块只能是注释"
    );
    Config::parse(&text).expect("草稿必须解析得动");
}

/// 草稿的 `[general]` 与 `config.example.toml` 的 `[general]` 必须逐项一致。
///
/// 两份文件各写一遍是没办法的事（一份是 Rust 里的字符串，一份是仓库里给人读的样例），
/// 但"改一处忘一处"可以变成一条红用例：草稿漏掉一个开关，用户就看不见它存在；草稿写出
/// 样例里没有的键，样例就不再是那份长篇解释该去的地方。
#[test]
fn the_draft_and_the_example_config_agree_on_the_general_switches() {
    let example_path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.example.toml");
    let example = std::fs::read_to_string(example_path).unwrap();

    let draft = config::draft(&[mouse_dongle()]);

    // 先证明抽取本身有效 —— 两边都抽出空列表的话，下面那条断言什么都没守。
    let from_draft = general_settings(&draft);
    assert!(
        from_draft.contains(&"primary = \"lowest\"".to_string()),
        "抽取失灵了，这条用例会变成一句空话：{from_draft:?}"
    );
    assert_eq!(from_draft, general_settings(&example));
}

/// `[general]` 那一节里的 `键 = 值`，去掉行尾注释和多余空白。
fn general_settings(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| line.trim() != "[general]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .filter_map(|line| {
            let stripped = line.split('#').next().unwrap_or("").trim();
            if stripped.is_empty() {
                None
            } else {
                Some(stripped.split_whitespace().collect::<Vec<_>>().join(" "))
            }
        })
        .collect()
}

/// ADR-0003 的另外半句："扫描结果与用户所写不一致时只提醒"（托盘把这一种记进日志，parking lot Q273）。
///
/// 不一致长这个样子：用户把 dongle 的 `0x1A05` 写进了 `[device.wired]`，而本机此刻在场的
/// 有线本体是 `0x1005`——两个 pid 只差一个字符，正是最容易写混的那一处。程序提醒，
/// 但一个字都不改。提醒是"不一致"那一种，不与"没补上"的几句混在一起。
#[test]
fn config_refresh_only_warns_when_the_scan_disagrees_with_what_the_user_wrote() {
    let before = "\
[[device]]
id = \"dragonfly3\"
name = \"Dragonfly 3 Master+\"
driver = \"vgn_mouse\"

  [device.wired]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8
";
    let refreshed = config::refresh(before, &[mouse_dongle(), mouse_wired()]).unwrap();

    assert_eq!(refreshed.text, before, "不一致只提醒，不动文件");
    assert!(refreshed.filled.is_empty(), "两块都写过了，没有空缺可补");
    assert!(
        refreshed.notes.iter().any(|note| matches!(
            note,
            config::Note::Disagrees(text) if text.contains("1A05") && text.contains("1005")
        )),
        "提醒里要把两条身份都摆出来，否则看不出说的是哪一条：{:?}",
        refreshed.notes
    );
}

/// 配置里写了、而此刻不在场，是完全正常的状态（鼠标没插线），**不该**每次都印一句提醒。
/// 噪音会把真正该看的那一句一起淹掉。
#[test]
fn config_refresh_says_nothing_about_a_configured_endpoint_that_is_merely_unplugged() {
    // ONE_DEVICE 只配了 Dongle24G，而本机此刻什么都没插。
    let refreshed = config::refresh(ONE_DEVICE, &[]).unwrap();

    assert_eq!(refreshed.text, ONE_DEVICE);
    assert!(refreshed.filled.is_empty());
    assert!(
        refreshed
            .notes
            .iter()
            .any(|note| matches!(note, config::Note::NotFilled(text) if text.contains("Wired"))),
        "Wired 空着又不在场，这一句要有 —— 用户正等着它被补上"
    );
    assert!(
        !refreshed
            .notes
            .iter()
            .any(|note| note.to_string().contains("Dongle24G")),
        "写过的块只是没插着，没什么可说的：{:?}",
        refreshed.notes
    );
}

/// 补上一条**没实测过**的身份时要说出来。
///
/// `is_present` 核对的只有 VID/PID/usage 四项，`report_id` 是一次枚举问不出来的——键盘
/// Wired 那一项是照它的 Dongle24G 抄的。而报文编号错了的帧会被设备静默丢弃，看起来和
/// "设备没反应"一模一样（docs/protocol.md 的教训），所以这件事不能不说。
#[test]
fn config_refresh_says_when_the_identity_it_filled_was_never_measured() {
    let before = "\
[[device]]
id = \"neon75\"
name = \"VGN Neon75\"
driver = \"vgn_keyboard\"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0
";
    let refreshed = config::refresh(before, &[keyboard_dongle(), keyboard_wired()]).unwrap();

    assert!(
        refreshed
            .filled
            .iter()
            .map(ToString::to_string)
            .any(|line| line.contains("report_id") && line.contains("没有实测")),
        "补上没实测过的身份要带上告知：{:?}",
        refreshed.filled
    );
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
/// 逐字段的缺省而不是整节的缺省：用户为了调一个 `stale_after` 写一行
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

// ---------------------------------------------------------------
// primary 回写：只改那一格
// ---------------------------------------------------------------

/// 一份**用户手写过的**配置：文件头一句注释、`[general]` 里 `primary` 上方一句注释、
/// 一个行尾注释、一个被他按自己的坑改过的 `report_id`，两台 Device。
///
/// 回写要动的只有 `primary` 那一格，别的每一个字节都得原样活下来。
const HANDWRITTEN_CONFIG: &str = r#"# 这份配置是我自己写的，一个字都不许动
[general]
# 托盘画哪个 Device："lowest" 是当前电量最低的那个
primary = "lowest"
poll_interval_wired = 45   # 线上不耗电，我调快了

[[device]]
id = "dragonfly3"
name = "我给它起的名字"
driver = "vgn_mouse"

  # 这一块我按自己的坑改过 report_id
  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 9

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0
"#;

/// 钉死在这个 id 上：菜单里点了某一台时交给回写的那条规则。
fn pin_to(id: &str) -> PrimaryRule {
    PrimaryRule::Pinned(id.to_string())
}

/// 钉死之后在菜单里切回"自动（电量最低）"：`primary` 那一格写回 `"lowest"`，别的一行都不动——连那一行
/// 行尾的注释也在。逐行比一遍，理由同 `primary_writeback_touches_nothing_else_the_user_wrote`。
#[test]
fn primary_writeback_switches_back_to_lowest_touching_only_that_line() {
    let before = HANDWRITTEN_CONFIG.replace(
        "primary = \"lowest\"",
        "primary = \"neon75\"   # 先钉着键盘",
    );

    let written = config::pin_primary(&before, &PrimaryRule::Lowest).unwrap();

    assert!(written.changed, "从钉死换回自动是改动");
    assert_eq!(
        Config::parse(&written.text).unwrap().general.primary,
        PrimaryRule::Lowest,
        "重启后生效的就是自动"
    );
    let before: Vec<&str> = before.lines().collect();
    let after: Vec<&str> = written.text.lines().collect();
    assert_eq!(before.len(), after.len(), "行数都不该变");
    let changed: Vec<(&str, &str)> = before
        .iter()
        .zip(&after)
        .filter(|(before, after)| before != after)
        .map(|(before, after)| (*before, *after))
        .collect();
    assert_eq!(
        changed,
        [(
            "primary = \"neon75\"   # 先钉着键盘",
            "primary = \"lowest\"   # 先钉着键盘"
        )],
        "只该动 primary 那一行，行尾注释原样留着"
    );
}

/// 本来就是自动时再点一次"自动（电量最低）"：一个字节都不写。`[general]` 整节没写时 `primary` 缺省就是自动，
/// 同样不写——不然会凭空多出一节来。
#[test]
fn primary_writeback_writes_nothing_when_it_is_already_lowest() {
    let written = config::pin_primary(HANDWRITTEN_CONFIG, &PrimaryRule::Lowest).unwrap();
    assert!(!written.changed);
    assert_eq!(written.text, HANDWRITTEN_CONFIG);

    let unwritten = config::pin_primary(ONE_DEVICE, &PrimaryRule::Lowest).unwrap();
    assert!(!unwritten.changed);
    assert_eq!(unwritten.text, ONE_DEVICE);
}

/// 用户在菜单里选了另一台，那个选择要落到 `primary` 这一格里——而**下一次启动读的就是它**，
/// 所以这里读回来的方式与真正启动时同一条（`Config::parse`），不是去文本里找字符串。
#[test]
fn primary_writeback_pins_the_device_the_user_chose() {
    let pinned = config::pin_primary(HANDWRITTEN_CONFIG, &pin_to("neon75")).unwrap();

    assert!(pinned.changed, "从 lowest 换成一台具体设备是改动");
    let config = Config::parse(&pinned.text).expect("回写之后必须还解析得动");
    assert_eq!(
        config.general.primary,
        PrimaryRule::Pinned("neon75".to_string()),
        "重启后生效的就是这一格"
    );
}

/// 已经钉在这一台上时**一个字节都不写**。原地编辑本身是保格式的，但"没改动却重写一遍文件"
/// 会白白改掉文件的修改时间，也让人以为程序动过它——自动补空块守的是同一条。
#[test]
fn primary_writeback_writes_nothing_when_it_is_already_that_device() {
    let once = config::pin_primary(HANDWRITTEN_CONFIG, &pin_to("neon75")).unwrap();
    let twice = config::pin_primary(&once.text, &pin_to("neon75")).unwrap();

    assert!(!twice.changed, "同一个 id 钉第二遍不是改动");
    assert_eq!(twice.text, once.text, "什么都没改的时候不该把文件重写一遍");
}

/// `primary` **那一行行尾**的注释也是用户写的。它住在那个**值**的 decor 里，不在键的
/// decor 里——照"新造一个值塞进这个键"的写法写，这一句会跟着旧值一起消失，而键上方那句
/// 却安然无事。两处注释因此各要一条断言。
#[test]
fn primary_writeback_keeps_the_comment_the_user_left_on_that_very_line() {
    let before = r#"[general]
# 上面这句在键的 decor 里
primary = "lowest"  # 而这句在值的 decor 里，先用自动的，等我想清楚再钉

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"
"#;

    let pinned = config::pin_primary(before, &pin_to("neon75")).unwrap();

    for kept in [
        "# 上面这句在键的 decor 里",
        "# 而这句在值的 decor 里，先用自动的，等我想清楚再钉",
    ] {
        assert!(
            pinned.text.contains(kept),
            "注释被洗掉了：{kept}\n回写之后是：\n{}",
            pinned.text
        );
    }
}

/// 用户的 `[general]` 在，但里面压根没写 `primary`——它有缺省值（`"lowest"`），不写也跑得动。
/// 回写要把这一项**补进那张表**，而不是无声地什么都没干。
#[test]
fn primary_writeback_adds_the_setting_when_the_general_section_has_none() {
    let before = r#"[general]
low_battery = 15   # 我这只鼠标撑不到 20 就该充了

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"
"#;

    let pinned = config::pin_primary(before, &pin_to("neon75")).unwrap();

    assert!(pinned.changed);
    assert_eq!(
        Config::parse(&pinned.text).unwrap().general.primary,
        PrimaryRule::Pinned("neon75".to_string())
    );
    assert!(
        pinned
            .text
            .contains("low_battery = 15   # 我这只鼠标撑不到 20 就该充了"),
        "同一张表里别的项一个字都不许动：\n{}",
        pinned.text
    );
}

/// 一份压根没有 `[general]` 的配置也要回写得进去——那一节整节缺席时每一项都取缺省值
/// （`falls_back_to_the_documented_intervals_when_general_is_absent`），所以只写了设备的
/// 配置是跑得动的。
///
/// 两条断言各守一件事：**建出来的得是一张真表**（不是行内表），以及**它得排在所有
/// `[[device]]` 前面**。两件事各自为什么要紧，写在 `config::pin_primary` 里建表那一段
/// 注释上，不在这里抄第二遍。
#[test]
fn primary_writeback_creates_the_general_section_when_the_config_has_none() {
    let two = format!(
        "{ONE_DEVICE}\n[[device]]\nid = \"neon75\"\nname = \"VGN Neon75\"\ndriver = \"vgn_keyboard\"\n"
    );

    let pinned = config::pin_primary(&two, &pin_to("neon75")).unwrap();

    let config = Config::parse(&pinned.text).expect("新建 [general] 之后必须还解析得动");
    assert_eq!(
        config.general.primary,
        PrimaryRule::Pinned("neon75".to_string())
    );
    assert_eq!(config.devices.len(), 2, "一台设备都不许丢");
    assert_eq!(config.devices[0].dongle_24g.as_ref().unwrap().report_id, 8);

    assert!(
        !pinned.text.contains("general = {"),
        "建出来的该是一张真表，不是行内表：\n{}",
        pinned.text
    );
    let general_at = pinned.text.find("[general]").expect("那一节要真的写出来");
    let first_device_at = pinned.text.find("[[device]]").unwrap();
    assert!(
        general_at < first_device_at,
        "新建的 [general] 要排在所有 [[device]] 前面：\n{}",
        pinned.text
    );
}

/// 不在册的 id **一个字节都不写**。写下去的话，下一次启动 `primary::select` 交回的是
/// `PinnedNotFound`：托盘上一台都不画，悬停提示说"配置里 primary 钉的 id 不在登记的 Device
/// 里"。让程序自己写出那种配置，等于替用户造一个他没犯的错——而菜单只可能把在册的设备列出来，
/// 所以这里收到一个不在册的 id 是**调用方的 bug**，该当场说出来。
#[test]
fn primary_writeback_refuses_an_id_that_is_not_a_registered_device() {
    let error = config::pin_primary(HANDWRITTEN_CONFIG, &pin_to("dragonfly4"))
        .expect_err("不在册的 id 不该写进去")
        .to_string();

    assert!(
        error.contains("dragonfly4"),
        "得说清是哪个 id 不在册：{error}"
    );
}

/// 一台 id 恰好叫 `lowest` 的 Device **钉不住**：那个词先被当成"电量最低的那个"这条规则
/// （`config.example.toml` 原话——"id 恰好叫 lowest 的 Device 钉不住，换个 id"）。
///
/// 照样写下去是最难发现的那一种错：用户在菜单里点了这一台，配置里出现的却是一条规则，
/// 而规则多数时候恰好也选中它。所以宁可当场拒绝。
#[test]
fn primary_writeback_refuses_a_device_whose_id_is_the_rule_word() {
    let before = r#"[[device]]
id = "lowest"
name = "起名起得不巧"
driver = "vgn_mouse"
"#;

    let error = config::pin_primary(before, &pin_to("lowest"))
        .expect_err("钉不住的就别写进去")
        .to_string();

    assert!(error.contains("lowest"), "得说清是哪个 id：{error}");
}

/// 票面最后一条：一份**带注释、含用户手改值**的配置，回写之后两者都原样保留。
/// ADR-0003 划死的边界就落在这一条上——程序自己管的字段只有 `primary` 那一格。
///
/// 除了列举几处该活下来的东西，还**逐行比一遍**：只有 `primary` 那一行允许不同，行数也不许变。
/// 逐行这一半才是真正守边界的那一半——列举只能证明我想到的那几处没丢。
#[test]
fn primary_writeback_touches_nothing_else_the_user_wrote() {
    let pinned = config::pin_primary(HANDWRITTEN_CONFIG, &pin_to("neon75")).unwrap();

    for kept in [
        "# 这份配置是我自己写的，一个字都不许动",
        "# 托盘画哪个 Device：\"lowest\" 是当前电量最低的那个",
        "poll_interval_wired = 45   # 线上不耗电，我调快了",
        "name = \"我给它起的名字\"",
        "  # 这一块我按自己的坑改过 report_id",
        "report_id = 9",
    ] {
        assert!(pinned.text.contains(kept), "不该动的东西被动了：{kept}");
    }

    let before: Vec<&str> = HANDWRITTEN_CONFIG.lines().collect();
    let after: Vec<&str> = pinned.text.lines().collect();
    assert_eq!(before.len(), after.len(), "行数都不该变");
    let mut changed_lines = 0;
    for (before, after) in before.iter().zip(&after) {
        if before == after {
            continue;
        }
        changed_lines += 1;
        assert!(
            before.starts_with("primary = "),
            "只该动 primary 那一行，却动了：{before} → {after}"
        );
    }
    assert_eq!(changed_lines, 1, "而 primary 那一行确实该变");

    // 用户手改过的那个值，读回来还是他写的那个（程序知道的是 8）。
    let config = Config::parse(&pinned.text).unwrap();
    assert_eq!(config.devices[0].dongle_24g.as_ref().unwrap().report_id, 9);
}

/// 用户把 `[general]` 写成了**行内表**（`general = { … }`）。那是合法 TOML，而回写这条路上
/// `doc["general"]["primary"]` 走的是 `toml_edit` 的 `IndexMut`——它对认不出的形状是
/// `.expect("index not found")`，也就是 panic。一个往用户配置里写字的程序 panic 是最糟的
/// 结局，所以这条形状要有人钉着。
#[test]
fn primary_writeback_survives_a_general_section_written_as_an_inline_table() {
    let before = r#"general = { primary = "lowest", low_battery = 15 }

[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"
"#;

    let pinned = config::pin_primary(before, &pin_to("neon75")).unwrap();

    assert_eq!(
        Config::parse(&pinned.text).unwrap().general.primary,
        PrimaryRule::Pinned("neon75".to_string())
    );
    assert!(
        pinned.text.contains("low_battery = 15"),
        "同一张行内表里别的项一样不许动：\n{}",
        pinned.text
    );
}

/// 厂商上位机那两项读得出来。
///
/// 它们以前是被静默忽略的键（草稿和样例里都写着，代码这边没人读），现在暂停那一维
/// 要用它们了。
#[test]
fn reads_the_vendor_hub_switch_and_the_process_names() {
    let config = Config::parse(
        r#"
        [general]
        pause_when_vendor_hub_running = false
        vendor_hub_processes = ["VGN VHUB.exe", "OtherHub.exe"]
        "#,
    )
    .unwrap();

    let general = &config.general;
    assert!(!general.pause_when_vendor_hub_running);
    assert_eq!(
        general.vendor_hub_processes,
        ["VGN VHUB.exe", "OtherHub.exe"]
    );
}

/// 缺省是**开着**的，进程名单缺省就是实测过的那一个。
///
/// 缺省不能靠 `#[derive(Default)]`：那个派生给的是 `false` 加一份空名单，而两者各自都
/// 让暂停**永远不触发**——而"没暂停"和"HUB 没在跑"长得一模一样，没有任何东西会指向那个
/// 缺省。缺省的那两个值与 `config.example.toml` 是同一份。
#[test]
fn pauses_for_the_vgn_hub_unless_the_config_says_otherwise() {
    let general = Config::parse(ONE_DEVICE).unwrap().general;

    assert!(general.pause_when_vendor_hub_running);
    assert_eq!(general.vendor_hub_processes, ["VGN VHUB.exe"]);
}

/// 低电阈值读得出来：`[general]` 里的 `low_battery`，单台 `[[device]]` 可以覆盖，没写的那台
/// 用 `[general]` 的。
///
/// 它以前是被静默忽略的键：样例和草稿里都写着，注释说"可被单个 `[[device]]` 覆盖"，代码这边
/// 没人读——图标状态里的低电就靠它，所以它从这里开始生效。覆盖按台，因为键鼠的电池容量和耗电
/// 差很多，同一个 20% 对两者的紧急程度并不相等。
#[test]
fn reads_the_low_battery_threshold_and_a_per_device_override() {
    let config = Config::parse(
        r#"
        [general]
        low_battery = 25

        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"
        low_battery = 15

        [[device]]
        id = "neon75"
        name = "VGN Neon75"
        driver = "vgn_keyboard"
        "#,
    )
    .unwrap();

    let general = &config.general;
    assert_eq!(general.low_battery, 25);
    assert_eq!(
        general.low_battery_for(&config.devices[0]),
        15,
        "覆盖了的那台用它自己的阈值"
    );
    assert_eq!(
        general.low_battery_for(&config.devices[1]),
        25,
        "没写的那台用 [general] 的"
    );
}

/// 缺省 20，与 `config.example.toml` 同一个数——`[general]` 整节缺席、或者写了别的项而没写它，
/// 都是 20。
///
/// 两条路各走一次，因为缺省值有两处出处（逐字段的 `#[serde(default = …)]` 与整节缺席时的
/// `General::default`）；这个结构最容易出的错就是两处各写一份、写得不一样。
#[test]
fn the_low_battery_threshold_defaults_to_twenty() {
    let absent = Config::parse(ONE_DEVICE).unwrap();
    assert_eq!(absent.general.low_battery, 20, "[general] 整节缺席");
    assert_eq!(absent.general.low_battery_for(&absent.devices[0]), 20);

    let partial = Config::parse("[general]\nstale_after = 60\n").unwrap();
    assert_eq!(
        partial.general.low_battery, 20,
        "[general] 在，只是没写这一项"
    );
}

// ---------------------------------------------------------------
// [tray]：ADR-0005 的八项设置
// ---------------------------------------------------------------

/// `[tray]` 整节缺席：八项都是 ADR-0005 写着的缺省值，一句"认不出"都没有。
#[test]
fn an_absent_tray_section_reads_as_the_adr_0005_defaults() {
    let config = Config::parse(ONE_DEVICE).unwrap();

    assert_eq!(
        config.tray.icon,
        IconSettings {
            style: Style::Bar,
            glyph: Glyph::Block,
            full: Full::Block,
            gray: Gray::Split,
            no_last_known: NoLastKnown::Dash,
            charging: Charging::Color,
        }
    );
    assert!(config.tray.menu_source);
    assert_eq!(config.tray.primary_mark, PrimaryMark::Both);
    assert!(config.tray.unrecognised.is_empty());
}

/// ADR-0005 的对外契约：每个键的每个取值都读得出来，读成它说的那一项。键与取值照 ADR 抄成字面量，不照实现的表反推。
#[test]
fn every_value_of_every_tray_key_reads_as_what_it_says() {
    let read = |line: &str| {
        let config = Config::parse(&format!("[tray]\n{line}\n")).unwrap();
        assert!(
            config.tray.unrecognised.is_empty(),
            "{line} 应当认得：{:?}",
            config.tray.unrecognised
        );
        config.tray
    };

    let styles = [
        ("number", Style::Number),
        ("battery", Style::Battery),
        ("ring", Style::Ring),
        ("bar", Style::Bar),
    ];
    for (value, style) in styles {
        assert_eq!(read(&format!("style = \"{value}\"")).icon.style, style);
    }
    let glyphs = [
        ("block", Glyph::Block),
        ("fine", Glyph::Fine),
        ("system", Glyph::System),
    ];
    for (value, glyph) in glyphs {
        assert_eq!(read(&format!("glyph = \"{value}\"")).icon.glyph, glyph);
    }
    let fulls = [
        ("digits", Full::Digits),
        ("cap_99", Full::Cap99),
        ("block", Full::Block),
    ];
    for (value, full) in fulls {
        assert_eq!(read(&format!("full = \"{value}\"")).icon.full, full);
    }
    let grays = [
        ("one", Gray::One),
        ("split", Gray::Split),
        ("split_pause", Gray::SplitPause),
    ];
    for (value, gray) in grays {
        assert_eq!(read(&format!("gray = \"{value}\"")).icon.gray, gray);
    }
    let no_last_knowns = [
        ("dash", NoLastKnown::Dash),
        ("question", NoLastKnown::Question),
        ("outline", NoLastKnown::Outline),
        ("logo", NoLastKnown::Logo),
    ];
    for (value, no_last_known) in no_last_knowns {
        assert_eq!(
            read(&format!("no_last_known = \"{value}\""))
                .icon
                .no_last_known,
            no_last_known
        );
    }
    let chargings = [
        ("color", Charging::Color),
        ("bolt_large", Charging::BoltLarge),
        ("bolt", Charging::Bolt),
    ];
    for (value, charging) in chargings {
        assert_eq!(
            read(&format!("charging = \"{value}\"")).icon.charging,
            charging
        );
    }
    assert!(read("menu_source = true").menu_source);
    assert!(!read("menu_source = false").menu_source);
    assert_eq!(
        read("primary_mark = \"both\"").primary_mark,
        PrimaryMark::Both
    );
    assert_eq!(
        read("primary_mark = \"radio\"").primary_mark,
        PrimaryMark::Radio
    );
}

/// 认不出的取值（笔误、写错了类型）按缺省值处理，**不让整份配置读不动**——与 `primary` 写错时一样；每一处认不出记一句，
/// 说清是哪个键、写的是什么、按什么处理（托盘把它写进日志）。同一张表里写对了的照常生效，设备照常读出来。
#[test]
fn an_unrecognised_tray_value_falls_back_to_its_default_and_says_so() {
    let text = format!(
        "[tray]\nstyle = \"squre\"\nglyph = \"fine\"\nmenu_source = \"yes\"\nprimary_mark = 2\n{ONE_DEVICE}"
    );

    let config = Config::parse(&text).expect("认不出的 [tray] 取值不该让整份配置读不动");

    assert_eq!(config.tray.icon.style, Style::Bar);
    assert_eq!(config.tray.icon.glyph, Glyph::Fine, "写对了的照常生效");
    assert!(config.tray.menu_source);
    assert_eq!(config.tray.primary_mark, PrimaryMark::Both);
    assert_eq!(config.devices.len(), 1);
    let said: Vec<String> = config
        .tray
        .unrecognised
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        said,
        [
            "[tray] 里的 style = \"squre\" 认不出，按缺省值 \"bar\" 处理",
            "[tray] 里的 menu_source = \"yes\" 认不出，按缺省值 true 处理",
            "[tray] 里的 primary_mark = 2 认不出，按缺省值 \"both\" 处理",
        ]
    );
}

/// `tray` 写成了别的东西（不是一张表）：八项都按缺省值处理，说一句，配置照样读得动。
#[test]
fn a_tray_key_that_is_not_a_table_falls_back_to_every_default() {
    let text = format!("tray = \"number\"\n{ONE_DEVICE}");

    let config = Config::parse(&text).expect("写错的 tray 不该让整份配置读不动");

    assert_eq!(config.tray.icon, IconSettings::default());
    let said: Vec<String> = config
        .tray
        .unrecognised
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        said,
        ["[tray] 不是一张表（写成了 \"number\"），八项都按缺省值处理"]
    );
}

/// 一份**用户手写过的** `[tray]`：表上方、键上方各一句注释，一个行尾注释，写了两项、其余没写。
const HANDWRITTEN_TRAY: &str = r#"# 我自己的配置
[general]
primary = "lowest"

# 托盘我自己调过
[tray]
# 数字大一点
style = "number"   # A 纯数字
charging = "bolt"

[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"
"#;

/// 回写之后逐行对一遍：`before` 与 `after` 行数一样时，交出不一样的那几对行。
fn changed_lines<'a>(before: &'a str, after: &'a str) -> Vec<(&'a str, &'a str)> {
    let (before, after): (Vec<&str>, Vec<&str>) =
        (before.lines().collect(), after.lines().collect());
    assert_eq!(
        before.len(),
        after.len(),
        "行数都不该变：\n{}",
        after.join("\n")
    );
    before
        .into_iter()
        .zip(after)
        .filter(|(before, after)| before != after)
        .collect()
}

/// 菜单里改了一项：只动 `[tray]` 里那一个键的那一行，行尾注释原样留着；读回来就是新的取值。
#[test]
fn tray_writeback_changes_only_that_key_on_its_own_line() {
    let written = config::write_tray(HANDWRITTEN_TRAY, &[TraySetting::Style(Style::Ring)]).unwrap();

    assert!(written.changed);
    assert_eq!(
        changed_lines(HANDWRITTEN_TRAY, &written.text),
        [(
            "style = \"number\"   # A 纯数字",
            "style = \"ring\"   # A 纯数字"
        )]
    );
    assert_eq!(
        Config::parse(&written.text).unwrap().tray.icon.style,
        Style::Ring
    );
}

/// `[tray]` 在、那个键没写：在这张表最后一个键后面补一行，别的一行都不动。
#[test]
fn tray_writeback_adds_the_key_at_the_end_of_the_tray_table_when_it_is_not_written() {
    let written = config::write_tray(HANDWRITTEN_TRAY, &[TraySetting::Glyph(Glyph::Fine)]).unwrap();

    assert!(written.changed);
    let mut expected: Vec<&str> = HANDWRITTEN_TRAY.lines().collect();
    let after_charging = expected
        .iter()
        .position(|line| *line == "charging = \"bolt\"")
        .unwrap()
        + 1;
    expected.insert(after_charging, "glyph = \"fine\"");
    assert_eq!(written.text.lines().collect::<Vec<_>>(), expected);
}

/// 配置里压根没有 `[tray]`：建一张真表（不是行内表），排在 `[general]` 后面、所有 `[[device]]` 前面，前面空一行，
/// 里面只有这一个键；别的一行都不动。
#[test]
fn tray_writeback_creates_the_tray_table_after_general_when_there_is_none() {
    let written =
        config::write_tray(HANDWRITTEN_CONFIG, &[TraySetting::MenuSource(false)]).unwrap();

    assert!(written.changed);
    let mut expected: Vec<&str> = HANDWRITTEN_CONFIG.lines().collect();
    let end_of_general = expected
        .iter()
        .position(|line| line.starts_with("poll_interval_wired"))
        .unwrap()
        + 1;
    for (offset, line) in ["", "[tray]", "menu_source = false"]
        .into_iter()
        .enumerate()
    {
        expected.insert(end_of_general + offset, line);
    }
    assert_eq!(written.text.lines().collect::<Vec<_>>(), expected);
    assert!(!Config::parse(&written.text).unwrap().tray.menu_source);
}

/// 配置里连 `[general]` 都没有：新建的 `[tray]` 排在所有 `[[device]]` 前面（写在后面，TOML 会把它读成那台 Device 的一张子表）。
#[test]
fn tray_writeback_puts_a_new_tray_table_before_every_device_when_there_is_no_general() {
    let written =
        config::write_tray(ONE_DEVICE, &[TraySetting::PrimaryMark(PrimaryMark::Radio)]).unwrap();

    let config = Config::parse(&written.text).unwrap();
    assert_eq!(config.tray.primary_mark, PrimaryMark::Radio);
    assert_eq!(config.devices.len(), 1);
    assert!(
        written.text.find("[tray]").unwrap() < written.text.find("[[device]]").unwrap(),
        "[tray] 要排在 [[device]] 前面：\n{}",
        written.text
    );
}

/// 本来就写着这个取值：一个字节都不写。没写、要的正是缺省值：同样不写——不然会凭空多出一张表、一行字来。
#[test]
fn tray_writeback_writes_nothing_when_the_value_is_already_in_effect_as_written() {
    let same = config::write_tray(HANDWRITTEN_TRAY, &[TraySetting::Style(Style::Number)]).unwrap();
    assert!(!same.changed);
    assert_eq!(same.text, HANDWRITTEN_TRAY);

    let default = config::write_tray(ONE_DEVICE, &[TraySetting::Style(Style::Bar)]).unwrap();
    assert!(!default.changed);
    assert_eq!(default.text, ONE_DEVICE);
}

/// 那个键写着一个认不出的取值（今天按缺省值画）：点了缺省值那一项，照样把那一行写成它——文件里的笔误就此没了，
/// 日志里那一句也不再出现。
#[test]
fn tray_writeback_replaces_an_unrecognised_value_even_with_the_default() {
    let before = "[tray]\nstyle = \"squre\"\n";

    let written = config::write_tray(before, &[TraySetting::Style(Style::Bar)]).unwrap();

    assert!(written.changed);
    assert_eq!(
        changed_lines(before, &written.text),
        [("style = \"squre\"", "style = \"bar\"")]
    );
    assert!(
        Config::parse(&written.text)
            .unwrap()
            .tray
            .unrecognised
            .is_empty()
    );
}

/// "恢复默认"：图标样式那六项里写着别的取值的，各自那一行写回缺省值；没写的不补；"菜单显示"那两项不动。
#[test]
fn restoring_the_icon_defaults_rewrites_only_the_icon_keys_that_differ() {
    let before = HANDWRITTEN_TRAY.replace(
        "charging = \"bolt\"",
        "charging = \"bolt\"\nmenu_source = false",
    );

    let written = config::write_tray(&before, &TraySetting::icon_defaults()).unwrap();

    assert!(written.changed);
    assert_eq!(
        changed_lines(&before, &written.text),
        [
            (
                "style = \"number\"   # A 纯数字",
                "style = \"bar\"   # A 纯数字"
            ),
            ("charging = \"bolt\"", "charging = \"color\""),
        ]
    );
    let tray = Config::parse(&written.text).unwrap().tray;
    assert_eq!(tray.icon, IconSettings::default());
    assert!(!tray.menu_source, "菜单显示那两项不归恢复默认管");
}

/// 用户把 `[tray]` 写成了行内表：照样只改那一项，同一张行内表里别的项不动。
#[test]
fn tray_writeback_survives_a_tray_section_written_as_an_inline_table() {
    let before = "tray = { style = \"number\", charging = \"bolt\" }\n";

    let written = config::write_tray(before, &[TraySetting::Style(Style::Ring)]).unwrap();

    let tray = Config::parse(&written.text).unwrap().tray;
    assert_eq!(tray.icon.style, Style::Ring);
    assert_eq!(tray.icon.charging, Charging::Bolt);
}

/// `tray` 写成了一张表以外的东西：写不进去，**当场说出来、一个字节都不写**，而不是 panic 或者把它整个换掉。
#[test]
fn tray_writeback_refuses_a_tray_key_that_is_not_a_table() {
    let error = config::write_tray("tray = \"number\"\n", &[TraySetting::Style(Style::Ring)])
        .expect_err("写不进一张不是表的 [tray]")
        .to_string();

    assert!(error.contains("[tray]"), "得说清是哪一处：{error}");
}

// ---------------------------------------------------------------
// 蓝牙登记与解除：菜单"登记设备"里点的那一下，只动那一块
// ---------------------------------------------------------------

/// 把本机扫到的一个蓝牙地址登记到 dragonfly3（它还没有蓝牙地址）：文件里只多出块名、地址与一行空行，落在
/// dragonfly3 已有的那一块后面、下一个 `[[device]]` 前面；原有的每一行原样、按原次序都在。
///
/// 登记到**第一台**而不是最后一台，守的是 `config_refresh_puts_the_block_under_the_right_device` 同一个坑：块排错了
/// 位置，TOML 就把它算到下一台头上。
#[test]
fn ble_registration_adds_the_bluetooth_block_under_that_device_and_nothing_else() {
    let written = config::register_ble(HANDWRITTEN_CONFIG, "dragonfly3", "f4ee2553b27e")
        .unwrap()
        .expect("dragonfly3 还没有蓝牙地址，该写");

    let (at, added) = inserted_lines(HANDWRITTEN_CONFIG, &written);
    assert_eq!(
        added,
        [
            "  [device.bluetooth]\n",
            "  address = \"f4ee2553b27e\"\n",
            "\n"
        ],
        "只多出这一块：\n{written}"
    );
    assert_eq!(
        HANDWRITTEN_CONFIG.split_inclusive('\n').nth(at - 1),
        Some("\n"),
        "块前面空一行，接在 dragonfly3 那一块后面：\n{written}"
    );
    let config = Config::parse(&written).unwrap();
    assert!(
        config.devices[0]
            .bluetooth
            .as_ref()
            .is_some_and(|bluetooth| bluetooth.matches("f4ee2553b27e")),
        "地址落在 dragonfly3 上"
    );
    assert!(config.devices[1].bluetooth.is_none(), "neon75 一个字没动");
}

/// 这台已经登记着别的地址：拒绝，一个字节都不写——换地址是先解除、再登记，一点不会悄悄覆盖旧地址。登记的正是这个
/// 地址（托盘还没重读到上一次登记时又点了一下）就什么都不必写，换个写法也是同一个地址。
#[test]
fn ble_registration_never_overwrites_an_address_the_device_already_has() {
    let registered = config::register_ble(HANDWRITTEN_CONFIG, "neon75", "f4ee2553b27e")
        .unwrap()
        .expect("neon75 还没有蓝牙地址，该写");

    let error = config::register_ble(&registered, "neon75", "38184c8f1a2b")
        .expect_err("已经有地址的不许覆盖")
        .to_string();
    assert!(
        error.contains("f4ee2553b27e") && error.contains("解除"),
        "得说清旧地址是哪个、要先解除：{error}"
    );
    assert_eq!(
        config::register_ble(&registered, "neon75", "F4:EE:25:53:B2:7E").unwrap(),
        None,
        "同一个地址，一个字节都不必写"
    );
}

/// 同一个地址已经登记在另一台上：拒绝。两台说的会是同一台设备的电量。
#[test]
fn ble_registration_refuses_an_address_another_device_already_has() {
    let registered = config::register_ble(HANDWRITTEN_CONFIG, "neon75", "f4ee2553b27e")
        .unwrap()
        .expect("neon75 还没有蓝牙地址，该写");

    let error = config::register_ble(&registered, "dragonfly3", "f4ee2553b27e")
        .expect_err("一个地址不登记给两台")
        .to_string();

    assert!(
        error.contains("neon75"),
        "得说清它已经登记在哪一台上：{error}"
    );
}

/// 不在册的 id：拒绝，得说清是哪个 id。
#[test]
fn ble_registration_refuses_an_id_that_is_not_a_registered_device() {
    let error = config::register_ble(HANDWRITTEN_CONFIG, "dragonfly4", "f4ee2553b27e")
        .expect_err("不在册的 id 不该写进去")
        .to_string();

    assert!(
        error.contains("dragonfly4"),
        "得说清是哪个 id 不在册：{error}"
    );
}

/// 解除蓝牙登记：删掉那一台的 `[device.bluetooth]` 那一块，别的一个字节都不动——登记之后再解除，文件逐字节回到登记
/// 之前（登记时补在块前面的那一行空行也跟着走）。
#[test]
fn ble_unregistration_after_registration_gives_back_the_file_byte_for_byte() {
    let registered = config::register_ble(HANDWRITTEN_CONFIG, "dragonfly3", "f4ee2553b27e")
        .unwrap()
        .expect("dragonfly3 还没有蓝牙地址，该写");

    let unregistered = config::unregister_ble(&registered, "dragonfly3")
        .unwrap()
        .expect("dragonfly3 登记着地址，该写");

    assert_eq!(unregistered, HANDWRITTEN_CONFIG);
}

/// 解除只删块名与块里那几行键：用户写在块上面的注释一句不丢（ADR-0003）。用户照草稿里 Ble 那段占位的说明，自己去掉
/// `#`、填上地址，那段说明就在块名上面，正是这个样子。
#[test]
fn ble_unregistration_keeps_the_comments_written_above_the_block() {
    let before = r#"[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"

  [device.wireless_24g]
  vid = 0x3151
  pid = 0x5038
  usage_page = 0xFFFF
  usage = 0x0002
  report_id = 0

  # 键盘的蓝牙，配对在这台电脑上
  [device.bluetooth]
  address = "f4ee2553b27e"

[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"
"#;

    let after = config::unregister_ble(before, "neon75")
        .unwrap()
        .expect("neon75 登记着地址，该写");

    let (_, removed) = inserted_lines(&after, before);
    assert_eq!(
        removed,
        ["  [device.bluetooth]\n", "  address = \"f4ee2553b27e\"\n"],
        "只删块名与地址那两行，上面那句注释留着：\n{after}"
    );
    let config = Config::parse(&after).unwrap();
    assert!(config.devices[0].bluetooth.is_none());
    assert_eq!(config.devices.len(), 2);
}

/// 这台本来就没有蓝牙地址（托盘还没重读到上一次解除时又点了一下）：一个字节都不必写，也不是错。
#[test]
fn ble_unregistration_writes_nothing_when_the_device_has_no_address() {
    assert_eq!(
        config::unregister_ble(HANDWRITTEN_CONFIG, "neon75").unwrap(),
        None
    );
}

/// 蓝牙那一块写成行内表：程序不去拆一个没想到的形状，说出来，一个字节都不删。
#[test]
fn ble_unregistration_refuses_a_block_it_does_not_recognise() {
    let before = r#"[[device]]
id = "neon75"
name = "VGN Neon75"
driver = "vgn_keyboard"
bluetooth = { address = "f4ee2553b27e" }
"#;

    let error = config::unregister_ble(before, "neon75")
        .expect_err("认不出的形状不删")
        .to_string();

    assert!(error.contains("手动"), "得说清要用户自己去删：{error}");
}

// ---------------------------------------------------------------
// 新建一台 Device：菜单"登记设备"里点的"新建一台 Device"，追加在末尾
// ---------------------------------------------------------------

/// 本机扫到、还没登记的蓝牙设备 WH-1000XM5，新建成一台 Device：追加在文件末尾，名字取蓝牙名、id 由名字派生，只写蓝牙块、
/// 不写 `driver`；原有的每一行原样、按原次序都在（ADR-0003）。
#[test]
fn a_new_ble_device_is_appended_at_the_end_with_only_its_bluetooth_block() {
    let written = add(HANDWRITTEN_CONFIG, &new_ble("WH-1000XM5", "38184c8f1a2b"));

    let (at, added) = inserted_lines(HANDWRITTEN_CONFIG, &written);
    assert_eq!(
        at,
        HANDWRITTEN_CONFIG.split_inclusive('\n').count(),
        "追加在末尾：\n{written}"
    );
    assert_eq!(
        added,
        [
            "\n",
            "[[device]]\n",
            "id = \"wh-1000xm5\"\n",
            "name = \"WH-1000XM5\"\n",
            "\n",
            "  [device.bluetooth]\n",
            "  address = \"38184c8f1a2b\"\n",
        ],
        "只多出这一台：\n{written}"
    );
    let device = Config::parse(&written)
        .unwrap()
        .devices
        .pop()
        .expect("多了一台");
    assert_eq!(device.driver, None, "只走蓝牙的不写驱动");
}

/// 身份表认得、本机插着接收器、配置里却没有的 VGN Neon75，新建成一台 Device：追加在末尾，名字、id、`driver` 照身份表写，
/// Endpoint 块只写此刻在场的 Dongle24G——没插的 Wired 不写（插上之后自动补空块照样补得上它）。
#[test]
fn a_new_hid_device_is_appended_with_its_driver_and_the_endpoints_present() {
    let written = add(ONE_DEVICE, &new_hid("neon75", &[EndpointKind::Dongle24G]));

    let (at, added) = inserted_lines(ONE_DEVICE, &written);
    assert_eq!(
        at,
        ONE_DEVICE.split_inclusive('\n').count(),
        "追加在末尾：\n{written}"
    );
    assert_eq!(
        added,
        [
            "\n",
            "[[device]]\n",
            "id = \"neon75\"\n",
            "name = \"VGN Neon75\"\n",
            "driver = \"vgn_keyboard\"\n",
            "\n",
            "  [device.wireless_24g]\n",
            "  vid = 0x3151\n",
            "  pid = 0x5038\n",
            "  usage_page = 0xFFFF\n",
            "  usage = 0x0002\n",
            "  report_id = 0\n",
        ],
        "只多出这一台：\n{written}"
    );
    let config = Config::parse(&written).unwrap();
    assert!(config.devices[1].wired.is_none(), "没插的 Wired 不写");
}

/// 新建的蓝牙 Device 的 id 由名字派生：小写，字母数字以外的连成一个 `-`。撞了已有的 id 就加后缀 `-2`、`-3`……；`lowest`
/// 也算撞——那是 `primary` 里规则的写法，id 叫它的 Device 钉不住。
#[test]
fn a_new_device_id_is_derived_from_its_name_and_suffixed_when_taken() {
    let first = add(
        HANDWRITTEN_CONFIG,
        &new_ble("Galaxy Buds2 Pro", "a1b2c3d4e5f6"),
    );
    let second = add(&first, &new_ble("Galaxy Buds2 Pro", "a1b2c3d4e5f7"));
    let third = add(&second, &new_ble("galaxy  buds2_pro", "a1b2c3d4e5f8"));
    let last = add(&third, &new_ble("Lowest", "a1b2c3d4e5f9"));

    let ids: Vec<String> = Config::parse(&last)
        .unwrap()
        .devices
        .into_iter()
        .map(|device| device.id)
        .collect();
    assert_eq!(
        ids,
        [
            "dragonfly3",
            "neon75",
            "galaxy-buds2-pro",
            "galaxy-buds2-pro-2",
            "galaxy-buds2-pro-3",
            "lowest-2",
        ]
    );
}

/// 新建的 HID Device 的 id 取身份表里的那一个（与草稿写的一样，删掉一台再从菜单加回来，还是原来那个 id），撞了照样加后缀：
/// 这里 id 叫 neon75 的那台配的是鼠标的身份，它不是 VGN Neon75。
#[test]
fn a_new_hid_device_takes_its_id_from_the_identity_table_and_is_suffixed_when_taken() {
    let taken = r#"[[device]]
id = "neon75"
name = "其实是一只鼠标"
driver = "vgn_mouse"

  [device.wireless_24g]
  vid = 0x391D
  pid = 0x1A05
  usage_page = 0xFF02
  usage = 0x0002
  report_id = 8
"#;

    let written = add(taken, &new_hid("neon75", &[EndpointKind::Dongle24G]));

    let config = Config::parse(&written).unwrap();
    assert_eq!(config.devices[1].id, "neon75-2");
    assert_eq!(config.devices[1].name, "VGN Neon75");
}

/// 要新建的那一台已经在配置里了（托盘还没重读到上一次新建时又点了一下）：一个字节都不必写，也不是错。蓝牙设备看地址，换个
/// 写法也算；HID 设备看配置里有没有哪一台认得出是它。
#[test]
fn a_new_device_writes_nothing_when_it_is_already_in_the_config() {
    let added = add(HANDWRITTEN_CONFIG, &new_ble("WH-1000XM5", "38184c8f1a2b"));

    assert_eq!(
        config::add_device(&added, &new_ble("WH-1000XM5", "38:18:4C:8F:1A:2B")).unwrap(),
        None,
        "这个地址已经登记着"
    );
    assert_eq!(
        config::add_device(
            HANDWRITTEN_CONFIG,
            &new_hid("neon75", &[EndpointKind::Dongle24G])
        )
        .unwrap(),
        None,
        "配置里已经有 neon75"
    );
}

/// 死角收掉了（`docs/gaps.md`）：首次运行时一台设备都没插，草稿里一台 Device 都没有；插上接收器之后在菜单里新建一台，文件读得
/// 动、有了这一台；之后插上它的线，自动补空块照样补得上它的 Wired。
#[test]
fn the_empty_draft_gets_its_first_device_from_the_menu_and_refresh_fills_the_rest() {
    let empty = config::draft(&[]);

    let added = add(&empty, &new_hid("dragonfly3", &[EndpointKind::Dongle24G]));
    let refreshed = config::refresh(&added, &[mouse_dongle(), mouse_wired()]).unwrap();

    let config = Config::parse(&refreshed.text).unwrap();
    let [device] = config.devices.as_slice() else {
        panic!("该有且只有新建的那一台：\n{}", refreshed.text);
    };
    assert_eq!(device.id, "dragonfly3");
    assert!(device.dongle_24g.is_some(), "新建时在场的 Dongle24G");
    assert!(device.wired.is_some(), "插上线之后补上的 Wired");
}

/// 菜单里点的"新建一台 Device"：本机扫到的一台蓝牙设备，名字 `name`、地址 `address`。
fn new_ble(name: &str, address: &str) -> config::NewDevice {
    config::NewDevice::Ble {
        name: name.to_string(),
        address: address.to_string(),
    }
}

/// 菜单里点的"新建一台 Device"：身份表里 id 是 `known_id` 的那一台，本机在场的是 `present` 那几条。
fn new_hid(known_id: &str, present: &[EndpointKind]) -> config::NewDevice {
    config::NewDevice::Hid {
        known_id: known_id.to_string(),
        present: present.to_vec(),
    }
}

/// 把 `new` 新建进 `text`，交回写好的全文；它还不在配置里，该写。
fn add(text: &str, new: &config::NewDevice) -> String {
    config::add_device(text, new)
        .unwrap()
        .expect("还不在配置里，该写")
}
