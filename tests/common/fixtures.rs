//! 实测抓到的真实字节。
//!
//! **不要手编样本。**这里每一帧都来自真硬件、CRC 都验算过，出处见
//! `docs/protocol.md`。手编的样本只能证明"实现符合我的想象"，而这些帧证明的是
//! "实现对着真设备是对的"——今晚整条键盘链路失败正是因为想象与设备不一致。
//!
//! 帧一律**不含 Report ID**：Transport 接缝两侧交换的就是这个视图，下标因此
//! 与 `docs/protocol.md` 里的下标直接对得上，不用 +1。

use juicebar::sources::vgn_mouse::FRAME_LEN;

/// 鼠标那条通路的 Report ID。校验和算法里减的就是它。
pub const MOUSE_REPORT_ID: u8 = 8;

/// 鼠标 cmd 4（BatteryLevel）的请求帧。
///
/// 末字节 `0x49` 是实测发出去的校验字节（`0x55 − 0x04 − 0x08`）。整帧照抄
/// `docs/protocol.md`「实测记录」里那一行 `发送 [id 08] 04 00 … 49`。
pub const MOUSE_BATTERY_REQUEST: [u8; FRAME_LEN] = [
    0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49,
];

/// 鼠标 cmd 4 回包：拔线静置、刚充满。level 100 / 未充电 / 4190 mV。
pub const MOUSE_RESTING_FULL: [u8; FRAME_LEN] = [
    0x04, 0x00, 0x00, 0x00, 0x02, 0x64, 0x00, 0x10, 0x5E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x75,
];

/// 鼠标 cmd 4 回包：拔线静置。level 95 / 未充电 / 4158 mV。
///
/// 这一帧和上面那一帧是同一只鼠标充电前后的对照：电压涨了 32 mV，固件自报的
/// Reported Level 跟着从 95 变成 100。
pub const MOUSE_RESTING_95: [u8; FRAME_LEN] = [
    0x04, 0x00, 0x00, 0x00, 0x02, 0x5F, 0x00, 0x10, 0x3E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x9A,
];

/// 鼠标 cmd 4 回包：插着线充电中。level 95 / 充电中 / 4235 mV。
///
/// 4235 mV 高于静置电压是物理上应该的，也正因为它，合理性校验的电压上界不能取 4110。
pub const MOUSE_CHARGING: [u8; FRAME_LEN] = [
    0x04, 0x00, 0x00, 0x00, 0x02, 0x5F, 0x01, 0x10, 0x8B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4C,
];

/// 鼠标 cmd 3（DeviceOnLine）的回包，用来验证"别的命令的应答"不会被当成本次结果。
///
/// 文档里这一帧的可见部分记到 `03 00 00 00 01 00 35 D4 97 …`，末字节另有明记：
/// 「cmd 3 的回包同样验通（`0xA4→0xA9`）」。
///
/// **末字节必须是真的 `0xA9`。**票 03 会加上"校验和一致"的校验；那时若这里是编的，
/// `rejects_a_response_left_over_from_another_command` 会因为校验和不过而变绿，
/// 而不是因为 cmd 回显不对——它宣称守的那件事就没人守了。
pub const MOUSE_CMD3_RESPONSE: [u8; FRAME_LEN] = [
    0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x35, 0xD4, 0x97, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA9,
];
