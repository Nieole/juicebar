//! Primary Device 的选择：`primary = "lowest"` 时谁参与比较、谁不参与，以及钉死某一个
//! 的那一种。
//!
//! 被测的全是纯函数，没有一个假实现、也没有时钟——"新鲜"与"可信"这两件事在别处判完了
//! （`tests/staleness.rs` 与 `tests/level.rs`），这个文件只断言**拿到那两个结论之后
//! 选出谁**。

use juicebar::config::Config;
use juicebar::primary::{self, Candidate, CandidateReading, PrimaryRule, Selection};
use juicebar::sources::Reading;
use juicebar::sources::level::{Level, LevelSource, level_for};
use juicebar::staleness::Freshness;

/// 一台读到了读数的 Device：这一轮显示的百分比取自哪里，以及这份读数还算不算现状。
fn candidate(id: &str, level: Level, freshness: Freshness) -> Candidate<'_> {
    Candidate {
        id,
        reading: Some(CandidateReading { level, freshness }),
    }
}

/// 一台失联的 Device：一条 Endpoint 都没读到。**它不是一份 Unknown 的读数**
/// （`CONTEXT.md`：「Unknown 不等于 0%，也不等于设备离线」）。
fn lost(id: &str) -> Candidate<'_> {
    Candidate { id, reading: None }
}

/// 票面第 1 条：`primary = "lowest"` 时选出电量最低的 Device。
///
/// 两个候选一个 Derived 一个 Reported：比的是**用户在那一行上看到的那个数字**，
/// 不管它取自哪里（两个来源的数可能不一致，而屏幕上只有一个）。
#[test]
fn picks_the_lowest_level_among_fresh_readings() {
    let candidates = [
        candidate("dragonfly3", Level::Derived(62), Freshness::Fresh),
        candidate("neon75", Level::Reported(28), Freshness::Fresh),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Lowest("neon75")
    );
}

/// 票面第 2 条的前一半：**新鲜**。
///
/// 两个数字都比那台新鲜的低，所以少了这道筛子这条用例立刻红——票面那句"一个几天前的
/// 蓝牙缓存值不该抢走这个位置"说的就是它。三档里只有 `Fresh` 参与，另两档一并断言。
#[test]
fn a_stale_reading_does_not_take_the_lowest_spot() {
    let candidates = [
        candidate("dragonfly3", Level::Reported(62), Freshness::Fresh),
        candidate("neon75", Level::Reported(28), Freshness::Stale),
        candidate("headset", Level::Reported(5), Freshness::VeryStale),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Lowest("dragonfly3")
    );
}

/// 票面第 2 条的后一半：**可信**。
///
/// `CONTEXT.md`：「Unknown 不等于 0%，也不等于设备离线」。所以 Unknown 不是"最低的
/// 那个"，它是**根本不参与**——把它当 0 会让一台答不出电量的设备永久占住托盘图标。
#[test]
fn an_unknown_level_does_not_take_the_lowest_spot() {
    let candidates = [
        candidate("dragonfly3", Level::Reported(62), Freshness::Fresh),
        candidate("neon75", Level::Unknown, Freshness::Fresh),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Lowest("dragonfly3")
    );
}

/// 失联的也不参与。**它与 Unknown 是两回事**（`CONTEXT.md` 的 Unknown 那一条明写着
/// "也不等于设备离线"），所以类型上是"没有读数"而不是一份 Unknown 的读数。
#[test]
fn a_lost_device_does_not_take_the_lowest_spot() {
    let candidates = [
        candidate("dragonfly3", Level::Reported(62), Freshness::Fresh),
        lost("neon75"),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Lowest("dragonfly3")
    );
}

