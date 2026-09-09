//! 状态文件的外部行为：一份读数写下去、重启之后读回来，以及那个历史值什么时候
//! 该被丢掉。
//!
//! 多数用例走的是纯文本那一段（`to_toml` / `parse`），不碰磁盘——"写盘再读回来"的
//! 全部意义在于**跨一次进程存活**，而那件事由一次序列化加一次反序列化就完整表达了。
//! 真去碰磁盘的只有"文件不在"和"文件坏了"那几条：它们要断言的恰恰是文件系统那一侧
//! 的意外。

mod common;

use std::path::PathBuf;

use common::{NOW, default_general, scanned_ble};
use juicebar::clock::Timestamp;
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::sources::Reading;
use juicebar::state::LastKnown;

/// 用例里那只插着线充电的鼠标：一条 Wired 读数，三项都答得上。
///
/// 住在这个文件而不是 `tests/common/`：只有状态文件这一组用例要它，而 `common` 放的是
/// 几个测试二进制都用得上的假实现。
fn wired_reading(reported_level: u8, taken_at: Timestamp) -> EndpointReading {
    EndpointReading::from_hid(
        EndpointKind::Wired,
        Reading {
            reported_level,
            charging: Some(true),
            voltage_mv: Some(4_235),
        },
        taken_at,
    )
}

/// 用例里那把键盘：一条 Dongle24G 读数，**充电态与电压都说不上来**（回包里根本没有这两项）。
///
/// 与上面那一份分开，是为了让"两个 `None` 一路存回来"不必靠一份人造的读数——它就是键盘那条
/// 通路的真实形状。
fn dongle_reading(reported_level: u8, taken_at: Timestamp) -> EndpointReading {
    EndpointReading::from_hid(
        EndpointKind::Dongle24G,
        Reading {
            reported_level,
            charging: None,
            voltage_mv: None,
        },
        taken_at,
    )
}

/// 同一份状态文件，隔着一次重启看过去。
///
/// 走文本而不是磁盘：跨重启存活这件事由一次序列化加一次反序列化就完整表达了，而磁盘那一段
/// 另有两条用例专门盯着（文件不在、文件坏了）。
fn after_a_restart(store: &LastKnown) -> LastKnown {
    LastKnown::parse(&store.to_toml().expect("自己写的该序列化得动"))
}

/// 一份成功的读数写进状态文件，重启之后读回来还是同一份。
///
/// 这是这张票的地基：`status` 是一次性命令，进程一退内存里什么都不剩，所以"上次是多少电"
/// 只可能来自磁盘。
#[test]
fn a_reading_survives_a_restart() {
    let reading = wired_reading(95, NOW);

    let mut written = LastKnown::default();
    written.record("dragonfly3", &reading, NOW);
    let reread = after_a_restart(&written);

    assert_eq!(
        reread.reading_for("dragonfly3", &default_general(), NOW),
        Some(reading),
        "读回来的该是原样那一份"
    );
}

/// 超过 `very_stale_after` 的持久化读数**被丢弃**，不是被标注。
///
/// 票面第 3 条。丢在读回来这一步而不是交给陈旧判定：那一侧对两条 HID 根本没有第二档
/// （parking lot Q29），一份存了三天的有线读数在那里只会得到一句"已陈旧"，而这张票要的是
/// 它压根不出现——历史值必须**会过期**，否则它就是一个永远躺在那儿的假现状。
#[test]
fn discards_a_persisted_reading_older_than_very_stale_after() {
    let general = default_general();
    // 缺省 very_stale_after = 86400，也就是一天。
    let written = |taken_at| {
        let mut store = LastKnown::default();
        store.record("dragonfly3", &wired_reading(95, taken_at), taken_at);
        after_a_restart(&store)
    };

    // 整整一天前的读数还在：边界与陈旧判定同一侧，"超过"才算（票 06 那条用例的原话）。
    let at_threshold = written(NOW.minus_secs(86_400));
    assert!(
        at_threshold
            .reading_for("dragonfly3", &general, NOW)
            .is_some(),
        "整一天还不算超过"
    );

    // 多一秒就没有了。
    let past_threshold = written(NOW.minus_secs(86_401));
    assert_eq!(
        past_threshold.reading_for("dragonfly3", &general, NOW),
        None,
        "过了期限的历史值该压根不出现"
    );
}

