//! `cli::status` 的外部行为，也就是**那一行印成什么样**：电量那一段（含两个百分比里印
//! 哪一个）、来源那一段、陈旧与暂停两个标注，以及那几行的合成——Primary 标注落在哪一行、
//! 未登记的 BLE 设备列不列。
//!
//! 全程经假枚举 + 假 Transport，不接任何硬件。这里多数用例要一份真的读数才排得出版，
//! 那一份走的是真的取数那一步（`juicebar::readout`）而不是手搓一个：排版与取数对
//! `EndpointReading` 的理解只有一份，两处各自可能漂开的知识里只有一份是被断言过的。
//!
//! 取数本身（先试哪一条、怎么降级、什么时候才算失联、取不到时退到上次已知值）在
//! `tests/readout.rs`。

mod common;

use common::fixtures::{
    KEYBOARD_REPORT_ID, KEYBOARD_RESTING_FULL, MOUSE_CHARGING, MOUSE_REPORT_ID, MOUSE_RESTING_FULL,
    mouse_frame_with,
};
use common::{
    FakeEndpoints, NOW, default_general, keyboard_with_dongle_endpoint,
    mouse_with_all_three_endpoints, mouse_with_both_endpoints, scanned_ble, vendor_hub_running,
};
use juicebar::cli::status;
use juicebar::cli::status::DeviceRow;
use juicebar::clock::Timestamp;
use juicebar::config::Config;
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::primary::{CandidateReading, PrimaryRule};
use juicebar::readout;
use juicebar::sources::Reading;
use juicebar::sources::level::{Level, LevelSource};
use juicebar::staleness::{Freshness, Staleness};
use juicebar::state::{LastKnown, Provenance};

/// 一份读数在 [`NOW`] 这一刻印成的那一行。
///
/// 把"判一次 + 排一次版"收成一句：这个文件关心的是**那一行印成什么样**，而阈值本身怎么
/// 从轮询间隔推导出来在 `tests/staleness.rs`。
///
/// `level_source` 取缺省的 `Auto`：这里的用例关心 Endpoint 合成、来源标注和陈旧那几档，
/// 不是"两个百分比里印哪一个"——后者在 `tests/level.rs` 里单独断言。要按 Device 指定
/// 来源的用例走下面那个 [`render`]。
fn line_of(reading: &EndpointReading) -> String {
    line_of_taken(reading, Provenance::JustRead)
}

/// 同一份读数，但它是设备失联之后**从状态文件里拿出来的**上次已知值。
fn last_known_line_of(reading: &EndpointReading) -> String {
    line_of_taken(reading, Provenance::LastKnown)
}

fn line_of_taken(reading: &EndpointReading, provenance: Provenance) -> String {
    status::render(
        reading,
        LevelSource::Auto,
        &Staleness::assess(reading, &default_general(), NOW),
        provenance,
        None,
    )
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
    let reading = readout::read(&device, &fresh, None, NOW).unwrap();
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
    let reading = readout::read(&device, &stale, None, NOW).unwrap();
    let line = line_of(&reading);
    assert!(line.contains("62%"), "这一档数字还是照印：{line}");
    assert!(line.contains("陈旧"), "但要明确标注出来：{line}");
    assert!(line.contains("2 小时前"), "「多久前」也照印：{line}");
}

