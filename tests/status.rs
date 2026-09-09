//! `cli::status` 的外部行为，也就是 Endpoint 合成：在场的几条 Endpoint 里先试哪一条、
//! 靠前的那条不行时怎么降级、什么时候才算失联、那一行印成什么样。
//!
//! 全程经假枚举 + 假 Transport，不接任何硬件——尤其是"拔线"这件事，真机上拔一次
//! 线才复现一次。
//!
//! `src/endpoints.rs` 没有自己的测试文件是有意的：那个模块除了接缝本身只有
//! `HidEndpoints`，而它要真去枚举本机的 HID collection——那正是这条接缝存在的理由。

mod common;

use common::fixtures::{
    MOUSE_BATTERY_REQUEST, MOUSE_CHARGING, MOUSE_REPORT_ID, MOUSE_RESTING_FULL,
};
use common::{FakeEndpoints, mouse_with_both_endpoints};
use juicebar::cli::status;
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
    assert!(reading.reading.charging);
    assert_eq!(reading.reading.voltage_mv, 4235);
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
