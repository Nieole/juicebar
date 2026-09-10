//! `vendor_hub` 的外部行为：本机在跑的进程里认不认得出厂商上位机，以及它暂停哪几条
//! Endpoint。
//!
//! 全程经假进程接缝，不去问真系统——那正是这条接缝存在的理由：一次进程枚举要碰真
//! Windows，而这个仓库的用例不接硬件也跑得通。`SystemProcesses` 那一半不测。
//!
//! 配置那两项的解析在 `tests/config.rs`；这里只断言 `detect` 怎么用它们。

mod common;

use common::{FakeProcesses, default_general};
use juicebar::config::Config;
use juicebar::endpoints::EndpointKind;
use juicebar::vendor_hub::VendorHub;

/// 名单里那个进程在跑，就该撞见它。
#[test]
fn spots_the_vendor_hub_by_the_configured_process_name() {
    let processes = FakeProcesses::new(["explorer.exe", "VGN VHUB.exe", "cargo.exe"]);

    let hub = VendorHub::detect(&default_general(), &processes).expect("枚举没有失败");

    assert_eq!(
        hub.expect("该撞见 VGN VHUB.exe").process(),
        "VGN VHUB.exe",
        "撞见的是哪个进程要说得出来——那句话是用户唯一能据以行动的东西"
    );
}

/// 名单里的写法和本机报的写法大小写不同，照样算撞见。
///
/// 这不是宽松：认不出来的症状是**暂停永远不触发**，而它和"HUB 没在跑"长得一模一样
/// ——没有任何东西会指向那个大小写。同一条理由在
/// `config::BluetoothEndpoint::matches`（MAC 的写法）上已经付过一次代价。
#[test]
fn spots_the_vendor_hub_however_the_process_name_is_capitalised() {
    let processes = FakeProcesses::new(["vgn vhub.exe"]);

    let hub = VendorHub::detect(&default_general(), &processes)
        .unwrap()
        .expect("大小写不同也该撞见");

    assert_eq!(
        hub.process(),
        "vgn vhub.exe",
        "印出来的是本机报的那个写法——用户要去任务管理器里找的是它"
    );
}

/// 名单里一个都没在跑，就什么都没撞见。
#[test]
fn spots_nothing_when_no_configured_process_is_running() {
    let processes = FakeProcesses::new(["explorer.exe", "cargo.exe"]);

    assert!(
        VendorHub::detect(&default_general(), &processes)
            .unwrap()
            .is_none()
    );
    assert_eq!(processes.enumerations(), 1, "开关开着就该真去问一次");
}

/// 开关关着时**连进程都不枚举**（票面第 6 条）。
///
/// 断言的是"没去问"，而不是"没暂停"：一次进程枚举不便宜，而开关关着就意味着用户说了
/// "别管这件事"。这两件事在产出上分不开——不枚举与枚举了但没撞见，交出来的都是 `None`。
#[test]
fn does_not_enumerate_processes_at_all_when_the_switch_is_off() {
    let switched_off = Config::parse(
        r#"
        [general]
        pause_when_vendor_hub_running = false
        "#,
    )
    .unwrap()
    .general;
    let processes = FakeProcesses::new(["VGN VHUB.exe"]);

    assert!(
        VendorHub::detect(&switched_off, &processes)
            .unwrap()
            .is_none(),
        "开关关着就不暂停，哪怕 HUB 真在跑"
    );
    assert_eq!(
        processes.enumerations(),
        0,
        "开关关着时一次都不该去枚举进程"
    );
}

/// 暂停的是两条 HID，`Ble` 照常（票面第 3 条）。
///
/// `Ble` 不参与那个竞争：它读的是 Windows 攒的属性缓存，压根不往设备发字节
/// （`CONTEXT.md` 的 `Ble` 条目）。而 HUB 同样驱动有线设备，所以 `Wired` 也得停
/// ——不是只停 `Dongle24G`。
#[test]
fn pauses_the_two_hid_endpoints_and_leaves_ble_alone() {
    let hub = VendorHub::detect(&default_general(), &FakeProcesses::new(["VGN VHUB.exe"]))
        .unwrap()
        .unwrap();

    assert!(hub.pauses(EndpointKind::Wired));
    assert!(hub.pauses(EndpointKind::Dongle24G));
    assert!(!hub.pauses(EndpointKind::Ble));
}

/// 问不出来时交 `Err`，**不悄悄当成"没在跑"**。
///
/// 两件事的处置不同：一个都没在跑什么都不必说，而问不出来要作为这一轮的一条告警说出来、然后
/// 照常取数（parking lot Q34 定的 fail open，见 `tests/round.rs`）。压成同一个 `None`，一台
/// Win32 调用失败的机器就会永远不暂停、而且一个字都不说。
#[test]
fn tells_a_failed_enumeration_apart_from_nothing_running() {
    let processes = FakeProcesses::failing();

    let failed = VendorHub::detect(&default_general(), &processes).unwrap_err();

    assert!(
        format!("{failed:#}").contains("认不出厂商上位机"),
        "那句话要说清这一次为什么没有答案：{failed:#}"
    );
    assert_eq!(processes.enumerations(), 1, "它确实去问过");
}
