//! 「这一轮的结果」：每台 Device 的图标状态、电量、来源 Endpoint、多久前、一句短原因；Primary
//! Device 是谁、怎么选出来的；这一轮的告警。
//!
//! 被测的是一个纯函数（`juicebar::round`）：输入是配置、各台 Device 这一轮手上有的东西、"当下"，
//! 不碰 Win32 界面、不碰硬件。手上有的东西多半是**罐装的一次取数结果**——`RowReading` 与
//! `NoReading` 直接造，不走假枚举：这个文件断言的是"拿到这些之后判成什么"，一次取数本身怎么走
//! 在 `tests/readout.rs`。只有进程枚举失败那一条走真的取数，因为它要证的正是"问不出来时，取数
//! 照常去试那几条 HID Endpoint"。
//!
//! 图标状态的优先顺序照 `CONTEXT.md`「图标状态」：暂停 > 取数失败 / Unknown / 无已知值 >
//! 充电中 > 低电 > Stale > 正常。下面每一处边界至少一条用例。

mod common;

use common::fixtures::{MOUSE_REPORT_ID, MOUSE_RESTING_FULL};
use common::{
    FakeEndpoints, FakeProcesses, NOW, default_general, keyboard_with_dongle_endpoint,
    mouse_with_all_three_endpoints, mouse_with_both_endpoints, scanned_ble, vendor_hub_running,
};
use juicebar::config::{Config, Device};
use juicebar::endpoints::{EndpointKind, EndpointReading};
use juicebar::icon::IconState;
use juicebar::primary::Selection;
use juicebar::readout::{self, RowReading};
use juicebar::round::{InHand, PauseCheck, Round, Warning};
use juicebar::sources::Reading;
use juicebar::sources::level::Level;
use juicebar::state::{LastKnown, Provenance};
use juicebar::vendor_hub::VendorHub;

/// 这一次取数当场从 Wired 读到的一份：电量 `level`，充电与否 `charging`。
///
/// 电压给 4200 mV，越过查表的表顶，所以缺省的 `auto` 采信的就是这个 `level`（`docs/adr/0002`）
/// ——用例里写下的那个数就是这一行显示的那个数，不必再过一遍查表。
fn just_read(level: u8, charging: bool) -> InHand {
    wired(level, charging, 0, Provenance::JustRead)
}

/// 当场从 Wired 读到、但那是 `secs_ago` 秒之前的事：常驻之后，一台设备两次取数之间的每一轮
/// 手上都是这么一份。
fn read_long_ago(level: u8, secs_ago: u64) -> InHand {
    wired(level, false, secs_ago, Provenance::JustRead)
}

/// 这一次取数没读到、退到的**上次已知值**：`secs_ago` 秒之前从 Wired 读到的那一份。
fn last_known(level: u8, charging: bool, secs_ago: u64) -> InHand {
    wired(level, charging, secs_ago, Provenance::LastKnown)
}

fn wired(level: u8, charging: bool, secs_ago: u64, provenance: Provenance) -> InHand {
    InHand::Reading(RowReading {
        reading: EndpointReading::from_hid(
            EndpointKind::Wired,
            Reading {
                reported_level: level,
                charging: Some(charging),
                voltage_mv: Some(4_200),
            },
            NOW.minus_secs(secs_ago),
        ),
        provenance,
        paused_by: None,
    })
}

/// 同一份读数，但这一次取数有 Endpoint 为厂商上位机让开过。
///
/// 当场读到的 Wired 读数带着暂停，真机上走不到（Wired 正是让开的那一条）；这里造它，是因为
/// 纯函数照样得对它给出一个答案，而"暂停压过充电中"只有这么造才断言得到。
fn paused(in_hand: InHand, hub: &VendorHub) -> InHand {
    match in_hand {
        InHand::Reading(row) => InHand::Reading(RowReading {
            paused_by: Some(hub.clone()),
            ..row
        }),
        _ => panic!("只给读数加暂停"),
    }
}

