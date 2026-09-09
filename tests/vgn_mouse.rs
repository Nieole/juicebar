//! VGN 鼠标驱动的外部行为：往设备发了哪些字节，收到实测回包时产出什么 Reading。
//!
//! 全程经假 Transport，不接任何硬件。

mod common;

use common::FakeTransport;
use common::fixtures::{
    MOUSE_BATTERY_REQUEST, MOUSE_CHARGING, MOUSE_CMD3_RESPONSE, MOUSE_REPORT_ID, MOUSE_RESTING_95,
    MOUSE_RESTING_FULL,
};
use juicebar::sources::vgn_mouse;

/// 只发一帧 cmd 4，且那一帧和实测抓到的逐字节相同。
///
/// 一条断言同时守住四件事：命令码是 4（不是 3）、帧长 16、末字节是算出来的校验和
/// `0x49`、整个取数只有一次往返。校验和是这里最要紧的一个字节——键盘那条链路
/// 曾因为帧少一个校验字节而被设备静默丢弃，而 API 一路返回成功。
#[test]
fn sends_one_cmd_4_frame_that_matches_the_captured_bytes() {
    let transport = FakeTransport::new(MOUSE_REPORT_ID, [MOUSE_RESTING_FULL.to_vec()]);

    vgn_mouse::read_battery(&transport).unwrap();

    assert_eq!(transport.sent(), vec![MOUSE_BATTERY_REQUEST.to_vec()]);
}

#[test]
fn parses_a_resting_full_battery_frame() {
    let reading = read(&MOUSE_RESTING_FULL).unwrap();

    assert_eq!(reading.reported_level, 100);
    assert!(!reading.charging);
    assert_eq!(reading.voltage_mv, 4190);
}

#[test]
fn parses_a_resting_frame_below_full() {
    let reading = read(&MOUSE_RESTING_95).unwrap();

    assert_eq!(reading.reported_level, 95);
    assert!(!reading.charging);
    assert_eq!(reading.voltage_mv, 4158);
}

#[test]
fn parses_a_charging_frame() {
    let reading = read(&MOUSE_CHARGING).unwrap();

    assert_eq!(reading.reported_level, 95);
    assert!(reading.charging);
    assert_eq!(reading.voltage_mv, 4235);
}

/// 别的命令的应答不能被当成本次结果。
///
/// 这不是假想的情况：dongle 侧的缓冲区保存的是"最近一次应答"，不校验 cmd 回显就
/// 可能把上一条命令的结果当成电量。
#[test]
fn rejects_a_response_left_over_from_another_command() {
    let error = read(&MOUSE_CMD3_RESPONSE).unwrap_err().to_string();

    assert!(error.contains("cmd"), "错误信息要说清是哪一步不对：{error}");
}

/// 读到半截的回包不能被当成有效读数——`ReadFile` 会按真实读到的字节数截断。
#[test]
fn rejects_a_truncated_frame() {
    assert!(read(&[0x04, 0x00, 0x00]).is_err());
}

/// 校验和这一份实现是驱动与 `probe --vgn-crc` 共用的，所以直接对着它断言一次：
/// 一帧空的 cmd 4 补上校验和之后，应当与实测发出去的那一帧完全相同。
#[test]
fn applies_the_captured_checksum_to_a_bare_cmd_4_frame() {
    let mut frame = [0u8; vgn_mouse::FRAME_LEN];
    frame[0] = 4;
    // 构造时末字节先填占位 0xEF（见 docs/protocol.md），它不参与求和，覆盖掉即可。
    frame[vgn_mouse::FRAME_LEN - 1] = 0xEF;

    vgn_mouse::apply_checksum(&mut frame, MOUSE_REPORT_ID).unwrap();

    assert_eq!(frame, MOUSE_BATTERY_REQUEST);
}

/// `probe` 把用户手打的十六进制原样递进来，长度不对必须当场说清楚。
#[test]
fn refuses_to_checksum_a_frame_of_the_wrong_length() {
    let mut frame = [0u8; 8];

    assert!(vgn_mouse::apply_checksum(&mut frame, MOUSE_REPORT_ID).is_err());
}

/// 喂一帧回包，取一次数。
fn read(response: &[u8]) -> anyhow::Result<juicebar::sources::Reading> {
    let transport = FakeTransport::new(MOUSE_REPORT_ID, [response.to_vec()]);
    vgn_mouse::read_battery(&transport)
}