/// **说不出取得时刻的历史值也会过期**，靠的是它在文件里躺了多久。
///
/// 这一条是本票的标题句（"这个历史值必须会过期"）对最难的那一种记录的兑现。`taken_at` 缺席
/// 是那台 BLE 设备根本没有更新时间戳，于是它算不出"这个数是多久前的"——只按取得时刻判，
/// 这种记录**永远**不过期，一个六个月前的 50% 可以一直挂在那儿。所以记录里另存一个写盘时刻：
/// 那个数程序自己一定说得出来，它答的是另一个问题（"这条记录躺了多久"），而两个问题里任何
/// 一个超过 `very_stale_after` 都该丢。
///
/// 写盘时刻**不印给用户看**：用户要知道的是那个百分比是什么时候的，而这一种恰恰说不出来
/// ——那一行照旧印"无时间戳"。
#[test]
fn discards_a_dateless_reading_once_it_has_sat_in_the_file_too_long() {
    let general = default_general();
    let undated = EndpointReading::from_ble_cache(
        &scanned_ble("Dragonfly 3 Master+", "e452430072a9", Some(50), None),
        NOW,
    )
    .expect("这台设备有电量属性");
    assert_eq!(undated.taken_at, None, "这台设备没有更新时间戳");

    // 一天前写下的：还在，而且照旧说不出取得时刻。
    let mut a_day_ago = LastKnown::default();
    a_day_ago.record("dragonfly3", &undated, NOW.minus_secs(86_400));
    let survivor = after_a_restart(&a_day_ago)
        .reading_for("dragonfly3", &general, NOW)
        .expect("整一天还不算超过");
    assert_eq!(survivor.reading.reported_level, 50);
    assert_eq!(survivor.taken_at, None, "读回来照旧说不出这个数是哪天的");

    // 多一秒就没有了——它不再是"一个没有时间戳的读数"，而是一条该扫掉的旧记录。
    let mut too_long_ago = LastKnown::default();
    too_long_ago.record("dragonfly3", &undated, NOW.minus_secs(86_401));
    assert_eq!(
        after_a_restart(&too_long_ago).reading_for("dragonfly3", &general, NOW),
        None,
        "躺过了期限的记录该压根不出现"
    );
}

/// 持久化的 Ble 读数读回来时，"多久以前"说的是**现在**那份缓存有多旧，不是写盘那一刻
/// Windows 报的数。
///
/// `cache_age_secs` 因此不写盘：它是一个相对量（"5 分钟之前更新过"），而写盘那一刻的
/// "之前"三小时后就不成立了。照原样存回来，一份放了三天的缓存会一直自称五分钟前的
/// ——这正是票 06 拒绝"每次推算取得时刻"时点名的那个错（parking lot Q26），只是方向
/// 相反。绝对的取得时刻存下来，年龄由它和当下重新减出来，两侧就不会漂。
#[test]
fn a_persisted_ble_reading_says_how_old_the_cached_value_is_now() {
    // 写盘那一刻：Windows 说这份缓存五分钟前更新过。
    let written_at = NOW.minus_secs(3 * 3_600);
    let cached = EndpointReading::from_ble_cache(
        &scanned_ble("Dragonfly 3 Master+", "e452430072a9", Some(62), Some(300)),
        written_at,
    )
    .expect("这台设备有电量属性");
    assert_eq!(
        cached.cache_age_secs,
        Some(300),
        "写盘前是 Windows 报的那个数"
    );

    let mut store = LastKnown::default();
    store.record("dragonfly3", &cached, written_at);
    let reread = after_a_restart(&store);
    let last = reread
        .reading_for("dragonfly3", &default_general(), NOW)
        .expect("三小时前的记录还没过期");

    assert_eq!(last.taken_at, cached.taken_at, "取得时刻原样存回来");
    assert_eq!(
        last.cache_age_secs,
        Some(3 * 3_600 + 300),
        "缓存年龄按当下重算：那个 62% 现在是三小时五分钟前的事"
    );
}

