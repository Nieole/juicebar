//! 电量数值的可信度：电压查表算出的 Derived Level、合理性校验、以及"这一行该显示
//! 哪个百分比"的取舍。
//!
//! 全是纯函数，不接硬件也不经 Transport。期望值取自 `docs/protocol.md` 记的
//! `voltageToLevel` 原始算法和那张 21 档换算表——**不是照着实现算一遍**，否则断言
//! 永远不会不同意实现。

use juicebar::sources::Reading;
use juicebar::sources::level::{Level, LevelSource, derived_level, level_for, require_plausible};

/// 换算表上的档位电压要落在它自己那一档的百分比上。
///
/// 这几个数直接抄 `docs/protocol.md` 那张表（上排电压、下排百分比一一对应），
/// 不经过任何计算：3880 mV 是第 11 档，第 11 档写着 50%。
#[test]
fn reads_the_voltage_table_anchors_off_the_table() {
    assert_eq!(derived_level(3420, Some(false)), Level::Derived(5));
    assert_eq!(derived_level(3480, Some(false)), Level::Derived(10));
    assert_eq!(derived_level(3600, Some(false)), Level::Derived(20));
    assert_eq!(derived_level(3880, Some(false)), Level::Derived(50));
    assert_eq!(derived_level(4080, Some(false)), Level::Derived(95));
}

/// 算出来正好是 0 或 15 时加一。**这是上位机的怪癖，照原样保留。**
///
/// 它没有物理含义（为什么偏偏是 0 和 15？），但对着 `app.asar` 核对的人应当在这里
/// 逐行对得上；"顺手把它抹平"会让两份实现在两个电压上悄悄分岔。
///
/// 期望值按 `docs/protocol.md` 的算法手算：3050 mV 是表的最底一档 → 0 → 加一得 1；
/// 3540 mV 是第 4 档 → 15 → 加一得 16。表底以下同样落到 0 → 1。
#[test]
fn keeps_the_plus_one_quirk_at_zero_and_fifteen() {
    assert_eq!(derived_level(3050, Some(false)), Level::Derived(1));
    assert_eq!(derived_level(3000, Some(false)), Level::Derived(1));
    assert_eq!(derived_level(3540, Some(false)), Level::Derived(16));
}

/// 档与档之间线性插值。
///
/// 期望值手算：3880 mV 是 50%、3920 mV 是 55%，一档 5% 所以 8 mV 一个百分点；
/// 3900 mV 高出下界 20 mV = 2.5 个百分点 → 52.5 → 取整 53。
/// 3700 mV 高出 3660 mV（25%）40 mV，这一档 12 mV 一个百分点 → 25 + 3.33 → 28。
#[test]
fn interpolates_between_two_table_steps() {
    assert_eq!(derived_level(3900, Some(false)), Level::Derived(53));
    assert_eq!(derived_level(3700, Some(false)), Level::Derived(28));
}

/// 表顶以上硬 clamp：充电中给 99，否则给 100。
///
/// **99 那个值的全部意义是"充电中别显示满电"。**表顶那一档就是 100%，表上再没有
/// 更高的档可以插值，所以越过表顶的电压全都被压成同一个数——这正是它在这一段不可信
/// 的原因（4155 与 4190 mV 会得到同一个 100），见 `docs/adr/0002`。
///
/// 4235 mV 是实测充电中的电压；4110 mV 是表顶那一档本身。
#[test]
fn clamps_above_the_top_of_the_table() {
    assert_eq!(derived_level(4155, Some(false)), Level::Derived(100));
    assert_eq!(derived_level(4190, Some(false)), Level::Derived(100));
    assert_eq!(derived_level(4235, Some(true)), Level::Derived(99));
    assert_eq!(derived_level(4110, Some(false)), Level::Derived(100));
    assert_eq!(derived_level(4110, Some(true)), Level::Derived(99));
}

/// 充电与否说不上来时不给 99。
///
/// 99 是一处**故意偏离表格**的取值，唯一的理由是"知道它在充电"。连是否在充电都
/// 不知道的时候取 99，就是替设备做了一个没人验过的断言——那正是键盘的 `charging`
/// 交 `None` 要防的事（parking lot Q8）。
#[test]
fn does_not_claim_ninety_nine_when_charging_is_unknown() {
    assert_eq!(derived_level(4235, None), Level::Derived(100));
}

