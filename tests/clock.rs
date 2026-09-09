//! Clock 接缝：陈旧判定要的"当下"，以及一个时刻怎么变成人读的日期。
//!
//! 这一整个文件里**没有一次 sleep、也没有一次真实时钟**——那正是这条接缝存在的理由
//! （spec 的第 37 条 user story：「我想让陈旧判定可测且不依赖真实时间流逝」）。

use juicebar::clock::Timestamp;

/// 一个时刻交出去的秒数就是它收进来的那个。
///
/// 断言它是因为一个只进不出的 newtype 是个黑洞：票 08 要把取得时刻写盘再读回来，
/// 而那一步除了这两个函数没有别的路。
#[test]
fn a_timestamp_gives_back_the_seconds_it_was_built_from() {
    assert_eq!(Timestamp::from_unix_secs(0).as_unix_secs(), 0);
    assert_eq!(
        Timestamp::from_unix_secs(1_773_532_800).as_unix_secs(),
        1_773_532_800
    );
}

/// 一个时刻要能印成日历上的一天。
///
/// 这是"超过 `very_stale_after` 只显示日期，不显示百分比"那一步唯一需要的形式：
/// 一个几个月前的百分比不该再显示，但"这个数是哪天的"照样得说得出来，否则用户
/// 只知道"旧"，不知道旧到什么地步。
///
/// 期望值不是照着实现算的：1970-01-01 是 Unix 纪元的定义，2026-03-15 那个秒数是
/// 手工数出来的（1970 到 2026 共 56 年、其中 14 个闰日 → 20454 天到 2026-01-01，
/// 再加 73 天到 3 月 15 日 → 20527 天 × 86400）。
#[test]
fn renders_a_timestamp_as_a_calendar_date() {
    assert_eq!(Timestamp::from_unix_secs(0).utc_date_text(), "1970-01-01");
    assert_eq!(
        Timestamp::from_unix_secs(20_527 * 86_400).utc_date_text(),
        "2026-03-15"
    );
    // 同一天里的任何时刻都是同一天：日期不该随时分秒抖动。
    assert_eq!(
        Timestamp::from_unix_secs(20_527 * 86_400 + 86_399).utc_date_text(),
        "2026-03-15"
    );
}

/// `Ble` 的取得时刻只能倒推出来：Windows 只说那份缓存"多少秒之前更新过"。
///
/// 实测那台鼠标的缓存是 10 天前的（票 05 的真机记录），而"10 天前是哪一天"是
/// `very_stale_after` 那一档唯一还说得出的话。
#[test]
fn winds_a_timestamp_back_by_the_age_of_a_cached_reading() {
    let now = Timestamp::from_unix_secs(20_537 * 86_400);
    assert_eq!(now.minus_secs(10 * 86_400).utc_date_text(), "2026-03-15");
}

/// 取得时刻距今多少秒——"多久前"那一列就是它。
///
/// 时钟被回拨过时（`earlier` 比"当下"还晚）交 0，而不是一个下溢出来的天文数字：
/// 那一侧最保守，绝不会把一份陈旧读数说成新鲜的。
#[test]
fn measures_how_long_ago_a_reading_was_taken() {
    let taken_at = Timestamp::from_unix_secs(20_527 * 86_400);
    let now = taken_at.minus_secs(0);
    assert_eq!(now.secs_since(taken_at), 0);

    let later = Timestamp::from_unix_secs(20_527 * 86_400 + 7_200);
    assert_eq!(later.secs_since(taken_at), 7_200);

    // 系统时钟回拨：当下反而早于取得时刻。
    assert_eq!(taken_at.secs_since(later), 0);
}
