//! `readout` 的外部行为，也就是 Endpoint 合成：在场的几条 Endpoint 里先试哪一条、
//! 靠前的那条不行时怎么降级、什么时候才算失联、以及取不到时怎么退到上次已知值。
//!
//! 全程经假枚举 + 假 Transport，不接任何硬件——尤其是"拔线"这件事，真机上拔一次
//! 线才复现一次。
//!
//! `src/endpoints.rs` 那一半里不需要真机的东西在 `tests/endpoints.rs`；`SystemEndpoints`
//! 要真去枚举本机的 HID collection 和 BLE 设备，那正是这条接缝存在的理由，不测。
//!
//! **这个文件断言的是取数交出来的那份原料**：读到了哪一条 Endpoint、失败那个枚举说的是
//! 哪一句、历史值退不退得到、以及那几条 Endpoint 到底有没有被打开。原料再往后怎么排成
//! 一行在 `tests/status.rs`——凡是要看排好版的那一行的用例都留在那边，暂停那几条里断言
//! 末尾标注的也一样。

mod common;

use common::fixtures::{
    MOUSE_BATTERY_REQUEST, MOUSE_CHARGING, MOUSE_REPORT_ID, MOUSE_RESTING_FULL, mouse_frame_with,
};
use common::{
    FakeEndpoints, NOW, default_general, mouse_with_all_three_endpoints, mouse_with_both_endpoints,
    scanned_ble, vendor_hub_running,
};
use juicebar::config::Config;
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::readout::{self, NoReading};
use juicebar::sources::Reading;
use juicebar::state::{LastKnown, Provenance};

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

    let reading = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW).unwrap();

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

    readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW).unwrap();

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

    let reading = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW).unwrap();

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
        readout::read(&device, &plugged, None, NOW)
            .unwrap()
            .endpoint,
        EndpointKind::Wired
    );

    // 拔掉线：那条 Endpoint 不再被枚举出来。
    let unplugged = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()])],
    );

    let reading = readout::read(&device, &unplugged, None, NOW).unwrap();

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

    let reading = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW).unwrap();

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
/// 试过了都失败的那一种（这里两条都超时）：整句话以「没有可信的读数」开头，而且**只有
/// 这一个标记**——外面再套一句"读不到"就是同一句话说两遍。
///
/// 那句标记要把每一条的原因都带上：只说标记，用户没法知道是两条都哑了，还是只有一条
/// 哑了而另一条压根没被试。
#[test]
fn reports_a_lost_device_only_after_every_endpoint_failed() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![]),
        ],
    );

    let error = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW)
        .unwrap_err()
        .to_string();

    assert!(
        error.starts_with("没有可信的读数 —— "),
        "试过了都失败，标记是中性的那一句：{error}"
    );
    assert!(
        !error.contains("读不到"),
        "一句标记换掉两个前缀，「读不到」是一条都不在场那一支的话：{error}"
    );
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

/// 一条超时、一条回了坏帧时，那句标记**对两者都成立**。
///
/// 这是那句总结非中性不可的理由：同一行里两条 Endpoint 失败得不是一回事，任何点名一种
/// 失败的说法都会对另一条撒谎——"都没读到"对回了坏帧的那条就是假话，我们读到了，只是
/// 读到的东西不可信。卖掉的那点精确度由每条自己的原因紧跟其后补上。
#[test]
fn says_no_trustworthy_reading_when_a_timeout_and_an_anomaly_share_the_line() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            // 回包脚本空着，相当于超时。
            (EndpointKind::Wired, vec![]),
            // 帧是完整的，只是里头的电压物理上不可能——读取异常。
            (
                EndpointKind::Dongle24G,
                vec![mouse_frame_with(100, 2000).to_vec()],
            ),
        ],
    );

    let line = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW)
        .unwrap_err()
        .to_string();

    assert!(
        line.starts_with("没有可信的读数 —— "),
        "一句对两种失败都成立的标记：{line}"
    );
    assert!(
        !line.contains("都没读到"),
        "Dongle24G 答了话，这一行不许声称都没读到：{line}"
    );
    assert!(
        line.contains("Wired") && line.contains("读取异常"),
        "两条各自怎么失败的都还在：{line}"
    );
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

    let reading = readout::read(&mouse_with_all_three_endpoints(), &endpoints, None, NOW).unwrap();

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

    let reading = readout::read(&mouse_with_all_three_endpoints(), &endpoints, None, NOW).unwrap();

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

    let reading = readout::read(&mouse_with_all_three_endpoints(), &endpoints, None, NOW).unwrap();

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

    let reading = readout::read(&device, &endpoints, None, NOW).unwrap();

    assert_eq!(reading.endpoint, EndpointKind::Ble);
    assert_eq!(reading.reading.reported_level, 71);
}

