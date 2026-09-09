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
    KEYBOARD_REPORT_ID, KEYBOARD_RESTING_FULL, MOUSE_BATTERY_REQUEST, MOUSE_CHARGING,
    MOUSE_REPORT_ID, MOUSE_RESTING_FULL, mouse_frame_with,
};
use common::{
    FakeEndpoints, NOW, default_general, keyboard_with_dongle_endpoint,
    mouse_with_all_three_endpoints, mouse_with_both_endpoints, scanned_ble,
};
use juicebar::cli::status;
use juicebar::cli::status::DeviceRow;
use juicebar::clock::Timestamp;
use juicebar::config::Config;
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::primary::{CandidateReading, PrimaryRule};
use juicebar::sources::level::{Level, LevelSource};
use juicebar::staleness::{Freshness, Staleness};

/// 一份读数在 [`NOW`] 这一刻印成的那一行。
///
/// 把"判一次 + 排一次版"收成一句：这个文件关心的是**那一行印成什么样**，而阈值本身怎么
/// 从轮询间隔推导出来在 `tests/staleness.rs`。
///
/// `level_source` 取缺省的 `Auto`：这里的用例关心 Endpoint 合成、来源标注和陈旧那几档，
/// 不是"两个百分比里印哪一个"——后者在 `tests/level.rs` 里单独断言。要按 Device 指定
/// 来源的用例走下面那个 [`render`]。
fn line_of(reading: &EndpointReading) -> String {
    status::render(
        reading,
        LevelSource::Auto,
        &Staleness::assess(reading, &default_general(), NOW),
    )
}

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

    let reading = status::read(&mouse_with_both_endpoints(), &endpoints, NOW).unwrap();

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

    status::read(&mouse_with_both_endpoints(), &endpoints, NOW).unwrap();

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

    let reading = status::read(&mouse_with_both_endpoints(), &endpoints, NOW).unwrap();

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
        status::read(&device, &plugged, NOW).unwrap().endpoint,
        EndpointKind::Wired
    );

    // 拔掉线：那条 Endpoint 不再被枚举出来。
    let unplugged = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()])],
    );

    let reading = status::read(&device, &unplugged, NOW).unwrap();

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

    let reading = status::read(&mouse_with_both_endpoints(), &endpoints, NOW).unwrap();

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

    let error = status::read(&mouse_with_both_endpoints(), &endpoints, NOW)
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

    let reading = status::read(&mouse_with_all_three_endpoints(), &endpoints, NOW).unwrap();

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

    let reading = status::read(&mouse_with_all_three_endpoints(), &endpoints, NOW).unwrap();

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

    let reading = status::read(&mouse_with_all_three_endpoints(), &endpoints, NOW).unwrap();

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

    let reading = status::read(&device, &endpoints, NOW).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Ble);
    assert_eq!(reading.reading.reported_level, 71);
}

/// 一条都不在场是另一回事：多半是设备没插或者配置写错，而不是设备那头哑了。
/// 这两种说法要用户做的事完全不同，所以话也不同。
#[test]
fn says_no_endpoint_is_present_rather_than_that_reading_failed() {
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    let error = status::read(&mouse_with_both_endpoints(), &endpoints, NOW)
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

    let error = status::read(&mouse_with_all_three_endpoints(), &endpoints, NOW)
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
    let line = render(&device, &wired);
    assert!(line.contains("Wired"), "该标出来源 Endpoint：{line}");
    assert!(line.contains("95%"), "电量照常显示：{line}");
    assert!(line.contains("充电中"), "插着线读到的正是充电中：{line}");

    let dongle = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()])],
    );
    let line = render(&device, &dongle);
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
/// 这条用例只断言"说得出来"。阈值怎么推导在 `tests/staleness.rs`，标注与"只显示日期"
/// 在下面那几条——这一条守的是来源那一段本身的措辞。
#[test]
fn the_status_line_tells_the_ble_cache_apart_from_the_two_hid_endpoints() {
    let device = mouse_with_all_three_endpoints();

    let cached = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));
    let line = render(&device, &cached);
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
    let line = render(&device, &wired);
    assert!(line.contains("来自 Wired"), "来源那一段照旧：{line}");
    assert!(
        !line.contains("缓存"),
        "HID 是当场往返，没有缓存这回事：{line}"
    );
}