/// 真去取一次数：经假枚举，状态文件里什么都没有。
///
/// 手上那一份就是命令行今天交给这一轮的东西——`read_or_last_known` 交出来的那个 `Result`。
fn fetched(device: &Device, endpoints: &FakeEndpoints, paused_by: Option<&VendorHub>) -> InHand {
    InHand::from(readout::read_or_last_known(
        device,
        endpoints,
        paused_by,
        &mut LastKnown::default(),
        &default_general(),
        NOW,
    ))
}

/// 一台 Device、手上这一份东西，在缺省配置下判成哪一种图标状态。
fn icon_state_of(device: &Device, in_hand: &InHand) -> IconState {
    let general = default_general();
    let round = Round::assess(&general, [(device, in_hand)], None, Vec::new(), NOW);
    round.devices[0].icon_state
}

/// 当场读到、新鲜、电量不低 —— 正常。
#[test]
fn a_fresh_reading_that_is_not_low_is_normal() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &just_read(62, false)),
        IconState::Normal
    );
}

/// 电量**低于**阈值 —— 低电；刚好等于阈值还不算。
///
/// 缺省阈值 20。边界要有一侧：`CONTEXT.md` 写的是"低于低电量阈值"，所以 20% 不低、19% 低。
#[test]
fn a_reading_below_the_threshold_is_low_and_one_exactly_at_it_is_not() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(icon_state_of(&mouse, &just_read(19, false)), IconState::Low);
    assert_eq!(
        icon_state_of(&mouse, &just_read(20, false)),
        IconState::Normal
    );
}

/// 上次已知值 —— Stale，**哪怕它是十秒前取的**：它有多新，和设备此刻在不在，是两件事
/// （`CONTEXT.md`「上次已知值」：一律标注成 Stale）。
#[test]
fn a_last_known_value_is_stale_even_when_it_was_taken_seconds_ago() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &last_known(62, false, 10)),
        IconState::Stale
    );
}

/// 当场读到、而取得时刻已经过了陈旧阈值 —— Stale。Wired 的阈值是 3 × 30 秒。
#[test]
fn a_reading_older_than_its_stale_threshold_is_stale() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &read_long_ago(62, 91)),
        IconState::Stale
    );
}

/// **低电压过 Stale**：上次已知值低于阈值，即使已经 Stale 也算低电。
///
/// 一台读不到了的设备，最常见的原因就是没电关机——那正是最该提醒的时候，而画成 Stale 就把它
/// 藏进了"等一等"那一类。
#[test]
fn a_low_last_known_value_is_low_even_though_it_is_stale() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &last_known(15, false, 1_800)),
        IconState::Low
    );
    assert_eq!(
        icon_state_of(&mouse, &read_long_ago(15, 91)),
        IconState::Low,
        "当场读到而陈旧了的也一样"
    );
}

/// 陈旧到不该再显示数字的那一档——只有 `Ble` 到得了：Windows 那份缓存超过了 `very_stale_after`
/// ——照 Stale 一类判，照样参与低电判定，电量也照样交出去（parking lot Q151）。
///
/// 命令行那一行这时只印日期，那是它自己的排版决定；而图标上，一台十天没更新过、上次还剩 12% 的
/// 蓝牙设备，最可能的下场正是没电关了机——低电压过 Stale 的那条理由在这一档上一样成立。
#[test]
fn a_very_stale_reading_is_judged_like_any_stale_one() {
    let with_ble = mouse_with_all_three_endpoints();
    let general = default_general();
    let cached_ten_days_ago = |level| {
        FakeEndpoints::new(MOUSE_REPORT_ID, []).with_ble_cache(scanned_ble(
            "Dragonfly 3 Master+",
            "e452430072a9",
            Some(level),
            Some(10 * 86_400),
        ))
    };
    let not_low = fetched(&with_ble, &cached_ten_days_ago(62), None);
    let low = fetched(&with_ble, &cached_ten_days_ago(12), None);

    let round = Round::assess(
        &general,
        [(&with_ble, &not_low), (&with_ble, &low)],
        None,
        Vec::new(),
        NOW,
    );

    assert_eq!(round.devices[0].icon_state, IconState::Stale);
    assert_eq!(round.devices[1].icon_state, IconState::Low);
    assert_eq!(round.devices[1].level(), Some(Level::Reported(12)));
}