/// 票面第 3 条：正在充电的照常参与。
///
/// 这条不是废话。直觉上"在充电的就别报警了"，但一台插着线还剩不多的设备恰恰是最该盯的
/// ——它可能根本没充上。用实测那两帧走一趟 `level_for`，好让"充电"这件事真的出现在
/// 输入里，而不是由用例自己断言一个百分比。
#[test]
fn a_charging_device_still_takes_part_in_the_comparison() {
    // 实测「鼠标 cmd 4 充电中」：level 95、充电中、4235 mV。
    let charging = Reading {
        reported_level: 95,
        charging: Some(true),
        voltage_mv: Some(4235),
    };
    // 实测「鼠标 cmd 4 静置满电」：level 100、未充电、4190 mV。
    let resting = Reading {
        reported_level: 100,
        charging: Some(false),
        voltage_mv: Some(4190),
    };
    let candidates = [
        candidate(
            "dragonfly3",
            level_for(&charging, LevelSource::Auto),
            Freshness::Fresh,
        ),
        candidate(
            "neon75",
            level_for(&resting, LevelSource::Auto),
            Freshness::Fresh,
        ),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Lowest("dragonfly3")
    );
}

/// 票面第 4 条：全部不可信时保持上次的选择，不来回跳。
///
/// `status` 手上永远没有"上次"（一次性命令：枚举、取数、印几行、退出），所以这一条只有
/// 在这里断言得到——那份记忆的家见 parking lot Q41。
#[test]
fn holds_on_to_the_last_choice_when_nothing_is_trustworthy() {
    let candidates = [
        candidate("dragonfly3", Level::Unknown, Freshness::Fresh),
        candidate("neon75", Level::Reported(28), Freshness::Stale),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, Some("neon75")),
        Selection::HeldOver("neon75")
    );
}

/// 全部不可信、而且**没有**上次的选择：选不出来，别硬选一个。
///
/// 这是首次运行、每台设备都还没读到过的那一刻。硬挑一个（比如配置里第一台）会让托盘
/// 画一个没有任何读数支撑的 Device——那正是这个项目一直在防的"看着合理但其实是编的"。
#[test]
fn picks_nobody_when_nothing_is_trustworthy_and_there_is_no_last_choice() {
    let candidates = [
        candidate("dragonfly3", Level::Unknown, Freshness::Fresh),
        lost("neon75"),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Undecided
    );
}

/// 上次选的那台已经不在配置里了：同样选不出来，不保持一个印不出来的名字。
///
/// 保持它的后果是那一行根本不存在——`status` 一行都不标，而它说的是"保持上次的选择"，
/// 用户找不到那一行。
#[test]
fn does_not_hold_on_to_a_device_that_is_no_longer_configured() {
    let candidates = [candidate("dragonfly3", Level::Unknown, Freshness::Fresh)];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, Some("retired-mouse")),
        Selection::Undecided
    );
}

/// 票面第 5 条：`primary = "<设备 id>"` 时钉死该 Device。
///
/// 钉死就是钉死：那台设备这一轮陈旧、Unknown、失联都照旧钉着，另一台再新鲜也抢不走。
/// 用户钉它的理由恰恰是"我只关心这一个"，让一份读数把它挤掉就是没钉住。
#[test]
fn pins_the_device_named_in_the_config() {
    let candidates = [
        candidate("dragonfly3", Level::Reported(5), Freshness::Fresh),
        lost("neon75"),
    ];

    assert_eq!(
        primary::select(
            &PrimaryRule::Pinned("neon75".to_string()),
            &candidates,
            None
        ),
        Selection::Pinned("neon75")
    );
}

/// 钉死的 id 在登记的 Device 里根本没有——一次笔误。
///
/// **不静默退回 `lowest`。**用户钉住某一台的理由本来就是"别的我不关心"，静默换成
/// 动态选择会让他以为钉住了、其实没有，而屏幕上一切正常（Q15 记的是同一种不对称）。
/// 这里也不让整份配置读不动：合法值里有一个是"任意 Device id"，而 id 是自由文本，
/// 分辨不出笔误和一个还没登记的 id（见 parking lot Q42）。所以是**说出来**。
#[test]
fn says_so_when_the_pinned_id_names_no_configured_device() {
    let candidates = [candidate(
        "dragonfly3",
        Level::Reported(62),
        Freshness::Fresh,
    )];

    assert_eq!(
        primary::select(
            &PrimaryRule::Pinned("dragonfly4".to_string()),
            &candidates,
            None
        ),
        Selection::PinnedNotFound("dragonfly4")
    );
}