/// 一条都不在场是另一回事：多半是设备没插或者配置写错，而不是设备那头哑了。
/// 这两种说法要用户做的事完全不同，所以话也不同。
///
/// 这一种的标记**照旧是"读不到"，一个字没改**（它被打磨过两轮）：一条通路都没枚举到，
/// 我们确实什么都没读到，这句话在这里是实话。
#[test]
fn says_no_endpoint_is_present_rather_than_that_reading_failed() {
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    let error = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW)
        .unwrap_err()
        .to_string();

    assert!(
        error.starts_with("读不到 —— "),
        "一条都不在场那一支的标记照旧：{error}"
    );
    assert!(error.contains("不在场"), "一条都没枚举到时的说法：{error}");
    assert!(error.contains("Wired") && error.contains("Dongle24G"));
}

/// 一条 Endpoint 都没配的那个 Device：话说到"你什么都没配"为止，**不去替它猜设备没插**。
///
/// 与上面一条同一个标记（"读不到"，我们确实什么都没读到），但后半句完全不同：一份空名单
/// 摆给用户看只会让他去查一个不存在的连接问题，而这里该改的是配置。
///
/// 这一支在本票之前**一条用例都没有**：那句话原先靠失败枚举的 `Display` 补上前缀，整句
/// 长什么样没有任何东西守着。
#[test]
fn says_the_device_configured_no_endpoint_at_all() {
    let device = Config::parse(
        r#"
        [[device]]
        id = "nothing_configured"
        name = "一条都没配的那台"
        driver = "vgn_mouse"
        "#,
    )
    .expect("用例里的配置应当解析得动")
    .devices
    .remove(0);
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    let error = readout::read(&device, &endpoints, None, NOW)
        .unwrap_err()
        .to_string();

    assert_eq!(error, "读不到 —— 这个 Device 一条 Endpoint 都没配置");
}

/// 一条都不在场时印出去的那份"这个 Device 配了哪几条"的名单里，**蓝牙不能漏**。
///
/// 它最容易漏：蓝牙配的不是 `HidEndpoint`，所以"这条 HID 配了吗"对它永远答不上来。
/// 漏掉的后果是用户拿着一份名单去找原因，而真正没在场的那一条根本不在名单上。
#[test]
fn lists_the_ble_endpoint_among_the_ones_this_device_configured() {
    // 一条都不在场：两条 HID 没枚举到，本机的 BLE 设备里也没有这个地址。
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    let error = readout::read(&mouse_with_all_three_endpoints(), &endpoints, None, NOW)
        .unwrap_err()
        .to_string();

    assert!(error.contains("不在场"), "一条都没枚举到时的说法：{error}");
    assert!(
        error.contains("Wired") && error.contains("Dongle24G") && error.contains("Ble"),
        "配了三条就该说三条：{error}"
    );
}

/// 失联的 Device **仍然拿得出上次已知值**，重启不等于失忆。
///
/// 票面第 2 条的前一半（后一半"标注成陈旧"在下面那条）。这是这张票的由来：鼠标收进抽屉、
/// 键盘关了机，这一次取数一条 Endpoint 都读不到，而 `status` 刚启动内存里什么都没有——那个
/// "上次是多少电"只可能来自状态文件。
#[test]
fn a_lost_device_still_shows_its_last_known_value() {
    let device = mouse_with_both_endpoints();
    // 两条 Endpoint 都在场，回包脚本都空着——相当于两条都超时。
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![]),
        ],
    );
    let general = default_general();

    // 手上一份历史值都没有时，仍然是那一次取数的原因（这里两条都超时，那句话是"没有可信
    // 的读数"）——那句诊断没有被历史值挤掉。
    assert!(
        readout::read_or_last_known(
            &device,
            &endpoints,
            None,
            &mut LastKnown::default(),
            &general,
            NOW
        )
        .is_err(),
        "没有历史值就该照旧报失联"
    );

    // 半小时前读到过一次。
    let previous = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 44,
            charging: Some(true),
            voltage_mv: Some(3_950),
        },
        NOW.minus_secs(1_800),
    );
    let mut last_known = LastKnown::default();
    last_known.record(&device.id, &previous, NOW.minus_secs(1_800));

    let readout =
        readout::read_or_last_known(&device, &endpoints, None, &mut last_known, &general, NOW)
            .expect("退得到上次已知值");

    assert_eq!(readout.reading, previous, "拿出来的就是上次那一份");
    assert_eq!(
        readout.provenance,
        Provenance::LastKnown,
        "并且说得清它是历史值，不是这一次取数读到的"
    );
}