/// **充电中压过低电**：插着线还剩 15% 的那台，用户已经在处理了（spec 用户故事 12）。充电中也压过
/// Stale——当场读到的充电态，哪怕那一份已经过了陈旧阈值，也比"等一等"说得多。
#[test]
fn a_charging_device_is_charging_even_when_its_level_is_low() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &just_read(15, true)),
        IconState::Charging
    );
    assert_eq!(
        icon_state_of(&mouse, &wired(62, true, 91, Provenance::JustRead)),
        IconState::Charging,
        "陈旧的那一份也一样"
    );
}

/// 上次已知值里记着的"充电中"**不让它成为充电中**：那是当时的状态，不是现在的——设备被收进抽屉、
/// 或者刚拔了线，变的恰恰是它。命令行那一行不印它是同一条理由（`cli::status::render` 那一格）。
///
/// 于是那份历史值照常走下面几档：低于阈值就是低电，否则 Stale。
#[test]
fn a_charging_state_from_a_last_known_value_does_not_count_as_charging() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &last_known(62, true, 1_800)),
        IconState::Stale
    );
    assert_eq!(
        icon_state_of(&mouse, &last_known(15, true, 1_800)),
        IconState::Low
    );
}

/// 电量字段采信不了 —— Unknown，压过充电中与 Stale。
///
/// 固件报 0、电压又越过表顶，缺省的 `auto` 就只剩一个采信不了的数（`docs/adr/0002`）。
/// **Unknown 不等于 0%**（`CONTEXT.md`），所以它也不是"低电"：画成低电就是替设备说了一个它没说的数。
#[test]
fn an_untrustworthy_level_is_unknown_even_when_charging_or_stale() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &just_read(0, false)),
        IconState::Unknown
    );
    assert_eq!(
        icon_state_of(&mouse, &just_read(0, true)),
        IconState::Unknown,
        "压过充电中"
    );
    assert_eq!(
        icon_state_of(&mouse, &last_known(0, false, 1_800)),
        IconState::Unknown,
        "压过 Stale"
    );
}

/// 取数失败、又退不到上次已知值 —— 取数失败。
///
/// 手上有上次已知值时不是这一种：那时画的是那份历史值（Stale，或者低电，见上面），因为"读不到了"
/// 与"上次是多少电"两件事都成立，而后者才是用户还能用的那一件。
#[test]
fn a_failed_fetch_with_nothing_to_fall_back_on_is_a_fetch_failure() {
    let mouse = mouse_with_both_endpoints();
    let put_away = FakeEndpoints::new(MOUSE_REPORT_ID, []);

    assert_eq!(
        icon_state_of(&mouse, &fetched(&mouse, &put_away, None)),
        IconState::FetchFailed
    );
}

/// 还没有一次取数有过结果、也没有上次已知值 —— 无已知值。
///
/// 常驻之后这是每台设备刚启动时的样子：它的第一次取数还没回来，状态文件里也没有它。它**不是**
/// 取数失败——我们还没去问，没有什么失败了，用户该做的只是等第一次读到。
#[test]
fn a_device_with_nothing_in_hand_has_no_known_value() {
    let mouse = mouse_with_both_endpoints();

    assert_eq!(
        icon_state_of(&mouse, &InHand::NoKnownValue),
        IconState::NoKnownValue
    );
}

