//! `endpoints` 的外部行为里**不需要真机的那一半**。
//!
//! 这个模块的大头是 `SystemEndpoints`，而它要真去枚举本机的 HID collection 和 BLE 设备
//! ——那正是这条接缝存在的理由，测不到也不该测。留在这里的是从那一步之后就纯了的东西：
//! 一台已经扫到手的 BLE 设备读成一份什么样的 Reading。
//!
//! 用它的那两条路径（取数时的降级、`status` 那一行）在 `tests/status.rs` 里。

mod common;

use common::scanned_ble;
use juicebar::endpoints::{EndpointKind, EndpointReading};

/// 一台扫到的 BLE 设备读成一份什么样的 Reading。
///
/// `charging` 与 `voltage_mv` 一律说不上来，理由与键盘那两项同一条（parking lot Q7/Q8）：
/// Windows 的电量属性里**根本没有**这两项。填 `Some(false)` / `Some(0)` 交出去，票 03 的
/// 电压值域校验会把每一条蓝牙读数判成异常，`level_source = "auto"` 还会拿 0 mV 查表算出
/// 一个理直气壮的 0%。
#[test]
fn a_ble_cache_reading_says_nothing_about_charging_or_voltage() {
    let cached = scanned_ble("VGN Neon75", "f4ee2553b27e", Some(48), Some(7200));

    let reading = EndpointReading::from_ble_cache(&cached).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Ble);
    assert_eq!(reading.reading.reported_level, 48);
    assert_eq!(reading.reading.charging, None);
    assert_eq!(reading.reading.voltage_mv, None);
    assert_eq!(reading.cache_age_secs, Some(7200));
}

/// 答不出电量的 BLE 设备是"这条通路这一次读不到"，**不是"电量 0"**。
///
/// 本机扫描那一步（`bluetooth::enumerate`）就把这种设备滤掉了，所以产品里走不到这一支；
/// 断言它是因为类型上它必须被交代一次，而**交代的话得是实话**。把缺席读成 0 就正好在最
/// 危险的那一格上撒谎——项目一路在防的就是这种看着合理其实是编的数字。
#[test]
fn a_ble_device_without_a_battery_property_is_not_a_reading_of_zero() {
    let cached = scanned_ble("某个耳机", "aabbccddeeff", None, Some(10));

    let error = EndpointReading::from_ble_cache(&cached)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("没有电量属性"),
        "说清是读不到而不是 0：{error}"
    );
}