/// 失联时拿出来的上次已知值**一律标注成陈旧**，哪怕它是十秒前取的。
///
/// 票面第 2 条。两条断言的是同一份读数：新鲜阈值说它新鲜（缺省 Wired 阈值 90 秒），
/// 而它此刻是历史值——设备不在了，这个数就不是现状，无论它多新。不标注就正好在最危险的
/// 方向上撒谎：一个十秒前的 95% 和一个此刻读到的 95% 长得一模一样，而前者说的是一只已经
/// 收进抽屉的鼠标。
#[test]
fn a_last_known_reading_is_marked_stale_even_when_it_was_taken_seconds_ago() {
    let reading = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 95,
            charging: Some(true),
            voltage_mv: Some(4235),
        },
        NOW.minus_secs(10),
    );

    let live = line_of(&reading);
    assert!(
        !live.contains("陈旧"),
        "当场读到的十秒前读数是新鲜的：{live}"
    );

    let history = last_known_line_of(&reading);
    assert!(history.contains("95%"), "历史值照常显示百分比：{history}");
    assert!(history.contains("已陈旧"), "但一律标注成陈旧：{history}");
    assert!(
        history.contains("上次已知值"),
        "并且说清它是上次已知值，不是这一趟读到的：{history}"
    );
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

    let reading = readout::read(&device, &endpoints, None, NOW).unwrap();
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
    let reading = readout::read(&device, &wired, None, NOW).unwrap();
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

    let reading = readout::read(&device, &endpoints, None, NOW).unwrap();
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
    let reading = readout::read(&device, &at_threshold, None, NOW).unwrap();
    let line = line_of(&reading);
    assert!(!line.contains("陈旧"), "整一小时还不算超过：{line}");

    // 多一秒就算。
    let past_threshold = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(3_601),
    ));
    let reading = readout::read(&device, &past_threshold, None, NOW).unwrap();
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
    let reading = readout::read(&device, &endpoints, None, NOW).unwrap();

    // 取数在 NOW，判定在一千秒之后：此刻这份读数已经四千秒了，超过缺省的 stale_after。
    // 用 as_unix_secs / from_unix_secs 往前挪，而不是给 Timestamp 加一个只有用例用得到的
    // `plus_secs`——生产接口不为用例的方便而长。
    let later = Timestamp::from_unix_secs(NOW.as_unix_secs() + 1_000);
    let line = status::render(
        &reading,
        LevelSource::Auto,
        &Staleness::assess(&reading, &default_general(), later),
        Provenance::JustRead,
        None,
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

/// 取一次数并印成一行。
fn render(device: &juicebar::config::Device, endpoints: &FakeEndpoints) -> String {
    let reading = readout::read(device, endpoints, None, NOW).unwrap();
    status::render(
        &reading,
        device.level_source,
        &Staleness::assess(&reading, &default_general(), NOW),
        Provenance::JustRead,
        None,
    )
}

// ---------------------------------------------------------------
// 厂商上位机暂停（票 07）
//
// 让开了哪几条、以及那一趟到底算不算暂停，在 `tests/readout.rs` 里断言完了；这里断言的
// 是**那一行怎么说这件事**：末尾那句"已暂停"点不点名那个进程、什么时候一个字都不该多。
// ---------------------------------------------------------------

/// 一份 [`readout::RowReading`] 印成的那一行。
///
/// 暂停这几条用例都要它，而它比 [`line_of`] 多带两维（来路与暂停），所以另起一个而不是给
/// 那个加参数——`line_of` 是上面那些用例在用的。
fn line_of_row(
    device: &juicebar::config::Device,
    row: &readout::RowReading,
    general: &juicebar::config::General,
) -> String {
    status::render(
        &row.reading,
        device.level_source,
        &Staleness::assess(&row.reading, general, NOW),
        row.provenance,
        row.paused_by.as_ref(),
    )
}

/// 暂停期间**保留最后读数而不是清空**（票面第 5 条），并且说得清那是什么。
///
/// `status` 是一次性命令，程序刚启动内存里什么都没有，所以那个"最后读数"只可能来自状态
/// 文件（票 08 的 `reading_for`）。而那一行末尾非说一句不可：这一维的沉默是有理由的
/// （parking lot Q40：手上有历史值就意味着这套配置曾经读通过，那句失联诊断已经不成立），
/// **而厂商上位机是那条规则的例外**——用户此刻该被告知的正是"上位机在跑"，因为那是他唯一
/// 能动手的地方。
#[test]
fn keeps_the_last_known_value_while_paused_and_says_which_it_is() {
    let device = mouse_with_both_endpoints();
    // 两条 Endpoint 都在场、脚本也都备着回包——它们没被读到是因为让开，不是因为读不到。
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [
            (EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()]),
            (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
        ],
    );
    let general = default_general();
    let hub = vendor_hub_running();

    // 半小时前读到过一次。
    let taken_at = NOW.minus_secs(1_800);
    let previous = EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level: 44,
            charging: Some(true),
            voltage_mv: Some(3_950),
        },
        taken_at,
    );
    let mut last_known = LastKnown::default();
    last_known.record(&device.id, &previous, taken_at);

    let readout = readout::read_or_last_known(
        &device,
        &endpoints,
        Some(&hub),
        &mut last_known,
        &general,
        NOW,
    )
    .expect("暂停期间该拿出最后读数，而不是什么都没有");

    assert_eq!(readout.reading, previous, "拿出来的就是上次那一份");
    assert_eq!(readout.provenance, Provenance::LastKnown);

    let line = line_of_row(&device, &readout, &general);
    // 3950 mV 在 `auto` 下取的是 Derived Level（63%），不是固件自报的 44——两个百分比里
    // 印哪一个在 `tests/level.rs` 里单独断言，这里要的只是"那个数还在，没被清空"。
    assert!(
        line.contains("63%（Derived Level）"),
        "那个数要留在那一行上：{line}"
    );
    assert!(line.contains("上次已知值"), "它仍然不是现状：{line}");
    assert!(
        line.contains("已暂停") && line.contains("VGN VHUB.exe"),
        "还要说清它为什么没被刷新：{line}"
    );
    assert!(
        endpoints.opens().is_empty(),
        "拿历史值顶上不代表可以去打开通道：{:?}",
        endpoints.opens()
    );
}