/// **暂停压过其余每一种**：厂商上位机在跑、我们主动让开了，这件事比手上那份数字更要紧——用户
/// 该动手的地方是那个上位机，而别的状态都在叫他去看设备（spec 用户故事 14）。
///
/// 手上有读数的那几种（`Ble` 顶上的、或者退到的上次已知值）逐一加上暂停；手上没有读数的两种
/// 走真的取数：让开之后剩下的也没读到（取数失败那一面），与让开之后什么都没有（无已知值那一面：
/// 从没读到过、也没有上次已知值，正是 `CONTEXT.md` 那一条的字面）。`InHand::NoKnownValue` 那一种
/// 不在这里：还没取过数，就还没有什么为上位机让开过（parking lot Q153）。
#[test]
fn a_paused_device_is_paused_whatever_else_it_has_in_hand() {
    let hub = vendor_hub_running();
    let mouse = mouse_with_both_endpoints();
    let with_ble = mouse_with_all_three_endpoints();
    let both_hid_present = || {
        FakeEndpoints::new(
            MOUSE_REPORT_ID,
            [
                (EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()]),
                (EndpointKind::Dongle24G, vec![MOUSE_RESTING_FULL.to_vec()]),
            ],
        )
    };
    // 让开了两条 HID，剩下那条 Ble 也没交出读数：那份缓存里没有电量属性。
    let ble_without_a_level = both_hid_present().with_ble_cache(scanned_ble(
        "Dragonfly 3 Master+",
        "e452430072a9",
        None,
        Some(300),
    ));

    let cases = [
        ("正常", &mouse, paused(just_read(62, false), &hub)),
        ("低电", &mouse, paused(just_read(15, false), &hub)),
        ("充电中", &mouse, paused(just_read(62, true), &hub)),
        ("Stale", &mouse, paused(last_known(62, false, 1_800), &hub)),
        ("Unknown", &mouse, paused(just_read(0, false), &hub)),
        (
            "取数失败",
            &with_ble,
            fetched(&with_ble, &ble_without_a_level, Some(&hub)),
        ),
        (
            "无已知值",
            &mouse,
            fetched(&mouse, &both_hid_present(), Some(&hub)),
        ),
    ];
    for (other, device, in_hand) in &cases {
        assert_eq!(
            icon_state_of(device, in_hand),
            IconState::Paused,
            "暂停该压过{other}"
        );
    }
}

/// 单台阈值覆盖：同一个 25%，写了 `low_battery = 30` 的那台是低电，跟着 `[general]`（缺省 20）
/// 的那台不是。
///
/// 每台用的是哪个阈值也交出来：这一轮之后还有别人要问"它低不低"（低电通知就是），答案得是同一个。
#[test]
fn each_device_is_judged_against_its_own_threshold() {
    let config = Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"
        low_battery = 30

        [[device]]
        id = "neon75"
        name = "VGN Neon75"
        driver = "vgn_keyboard"
        "#,
    )
    .unwrap();
    let (mouse, keyboard) = (&config.devices[0], &config.devices[1]);
    let (for_mouse, for_keyboard) = (just_read(25, false), just_read(25, false));

    let round = Round::assess(
        &config.general,
        [(mouse, &for_mouse), (keyboard, &for_keyboard)],
        None,
        Vec::new(),
        NOW,
    );

    assert_eq!(round.devices[0].icon_state, IconState::Low);
    assert_eq!(round.devices[0].low_battery, 30);
    assert_eq!(round.devices[1].icon_state, IconState::Normal);
    assert_eq!(round.devices[1].low_battery, 20);
}

