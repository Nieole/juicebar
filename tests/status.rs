//! `cli::status` 的外部行为，也就是 Endpoint 合成：在场的几条 Endpoint 里先试哪一条、
//! 靠前的那条不行时怎么降级、什么时候才算失联、那一行印成什么样。
//!
//! 全程经假枚举 + 假 Transport，不接任何硬件——尤其是"拔线"这件事，真机上拔一次
//! 线才复现一次。
//!
//! `src/endpoints.rs` 那一半里不需要真机的东西在 `tests/endpoints.rs`；`SystemEndpoints`
//! 要真去枚举本机的 HID collection 和 BLE 设备，那正是这条接缝存在的理由，不测。

mod common;

use common::fixtures::{
    MOUSE_BATTERY_REQUEST, MOUSE_CHARGING, MOUSE_REPORT_ID, MOUSE_RESTING_FULL,
};
use common::{
    FakeEndpoints, mouse_with_all_three_endpoints, mouse_with_both_endpoints, scanned_ble,
};
use juicebar::cli::status;
use juicebar::config::Config;
use juicebar::endpoints::EndpointKind;

/// 插着线的鼠标：两条 Endpoint 都在场时用 Wired。
///
/// 两条脚本给的是**不同**的回包，所以"用了哪一条"是从读出来的数上看得见的，
/// 不必去问实现。
#[test]
fn reads_from_the_wired_endpoint_when_it_is_present() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );

    let reading = status::read(&mouse_with_both_endpoints(), &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Wired);
    // `Some(true)` 而不是"真值"：票 02 把这两项改成了 Option，因为键盘答不上来。
    // 断言写成 Some 才分得清"确实在充电"和"这条协议说不上来"。
    assert_eq!(reading.reading.charging, Some(true));
    assert_eq!(reading.reading.voltage_mv, Some(4235));
}

/// Wired 在场时**一个字节都不往 Dongle24G 发**。
///
/// 这不是洁癖：往 2.4G 发包要走一次到设备的空中往返，耗的是设备自己的电，而此刻
/// 那条通路本来就在超时（插线时鼠标不再经 2.4G 传数据）。白发一遍既慢又费电。
#[test]
fn does_not_disturb_the_dongle_24g_endpoint_while_wired_is_present() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );

    status::read(&mouse_with_both_endpoints(), &endpoints).unwrap();

    assert_eq!(
        endpoints.transport(EndpointKind::Wired).sent(),
        vec![MOUSE_BATTERY_REQUEST.to_vec()]
    );
    assert!(
        endpoints
            .transport(EndpointKind::Dongle24G)
            .sent()
            .is_empty(),
        "Wired 在场时不该往 Dongle24G 发任何帧"
    );
}

/// 优先级不是枚举给的顺序。
///
/// 枚举只回答"谁在场"，先试谁由取数那一步定。这条用例把假枚举的顺序倒过来：
/// 若实现偷懒地信任了枚举给的顺序，它就会读到 Dongle24G 那一帧。
#[test]
fn prefers_wired_even_when_the_enumeration_lists_dongle_24g_first() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
            (EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()]),
        ],
    );

    let reading = status::read(&mouse_with_both_endpoints(), &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Wired);
}

/// 拔线：`Wired` 从枚举里消失，同一个 Device 立刻改用 `Dongle24G`，不空窗。
///
/// 这是实测确认过的行为——鼠标插线时自动切到有线传数据，拔掉就切回去。
#[test]
fn falls_back_to_dongle_24g_when_wired_disappears_from_the_enumeration() {
    let device = mouse_with_both_endpoints();

    let plugged = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );
    assert_eq!(
        status::read(&device, &plugged).unwrap().endpoint,
        EndpointKind::Wired
    );

    // 拔掉线：那条 Endpoint 不再被枚举出来。
    let unplugged = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()])],
    );

    let reading = status::read(&device, &unplugged).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Dongle24G);
    assert_eq!(reading.reading.reported_level, 100);
}