/// 陈旧的读数被**明确标注**，与新鲜的分得清。
///
/// 这是这张票的全部意义：蓝牙缓存可能过期几个月，在最需要它的时候给出假的安全感。
/// 一个不带任何标注的 `62%` 和一个刚问出来的 `62%` 长得一模一样，而它们的可信度差着
/// 一个数量级。
#[test]
fn the_status_line_marks_a_stale_reading_apart_from_a_fresh_one() {
    let device = mouse_with_all_three_endpoints();

    // 五分钟前的缓存：缺省 stale_after 是一小时，还算现状。
    let fresh = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));
    let reading = status::read(&device, &fresh, NOW).unwrap();
    let line = line_of(&reading);
    assert!(line.contains("62%"), "新鲜的读数照常显示百分比：{line}");
    assert!(
        !line.contains("陈旧"),
        "五分钟前的缓存不该被标成陈旧：{line}"
    );

    // 两小时前的缓存：超过 stale_after，数字照印但要标出来。
    let stale = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(7_200),
    ));
    let reading = status::read(&device, &stale, NOW).unwrap();
    let line = line_of(&reading);
    assert!(line.contains("62%"), "这一档数字还是照印：{line}");
    assert!(line.contains("陈旧"), "但要明确标注出来：{line}");
    assert!(line.contains("2 小时前"), "「多久前」也照印：{line}");
}

/// 一台**没有更新时间戳**的蓝牙设备不该看着像刚问出来的。
///
/// `bluetooth.rs` 的原话：`age_secs` 为 `None` 是"这台设备根本没有更新时间戳"。它和
/// 两条 HID 那个恒为 `None` 的缓存年龄是两个完全不同的意思，混起来正好在最危险的方向上
/// 撒谎——说不出这个数多旧的时候，最像真的那个说法恰恰是"刚取的"。
#[test]
fn a_ble_reading_without_a_timestamp_does_not_look_freshly_taken() {
    let device = mouse_with_all_three_endpoints();

    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        None,
    ));

    let reading = status::read(&device, &endpoints, NOW).unwrap();
    let line = line_of(&reading);

    assert!(line.contains("无时间戳"), "说清它没有时间戳：{line}");
    assert!(line.contains("陈旧"), "说不出多旧的读数不算新鲜：{line}");
}

/// 每一行都说得出"多久前"——包括两条当场往返的 HID。
///
/// 今天 `status` 是一次性命令，HID 那一格永远是"0 秒前"；写出来是因为常驻轮询之后
/// 它才是真话，而那时**看不出这一行是几秒前还是几分钟前取的**才是真正的问题。
#[test]
fn every_status_line_says_how_long_ago_the_reading_was_taken() {
    let device = mouse_with_both_endpoints();

    let wired = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()])],
    );
    let reading = status::read(&device, &wired, NOW).unwrap();
    let line = line_of(&reading);

    assert!(line.contains("0 秒前"), "当场问出来的也要说一句：{line}");
    assert!(!line.contains("陈旧"), "刚取的读数不是陈旧的：{line}");
    assert!(
        !line.contains("缓存"),
        "HID 是当场往返，没有缓存这回事：{line}"
    );
}