/// 手上有读数时，这一轮交出它的电量、来源 Endpoint 与多久前——悬停提示与菜单那一行要的就是这几样。
/// 这时没有短原因：有数可印，就不必解释为什么没有。
#[test]
fn a_device_with_a_reading_shows_its_level_its_source_and_how_long_ago() {
    let mouse = mouse_with_both_endpoints();
    let general = default_general();
    let in_hand = read_long_ago(62, 45);

    let round = Round::assess(&general, [(&mouse, &in_hand)], None, Vec::new(), NOW);
    let state = &round.devices[0];

    assert_eq!(state.level(), Some(Level::Reported(62)));
    assert_eq!(state.source(), Some(EndpointKind::Wired));
    assert_eq!(state.age_secs(), Some(45), "多久前按这一轮的当下算");
    assert!(state.reason().is_none());
}

/// 手上没有读数时，这一轮交出**一句短原因**——取数那一层说的那一句，原样——而电量、来源与多久前
/// 都没有：一样都说不出来，就一样都不编。
///
/// 无已知值那一种连原因都没有：没有什么失败了，只是还没读到。
#[test]
fn a_device_without_a_reading_shows_why_and_nothing_else() {
    let mouse = mouse_with_both_endpoints();
    let general = default_general();
    let failed = fetched(&mouse, &FakeEndpoints::new(MOUSE_REPORT_ID, []), None);
    let nothing = InHand::NoKnownValue;

    let round = Round::assess(
        &general,
        [(&mouse, &failed), (&mouse, &nothing)],
        None,
        Vec::new(),
        NOW,
    );

    let state = &round.devices[0];
    let reason = state.reason().expect("取数失败要说得出原因").to_string();
    assert!(
        reason.starts_with("读不到 —— 配置的 Endpoint（Wired、Dongle24G）一条都不在场"),
        "就是取数那一层交出的那一句：{reason}"
    );
    assert_eq!(state.level(), None);
    assert_eq!(state.source(), None);
    assert_eq!(state.age_secs(), None);

    let state = &round.devices[1];
    assert!(state.reason().is_none(), "无已知值没有什么失败了");
    assert_eq!(state.level(), None);
    assert_eq!(state.source(), None);
    assert_eq!(state.age_secs(), None);
}

// ---------------------------------------------------------------
// Primary Device：是谁、怎么选出来的
//
// 规则本身（谁参与 lowest、钉死的 id 不在册时怎样）在 `tests/primary.rs` 里断言完了；这里断言的是
// **这一轮把该递的都递了进去**：配置里那条规则、每台这一轮的样子、以及上一轮选的是谁。
// ---------------------------------------------------------------

/// 自动（电量最低）：两台都新鲜可信，电量低的那台就是 Primary Device。
#[test]
fn the_primary_device_is_the_lowest_when_the_rule_is_lowest() {
    let general = default_general();
    let (mouse, keyboard) = (mouse_with_both_endpoints(), keyboard_with_dongle_endpoint());
    let (for_mouse, for_keyboard) = (just_read(62, false), just_read(28, false));

    let round = Round::assess(
        &general,
        [(&mouse, &for_mouse), (&keyboard, &for_keyboard)],
        None,
        Vec::new(),
        NOW,
    );

    assert_eq!(round.primary, Selection::Lowest("neon75"));
}

/// 钉死：配置里写了哪一台就是哪一台，电量高低不论。
#[test]
fn the_primary_device_is_the_pinned_one_when_the_config_pins_it() {
    let general = Config::parse("[general]\nprimary = \"dragonfly3\"\n")
        .unwrap()
        .general;
    let (mouse, keyboard) = (mouse_with_both_endpoints(), keyboard_with_dongle_endpoint());
    let (for_mouse, for_keyboard) = (just_read(62, false), just_read(28, false));

    let round = Round::assess(
        &general,
        [(&mouse, &for_mouse), (&keyboard, &for_keyboard)],
        None,
        Vec::new(),
        NOW,
    );

    assert_eq!(round.primary, Selection::Pinned("dragonfly3"));
}

