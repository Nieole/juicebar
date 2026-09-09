//! VGN 鼠标驱动的外部行为：往设备发了哪些字节，收到实测回包时产出什么 Reading。
//!
//! 全程经假 Transport，不接任何硬件。

mod common;

use common::FakeTransport;
use common::fixtures::{
    MOUSE_BATTERY_REQUEST, MOUSE_CHARGING, MOUSE_CMD3_RESPONSE, MOUSE_REPORT_ID, MOUSE_RESTING_95,
    MOUSE_RESTING_FULL, mouse_frame_with,
};
use juicebar::sources::{ReportKind, driver_for, vgn_mouse};

/// 配置里写 `driver = "vgn_mouse"`，取到的就是鼠标那套协议，而且它自己说要走
/// output + input 报文、等 3000 ms。
///
/// 超时是**协议**的性质而不是 HID 的：cmd 4 要走一次到鼠标的空中往返，实测 1000 ms
/// 内超时过。这里写死 3000 而不是引那个常量——引常量的断言永远不会不同意实现。
#[test]
fn is_reachable_by_its_config_name_and_asks_for_output_reports() {
    let driver = driver_for("vgn_mouse").expect("配置里的驱动名认不出来");
    let transport = FakeTransport::new(MOUSE_REPORT_ID, [MOUSE_RESTING_FULL.to_vec()]);

    let reading = driver.read_battery(&transport).unwrap();

    assert_eq!(
        driver.report_kind(),
        ReportKind::OutputAndInput {
            read_timeout_ms: 3000
        }
    );
    assert_eq!(reading.reported_level, 100);
    assert_eq!(transport.sent(), vec![MOUSE_BATTERY_REQUEST.to_vec()]);
}

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
    assert_eq!(reading.charging, Some(false));
    assert_eq!(reading.voltage_mv, Some(4190));
}

#[test]
fn parses_a_resting_frame_below_full() {
    let reading = read(&MOUSE_RESTING_95).unwrap();

    assert_eq!(reading.reported_level, 95);
    assert_eq!(reading.charging, Some(false));
    assert_eq!(reading.voltage_mv, Some(4158));
}

#[test]
fn parses_a_charging_frame() {
    let reading = read(&MOUSE_CHARGING).unwrap();

    assert_eq!(reading.reported_level, 95);
    assert_eq!(reading.charging, Some(true));
    assert_eq!(reading.voltage_mv, Some(4235));
}

/// 别的命令的应答不能被当成本次结果。
///
/// 这不是假想的情况：dongle 侧的缓冲区保存的是"最近一次应答"，不校验 cmd 回显就
/// 可能把上一条命令的结果当成电量。
///
/// **它必须因为 cmd 回显不对而红/绿，不能因为校验和不过。**票 03 加上校验和校验之后
/// 这条用例有了第二种变绿的方式，而那一种守的是另一件事。夹具那一帧的末字节是实测
/// 真值 `0xA9`（`docs/protocol.md` 逐帧验算过），所以它过得了校验和、只倒在 cmd 回显上
/// ——下面那条 `!contains("校验和")` 就是把这件事钉住的钉子。
#[test]
fn rejects_a_response_left_over_from_another_command() {
    let error = read(&MOUSE_CMD3_RESPONSE).unwrap_err().to_string();

    assert!(error.contains("cmd"), "错误信息要说清是哪一步不对：{error}");
    assert!(
        !error.contains("校验和"),
        "这一帧的校验和是实测真值，该倒在 cmd 回显上而不是校验和上：{error}"
    );
}

/// 校验和对不上的回包不是有效读数。
///
/// **回包也带 CRC，算法与请求相同**（`0x55 − sum − reportId`，`docs/protocol.md`
/// 第 1 节逐帧验算过）。所以"这一帧是不是我们以为的那一帧"协议自己留了字节可以回答
/// ——而 cmd 回显只认得出"这是别的命令的应答"，认不出"这一帧被写坏了"。
///
/// 这里不新增夹具：拿实测的好帧翻掉末字节，得到的正是一帧被写坏的真帧。
#[test]
fn rejects_a_frame_whose_checksum_does_not_add_up() {
    let mut broken = MOUSE_RESTING_FULL;
    broken[vgn_mouse::FRAME_LEN - 1] ^= 0xFF;

    let error = read(&broken).unwrap_err().to_string();

    assert!(
        error.contains("校验和"),
        "错误信息要说清是哪一步不对：{error}"
    );
}

/// 帧完整、cmd 回显也对，但里头的电压物理上不可能——照样是读取异常。
///
/// 这是**固件漂移**的形状：厂商推一版新固件把某个字段挪了位，帧仍然完整、CRC 仍然
/// 验通、cmd 回显仍然是 4，而按老下标取出来的仍然是一串看着像数字的字节。校验和与
/// cmd 回显都拦不住它，只有值域能。
///
/// 造帧的办法见 `fixtures::mouse_frame_with`：实测好帧只改两个字段、校验和由被测代码
/// 自己补上，所以这一帧在结构上无可指摘，只有那个数说不通。
#[test]
fn rejects_a_frame_whose_voltage_is_impossible_even_though_it_is_intact() {
    let error = read(&mouse_frame_with(100, 2000)).unwrap_err().to_string();

    assert!(
        error.contains("读取异常") && error.contains("2000"),
        "错误信息要说清是哪个数说不通：{error}"
    );
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
