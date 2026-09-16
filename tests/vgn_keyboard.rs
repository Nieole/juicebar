//! VGN 键盘驱动的外部行为：往设备发了哪些字节，收到实测回包时产出什么 Reading。
//!
//! 全程经假 Transport，不接任何硬件。

mod common;

use common::FakeTransport;
use common::fixtures::{
    KEYBOARD_CAPTURED_98, KEYBOARD_DONGLE_DATA_REQUEST, KEYBOARD_NOT_READY, KEYBOARD_REPORT_ID,
    KEYBOARD_RESTING_FULL, KEYBOARD_RESTING_FULL_BEFORE_CHARGE, KEYBOARD_STALE_RESIDUE,
};
use juicebar::sources::{ReportKind, driver_for, vgn_keyboard};

/// 配置里写 `driver = "vgn_keyboard"`，取到的就是键盘那套协议，而且它自己说要走
/// feature 报文。
///
/// 报文种类必须由驱动来答：键盘的 vendor collection 实测 `in:0 out:0 feat:65`，
/// 拿"能发 output 报文"的条件去找它会一条都找不到，报出来的是一句误导人的"设备没插"。
#[test]
fn is_reachable_by_its_config_name_and_asks_for_feature_reports() {
    let driver = driver_for("vgn_keyboard").expect("配置里的驱动名认不出来");
    let transport = FakeTransport::new(KEYBOARD_REPORT_ID, [KEYBOARD_RESTING_FULL.to_vec()]);

    let reading = driver.read_battery(&transport).unwrap();

    assert_eq!(driver.report_kind(), ReportKind::Feature);
    assert_eq!(reading.reported_level, 100);
    assert_eq!(
        transport.sent(),
        vec![KEYBOARD_DONGLE_DATA_REQUEST.to_vec()]
    );
}

/// 拼出来的 `0xF7` 帧和实测发出去的那一帧逐字节相同。
///
/// 一条断言同时守住四件事：命令码是 `0xF7`（不是有线本体那条路的 `0x82`）、末位那个
/// 校验字节在（`255 − (sum & 255)` = `0x08`）、余下补零、整帧 64 字节。
/// **校验字节是这里最要紧的一个**——整条键盘链路曾经完全失效，根因就是帧少了它，
/// 而 `HidD_SetFeature` 照样返回成功。
#[test]
fn builds_the_captured_dongle_data_frame_checksum_and_all() {
    let frame = vgn_keyboard::build_frame(&[0xF7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]).unwrap();

    assert_eq!(frame, KEYBOARD_DONGLE_DATA_REQUEST);
}

/// 一帧长到放不下校验字节，必须当场判成构造错误。
///
/// 这条守的是那个根因本身：宁可一个字节都不发，也不发一帧没有校验和的出去——
/// 那样的帧会被设备静默丢弃，而随后的 `HidD_GetFeature` 会交回一份看起来很正常的
/// 陈旧残留，整条链路"看起来在工作"地全错。
#[test]
fn refuses_to_build_a_frame_with_no_room_for_the_checksum() {
    assert!(vgn_keyboard::build_frame(&[0u8; vgn_keyboard::FRAME_LEN]).is_err());
}

#[test]
fn parses_a_resting_full_battery_frame() {
    let reading = read(&KEYBOARD_RESTING_FULL).unwrap();

    assert_eq!(reading.reported_level, 100);
    // 这两项键盘都答不上来，而"答不上来"不等于 0 mV、也不等于"没在充电"：
    // 充电位 `[9]` 至今没有实测样本，spec 因此明写「键盘暂不显示充电态」。
    assert_eq!(reading.charging, None);
    assert_eq!(reading.voltage_mv, None);
}

/// 充电那一轮之前抓的同一台键盘：只有 `[3]` 不同，Reading 必须一模一样。
#[test]
fn reads_the_same_battery_from_the_frame_captured_before_the_charge_round() {
    let before = read(&KEYBOARD_RESTING_FULL_BEFORE_CHARGE).unwrap();

    assert_eq!(before, read(&KEYBOARD_RESTING_FULL).unwrap());
}

