//! VGN 鼠标（Dragonfly 3 Master+）的 2.4G 私有协议。细节见 `docs/protocol.md` 第 1 节。

use anyhow::{Result, anyhow, bail};

use crate::sources::{Driver, Reading, ReportKind, Transport, require_frame_len};

/// VGN 鼠标这一族的驱动。配置里写 `driver = "vgn_mouse"` 取到的就是它。
pub struct VgnMouse;

impl Driver for VgnMouse {
    fn report_kind(&self) -> ReportKind {
        ReportKind::OutputAndInput {
            read_timeout_ms: READ_TIMEOUT_MS,
        }
    }

    fn read_battery(&self, transport: &dyn Transport) -> Result<Reading> {
        read_battery(transport)
    }
}

/// 一帧的长度，不含 Report ID。
pub const FRAME_LEN: usize = 16;

/// 读超时。
///
/// **不能取 1000 ms。**cmd 4 要走一次到鼠标的空中往返，实测 1000 ms 内超时过；
/// 改 3000 ms 后每次都首读命中。这不是留余量，是实测值。
pub const READ_TIMEOUT_MS: u32 = 3000;

/// BatteryLevel。
///
/// **只发这一条。**HUB 的做法是先发 cmd 3（DeviceOnLine）问在线再问电量，juicebar
/// 不照抄：cmd 4 回不回有效帧本身就蕴含在线与否，而 cmd 3 的 `[5]` 语义存疑
/// （实测更像充电标志而不是在线标志）。少一次空中往返，也少一个偏移存疑的字段。
const CMD_BATTERY_LEVEL: u8 = 4;

/// 取一次电量。
pub fn read_battery(transport: &dyn Transport) -> Result<Reading> {
    let request = battery_request(transport.report_id());
    let response = transport.exchange(&request)?;
    parse_battery(&response)
}

/// 拼一帧 cmd 4：命令码在 `[0]`，不带参数所以中间全零，校验和落在末字节。
fn battery_request(report_id: u8) -> [u8; FRAME_LEN] {
    let mut frame = [0u8; FRAME_LEN];
    frame[0] = CMD_BATTERY_LEVEL;
    let crc = checksum(&frame, report_id);
    frame[FRAME_LEN - 1] = crc;
    frame
}

/// 解析 cmd 4 的回包。
fn parse_battery(frame: &[u8]) -> Result<Reading> {
    require_frame_len(frame, FRAME_LEN)?;
    // dongle 侧交回来的可能是别的命令留下的应答，不校验 cmd 回显就会把它当成电量。
    if frame[0] != CMD_BATTERY_LEVEL {
        bail!(
            "回包的 cmd 回显是 {}，不是 {CMD_BATTERY_LEVEL} —— 这一帧不是本次请求的应答",
            frame[0]
        );
    }
    Ok(Reading {
        reported_level: frame[5],
        // 鼠标这一位已校准：实测插线时 `[6]` 由 `00` 翻为 `01`。
        charging: Some(frame[6] != 0),
        voltage_mv: Some(u16::from_be_bytes([frame[7], frame[8]])),
    })
}

/// 把校验字节写进帧末字节。
///
/// 驱动与 `probe --vgn-crc` 共用这一份——两处各存一份校验和，迟早会有一处
/// 悄悄改错，而校验和错了的帧会被设备**静默丢弃**，看起来和"设备没反应"一模一样。
pub fn apply_checksum(frame: &mut [u8], report_id: u8) -> Result<()> {
    let len = frame.len();
    let frame: &mut [u8; FRAME_LEN] = frame
        .try_into()
        .map_err(|_| anyhow!("VGN 鼠标帧必须是 {FRAME_LEN} 字节，当前 {len}"))?;
    let crc = checksum(frame, report_id);
    frame[FRAME_LEN - 1] = crc;
    Ok(())
}

/// `crc = 0x55 − sum(帧[0..15]) − report_id`。求和**不含**末字节。
///
/// 全程 u8 回绕：原始 JS 依赖 `Uint8Array` 赋值的截断行为。
fn checksum(frame: &[u8; FRAME_LEN], report_id: u8) -> u8 {
    let sum = frame[..FRAME_LEN - 1]
        .iter()
        .fold(0u8, |acc, b| acc.wrapping_add(*b));
    0x55u8.wrapping_sub(sum).wrapping_sub(report_id)
}