/// 退到了上次已知值，这一次取数自己为什么没读到也一并交出来。交给这一轮的那一份里没有它的位置——
/// 那一份是上次已知值——而托盘要把它写进日志：图标上那只是一个灰的旧数，这却是最常见的那一种失败。
#[test]
fn falling_back_to_the_last_known_value_still_hands_over_why_this_fetch_did_not_read() {
    let device = mouse_with_both_endpoints();
    // 两条 Endpoint 都在场，回包脚本都空着——相当于两条都超时。
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![]),
        ],
    );
    let previous = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 44,
            charging: Some(false),
            voltage_mv: Some(3_950),
        },
        NOW.minus_secs(1_800),
    );
    let mut last_known = LastKnown::default();
    last_known.record(&device.id, &previous, NOW.minus_secs(1_800));

    let outcome = readout::read_or_last_known_with_reason(
        &device,
        &endpoints,
        None,
        &mut last_known,
        &default_general(),
        NOW,
    );

    let row = outcome.row.expect("退得到上次已知值");
    assert_eq!(row.provenance, Provenance::LastKnown);
    let reason = outcome
        .fell_back_because
        .expect("没读到的原因跟着交出来")
        .to_string();
    assert!(
        reason.starts_with("没有可信的读数 —— Wired: "),
        "交出来的是这一次取数自己的那句话：{reason}"
    );
}

/// 这一次取数读得到的时候，**状态文件里那一份不许插队**。
///
/// 历史值只是失联时的退路。让它参与竞争，一份存了半天的读数就可能盖掉当场问出来的那个数
/// ——而当场问出来的那个才是现状，这条链路上其余每一处（Endpoint 优先级、`Ble` 排最后）
/// 守的都是同一件事。
#[test]
fn a_reading_taken_this_run_wins_over_the_one_in_the_state_file() {
    let device = mouse_with_both_endpoints();
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()])],
    );

    let mut last_known = LastKnown::default();
    last_known.record(
        &device.id,
        &EndpointReading::from_hid(
            EndpointKind::Wired,
            Reading {
                reported_level: 44,
                charging: Some(true),
                voltage_mv: Some(3_950),
            },
            NOW.minus_secs(1_800),
        ),
        NOW.minus_secs(1_800),
    );

    let readout = readout::read_or_last_known(
        &device,
        &endpoints,
        None,
        &mut last_known,
        &default_general(),
        NOW,
    )
    .expect("Wired 这一次读得到");

    assert_eq!(
        readout.reading.reading.reported_level, 100,
        "印的是这一次取数读到的那个数，不是文件里那个 44"
    );
    assert_eq!(readout.provenance, Provenance::JustRead);
}

/// 每一次成功的读数**进状态文件**。
///
/// 票面第 1 条。写盘这件事与取数住在一处（`read_or_last_known`）而不是留在 `run` 里：留在
/// `run` 里它就只能靠读代码来确认，而票 06 的 review 正是在 `run` 里抓到过一个用例一条都
/// 碰不到的真 bug。
#[test]
fn a_successful_reading_is_written_to_the_state_file() {
    let device = mouse_with_both_endpoints();
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_CHARGING.to_vec()])],
    );
    let general = default_general();
    let mut last_known = LastKnown::default();

    let readout =
        readout::read_or_last_known(&device, &endpoints, None, &mut last_known, &general, NOW)
            .expect("Wired 这一次读得到");

    assert_eq!(
        last_known.reading_for(&device.id, &general, NOW),
        Some(readout.reading),
        "这一次取数读到的那一份该记下来了"
    );
}