/// 靠前的 Endpoint 失败时**在同一次取数里**接着试下一条，不等下一轮。
///
/// 这里的 `Wired` 在场却读不到（回包脚本是空的，相当于超时）——枚举预知不了的失败，
/// 只有真去试一次才知道。若要等下一个轮询周期才降级，用户就会看到一次空窗。
#[test]
fn degrades_to_dongle_24g_within_the_same_cycle_when_wired_fails() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );

    let reading = status::read(&mouse_with_both_endpoints(), &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Dongle24G);
    assert_eq!(reading.reading.reported_level, 100);
    assert_eq!(
        endpoints.transport(EndpointKind::Wired).sent().len(),
        1,
        "降级之前该真的试过 Wired 一次"
    );
}

/// 在场的 Endpoint **全都**没读到，才算失联。
///
/// 失联那句话要把每一条的原因都带上：只说"读不到"，用户没法知道是两条都哑了，
/// 还是只有一条哑了而另一条压根没被试。
#[test]
fn reports_a_lost_device_only_after_every_endpoint_failed() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![]),
        ],
    );

    let error = status::read(&mouse_with_both_endpoints(), &endpoints)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("Wired"),
        "失联那句话要说清每一条的原因：{error}"
    );
    assert!(
        error.contains("Dongle24G"),
        "失联那句话要说清每一条的原因：{error}"
    );
    assert_eq!(endpoints.transport(EndpointKind::Wired).sent().len(), 1);
    assert_eq!(endpoints.transport(EndpointKind::Dongle24G).sent().len(), 1);
}

/// 两条 HID 通路都不可用时（设备关机、收进抽屉了、接收器拔了），退到蓝牙缓存兜底。
///
/// 这一级不经过协议驱动、不经过 Transport：它读的是 Windows 攒好的属性，不是一次到
/// 设备的往返。所以这里两条 HID 的脚本都是空的，而 Ble 照样交得出一个数。
#[test]
fn falls_back_to_the_ble_cache_when_both_hid_endpoints_fail() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![]),
        ],
    )
    .with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));

    let reading = status::read(&mouse_with_all_three_endpoints(), &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Ble);
    assert_eq!(reading.reading.reported_level, 62);
    assert_eq!(reading.cache_age_secs, Some(300));
}

/// 当场问得出来的时候，**一次都不去读那份缓存**。
///
/// 蓝牙缓存里那个数可能是几个月前的（`bluetooth.rs` 开头记着实测见过 2025 年 3 月的
/// 读数被当成"当前电量"）。这条用例给的缓存是一天前的 62%，而 Wired 当场读出 95%——
/// 若实现照枚举给的顺序试（假枚举**故意把 Ble 排在最前**），读到的就会是那个旧数。
#[test]
fn does_not_read_the_ble_cache_while_a_hid_endpoint_works() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()])],
    )
    .with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(86_400),
    ));

    let reading = status::read(&mouse_with_all_three_endpoints(), &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Wired);
    assert_eq!(reading.reading.reported_level, 95);
    assert_eq!(
        endpoints.ble_reads(),
        0,
        "HID 读得到时不该去碰那份缓存，哪怕枚举把 Ble 排在最前"
    );
}

/// `Dongle24G` 也排在 `Ble` 前面——优先级那三级里，中间这一级同样不该被缓存抢掉。
///
/// 单测 Wired 是不够的：`PRIORITY` 里 `Wired` 和 `Ble` 隔着一个，只断言两头，"中间那个
/// 排在哪"就没人守。这里 Wired 不在场（线拔了），当场问得出话的是 2.4G。
#[test]
fn prefers_dongle_24g_over_the_ble_cache() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()])],
    )
    .with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(86_400),
    ));

    let reading = status::read(&mouse_with_all_three_endpoints(), &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Dongle24G);
    assert_eq!(reading.reading.reported_level, 100);
    assert_eq!(
        endpoints.ble_reads(),
        0,
        "2.4G 答得出话时不该去碰那份缓存，哪怕枚举把 Ble 排在最前"
    );
}