/// 状态文件不在，程序照常跑，只是没有已知值。
///
/// 票面第 5 条。这是缺省状态——第一次运行、刚装好、用户手动删掉了它，都长这样。
#[test]
fn a_missing_state_file_reads_as_no_last_known_value() {
    let missing = scratch_dir("missing").join("state.toml");
    assert!(!missing.exists(), "这条用例要的就是它不在");

    let store = LastKnown::load(&missing);

    assert_eq!(
        store.reading_for("dragonfly3", &default_general(), NOW),
        None
    );
}

/// 状态文件坏了也一样：不影响启动，按"没有已知值"处理。
///
/// 票面第 5 条的另一半。四种坏法各写一条，因为它们坏在不同的层：空文件根本没有表，
/// 语法坏了连 TOML 都不成立，字段类型不对是 TOML 成立而 schema 不成立，认不出的
/// Endpoint 块名是 schema 也成立而值没有意义。
///
/// **这个文件坏了没有需要用户处置的错误状态**：这一趟末尾它会被整份重写，损坏自愈。
/// 所以走的不是"报一个错让人看见"，而是"当它不存在"——它是缓存，不是用户的数据。
#[test]
fn a_corrupt_state_file_reads_as_no_last_known_value() {
    let dir = scratch_dir("corrupt");
    let general = default_general();

    for (case, text) in [
        ("empty", ""),
        (
            "broken-syntax",
            "[last_known.dragonfly3
reported_level = ",
        ),
        (
            "wrong-type",
            "[last_known.dragonfly3]
endpoint = \"wired\"
reported_level = \"满的\"
",
        ),
        (
            "unknown-endpoint",
            "[last_known.dragonfly3]
endpoint = \"carrier_pigeon\"
reported_level = 95
",
        ),
    ] {
        let path = dir.join(format!("{case}.toml"));
        std::fs::write(&path, text).expect("写得进临时目录");

        let store = LastKnown::load(&path);

        assert_eq!(
            store.reading_for("dragonfly3", &general, NOW),
            None,
            "{case} 该当成没有已知值"
        );
    }
}

/// **缺字段不算损坏**：那正是"这条通路说不上来"落盘的样子。
///
/// `taken_at` 缺席是那台 BLE 设备没有更新时间戳（`bluetooth.rs` 的原话），`charging`
/// 与 `voltage_mv` 缺席是那条协议答不上这两项（键盘的回包里根本没有）。把缺字段当成
/// 损坏，等于让这几种设备一份历史值都留不下来——而它们恰恰是最需要历史值的那几种。
#[test]
fn a_record_without_the_optional_fields_is_not_corrupt() {
    let store = LastKnown::parse(&format!(
        "[last_known.neon75]
endpoint = \"wireless_24g\"
reported_level = 62
stored_at = {}
",
        NOW.as_unix_secs()
    ));

    let last = store
        .reading_for("neon75", &default_general(), NOW)
        .expect("缺的都是可以缺的字段");

    assert_eq!(last.reading.reported_level, 62);
    assert_eq!(last.reading.charging, None, "缺席不是「没在充电」");
    assert_eq!(last.reading.voltage_mv, None, "缺席不是「0 mV」");
    assert_eq!(last.taken_at, None, "缺席不是「刚取的」");
}

/// 一条**连写盘时刻都没有**的记录被丢掉,不是当成刚写下的。
///
/// 与上面那条正相反,而两者的分界正是这个模块的立场:设备说不上来的事(充电态、电压、
/// 取得时刻)照实存成缺席;而写盘时刻是**程序自己的**时刻,它一定说得出来,所以一条没有它的
/// 记录只可能来自手改或者一个更老的版本。那种记录说不清自己躺了多久,于是永远过不了期
/// ——宁可丢掉,也不要拿一份不知年月的读数去当上次已知值。
#[test]
fn a_record_that_cannot_say_when_it_was_written_is_dropped() {
    let store = LastKnown::parse(
        "[last_known.neon75]
endpoint = \"wireless_24g\"
reported_level = 62
",
    );

    assert_eq!(
        store.reading_for("neon75", &default_general(), NOW),
        None,
        "说不清什么时候写下的记录不该当上次已知值"
    );
}

