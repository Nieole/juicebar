//! 陈旧判定：一份读数的取得时刻距今多久算"不该再当作现状"。
//!
//! 全是纯函数，喂进去的是三个数（取得时刻、这条 Endpoint 的阈值、当下），**一次 sleep
//! 都没有**。这也是这条判定唯一能被验到的地方：`status` 是一次性命令，当场枚举、当场
//! 取数、印一行就退出，所以今天的 HID 读数永远是刚取的，3 × 轮询间隔那一档端到端看不见
//! ——常驻轮询是后面的票。

mod common;

use common::{NOW, default_general};
use juicebar::config::Config;
use juicebar::endpoints::EndpointKind;
use juicebar::staleness::{Freshness, StalenessPolicy};

/// HID 的陈旧阈值 = 3 × 该 Endpoint 的轮询间隔，自动推导，不设配置项。
///
/// 3 倍而不是 1 倍：错过一次轮询是常事（设备忙、一次超时），把它就地判成陈旧会让
/// 那一格不停地闪。连错三次才是"这条通路真的不说话了"。
#[test]
fn a_hid_reading_goes_stale_after_three_polling_intervals() {
    let general = default_general();
    let wired = StalenessPolicy::for_endpoint(EndpointKind::Wired, &general);
    // 缺省 poll_interval_wired = 30，所以阈值是 90 秒。
    assert_eq!(wired.stale_after_secs, 90);

    let just_inside = Some(NOW.minus_secs(89));
    assert_eq!(wired.verdict(just_inside, NOW), Freshness::Fresh);

    let just_outside = Some(NOW.minus_secs(91));
    assert_eq!(wired.verdict(just_outside, NOW), Freshness::Stale);
}

/// 阈值是**该 Endpoint 自己**的轮询间隔推出来的，不是一个全局的数。
///
/// 三条 Endpoint 的取数成本差着数量级，间隔本来就分开配（Wired 走线不耗设备的电，
/// Dongle24G 要发无线包）。共用一个阈值就意味着省着轮询的那一级会被判得更严，
/// 而它慢恰恰是配置要它慢。
#[test]
fn derives_each_hid_threshold_from_that_endpoints_own_polling_interval() {
    let general = Config::parse(
        r#"
        [general]
        poll_interval_wired = 30
        poll_interval_24g = 60
        "#,
    )
    .unwrap()
    .general;

    let taken_at = Some(NOW.minus_secs(100));

    // Wired 阈值 90 秒：100 秒前的读数已经陈旧。
    let wired = StalenessPolicy::for_endpoint(EndpointKind::Wired, &general);
    assert_eq!(wired.verdict(taken_at, NOW), Freshness::Stale);

    // Dongle24G 阈值 180 秒：同一份读数在它那里还新鲜。
    let dongle = StalenessPolicy::for_endpoint(EndpointKind::Dongle24G, &general);
    assert_eq!(dongle.verdict(taken_at, NOW), Freshness::Fresh);
}

/// `Ble` 用配置里的 `stale_after` / `very_stale_after`，而**不是**它的轮询间隔。
///
/// 那一级读的是 Windows 攒的缓存：轮询得再勤也不会让缓存里的数字变新，所以"多久算旧"
/// 只能由用户给。它也是唯一有第三档的一级——`CONTEXT.md`：「只有 Ble 的 Reading 会陈旧到
/// 有实际影响。」
#[test]
fn a_ble_reading_uses_the_two_configured_thresholds() {
    let general = default_general();
    let ble = StalenessPolicy::for_endpoint(EndpointKind::Ble, &general);
    // 缺省 stale_after = 3600、very_stale_after = 86400。
    assert_eq!(ble.stale_after_secs, 3_600);
    assert_eq!(ble.very_stale_after_secs, Some(86_400));

    // 半小时前的缓存还算现状。
    assert_eq!(
        ble.verdict(Some(NOW.minus_secs(1_800)), NOW),
        Freshness::Fresh
    );
    // 两小时前：超过 stale_after，标注为陈旧，数字照印。
    assert_eq!(
        ble.verdict(Some(NOW.minus_secs(7_200)), NOW),
        Freshness::Stale
    );
    // 实测那台鼠标的缓存是 10 天前的（票 05 的真机记录），早就过了 very_stale_after
    // 的一天——它当时还照常印着 95%，那正是这张票要拦下的假的安全感。
    assert_eq!(
        ble.verdict(Some(NOW.minus_secs(10 * 86_400)), NOW),
        Freshness::VeryStale
    );
}

/// 两条 HID **没有** VeryStale 这一档。
///
/// 那一档的表现是"不显示百分比，只显示日期"，而 HID 的阈值只有 3 × 轮询间隔一个数：
/// 拿它当 VeryStale 就意味着一份 91 秒前、间隔 30 秒的有线读数会立刻只剩一个日期，
/// 而它其实是一分半钟前当场问出来的。`config.example.toml` 也写明那两个阈值只对 Ble 生效。
#[test]
fn a_hid_reading_never_becomes_very_stale() {
    let general = default_general();

    for endpoint in [EndpointKind::Wired, EndpointKind::Dongle24G] {
        let policy = StalenessPolicy::for_endpoint(endpoint, &general);
        assert_eq!(policy.very_stale_after_secs, None, "{endpoint} 没有这一档");
        // 十天前的 HID 读数（只可能来自持久化）陈旧，但仍然是数字而不是一个日期。
        assert_eq!(
            policy.verdict(Some(NOW.minus_secs(10 * 86_400)), NOW),
            Freshness::Stale,
            "{endpoint}"
        );
    }
}

/// 说不出取得时刻的读数**不算新鲜**。
///
/// `bluetooth.rs` 的原话：`age_secs` 为 `None` 是"这台设备根本没有更新时间戳"。
/// 把它当成"刚问出来的"，一台没时间戳的蓝牙设备就会看着像当场读的——那正是这张票要防的
/// 假的安全感。它落在 Stale 而不是 VeryStale：VeryStale 的表现是"只显示日期"，
/// 而这里恰恰是**连日期都说不出来**的那一种。
#[test]
fn a_reading_with_no_known_moment_is_not_treated_as_fresh() {
    let general = default_general();

    for endpoint in EndpointKind::PRIORITY {
        let policy = StalenessPolicy::for_endpoint(endpoint, &general);
        assert_eq!(policy.verdict(None, NOW), Freshness::Stale, "{endpoint}");
    }
}
