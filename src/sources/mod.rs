//! 协议驱动，一个协议家族一个模块。
//!
//! 驱动只面对 [`Transport`] 这一个方法，不直接碰 HID：两种协议的 I/O 差异
//! （output+input 还是 feature、哪个 Report ID、多长的超时）全部被吸收在 Transport
//! 的实现里，驱动那一侧看到的只是一来一回两串字节。这条接缝也是"不接硬件就能跑
//! 全部测试"的支点。
//!
//! [`level`] 不是协议家族，是这一侧的另一半：驱动把字节变成 [`Reading`]，`level`
//! 回答"这份 Reading 可信吗、要显示的百分比取自哪里"——两问对每个家族都同一个答案。

pub mod hid_transport;
pub mod level;
pub mod vgn_keyboard;
pub mod vgn_mouse;

use anyhow::Result;

/// 一个协议家族的驱动。
///
/// **一个驱动覆盖该家族的所有 Endpoint 种类**（spec「模块划分」）：键盘的 `Wired` 与
/// `Dongle24G` 命令不同，鼠标两者相同、分支自然退化。这个 trait 目前不带 Endpoint 种类
/// 参数——键盘有线本体至今没实测过（spec 明确列为 Out of Scope），现在猜一个参数形状，
/// 猜的是没人验过的那一半。
///
/// 协议逻辑仍然写成各模块里的自由函数，trait 这一层只做分发：驱动自己的测试对着自由
/// 函数断言，不必绕 `dyn`；而取数那一层（`crate::readout`）拿到的是一个 `&dyn Driver`。
pub trait Driver {
    /// 这个家族的 HID Endpoint 走哪种报文。
    fn report_kind(&self) -> ReportKind;

    /// 取一次电量。
    fn read_battery(&self, transport: &dyn Transport) -> Result<Reading>;
}

/// 一个协议家族的 HID Endpoint 走哪种报文。
///
/// 这是**协议家族**的性质而不是某台机器的性质，所以由驱动来答、不进配置：配置里已经
/// 有一个 `driver` 字段说明了这件事，再写一遍就多一处会走样的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// output 报文发命令、input 报文收回包。VGN 鼠标走这条。
    OutputAndInput {
        /// 等 input 回包的毫秒数。它是**协议**的性质（鼠标 3000 ms），不是 HID 的。
        read_timeout_ms: u32,
    },
    /// feature 报文一发一读。VGN 键盘走这条——它的 vendor collection 实测
    /// `in:0 out:0 feat:65`，output 报文根本发不出去。
    Feature,
}

/// 按配置里的 `driver` 字符串取到驱动。
///
/// 全项目唯一一处把驱动名映射到实现的地方：接一个新协议家族，就是加一个模块加这里
/// 一行。认不出来给 `None`——配置里可以出现本次编译还没实现的驱动名，那该是取数时
/// 那一行写着"尚未实现"，而不是让整份配置读不动。
pub fn driver_for(name: &str) -> Option<&'static dyn Driver> {
    match name {
        "vgn_mouse" => Some(&vgn_mouse::VgnMouse),
        "vgn_keyboard" => Some(&vgn_keyboard::VgnKeyboard),
        _ => None,
    }
}

/// 包住一次到设备的字节往返。
pub trait Transport {
    /// 发一帧、收一帧。
    ///
    /// 入参和返回值都是**不含 Report ID 的帧**——Report ID 是这条通路的性质，
    /// 由实现自己补上和剥掉。这样驱动里的下标与 `docs/protocol.md` 直接对得上，
    /// 不必到处 +1（那是文档点名的"最容易一次性写错的地方"）。
    ///
    /// 失败即"这条 Endpoint 这一次没交出读数"：超时、设备已拔、通道被厂商上位机占着，
    /// 对调用方是同一件事。**要分开的只有回来了、却不可采信的那一帧**：实现把它包成
    /// [`BadFrame`]，取数那一层靠它把读取异常与失联分开。
    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>>;

    /// 这条通路用的 Report ID。
    ///
    /// 之所以要暴露出来：VGN 鼠标的校验和是 `0x55 − sum − report_id`，报文编号
    /// 参与了协议本身的运算。让驱动向 Transport 问，好过把同一个数从配置里取两遍
    /// 再指望两处不走样。
    fn report_id(&self) -> u8;
}

/// 从某一条 Endpoint 上取到的一次结果。
///
/// 目前只有驱动能从一帧里解析出来的三个字段。取得时刻由 Clock 接缝补上、来自哪条
/// Endpoint 由 Endpoint 合成那一步补上，都不在最小链路里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// 设备固件自己上报的电量百分比，即 Reported Level。
    ///
    /// **不要把它当成最终要显示的数字**：项目里还有一个由电压查表算出的
    /// Derived Level，两者是独立的两个数，可能不一致。选哪一个是另一步的事。
    pub reported_level: u8,
    /// 设备正在充电。**不是每个协议都答得上这一项**——键盘那一位至今没有实测样本。
    ///
    /// `None` 是"这条协议现在说不出充电与否"，不是"没在充电"：spec 明写着
    /// 「键盘暂不显示充电态」，而把没验过的位当成 `false` 交出去，下游就再也分不清
    /// "确实没充电"和"我不知道"了。
    pub charging: Option<bool>,
    /// 电池电压，毫伏。**不是每个协议都给这一项**——键盘的回包里就没有。
    ///
    /// `None` 与"0 mV"必须分开：合理性校验的电压值域是 3050..=4350，拿 0 顶上去
    /// 会把每一条键盘读数判成异常；而 Derived Level 拿 0 mV 查表会算出一个理直气壮
    /// 的 0%。两样都正是这个项目一直在防的"看着合理但其实是编的数字"。
    pub voltage_mv: Option<u16>,
}

/// 回包短于一整帧就不是有效读数——`ReadFile` 会按真实读到的字节数截断，而驱动接下来
/// 就要按固定下标取值。两个驱动共用这一句，帧长各自不同。
pub fn require_frame_len(frame: &[u8], frame_len: usize) -> Result<()> {
    if frame.len() < frame_len {
        anyhow::bail!("回包只有 {} 字节，不足一帧的 {frame_len} 字节", frame.len());
    }
    Ok(())
}

/// 设备答了话，而回来的这一帧不可采信——**读取异常**（`CONTEXT.md`「读取异常」）。
///
/// 驱动与 Transport 把这一种错包成它，取数那一层（`crate::readout`）按**类型**认出来，把取数失败
/// 分成读取异常与失联：菜单那一行的短原因两者不是同一句，而认那句话里的字是不行的
/// （`docs/adr/0004`）。**没包它的错一律算去问了、问不到。**
///
/// **包在哪儿**：设备答了话之后才发现不对的那一步。两个驱动都包在解析回包那一步外面——帧不足长、
/// 校验和对不上、应答不是本次请求的、字段落在物理上不可能的值域外，全在那一步里，往那一步里加一道
/// 校验也自动算上；两个 HID Transport 包的是回包空着、Report ID 不是这条通路的那两句。打不开通路、
/// 超时、dongle 连着未就绪，都是没答话，不包。
///
/// **印出去一个字不变**：包上它之后，完整原因逐字节还是原来那一句。
#[derive(Debug)]
pub struct BadFrame(pub anyhow::Error);

impl std::fmt::Display for BadFrame {
    /// 连错误链一起印里面那个错误，自己不加一个字。`source` 因此不交出它：交出去的话，外层的
    /// `{:#}` 会把那条链再印一遍。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for BadFrame {}