/// `Ble` 顶上的那一行**照样说一句"已暂停"**。
///
/// 那一刻没有任何东西读不到，所以不能靠"读不到"来交代——可一个可能是几个月前的缓存数字
/// 突然顶掉了当场问出来的那个数，用户得知道那是因为上位机在跑，而不是设备出了事。
#[test]
fn says_it_paused_even_when_the_ble_cache_answered() {
    let device = mouse_with_all_three_endpoints();
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()])],
    )
    .with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        Some(62),
        Some(300),
    ));
    let general = default_general();
    let hub = vendor_hub_running();

    let readout = readout::read_or_last_known(
        &device,
        &endpoints,
        Some(&hub),
        &mut LastKnown::default(),
        &general,
        NOW,
    )
    .expect("Ble 答得出话");

    let line = line_of_row(&device, &readout, &general);
    assert!(line.contains("来自 Ble"), "这个数来自缓存：{line}");
    assert!(
        line.contains("已暂停（VGN VHUB.exe 正在运行）"),
        "还要说清 Wired 为什么没顶上：{line}"
    );
}

/// 只配了 `Ble` 的设备**不因为上位机在跑就被标成暂停**。
///
/// 一副耳机压根没有让开的东西：对它说一句"已暂停"是一句与它无关的话，而那种噪音会把
/// 真正该看的那一行一起淹掉。判据因此是"这个 Device 真有在场的 Endpoint 被让开"，
/// 不是"本机有上位机在跑"。
#[test]
fn does_not_mark_a_ble_only_device_as_paused() {
    let headset = Config::parse(
        r#"
        [[device]]
        id = "headset"
        name = "某副耳机"
        driver = "vgn_mouse"

          [device.bluetooth]
          address = "f4ee2553b27e"
        "#,
    )
    .expect("用例里的配置应当解析得动")
    .devices
    .remove(0);
    let endpoints = FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
        "某副耳机",
        "f4ee2553b27e",
        Some(80),
        Some(60),
    ));
    let general = default_general();
    let hub = vendor_hub_running();

    let readout = readout::read_or_last_known(
        &headset,
        &endpoints,
        Some(&hub),
        &mut LastKnown::default(),
        &general,
        NOW,
    )
    .expect("Ble 答得出话");

    assert!(
        readout.paused_by.is_none(),
        "它一条 HID 都没有，没有任何东西被让开"
    );
    let line = line_of_row(&headset, &readout, &general);
    assert!(!line.contains("已暂停"), "不该多这一句：{line}");
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

    let lines = primary_lines(&PrimaryRule::Lowest, &rows);

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

    let lines = primary_lines(&PrimaryRule::Pinned("neon75".to_string()), &rows);

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

    let lines = primary_lines(&PrimaryRule::Lowest, &rows);

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
        primary_lines(&PrimaryRule::Lowest, &rows).len(),
        1,
        "只该有那一行 Device"
    );
}

