//! 配置文件这一头：解析、自举草稿、`config-refresh` 的补全、`primary` 的回写。
//!
//! 四样东西各住一个文件（`src/config/`），而公开面仍然是一处，所以用例也在同一个文件里。

use juicebar::config::{self, Config};
use juicebar::hid::HidInfo;
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
    assert_eq!(device.driver, "vgn_mouse");

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
    assert!(text.contains("config-refresh"), "要写明补全办法");
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
    // 草稿里写，那个 `#` 一旦掉了，用户会拿到一台地址是"把 scan 里那串…"的蓝牙设备。
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
    assert!(ble.contains("juicebar scan"), "还要说清去哪儿抄：{ble}");

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
}

// ---------------------------------------------------------------
// config-refresh：只填空缺
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
        refreshed.filled.iter().any(|line| line.contains("Wired")),
        "填了什么要说出来，不能悄悄改用户的文件"
    );
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
        refreshed
            .notes
            .iter()
            .any(|note| note.contains("Wired") && note.contains("不在场")),
        "要说明为什么没填"
    );
    assert!(
        refreshed
            .notes
            .iter()
            .any(|note| note.contains("插上 USB 线")),
        "还要说清楚怎么才能让它出现"
    );
}

/// 键盘那条最麻烦的 Endpoint 也要补得上：它的 vendor collection 实测是 `in:0 out:0 feat:65`
/// ——一条**发不出输出报文**的通路。取数那一步按"能发输出报文"筛通路，而往配置里写一条身份
/// 不该受那道筛子约束，否则最需要 `config-refresh` 的那一条永远补不上。
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
        refreshed.notes.iter().any(|note| note.contains("认不出")),
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
/// 几套身份之间没有能缝合的字段——所以它是注释掉的占位，并写明去哪儿抄地址。
#[test]
fn the_draft_leaves_room_for_the_ble_endpoint() {
    let text = config::draft(&[mouse_dongle()]);

    assert!(text.contains("# [device.bluetooth]"), "Ble 那一块要写出来");
    assert!(text.contains("juicebar scan"), "要写明去哪儿抄 MAC");
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

/// ADR-0003 的另外半句："扫描结果与用户所写不一致时只在命令行提醒"。
///
/// 不一致长这个样子：用户把 dongle 的 `0x1A05` 写进了 `[device.wired]`，而本机此刻在场的
/// 有线本体是 `0x1005`——两个 pid 只差一个字符，正是最容易写混的那一处。程序提醒，
/// 但一个字都不改。
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
        refreshed
            .notes
            .iter()
            .any(|note| note.contains("1A05") && note.contains("1005")),
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
        refreshed.notes.iter().any(|note| note.contains("Wired")),
        "Wired 空着又不在场，这一句要有 —— 用户正等着它被补上"
    );
    assert!(
        !refreshed
            .notes
            .iter()
            .any(|note| note.contains("Dongle24G")),
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

/// 用户在菜单里选了另一台，那个选择要落到 `primary` 这一格里——而**下一次启动读的就是它**，
/// 所以这里读回来的方式与真正启动时同一条（`Config::parse`），不是去文本里找字符串。
#[test]
fn primary_writeback_pins_the_device_the_user_chose() {
    let pinned = config::pin_primary(HANDWRITTEN_CONFIG, "neon75").unwrap();

    assert!(pinned.changed, "从 lowest 换成一台具体设备是改动");
    let config = Config::parse(&pinned.text).expect("回写之后必须还解析得动");
    assert_eq!(
        config.general.primary,
        PrimaryRule::Pinned("neon75".to_string()),
        "重启后生效的就是这一格"
    );
}

/// 已经钉在这一台上时**一个字节都不写**。原地编辑本身是保格式的，但"没改动却重写一遍文件"
/// 会白白改掉文件的修改时间，也让人以为程序动过它——`config-refresh` 守的是同一条。
#[test]
fn primary_writeback_writes_nothing_when_it_is_already_that_device() {
    let once = config::pin_primary(HANDWRITTEN_CONFIG, "neon75").unwrap();
    let twice = config::pin_primary(&once.text, "neon75").unwrap();

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

    let pinned = config::pin_primary(before, "neon75").unwrap();

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

    let pinned = config::pin_primary(before, "neon75").unwrap();

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

    let pinned = config::pin_primary(&two, "neon75").unwrap();

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
/// `PinnedNotFound`：一行都没标，命令行上多一句"配置里 primary 钉的 id 不在登记的 Device
/// 里"。让程序自己写出那种配置，等于替用户造一个他没犯的错——而菜单只可能把在册的设备列出来，
/// 所以这里收到一个不在册的 id 是**调用方的 bug**，该当场说出来。
#[test]
fn primary_writeback_refuses_an_id_that_is_not_a_registered_device() {
    let error = config::pin_primary(HANDWRITTEN_CONFIG, "dragonfly4")
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

    let error = config::pin_primary(before, "lowest")
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
    let pinned = config::pin_primary(HANDWRITTEN_CONFIG, "neon75").unwrap();

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

    let pinned = config::pin_primary(before, "neon75").unwrap();

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