/// 写到磁盘上的读数，下一趟从磁盘上读回来。
///
/// 上面那几条用例走的都是纯文本那一段。这一条把文件系统也走一遍：这张票的承诺是"重启
/// juicebar 不等于失忆"，而重启这件事跨的正是文件系统，不是一个字符串。
#[test]
fn a_reading_written_to_disk_comes_back_on_the_next_run() {
    let path = scratch_dir("round-trip").join("state.toml");
    let reading = dongle_reading(62, NOW.minus_secs(600));

    let mut writing = LastKnown::default();
    writing.record("neon75", &reading, NOW.minus_secs(600));
    writing.save(&path).expect("写得进临时目录");

    let next_run = LastKnown::load(&path);

    assert_eq!(
        next_run.reading_for("neon75", &default_general(), NOW),
        Some(reading)
    );
}

/// 只有一台设备读得到的那一趟，**别的 Device 的记录还在**。
///
/// 键鼠不会一起失联：鼠标插着线、键盘拨到了有线档没插线，是常态。这一趟把鼠标的记录更新掉、
/// 顺手把键盘那条抹掉，症状是键盘那一行在它最需要历史值的时候变成"读不到"——而它的历史值
/// 一分钟前还在文件里。整份重写因此必须建立在**读进来的那一份**上，不是一张空表。
#[test]
fn records_for_the_other_devices_survive_a_run_where_only_one_was_readable() {
    let path = scratch_dir("one-of-two").join("state.toml");
    let general = default_general();

    // 上一趟：两台都读到了。
    let mut previous = LastKnown::default();
    let earlier = NOW.minus_secs(1_800);
    previous.record("neon75", &dongle_reading(62, earlier), earlier);
    previous.record("dragonfly3", &wired_reading(40, earlier), earlier);
    previous.save(&path).expect("写得进临时目录");

    // 这一趟：只有鼠标读到了。
    let mut this_run = LastKnown::load(&path);
    this_run.record("dragonfly3", &wired_reading(44, NOW), NOW);
    this_run.save(&path).expect("写得进临时目录");

    let next_run = LastKnown::load(&path);
    assert_eq!(
        next_run
            .reading_for("dragonfly3", &general, NOW)
            .map(|last| last.reading.reported_level),
        Some(44),
        "读到的那台更新成新值"
    );
    assert_eq!(
        next_run
            .reading_for("neon75", &general, NOW)
            .map(|last| last.reading.reported_level),
        Some(62),
        "没读到的那台，历史值还在"
    );
}

/// 改了 `id` 的 Device **丢掉自己的历史值，而不是继承别人的**。
///
/// `id` 是用户可改的自由文本，改一个字旧记录就成了孤儿。丢一次历史值可以接受（下一次成功
/// 的读数就补回来了）；不能接受的是把 A 的历史值配给 B——那样鼠标那一行会印出键盘的电量，
/// 而它看上去和一个正常读数没有任何区别。所以键是 `id` 本身，不做任何模糊匹配。
#[test]
fn a_renamed_device_id_loses_its_history_instead_of_inheriting_another_devices() {
    let mut store = LastKnown::default();
    store.record("dragonfly3", &wired_reading(95, NOW), NOW);
    let reread = after_a_restart(&store);

    assert_eq!(
        reread.reading_for("dragonfly3-new", &default_general(), NOW),
        None,
        "改了 id 就是没有历史值，不是拿别人的顶上"
    );
}