/// **配置里的 `driver` 只作用于 HID Endpoint。**
///
/// 配了一个本次编译还没实现的驱动名时，两条 HID 一条也取不到数（没有驱动就没有帧可发），
/// 而 Ble 照样读得出来——它读的是 Windows 属性，根本不经过驱动。若哪天 Ble 那一支被顺手
/// 接到驱动分发上，这条用例会红。
#[test]
fn reads_the_ble_cache_even_when_the_driver_is_not_implemented() {
    let device = Config::parse(
        r#"
        [[device]]
        id = "someone_elses_mouse"
        name = "还没写驱动的那只"
        driver = "not_implemented_yet"

          [device.bluetooth]
          address = "e452430072a9"
        "#,
    )
    .unwrap()
    .devices
    .remove(0);
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "还没写驱动的那只",
        "e452430072a9",
        Some(71),
        Some(42),
    ));

    let reading = status::read(&device, &endpoints).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Ble);
    assert_eq!(reading.reading.reported_level, 71);
}

/// 一条都不在场是另一回事：多半是设备没插或者配置写错，而不是设备那头哑了。
/// 这两种说法要用户做的事完全不同，所以话也不同。
#[test]
fn says_no_endpoint_is_present_rather_than_that_reading_failed() {
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    let error = status::read(&mouse_with_both_endpoints(), &endpoints)
        .unwrap_err()
        .to_string();

    assert!(error.contains("不在场"), "一条都没枚举到时的说法：{error}");
    assert!(error.contains("Wired") && error.contains("Dongle24G"));
}

/// 一条都不在场时印出去的那份"这个 Device 配了哪几条"的名单里，**蓝牙不能漏**。
///
/// 它最容易漏：蓝牙配的不是 `HidEndpoint`，所以"这条 HID 配了吗"对它永远答不上来。
/// 漏掉的后果是用户拿着一份名单去找原因，而真正没在场的那一条根本不在名单上。
#[test]
fn lists_the_ble_endpoint_among_the_ones_this_device_configured() {
    // 一条都不在场：两条 HID 没枚举到，本机的 BLE 设备里也没有这个地址。
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    let error = status::read(&mouse_with_all_three_endpoints(), &endpoints)
        .unwrap_err()
        .to_string();

    assert!(error.contains("不在场"), "一条都没枚举到时的说法：{error}");
    assert!(
        error.contains("Wired") && error.contains("Dongle24G") && error.contains("Ble"),
        "配了三条就该说三条：{error}"
    );
}

/// `status` 那一行要标出这个读数来自哪条 Endpoint。
///
/// 同一个 Device 的两条 Endpoint 在不同时刻各自可用，不写出来就看不出这一行是插着线
/// 读的还是走 2.4G 读的——而"插着线"恰恰是那个最容易被误报成离线的时刻。
#[test]
fn the_status_line_says_which_endpoint_it_came_from() {
    let device = mouse_with_both_endpoints();

    let wired = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()])],
    );
    let line = status::render(&status::read(&device, &wired).unwrap());
    assert!(line.contains("Wired"), "该标出来源 Endpoint：{line}");
    assert!(line.contains("95%"), "电量照常显示：{line}");
    assert!(line.contains("充电中"), "插着线读到的正是充电中：{line}");

    let dongle = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()])],
    );
    let line = status::render(&status::read(&device, &dongle).unwrap());
    assert!(line.contains("Dongle24G"), "该标出来源 Endpoint：{line}");
    assert!(!line.contains("Wired"), "这一行不是从 Wired 读的：{line}");
}

/// 蓝牙那一行要与另两级**分得清**：它不是当场问出来的，是 Windows 攒的缓存。
///
/// spec 的第 6 条 user story：「我想知道某个读数是刚问出来的还是 Windows 攒了几天的
/// 缓存，这样我不会在关键时刻被假的安全感骗到」。所以来源那一段除了 Endpoint 的名字，
/// 还把那份缓存多久以前更新的一起带出来——`scan` 子命令的提示里早写过这句话：
/// 「蓝牙读数是 Windows 的缓存，『多久前』那一列才是它的真实可信度」。
///
/// 这条用例只断言"说得出来"，**不涉及任何陈旧判定**：阈值、变灰、只显示日期，
/// 连同 Clock 接缝都是票 06 的活。
#[test]
fn the_status_line_tells_the_ble_cache_apart_from_the_two_hid_endpoints() {
    let device = mouse_with_all_three_endpoints();

    let cached = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));
    let line = status::render(&status::read(&device, &cached).unwrap());
    assert!(line.contains("62%"), "电量照常显示：{line}");
    assert!(line.contains("Ble"), "该标出来源 Endpoint：{line}");
    assert!(
        line.contains("缓存"),
        "要说清这个数是缓存而不是当场问出来的：{line}"
    );
    assert!(
        line.contains("5 分钟前"),
        "「多久前」那一列才是它的真实可信度：{line}"
    );
    assert!(
        !line.contains("充电"),
        "Windows 的电量属性里没有充电这一项，说不上来就整格不印：{line}"
    );
    assert!(!line.contains("mV"), "也没有电压那一项：{line}");

    // 反过来：当场问出来的那两级不该被说成缓存。
    let wired = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()])],
    );
    let line = status::render(&status::read(&device, &wired).unwrap());
    assert!(line.contains("来自 Wired"), "来源那一段照旧：{line}");
    assert!(
        !line.contains("缓存"),
        "HID 是当场往返，没有缓存这回事：{line}"
    );
}