/// 超过 `very_stale_after` 的读数**只显示日期，不显示百分比**。
///
/// 票 05 的真机实测：`Dragonfly 3 Master+  95%（Reported Level）  来自 Ble（Windows 缓存，
/// 10 天前）`。缺省 `very_stale_after` 是一天，那一行早就过了，却还照常印着 95%——那个
/// 数字是 3 月 15 日的，而它看上去和现在的电量没有任何区别。这条用例就是把那一行拦下来。
///
/// 一起被拿掉的还有充电态与电压：几个月前"在充电"是同一句假话的另一半。
#[test]
fn shows_only_the_date_when_a_reading_is_older_than_very_stale_after() {
    let device = mouse_with_all_three_endpoints();

    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(95),
        Some(10 * 86_400),
    ));

    let reading = status::read(&device, &endpoints, NOW).unwrap();
    let line = line_of(&reading);

    assert!(!line.contains("95"), "百分比不该再出现：{line}");
    assert!(!line.contains('%'), "一个百分号都不该有：{line}");
    // NOW 是 2026-03-25，往回退十天。
    assert!(line.contains("2026-03-15"), "换上那个数是哪天的：{line}");
    assert!(line.contains("10 天前"), "「多久前」照印：{line}");
    assert!(line.contains("已陈旧"), "照旧明确标注：{line}");
}

/// 刚好卡在阈值上的读数**还不算**陈旧。
///
/// 边界要有一侧，写下来是因为两侧都说得通，而"超过才算"是配置注释的原话（"超过标灰"）。
#[test]
fn a_reading_exactly_at_the_threshold_is_still_current() {
    let device = mouse_with_all_three_endpoints();

    // 缺省 stale_after = 3600：整整一小时前的缓存还不算陈旧。
    let at_threshold = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(3_600),
    ));
    let reading = status::read(&device, &at_threshold, NOW).unwrap();
    let line = line_of(&reading);
    assert!(!line.contains("陈旧"), "整一小时还不算超过：{line}");

    // 多一秒就算。
    let past_threshold = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(3_601),
    ));
    let reading = status::read(&device, &past_threshold, NOW).unwrap();
    let line = line_of(&reading);
    assert!(line.contains("已陈旧"), "多一秒就算：{line}");
}

/// `Ble` 那一格的"多久前"是 **Windows 自己报的**秒数，不经过 Clock 换算回去。
///
/// parking lot Q21 给本票的原话：「Ble 的"多久以前"已经从 `cache_age_secs` 带出来了……
/// 它是 Windows 报的秒数、**不经过 Clock**（那份时间戳是系统攒的，本机时钟只能用来算差值，
/// 而 `bluetooth.rs` 已经算过了）」。
///
/// 两者平时看不出差别：取得时刻就是 `当下 − cache_age_secs`，减回去必然还是原数。所以这条
/// 用例**故意让判定的"当下"比取数的晚**——那时"来源那一段印的是哪一个数"才现形。真机上
/// 两个"当下"是同一个（`run` 一台设备只问一次时钟），这条用例守的是那个换算别偷偷长回来。
#[test]
fn the_ble_age_is_the_number_windows_reported_not_one_recomputed_from_the_clock() {
    let device = mouse_with_all_three_endpoints();

    // Windows 说这份缓存 3000 秒（50 分钟）之前更新过。
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(3_000),
    ));
    let reading = status::read(&device, &endpoints, NOW).unwrap();

    // 取数在 NOW，判定在一千秒之后：此刻这份读数已经四千秒了，超过缺省的 stale_after。
    // 用 as_unix_secs / from_unix_secs 往前挪，而不是给 Timestamp 加一个只有用例用得到的
    // `plus_secs`——生产接口不为用例的方便而长。
    let later = Timestamp::from_unix_secs(NOW.as_unix_secs() + 1_000);
    let line = status::render(
        &reading,
        LevelSource::Auto,
        &Staleness::assess(&reading, &default_general(), later),
    );

    assert!(
        line.contains("50 分钟前"),
        "来源那一段印 Windows 报的那个数：{line}"
    );
    assert!(
        !line.contains("66 分钟前"),
        "不该拿本机时钟把它重算一遍：{line}"
    );
    // 判定用的**是**晚一点的那个当下——陈旧与否看的是取得时刻距今多久。
    assert!(
        line.contains("已陈旧"),
        "四千秒已经过了 stale_after：{line}"
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

/// 那一行要说清这个百分比是两个来源里的哪一个。
///
/// 项目里有两个来源不同的百分比（Reported Level 与 Derived Level），它们可能不一致，
/// 混用过一次就已经导致过一个错误结论。哪一个都不标出来，看的人只能猜。
///
/// 实测帧的电压全都在 4110 mV 以上（4154–4235），所以查表那一段在纯实测夹具里走不到
/// ——半空那一帧的造法和理由见 `fixtures::mouse_frame_with`。
#[test]
fn the_status_line_says_which_of_the_two_percentages_it_is() {
    let device = mouse_with_both_endpoints();

    // 4235 mV 越过查表的表顶，改用固件自报的 95。
    let above_the_table = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()])],
    );
    let line = render(&device, &above_the_table);
    assert!(
        line.contains("95%") && line.contains("Reported Level"),
        "表顶以上要标成 Reported Level：{line}"
    );

    // 3900 mV 落在查表可用的区间，用查表算出的 53，而不是固件报的 20。
    let inside_the_table = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(
            EndpointKind::Wired,
            vec![mouse_frame_with(20, 3900).to_vec()],
        )],
    );
    let line = render(&device, &inside_the_table);
    assert!(
        line.contains("53%") && line.contains("Derived Level"),
        "表顶以下要标成 Derived Level：{line}"
    );
    assert!(!line.contains("20%"), "这一段不该采信固件自报值：{line}");
}