/// 保持上次的选择：这一轮的候选全都不可信时，沿用上一轮按规则选出的那一台，托盘不因为一轮
/// 读不到就跳开。
#[test]
fn the_primary_device_is_held_over_when_nothing_is_trustworthy_this_round() {
    let general = default_general();
    let (mouse, keyboard) = (mouse_with_both_endpoints(), keyboard_with_dongle_endpoint());
    // 一台无已知值，一台电量 Unknown：没有一个新鲜且可信的读数。
    let (for_mouse, for_keyboard) = (InHand::NoKnownValue, just_read(0, false));

    let round = Round::assess(
        &general,
        [(&mouse, &for_mouse), (&keyboard, &for_keyboard)],
        Some("dragonfly3"),
        Vec::new(),
        NOW,
    );

    assert_eq!(round.primary, Selection::HeldOver("dragonfly3"));
}

// ---------------------------------------------------------------
// 这一轮的告警
// ---------------------------------------------------------------

/// **进程枚举失败时按"没在跑"走**（fail open），并作为这一轮的一条告警交出来。
///
/// 反过来——问不出来就一律暂停——会让一台 Win32 调用失败的机器永久显示暂停、指名一个根本没在跑
/// 的进程：一句用户查不下去的假话，比一次可能读错的数更难修（parking lot Q34）。所以取数照常去
/// 试那几条 HID Endpoint；而那件没问出来的事要说出来，不能咽掉。Q34 记下它时，这条规则还只活在
/// `status` 的 `run` 里、测不到，这一条就是为它写的。
#[test]
fn a_failed_process_enumeration_pauses_nothing_and_is_a_warning_of_this_round() {
    let general = default_general();
    let mouse = mouse_with_both_endpoints();
    let endpoints = FakeEndpoints::new(
        MOUSE_REPORT_ID,
        [(EndpointKind::Wired, vec![MOUSE_RESTING_FULL.to_vec()])],
    );

    let pause = PauseCheck::detect(&general, &FakeProcesses::failing());
    let in_hand = fetched(&mouse, &endpoints, pause.paused_by());
    let round = Round::assess(
        &general,
        [(&mouse, &in_hand)],
        None,
        pause.warning().into_iter().collect(),
        NOW,
    );

    assert_eq!(
        endpoints.opens(),
        [EndpointKind::Wired],
        "问不出来就当没在跑：Wired 照常打开"
    );
    assert_eq!(round.devices[0].icon_state, IconState::Normal, "不是暂停");
    let warnings: Vec<String> = round.warnings.iter().map(ToString::to_string).collect();
    assert_eq!(
        warnings,
        [
            "认不出本机在跑哪些进程，这一轮不暂停 —— 枚举不出本机在跑哪些进程，认不出厂商上位机: 假接缝这一次故意枚举不动"
        ]
    );
}

/// 问得出来就没有告警——撞见了上位机也一样：那是暂停，由那几台自己的图标状态说，不是出了什么事。
#[test]
fn asking_about_processes_successfully_leaves_no_warning() {
    let general = default_general();

    for processes in [FakeProcesses::new([]), FakeProcesses::new(["VGN VHUB.exe"])] {
        let pause = PauseCheck::detect(&general, &processes);
        assert_eq!(pause.warning(), None);
    }
}

/// 这一轮的读数写不进状态文件时那一条告警：丢的是下次启动时的上次已知值，不是这一轮的结果。
///
/// 写盘在这一轮之后，由写盘的那一方造出这一条（这个纯函数碰不到磁盘）；它与进程那一条是同一种
/// 东西，所以住在同一个类型里，说法也在这里定。
#[test]
fn a_state_file_that_could_not_be_written_is_a_warning_too() {
    let warning = Warning::StateNotSaved("写不进状态文件 C:/juicebar/state.toml".to_string());

    assert_eq!(
        warning.to_string(),
        "记不下这一轮的读数（下次启动就没有上次已知值了）—— 写不进状态文件 C:/juicebar/state.toml"
    );
}