/// 退到上次已知值的那一次取数**一个字都不写**——历史值不会因为被读了一次就续命。
///
/// 把拿出来的历史值再写一遍是最自然的写法（"每一次取数都把当前状态存下来"），而它会把写盘时刻
/// 刷成现在，于是那条记录永远到不了 `very_stale_after`：一只收进抽屉半年的鼠标每天被看一眼
/// 就每天年轻一天。"这个历史值必须会过期"这句话在那种实现下永久落空,而且没有任何症状。
#[test]
fn falling_back_to_the_last_known_value_does_not_extend_its_life() {
    let device = mouse_with_both_endpoints();
    // 两条 Endpoint 都在场,回包脚本都空着——相当于两条都超时。
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![]),
            (EndpointKind::Dongle24G, vec![]),
        ],
    );
    let general = default_general();

    // 差一点就到期的一条:缺省 very_stale_after 是一天。
    let almost_expired = NOW.minus_secs(86_000);
    let mut last_known = LastKnown::default();
    last_known.record(
        &device.id,
        &EndpointReading::from_hid(
            EndpointKind::Wired,
            Reading {
                reported_level: 44,
                charging: Some(true),
                voltage_mv: Some(3_950),
            },
            almost_expired,
        ),
        almost_expired,
    );
    let before = last_known.to_toml().expect("序列化得动");

    readout::read_or_last_known(&device, &endpoints, None, &mut last_known, &general, NOW)
        .expect("退得到上次已知值");

    assert_eq!(
        last_known.to_toml().expect("序列化得动"),
        before,
        "失联那一次取数不该动状态文件里的任何一个字"
    );
}

/// 合理性校验不过时，那一行说"读取异常"，**一个百分比都不给**。
///
/// 帧完整、cmd 回显也对，只是里头的电压物理上不可能——固件漂移就是这个形状。这时
/// 拿那串字节按老下标算出来的任何数字都是编的，所以 `status` 交出去的是原因而不是数。
///
/// **守的就是最终印在屏幕上那一行**：`NoReading` 的 `Display` 印出去的正是 Device 名字
/// 之后的全部内容，`run()` 那一头只剩一句 `println!`。所以"这一行不再声称都没读到"在这里
/// 断言得到——parking lot Q13 记的"整行在 `run()` 里拼、而 `run()` 要真去枚举本机 HID，
/// 测不到"那个障碍，已经被后来两张票分两次提掉了。
#[test]
fn an_implausible_frame_reads_as_an_anomaly_rather_than_a_number() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(
            EndpointKind::Wired,
            vec![mouse_frame_with(100, 2000).to_vec()],
        )],
    );

    let line = readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW)
        .unwrap_err()
        .to_string();

    assert!(line.contains("读取异常"), "要说成读取异常：{line}");
    assert!(!line.contains('%'), "一个百分比都不该出现：{line}");
    assert!(
        !line.contains("都没读到"),
        "设备答了话，这一行不许声称都没读到：{line}"
    );
}

/// 设备答了坏帧的那一次取数落在**取数失败**那一支——那个变体的名字因此不能宣称"失联"。
///
/// `CONTEXT.md`「读取异常」的 _Avoid_ 明写着不许拿失联说它：设备答了话，只是那一帧不可
/// 采信。而它与超时、一条都不在场、一条都没配落在同一个变体里，因为对这一层它们是同一个
/// 下场——手上没有可印的数。所以那个变体叫它们的上一层（「取数失败」），与暂停切成一条
/// 干净的二分：一边是交不出可以印的数、而且不是因为我们没去问，一边是我们主动没去问。
///
/// 断言的是**那个变体本身**，不是印出去的那句话（那句话上面那条断言过了）：下游要按这一维
/// 分支，而一句话只能被再解析一次。
#[test]
fn answering_with_a_bad_frame_is_a_failed_readout() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(
            EndpointKind::Wired,
            vec![mouse_frame_with(100, 2000).to_vec()],
        )],
    );

    let no_reading =
        readout::read(&mouse_with_both_endpoints(), &endpoints, None, NOW).unwrap_err();

    assert!(
        matches!(no_reading, NoReading::Failed(_)),
        "读取异常是取数失败的一种来路：{no_reading:?}"
    );
}

// ---------------------------------------------------------------
// 厂商上位机暂停（票 07）
//
// 撞见上位机这件事本身在 `tests/vendor_hub.rs` 里断言完了，这里断言的是**取数那一步
// 怎么让开**：让开的那几条一个通道都不打开、`Ble` 照常、那一行说的是"暂停"而不是
// "读不到"。
// ---------------------------------------------------------------