/// **键盘电量读到 0 时那一行说 Unknown，绝不说 100%。**
///
/// 这是本票的另一条招牌回归，而且它端到端：配置里键盘没写 `level_source`（缺省 auto）、
/// 键盘的回包里没有电压所以 auto 只剩固件自报值、而那个值是 0 —— 于是 Unknown。
/// 上位机在这里显示 100%（`level == 0 ? 100 : level`），偏偏发生在最该提醒充电的时刻。
/// **Unknown 也不是 0%**：那同样是个采信不了的数字（`CONTEXT.md`）。
#[test]
fn the_status_line_says_unknown_instead_of_calling_a_zero_full() {
    let mut empty = KEYBOARD_RESTING_FULL;
    empty[1] = 0;
    let endpoints = FakeEndpoints::new(
        KEYBOARD_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![empty.to_vec()])],
    );

    let line = render(&keyboard_with_dongle_endpoint(), &endpoints);

    assert!(line.contains("Unknown"), "读不出来就说读不出来：{line}");
    assert!(!line.contains("100"), "绝不把 0 说成满电：{line}");
    assert!(!line.contains("0%"), "Unknown 也不等于 0%：{line}");
}

/// 合理性校验不过时，那一行说"读取异常"，**一个百分比都不给**。
///
/// 帧完整、cmd 回显也对，只是里头的电压物理上不可能——固件漂移就是这个形状。这时
/// 拿那串字节按老下标算出来的任何数字都是编的，所以 `status` 交出去的是原因而不是数。
///
/// **守的是 `read()` 交出来的那句原因，不是最终印在屏幕上那一行**：整行由 `run()` 拼
/// （`读不到 —— {原因}`），而 `run()` 要真去枚举本机 HID，测不到。差的这一截记在
/// parking lot Q13。
#[test]
fn an_implausible_frame_reads_as_an_anomaly_rather_than_a_number() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(
            EndpointKind::Wired,
            vec![mouse_frame_with(100, 2000).to_vec()],
        )],
    );

    let line = status::read(&mouse_with_both_endpoints(), &endpoints, NOW)
        .unwrap_err()
        .to_string();

    assert!(line.contains("读取异常"), "要说成读取异常：{line}");
    assert!(!line.contains('%'), "一个百分比都不该出现：{line}");
}

/// 取一次数并印成一行。
fn render(device: &juicebar::config::Device, endpoints: &FakeEndpoints) -> String {
    let reading = status::read(device, endpoints, NOW).unwrap();
    status::render(
        &reading,
        device.level_source,
        &Staleness::assess(&reading, &default_general(), NOW),
    )
}

// ---------------------------------------------------------------
// Primary Device 标注（票 09）
//
// 选出谁在 `tests/primary.rs` 里断言完了，这里断言的是**那几行印成什么样**：标注落在
// 哪一行上、末尾那句交代什么时候补。
// ---------------------------------------------------------------

