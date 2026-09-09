//! 配置文件这一头：解析、自举草稿、`config-refresh` 的补全。
//!
//! 三样东西住在同一个模块（`src/config.rs`），所以用例也在同一个文件里。

use juicebar::config::{self, Config};
use juicebar::hid::HidInfo;

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