/// 就绪标志为 0 的回包不是读数，是"还没准备好"：重发，等下一帧。
///
/// 这里同时守住"重发的是同一条 `0xF7`"——取一次数发出去的每一帧都得带校验和，
/// 重试路径上少一个字节和首发路径上少一个字节，后果一模一样。
#[test]
fn retries_instead_of_trusting_a_not_ready_frame() {
    let transport = FakeTransport::new(
        KEYBOARD_REPORT_ID,
        [KEYBOARD_NOT_READY.to_vec(), KEYBOARD_RESTING_FULL.to_vec()],
    );

    let reading = vgn_keyboard::read_battery(&transport).unwrap();

    assert_eq!(reading.reported_level, 100);
    assert_eq!(
        transport.sent(),
        vec![KEYBOARD_DONGLE_DATA_REQUEST.to_vec(); 2]
    );
}

/// dongle 一直不就绪，就得说读不到——绝不把未就绪帧里那个 `0x64` 当成 100% 交出去。
#[test]
fn refuses_to_read_a_battery_out_of_a_frame_that_is_never_ready() {
    let transport = FakeTransport::new(KEYBOARD_REPORT_ID, vec![KEYBOARD_NOT_READY.to_vec(); 16]);

    let error = vgn_keyboard::read_battery(&transport)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("未就绪"),
        "错误信息要说清是哪一步不对：{error}"
    );
}

/// 别的命令留在共享缓冲区里的残留不能被当成本次结果。
///
/// 这不是假想：那块 feature 缓冲区保存的是"最近一次应答"，而实测确实抓到过一帧
/// `F4 01 F4 01 …`。**`0xF7` 的回包里没有 cmd 回显可以对**（`[0]` 是就绪标志），
/// 所以挡住它的只能是就绪标志本身——照 HUB 那样"非零即就绪"，这一帧会被放行，
/// 读出一个理直气壮的 1%。
#[test]
fn rejects_a_response_left_over_from_another_command() {
    let error = read(&KEYBOARD_STALE_RESIDUE).unwrap_err().to_string();

    assert!(
        error.contains("就绪标志"),
        "错误信息要说清是哪一步不对：{error}"
    );
}

/// 就绪标志过了，但 `[1]` 里的百分比物理上不可能——照样是读取异常。
///
/// 键盘这条通路上能用来否决一帧的字节最少：回包**既没有 cmd 回显也没有校验和**
/// （parking lot Q9），就绪标志是唯一的结构性判据。所以值域这一道不是重复劳动，它是
/// 就绪标志之外仅剩的一道——一帧残留只要 `[0]` 恰好是 `0x01` 就能走到这里。
///
/// 这一帧是拿实测的好帧改掉 `[1]` 造出来的，其余字节一个没动。
#[test]
fn rejects_a_ready_frame_whose_level_is_impossible() {
    let mut drifted = KEYBOARD_RESTING_FULL;
    drifted[1] = 200;

    let error = read(&drifted).unwrap_err().to_string();

    assert!(
        error.contains("读取异常") && error.contains("200"),
        "错误信息要说清是哪个数说不通：{error}"
    );
}

/// 读到半截的回包不能被当成有效读数——更不能让驱动按下标越界崩掉。
#[test]
fn rejects_a_truncated_frame() {
    assert!(read(&[0x01, 0x64, 0x00]).is_err());
}

/// 2026-09-16 抓包实测回包：首字节 0x00，电量 98%，必须成功解析。
#[test]
fn parses_captured_real_dongle_frame_with_zero_header() {
    let reading = read(&KEYBOARD_CAPTURED_98).unwrap();
    assert_eq!(reading.reported_level, 98);
    assert_eq!(reading.charging, None);
    assert_eq!(reading.voltage_mv, None);
}

/// 喂一帧回包，取一次数。
fn read(response: &[u8]) -> anyhow::Result<juicebar::sources::Reading> {
    let transport = FakeTransport::new(KEYBOARD_REPORT_ID, [response.to_vec()]);
    vgn_keyboard::read_battery(&transport)
}