/// 百分比不在 0..=100 里，这一帧整个不可信。
///
/// 固件换一版、帧结构变一个字节，按老下标取出来的仍然是一串看着像数字的字节。
/// 一个诚实说"我不知道"的工具比一个自信显示 200% 的工具有用得多
/// （`docs/protocol.md` 第 6 节）。
#[test]
fn rejects_a_level_outside_zero_to_one_hundred() {
    let error = require_plausible(&reading(200, Some(false), Some(4000)))
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("读取异常"),
        "校验不过要说成读取异常，而不是一个数字：{error}"
    );
    assert!(error.contains("200"), "要说清读到的是什么：{error}");
}

/// 电压不在 3050..=4350 mV 里，这一帧整个不可信。
#[test]
fn rejects_a_voltage_outside_the_plausible_range() {
    for implausible in [2000, 3049, 4351, 9000] {
        let error = require_plausible(&reading(95, Some(false), Some(implausible)))
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("读取异常"),
            "校验不过要说成读取异常：{error}"
        );
        assert!(
            error.contains(&implausible.to_string()),
            "要说清读到的是什么：{error}"
        );
    }
}

/// **充电中的电压比静置更高，那是好帧。**
///
/// 4231–4235 mV 是插着线充电中的实测值，帧的 CRC 全部验通、结构完全正确。合理性
/// 校验的电压上界一度写的是 4110（查表的表顶），照那个界**每一次充完电都会被判成
/// "帧结构变了"**——不是边缘情况，是常态。4154–4190 mV 是拔线刚充满的实测值，
/// 同样必须过。
#[test]
fn accepts_the_measured_charging_and_freshly_charged_voltages() {
    for measured in [4154, 4155, 4158, 4190, 4231, 4235] {
        require_plausible(&reading(95, Some(true), Some(measured)))
            .unwrap_or_else(|e| panic!("{measured} mV 是实测到的好帧，不该被判成异常：{e:#}"));
    }
}

/// 一份根本没有电压的 Reading 照样是好帧——键盘的回包里就没有这一项。
///
/// "没有电压"不等于"0 mV"：拿 0 顶上去会把**每一条**键盘读数判成读取异常
/// （parking lot Q7）。
#[test]
fn accepts_a_reading_that_carries_no_voltage_at_all() {
    require_plausible(&reading(100, None, None)).expect("键盘的 Reading 没有电压，那是正常的");
}

/// `level == 0` 过得了合理性校验：它是 Unknown，不是读取异常。
///
/// 两件事的区别不是措辞：0 落在 0..=100 里，帧结构没有任何可疑之处，只是电量那**一个**
/// 字段读起来不可采信。判成读取异常就等于连同充电位和电压一起丢掉。
#[test]
fn accepts_a_level_of_zero_because_that_is_unknown_not_an_anomaly() {
    require_plausible(&reading(0, None, None)).expect("0 是 Unknown，不是帧异常");
}

/// **越过查表的表顶就改用 Reported Level。**这是本票的招牌回归。
///
/// 一次受控充电前后的实测：电压 4155 → 4190 mV、固件自报 95 → 100，而**查表两次都给
/// 100**（表顶硬 clamp，4155 与 4190 都越界）。电池确实增加了电量、固件跟上了，查表
/// 没有。所以 4155 mV + 固件 95 在 `auto` 下必须产出 **95**——给 100 就是那个"看着
/// 合理但其实是编的数字"。理由全文见 `docs/adr/0002` 第一节。
#[test]
fn prefers_the_reported_level_above_the_top_of_the_voltage_table() {
    let before = reading(95, Some(false), Some(4155));
    let after = reading(100, Some(false), Some(4190));

    assert_eq!(level_for(&before, LevelSource::Auto), Level::Reported(95));
    assert_eq!(level_for(&after, LevelSource::Auto), Level::Reported(100));
    // 查表在这一段确实分辨不出这两帧——这两条断言就是"固件更可信"的依据本身。
    // 期望值各自写成 100（表顶那一档的百分比），**不拿两次调用互相比**：
    // 拿实现比实现的断言永远不会不同意实现。
    assert_eq!(derived_level(4155, Some(false)), Level::Derived(100));
    assert_eq!(derived_level(4190, Some(false)), Level::Derived(100));
}