/// 未登记的 BLE 设备缺省**不列**。
///
/// 它是一条独立于每行 Device 的输出维度：本机扫得到、而配置里没有对应 `[[device]]` 的
/// 那些。缺省关着，因为一台机器上的 BLE 设备多数跟键鼠无关（耳机、手机、手环），
/// 全列出来只会把两行有用的埋掉。
#[test]
fn does_not_list_unregistered_ble_devices_by_default() {
    let config = Config::parse(REGISTERED_MOUSE).unwrap();
    let scanned = [scanned_ble("某个耳机", "aabbccddeeff", Some(80), Some(60))];

    assert!(status::unregistered_ble_lines(&config, &scanned).is_empty());
}

/// 打开 `show_unknown_ble` 就列出来，登记过的那台**不重复出现**——它已经是上面那一行了。
///
/// 认"登记过没有"靠 MAC：这里配置写的是带冒号的大写，而 Windows 报的是不带分隔符的
/// 小写，两者说的是同一台设备。认不出来的症状是那台设备既在上面那一行、又在这一段里
/// 出现一次，像是本机有两只鼠标。
#[test]
fn lists_unregistered_ble_devices_when_show_unknown_ble_is_on() {
    let config = Config::parse(&format!(
        "[general]\nshow_unknown_ble = true\n{REGISTERED_MOUSE}"
    ))
    .unwrap();
    let scanned = [
        scanned_ble("Dragonfly 3 Master+", "e452430072a9", Some(62), Some(300)),
        scanned_ble("某个耳机", "aabbccddeeff", Some(80), Some(60)),
    ];

    let lines = status::unregistered_ble_lines(&config, &scanned);

    assert_eq!(lines.len(), 1, "登记过的那台不该再出现一次：{lines:?}");
    assert!(lines[0].contains("某个耳机"), "{:?}", lines[0]);
    assert!(
        lines[0].contains("aabbccddeeff"),
        "MAC 要能直接抄走：{:?}",
        lines[0]
    );
    assert!(lines[0].contains("80%"), "{:?}", lines[0]);
}

/// 一个 Device 都没配的时候，本机扫到的**每一台**都是未登记的。
///
/// 这正是最需要这份名单的时刻：接新设备、或者配置还没自举，MAC 就在这几行里等着抄进
/// `[device.bluetooth]`。要是这时反而什么都不列，这条开关在它唯一有用的场合是坏的。
#[test]
fn lists_every_ble_device_when_no_device_is_registered_at_all() {
    let config = Config::parse("[general]\nshow_unknown_ble = true\n").unwrap();
    let scanned = [
        scanned_ble("Dragonfly 3 Master+", "e452430072a9", Some(62), Some(300)),
        scanned_ble("某个耳机", "aabbccddeeff", Some(80), Some(60)),
    ];

    let lines = status::unregistered_ble_lines(&config, &scanned);

    assert_eq!(
        lines.len(),
        2,
        "一台都没登记，那就两台都是未登记的：{lines:?}"
    );
}

/// 已登记的那只鼠标，蓝牙地址在配置里写成带冒号的大写。
const REGISTERED_MOUSE: &str = r#"
[[device]]
id = "dragonfly3"
name = "Dragonfly 3 Master+"
driver = "vgn_mouse"

  [device.bluetooth]
  address = "E4:52:43:00:72:A9"
"#;