/// 上次选出的那台从状态文件里来：这一轮全都不可信时，标注落在它头上。
///
/// **这条用例守的是接线，不是规则。**规则本身（`select` 遇到一轮全不可信时交 `HeldOver`）
/// 在 `tests/primary.rs` 里断言；这里断言的是 `primary_lines` 真的把那个 `previous` 递了
/// 进去——票 09 落地时它恒为 `None`，那份记忆的家（票 08 的状态文件）当时还没接上，
/// 于是"保持上次的选择"这条验收框对用户是空的（parking lot Q41）。
#[test]
fn holds_over_the_previous_choice_when_nothing_is_trustworthy_this_round() {
    let mouse = mouse_with_both_endpoints();
    let keyboard = keyboard_with_dongle_endpoint();
    // 两台都读不到 —— 一轮里没有一个新鲜且可信的读数。
    let rows = [
        row(&mouse, "读不到 —— 全部 Endpoint 都没读到", None),
        row(&keyboard, "读不到 —— 全部 Endpoint 都没读到", None),
    ];

    let (lines, chosen) = status::primary_lines(&PrimaryRule::Lowest, &rows, Some("neon75"));

    assert_eq!(chosen, Some("neon75"), "该交出上次那一台，好让它被记回去");
    assert!(
        lines[1].starts_with("VGN Neon75（Primary Device）"),
        "上次选的那台照旧标着，托盘不会因为一轮读不到就跳开：{:?}",
        lines[1]
    );
    assert!(
        !lines[0].contains("Primary Device"),
        "另一台不该被标上：{:?}",
        lines[0]
    );
}

/// 这一趟选出来的那个 id 要交出去 —— `run` 拿它记回状态文件。
///
/// 没有它，下一趟启动时 `previous` 又是 `None`，上面那条 `HeldOver` 永远走不到。
#[test]
fn reports_which_device_it_chose_so_the_caller_can_remember_it() {
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

    let (_, chosen) = status::primary_lines(&PrimaryRule::Lowest, &rows, None);

    assert_eq!(chosen, Some("neon75"), "电量最低的那台就是这一趟的选择");
}

/// 一轮什么都选不出、又没有上次可依时，**不交出任何 id**。
///
/// 这一条挡的是"随便记一个"：`remember_primary` 只有"设"没有"清"，所以一个不该被记的 id
/// 一旦写进去就再也退不掉，而下一趟它会伪装成"上次的选择"。
#[test]
fn chooses_nothing_when_there_is_no_candidate_and_no_previous() {
    let mouse = mouse_with_both_endpoints();
    let rows = [row(&mouse, "读不到 —— 全部 Endpoint 都没读到", None)];

    let (_, chosen) = status::primary_lines(&PrimaryRule::Lowest, &rows, None);

    assert_eq!(chosen, None);
}

/// 一个 Device 都没配的时候一句话都不补。
///
/// `run` 那时已经印过"一个 Device 都没有"，把原因说完了；再补一句"选不出 Primary Device"
/// 只是同一件事的第二遍。
#[test]
fn stays_silent_when_no_device_is_configured() {
    assert!(primary_lines(&PrimaryRule::Lowest, &[]).is_empty());
}

/// [`status::primary_lines`] 印出来的那几行。
///
/// 这个文件里的用例守的是**那几行印成什么样**；"这一趟选出了谁"那个返回值是给 `run` 记回
/// 状态文件用的（parking lot Q41），它自己在 `tests/primary.rs` 里由 `select` 直接断言。
fn primary_lines(rule: &PrimaryRule, rows: &[DeviceRow<'_>]) -> Vec<String> {
    status::primary_lines(rule, rows, None).0
}