/// 表顶以下仍然用 Derived Level —— **只在已知查表失效的那一段偏离厂商上位机。**
///
/// 不全盘改用固件自报值，是因为上位机特意绕开它很可能是踩过某些型号的坑，而我们只有
/// 一台设备的证据，不足以推翻它（`docs/adr/0002`）。这里给的固件值 20 与查表算出的
/// 53 差得很远，所以"用了哪一个"从结果上看得见，不必去问实现。
#[test]
fn uses_the_derived_level_inside_the_voltage_table() {
    let low = reading(20, Some(false), Some(3900));

    assert_eq!(level_for(&low, LevelSource::Auto), Level::Derived(53));
}

/// **没有电压就没有 Derived Level**，`auto` 对键盘因此自然退化成 Reported Level。
///
/// 这不是一条"如果是键盘就……"的特例判断，而是"那个数据不存在"的直接后果：键盘的
/// `0xF7` 回包里根本没有电压这一项。拿 0 mV 去查表会算出一个理直气壮的 0%——把满电的
/// 键盘显示成空电，正是这个项目一路在防的那种编出来的数字（parking lot Q7）。
#[test]
fn degrades_to_the_reported_level_when_the_protocol_has_no_voltage() {
    let keyboard = reading(100, None, None);

    assert_eq!(
        level_for(&keyboard, LevelSource::Auto),
        Level::Reported(100)
    );
}

/// `level_source = "reported"` 一律用 Reported Level，连查表可用的区间也不查。
///
/// 这个开关是整台设备的逃生门：上位机特意绕开固件自报值，大概是踩过某些型号的坑，
/// 而我们只有一台设备的证据（`docs/adr/0002`）。这里的电压 3900 mV 落在查表可用的
/// 区间里（会算出 53），固件却报 20——所以"有没有真的听话"看得见。
#[test]
fn always_uses_the_reported_level_when_the_device_asks_for_it() {
    assert_eq!(
        level_for(&reading(20, Some(false), Some(3900)), LevelSource::Reported),
        Level::Reported(20)
    );
    assert_eq!(
        level_for(&reading(95, Some(false), Some(4155)), LevelSource::Reported),
        Level::Reported(95)
    );
}

/// **Reported Level 读到 0 是 Unknown，绝不映射成 100。**
///
/// 上位机原文是 `battery.level = result[1] == 0 ? 100 : result[1]`，把 0 当满电。
/// 照抄的后果是键盘电量真读到 0 时托盘显示 100%——不崩、不报警、每次看都合情合理，
/// 而且偏偏发生在最该提醒充电的时刻，与本工具存在的全部意义直接冲突（`docs/adr/0002`
/// 第二节）。0 也不该显示成 0%：那同样是个我们采信不了的数字。
///
/// 三种取法都要守住：键盘（没有电压）、表顶以上、以及钉死 `reported` 的设备。
#[test]
fn calls_a_reported_level_of_zero_unknown_and_never_one_hundred() {
    assert_eq!(
        level_for(&reading(0, None, None), LevelSource::Auto),
        Level::Unknown
    );
    assert_eq!(
        level_for(&reading(0, Some(false), Some(4155)), LevelSource::Auto),
        Level::Unknown
    );
    assert_eq!(
        level_for(&reading(0, Some(false), Some(3900)), LevelSource::Reported),
        Level::Unknown
    );
}

/// **Derived Level 读到 0 照样是 0%，不是 Unknown。**"0 是 Unknown"只管固件那个字段。
///
/// 两个 0 的来历完全不同：固件字段的 0 与"这一格没填"不可区分，而查表的 0 是一个
/// **测出来的电压**换算出来的——3051 mV 确实已经见底（表底那一档就是 3050 mV）。
/// 把它也说成 Unknown，等于在最该提醒充电的时刻把唯一可信的那个数扣下来。
///
/// 顺带记一件反直觉的事：那条"算出来正好 0 就加一"的怪癖**挡不住 0%**，它只在电压
/// 正好落在档位上时触发。3051 mV 插值得 0.0135，取整回到 0。
#[test]
fn keeps_a_derived_zero_as_a_percentage_because_a_voltage_measured_it() {
    assert_eq!(derived_level(3051, Some(false)), Level::Derived(0));
    assert_eq!(
        level_for(&reading(50, Some(false), Some(3051)), LevelSource::Auto),
        Level::Derived(0)
    );
}

/// 手搓一份 Reading。这几项在真实链路上由驱动从一帧里解析出来。
fn reading(reported_level: u8, charging: Option<bool>, voltage_mv: Option<u16>) -> Reading {
    Reading {
        reported_level,
        charging,
        voltage_mv,
    }
}
