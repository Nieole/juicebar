//! 实测抓到的真实字节。
//!
//! **不要手编样本。**这里每一帧都来自真硬件、CRC 都验算过，出处见
//! `docs/protocol.md`。手编的样本只能证明"实现符合我的想象"，而这些帧证明的是
//! "实现对着真设备是对的"——今晚整条键盘链路失败正是因为想象与设备不一致。
//!
//! 帧一律**不含 Report ID**：Transport 接缝两侧交换的就是这个视图，下标因此
//! 与 `docs/protocol.md` 里的下标直接对得上，不用 +1。

use juicebar::sources::vgn_keyboard::FRAME_LEN as KEYBOARD_FRAME_LEN;
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

/// 键盘那条通路的 Report ID。feature 报文不带编号，实测是 0。
pub const KEYBOARD_REPORT_ID: u8 = 0;

/// 键盘 `0xF7`（getDongleData）的请求帧。
///
/// 照抄 `docs/protocol.md`「实测记录」里那一行
/// `发送 [id 00] F7 00 00 00 00 00 00 08 00 …(补零到 64)`。末字节 `0x08` 是校验字节
/// （`255 − (0xF7 & 255)`）——**整条键盘链路曾经完全失效，就是因为发出去的帧少了它。**
pub const KEYBOARD_DONGLE_DATA_REQUEST: [u8; KEYBOARD_FRAME_LEN] =
    keyboard_frame(&[0xF7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08]);

/// 键盘 `0xF7` 回包：满电静置。就绪 / level 100 / 未充电。
///
/// 抓于充电那一轮**之后**，所以 `[3]` 是 `01`。就是票 02 里引的那一帧
/// （票写成含 Report ID 的 `00 01 64 00 01 01 01 01 …`）。
pub const KEYBOARD_RESTING_FULL: [u8; KEYBOARD_FRAME_LEN] =
    keyboard_frame(&[0x01, 0x64, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00]);

/// 同一台键盘、同样满电，但抓于充电那一轮**之前**：`[3]` 是 `00`。
///
/// `docs/protocol.md`「实测记录」记的是这一帧。两帧只差 `[3]` 一个字节，而它们解析出来
/// 的 Reading 必须完全相同——**`[3]` 不承重**（含义至今未知，见第 7 节）。谁要是哪天
/// 顺手拿它当标志位，这两条用例会一起变红。
pub const KEYBOARD_RESTING_FULL_BEFORE_CHARGE: [u8; KEYBOARD_FRAME_LEN] =
    keyboard_frame(&[0x01, 0x64, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00]);

/// 键盘 `0xF7` 回包：dongle 还没准备好。`[0]` 是 `00`。
///
/// 出处是票 02 点名的那一帧（票写成含 Report ID 的 `00 00 64 00 00 01 01 01 …`），
/// 也就是上面那帧充电前的回包把就绪位清成 0。**`[1]` 照样是 `0x64`**——这正是它要守的
/// 那件事：未就绪的帧里也躺着一个看起来很像 100% 的字节，采信它就等于凭空发明一个读数。
pub const KEYBOARD_NOT_READY: [u8; KEYBOARD_FRAME_LEN] =
    keyboard_frame(&[0x00, 0x64, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00]);

/// 键盘那条 feature 通路上实测抓到过的**别的命令的残留**：`00 | F4 01 F4 01 …`。
///
/// `docs/protocol.md` 第 7 节把它解释清楚了：小端 `0x01F4` = 500，落在回报率 / 睡眠 /
/// 防抖的取值范围，是 HUB 拉设备信息时留在共享缓冲区里的。开头那个 `00` 是 Report ID，
/// 所以按本文件的约定剥掉之后 `[0]` 是 `0xF4`——**它非零**。
///
/// 这一帧就是"共享缓冲区"这件事的实物证据：HUB 原文只判 `result[0] == 0` 就重试，
/// 非零一律当就绪，照着抄会把这一帧放行并读出一个 level `0x01` = 1%。
/// `…` 之后是什么没记下来，所以这里只放实测到的这四个字节。
pub const KEYBOARD_STALE_RESIDUE: [u8; KEYBOARD_FRAME_LEN] =
    keyboard_frame(&[0xF4, 0x01, 0xF4, 0x01]);

/// 把实测抓到的那几个字节补零成一整帧。
///
/// 键盘的帧是 64 字节，其中有意义的只有开头那几个，其余是 `sendCmd` 补的零。
/// 把 50 多个零抄进源码，只会把真正实测到的那几个字节淹掉。
const fn keyboard_frame(head: &[u8]) -> [u8; KEYBOARD_FRAME_LEN] {
    let mut frame = [0u8; KEYBOARD_FRAME_LEN];
    let mut i = 0;
    while i < head.len() {
        frame[i] = head[i];
        i += 1;
    }
    frame
}