/// 并列时取配置里靠前的那一个。
///
/// "不来回跳"（票面第 4 条）说的不只是保持上次的选择：两台同电量的设备之间必须有一个
/// **稳定**的答案，否则每一轮都可能换一个。配置里的书写顺序是唯一现成的稳定次序。
#[test]
fn breaks_a_tie_by_the_order_written_in_the_config() {
    let candidates = [
        candidate("dragonfly3", Level::Reported(28), Freshness::Fresh),
        candidate("neon75", Level::Derived(28), Freshness::Fresh),
    ];

    assert_eq!(
        primary::select(&PrimaryRule::Lowest, &candidates, None),
        Selection::Lowest("dragonfly3")
    );
}

/// 配置里的 `primary`：缺省是 `"lowest"`。
///
/// 整节 `[general]` 缺席时也是这个值——托盘总得画一个，而"当前电量最低的那个"是
/// `config.example.toml` 里写着的推荐值。
#[test]
fn defaults_to_lowest_when_the_config_does_not_say() {
    assert_eq!(
        Config::parse("").unwrap().general.primary,
        PrimaryRule::Lowest
    );
}

/// `primary = "lowest"` 就是那条规则本身。
#[test]
fn reads_lowest_as_the_rule() {
    assert_eq!(
        Config::parse("[general]\nprimary = \"lowest\"\n")
            .unwrap()
            .general
            .primary,
        PrimaryRule::Lowest
    );
}

/// 别的任何字符串都是一个 Device id——**这一项的合法值里有一个是自由文本**，所以认不出的
/// 值不能像 `level_source` 那样让整份配置读不动（Q15 与 Q42 的差别就在这里）。
#[test]
fn reads_any_other_string_as_a_device_id_to_pin() {
    assert_eq!(
        Config::parse("[general]\nprimary = \"dragonfly3\"\n")
            .unwrap()
            .general
            .primary,
        PrimaryRule::Pinned("dragonfly3".to_string())
    );
}

/// 票面第 6 条：`status` 标出当前 Primary Device。
///
/// 用的是 `CONTEXT.md` 的词，原样——那一条的 _Avoid_ 正是「主设备、默认设备、当前设备
/// 混用」，所以这里不能是"主设备"、也不能只是一个星号。
#[test]
fn marks_the_primary_device_by_name() {
    assert_eq!(
        Selection::Lowest("dragonfly3").label("dragonfly3", "Dragonfly 3 Master+"),
        "Dragonfly 3 Master+（Primary Device）"
    );
}

/// 别的 Device 那一行原样印名字。
#[test]
fn leaves_the_other_device_names_alone() {
    assert_eq!(
        Selection::Lowest("neon75").label("dragonfly3", "Dragonfly 3 Master+"),
        "Dragonfly 3 Master+"
    );
}

/// 标注落在一行**没有新鲜读数**的 Device 上时要交代一句。
///
/// 保持上次的选择意味着那一行印的是"已陈旧"、Unknown 或者干脆"读不到"——不说一句，
/// 这个标注看起来就是个 bug。
#[test]
fn explains_a_marker_that_comes_from_the_last_round() {
    let note = Selection::HeldOver("neon75")
        .note()
        .expect("保持上次的选择要交代一句");

    assert!(note.contains("上次"), "说清这个标注是上一轮的：{note}");
}

/// 钉死的 id 不在册时，一行都不标——那更要说出来。
#[test]
fn explains_a_pinned_id_that_names_no_device() {
    let note = Selection::PinnedNotFound("dragonfly4")
        .note()
        .expect("钉了个不存在的 id 一定要说出来");

    assert!(note.contains("dragonfly4"), "把那个 id 原样印出来：{note}");
}

/// 一行都标不出来时也说一句，否则"没有标注"看着像 bug。
#[test]
fn explains_why_no_device_got_the_marker() {
    let note = Selection::Undecided.note().expect("选不出来也要交代一句");

    assert!(
        note.contains("Primary Device"),
        "说的是 Primary Device 这件事：{note}"
    );
}

/// 正常选出来的两种不多说话：那一行上的标注已经把话说完了。
#[test]
fn says_nothing_extra_when_the_marker_speaks_for_itself() {
    assert_eq!(Selection::Lowest("dragonfly3").note(), None);
    assert_eq!(Selection::Pinned("dragonfly3").note(), None);
}