/// 让开的那几条**一个通道都不打开**，而且那一行说的是暂停，不是失联。
///
/// 两件事一条用例：它们是同一个决定的两面。不打开通道是这张票的机械要求（一块共享
/// 缓冲区，两个程序同时发命令会互相覆盖应答）；说成"暂停"是它的用户可见面——说成失联，
/// 用户会去找一个不存在的硬件故障，而他真该做的事是关掉那个上位机。
#[test]
fn yields_the_two_hid_endpoints_without_opening_them_at_all() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );
    let hub = vendor_hub_running();

    let line = readout::read(&mouse_with_both_endpoints(), &endpoints, Some(&hub), NOW)
        .unwrap_err()
        .to_string();

    assert!(line.contains("暂停"), "那一行要说它暂停了：{line}");
    assert!(
        line.contains("VGN VHUB.exe"),
        "要点名是谁占着通路，那是用户唯一能据以行动的东西：{line}"
    );
    assert!(!line.contains("读不到"), "暂停不许说成失联：{line}");
    assert!(
        endpoints.opens().is_empty(),
        "暂停期间一个通道都不该打开，打开了就是{:?}",
        endpoints.opens()
    );
}

/// `Ble` 在暂停期间照常更新（票面第 3 条）。
///
/// 它不参与那个竞争：读的是 Windows 攒的属性缓存，压根不往设备发字节。所以让开两条 HID
/// **不该顺带把它也停掉**——那一刻它恰好是唯一还答得出话的一级。
#[test]
fn keeps_reading_the_ble_cache_while_the_two_hid_endpoints_are_paused() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    )
    .with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));
    let hub = vendor_hub_running();

    let reading = readout::read(
        &mouse_with_all_three_endpoints(),
        &endpoints,
        Some(&hub),
        NOW,
    )
    .expect("Ble 那一级照常答得出话");

    assert_eq!(reading.endpoint, EndpointKind::Ble);
    assert_eq!(reading.reading.reported_level, 62);
    assert!(
        endpoints.opens().is_empty(),
        "两条 HID 仍然一个通道都不该打开：{:?}",
        endpoints.opens()
    );
}

/// 暂停期间，**没让开的那几条自己的失败原因不能被吞掉**。
///
/// 让开两条 HID，而 `Ble` 在场却答不出电量：关掉上位机，那台设备的蓝牙那一头还是老样子。所以
/// 这一行以 `Ble` 自己的原因收尾，不接空列表那一支的"关掉它之后这几条才试得到"——那半句只在
/// 什么都没试过时才是全部实情。parking lot Q40 允许沉默的理由是"手上有历史值就意味着这套配置
/// 曾经读通过"，而这一支连历史值都没有，那句理由不成立。
#[test]
fn keeps_the_reason_of_the_endpoints_it_did_try_while_paused() {
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    )
    // 在场，但这台设备根本没有电量属性——它被试过了，而且失败了。
    .with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        None,
        Some(300),
    ));
    let hub = vendor_hub_running();

    let line = readout::read(
        &mouse_with_all_three_endpoints(),
        &endpoints,
        Some(&hub),
        NOW,
    )
    .unwrap_err()
    .to_string();

    assert!(line.contains("暂停中"), "它仍然是暂停，不是失联：{line}");
    assert!(
        line.contains("没有电量属性"),
        "Ble 自己那条原因不该被吞掉：{line}"
    );
    assert!(
        !line.contains("关掉它之后这几条才试得到"),
        "试过的那几条有原因要交代，不接空列表那一支的那半句：{line}"
    );
}

/// 接收器插着、设备本体关了机：暂停那一行**不许诺关掉上位机就能读到**。
///
/// 接收器有自己一组 VID/PID（`docs/adr/0001`），所以设备关着机 Dongle24G 照样枚举成在场、
/// 照样被让开；没有别的 Endpoint 顶上，原因列表是空的。而列表为空只说明这一次取数什么都
/// 没试过，不说明试了就会答话——关掉上位机之后，这条通路照旧超时。所以那一行说的是**我们**
/// 关掉它之后会做什么（去试让开的那几条），不说**设备**会怎样。
///
/// 整句断言：点名那个上位机、点名让开的那一条、再接那半句条件句。前两样是用户唯一能据以
/// 行动的东西，措辞变准不许拿它们去换。
#[test]
fn does_not_promise_a_reading_once_the_vendor_hub_is_closed() {
    // 只有接收器在场：设备关着机，`Wired` 枚举不出来。接收器那一头的脚本是空的——真去问
    // 也只会超时，而暂停期间它压根不会被问到。
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, [(EndpointKind::Dongle24G, vec![])]);
    let hub = vendor_hub_running();

    let line = readout::read(&mouse_with_both_endpoints(), &endpoints, Some(&hub), NOW)
        .unwrap_err()
        .to_string();

    assert_eq!(
        line,
        "暂停中 —— VGN VHUB.exe 正在运行，Dongle24G 让开（同时发命令会互相覆盖对方的应答），关掉它之后这几条才试得到"
    );
}