/// 状态文件里**只有运行时缓存，没有一项用户可见的配置选择**。
///
/// 票面第 4 条，也是 `docs/adr/0003` 划的那条界：Primary 的手动选择回写 `config.toml` 本身
/// （单一事实来源——用户打开配置就看得见当前生效的选择），这一份只放"上次已知读数"这类
/// 纯缓存。把键钉死是让这条界机械可查：往这里加一个 `primary` 或者 `level_source`
/// 会当场红一条用例，而不是等某天有人发现两份文件在争同一件事。
#[test]
fn the_state_file_holds_only_runtime_cache_not_user_visible_configuration() {
    let mut store = LastKnown::default();
    store.record("dragonfly3", &wired_reading(95, NOW), NOW);

    let text = store.to_toml().expect("自己写的该序列化得动");
    let document: toml::Value = toml::from_str(&text).expect("自己写的该解析得动");
    let record = document["last_known"]["dragonfly3"]
        .as_table()
        .expect("一个 Device 一张表");
    let mut keys: Vec<&str> = record.keys().map(String::as_str).collect();
    keys.sort_unstable();

    assert_eq!(
        keys,
        [
            "charging",
            "endpoint",
            "reported_level",
            "stored_at",
            "taken_at",
            "voltage_mv"
        ],
        "一条记录就是一份 Reading 加一个写盘时刻，多一项都不该有"
    );
}

/// 状态文件还记着**上次选出来的那台 Primary Device**，一个 id，跨重启活下来。
///
/// 这一格不在票面上，是编排者派的，依据是票 09 的 Q41：那张票的第 4 条验收框要"全都不可信时
/// 保持上次的选择"，而 `status` 是一次性命令、手上没有"上次"。它的家是这份文件而不是
/// `config.toml`——`primary` 那一项只有一个格子，`"lowest"`（一条规则）和一个 Device id
/// （一个具体选择）共用它，把自动选出来的 id 写回去，那条规则就**不见了**，工具从此永久钉在
/// 那一台上，而用户从没要求过。
///
/// 它与上次已知的读数是同一类东西：都是从读数推出来的运行时缓存，不是用户做的选择。
#[test]
fn the_last_selected_primary_device_survives_a_restart() {
    let mut store = LastKnown::default();
    assert_eq!(
        store.last_primary(),
        None,
        "第一趟没有「上次」，选择规则自己去处理这一种"
    );

    store.remember_primary("dragonfly3");

    assert_eq!(
        after_a_restart(&store).last_primary(),
        Some("dragonfly3"),
        "下一趟该记得上次选的是谁"
    );
}

/// 上次选出的 Primary **不会过期**,而上次已知的读数会。
///
/// 两者住在同一份文件里,但它们不是同一种东西:一份读数的价值随时间衰减(一个三天前的
/// 百分比不能当现状),而"上次选的是谁"是一个**没有年龄的事实**——它要答的问题是"别来回跳",
/// 而那个问题不因为过了一天就变。真正让它失效的是那台设备从配置里消失,而那件事由选择规则
/// 拿候选名单一比就知道(票 09 的 Q42:钉的 id 不在册时它会出声),不需要一个期限。
#[test]
fn the_last_selected_primary_does_not_expire_the_way_a_reading_does() {
    let general = default_general();
    let long_ago = NOW.minus_secs(30 * 86_400);

    let mut store = LastKnown::default();
    store.record("dragonfly3", &wired_reading(95, long_ago), long_ago);
    store.remember_primary("dragonfly3");
    let next_run = after_a_restart(&store);

    assert_eq!(
        next_run.reading_for("dragonfly3", &general, NOW),
        None,
        "一个月前的读数早该丢了"
    );
    assert_eq!(
        next_run.last_primary(),
        Some("dragonfly3"),
        "而上次选的是谁照旧记着——它没有年龄"
    );
}

/// 这条用例专用的一个空目录，在系统临时目录下。
///
/// **进来先清一次，而不是走的时候清**：一条失败的用例留下的文件正是要看的东西，
/// 而下一次跑照样从干净的状态开始。
fn scratch_dir(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("juicebar-state-tests").join(case);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建得了临时目录");
    dir
}