/// 攒一行：Device 名字之后那一段直接给字面量，因为这几条用例关心的是**标注**，
/// 而不是那一段怎么排版（那在上面）。
fn row<'a>(
    device: &'a juicebar::config::Device,
    line: &str,
    level: Option<Level>,
) -> DeviceRow<'a> {
    DeviceRow {
        device,
        line: line.to_string(),
        candidate: level.map(|level| CandidateReading {
            level,
            freshness: Freshness::Fresh,
        }),
    }
}

/// 票面第 6 条：`status` 标出当前 Primary Device —— 标注真的落在那一行上。
#[test]
fn marks_the_primary_device_on_its_own_line() {
    let mouse = mouse_with_both_endpoints();
    let keyboard = keyboard_with_dongle_endpoint();
    let rows = [
        row(&mouse, "62%（Reported Level）", Some(Level::Reported(62))),
        row(
            &keyboard,
            "28%（Reported Level）",
            Some(Level::Reported(28)),
        ),
    ];

    let lines = status::primary_lines(&PrimaryRule::Lowest, &rows);

    assert_eq!(
        lines,
        [
            "Dragonfly 3 Master+  62%（Reported Level）",
            "VGN Neon75（Primary Device）  28%（Reported Level）",
        ]
    );
}

/// 一台失联的 Device 照样标得出来 —— 钉死的那一种。
///
/// **这一条是把标注放在 Device 名字那一格、而不是放进 `render` 的理由**（parking lot
/// Q44）：这一行根本没有 Reading，`render` 压根没被调用过。
#[test]
fn marks_a_pinned_device_that_could_not_be_read() {
    let mouse = mouse_with_both_endpoints();
    let keyboard = keyboard_with_dongle_endpoint();
    let rows = [
        row(&mouse, "62%（Reported Level）", Some(Level::Reported(62))),
        row(&keyboard, "读不到 —— 全部 Endpoint 都没读到", None),
    ];

    let lines = status::primary_lines(&PrimaryRule::Pinned("neon75".to_string()), &rows);

    assert!(
        lines[1].starts_with("VGN Neon75（Primary Device）"),
        "钉死的那台读不到也照旧标着：{:?}",
        lines[1]
    );
    assert!(
        !lines[0].contains("Primary Device"),
        "另一台不该被标上：{:?}",
        lines[0]
    );
}

/// 一行都标不出来时，末尾补一句交代 —— 否则"没有标注"看着像 bug。
#[test]
fn adds_a_closing_note_when_no_device_got_the_marker() {
    let mouse = mouse_with_both_endpoints();
    let rows = [row(&mouse, "读不到 —— 全部 Endpoint 都没读到", None)];

    let lines = status::primary_lines(&PrimaryRule::Lowest, &rows);

    assert_eq!(lines.len(), 2, "一行 Device 加一句交代：{lines:?}");
    assert!(
        lines[1].contains("Primary Device"),
        "末尾那句说的是 Primary Device 这件事：{:?}",
        lines[1]
    );
}

/// 正常选出来时末尾不多话：那一行上的标注已经把话说完了。
#[test]
fn adds_no_closing_note_when_the_marker_speaks_for_itself() {
    let mouse = mouse_with_both_endpoints();
    let rows = [row(
        &mouse,
        "62%（Reported Level）",
        Some(Level::Reported(62)),
    )];

    assert_eq!(
        status::primary_lines(&PrimaryRule::Lowest, &rows).len(),
        1,
        "只该有那一行 Device"
    );
}

/// 一个 Device 都没配的时候一句话都不补。
///
/// `run` 那时已经印过"一个 Device 都没有"，把原因说完了；再补一句"选不出 Primary Device"
/// 只是同一件事的第二遍。
#[test]
fn stays_silent_when_no_device_is_configured() {
    assert!(status::primary_lines(&PrimaryRule::Lowest, &[]).is_empty());
}
