//! `endpoints` 的外部行为里**不需要真机的那一半**。
//!
//! 这个模块的大头是 `SystemEndpoints`，而它要真去枚举本机的 HID collection 和 BLE 设备
//! ——那正是这条接缝存在的理由，测不到也不该测。留在这里的是从那一步之后就纯了的东西：
//! 一台已经扫到手的 BLE 设备读成一份什么样的 Reading。
//!
//! 用它的那两条路径在 `tests/readout.rs`（取数时的降级）与 `tests/tray_hover.rs`（悬停提示那一行）里。

mod common;

use common::{NOW, scanned_ble};
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::sources::Reading;

/// 一台扫到的 BLE 设备读成一份什么样的 Reading。
///
/// `charging` 与 `voltage_mv` 一律说不上来，理由与键盘那两项同一条（parking lot Q7/Q8）：
/// Windows 的电量属性里**根本没有**这两项。填 `Some(false)` / `Some(0)` 交出去，票 03 的
/// 电压值域校验会把每一条蓝牙读数判成异常，`level_source = "auto"` 还会拿 0 mV 查表算出
/// 一个理直气壮的 0%。
#[test]
fn a_ble_cache_reading_says_nothing_about_charging_or_voltage() {
    let cached = scanned_ble("VGN Neon75", "f4ee2553b27e", Some(48), Some(7200));

    let reading = EndpointReading::from_ble_cache(&cached, NOW).unwrap();

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

    let error = EndpointReading::from_ble_cache(&cached, NOW)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("没有电量属性"),
        "说清是读不到而不是 0：{error}"
    );
}

/// **取得时刻是"这个数描述的那一刻"，不是"我们跑这次查询的那一刻"。**
///
/// 两条 HID 是当场往返，两者重合。`Ble` 读的是 Windows 攒的缓存：那个百分比描述的是
/// `cache_age_secs` 秒之前的设备，所以取得时刻要往回退那么多秒。把它记成"此刻"，
/// 一个 10 天前的数字就会看着像刚问出来的——`CONTEXT.md` 说 Stale 形容的是「取得时刻
/// 距今已久」，而按"查询时刻"记，`Ble` 就永远陈旧不了，那句定义会落空。
#[test]
fn a_reading_is_stamped_with_the_moment_its_number_was_true() {
    let hid = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 95,
            charging: Some(true),
            voltage_mv: Some(4180),
        },
        NOW,
    );
    assert_eq!(hid.taken_at, Some(NOW), "当场往返，就是此刻");
    assert_eq!(hid.cache_age_secs, None, "HID 没有缓存这回事");

    // 票 05 的真机记录：那台鼠标的缓存是 10 天前的。
    let cached = scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(95),
        Some(864_000),
    );
    let ble = EndpointReading::from_ble_cache(&cached, NOW).unwrap();
    assert_eq!(ble.taken_at, Some(NOW.minus_secs(864_000)));
    assert_eq!(ble.taken_at.unwrap().utc_date_text(), "2026-03-15");
}

/// 没有更新时间戳的 BLE 设备**说不出**取得时刻。
///
/// `bluetooth.rs` 的原话：`age_secs` 为 `None` 是"这台设备根本没有更新时间戳"。拿"此刻"
/// 顶上去，一台没时间戳的设备就会看着像当场读的——两种 `None` 混起来正是这张票要防的
/// 假的安全感。空着，陈旧判定那一步才分得清（它把这一种判成陈旧）。
#[test]
fn a_ble_device_without_an_update_timestamp_has_no_known_moment() {
    let cached = scanned_ble("VGN Neon75", "f4ee2553b27e", Some(100), None);

    let reading = EndpointReading::from_ble_cache(&cached, NOW).unwrap();

    assert_eq!(reading.taken_at, None);
    assert_eq!(reading.cache_age_secs, None);
}
