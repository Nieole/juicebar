//! 协议驱动，一个协议家族一个模块。
//!
//! 驱动只面对 [`Transport`] 这一个方法，不直接碰 HID：两种协议的 I/O 差异
//! （output+input 还是 feature、哪个 Report ID、多长的超时）全部被吸收在 Transport
//! 的实现里，驱动那一侧看到的只是一来一回两串字节。这条接缝也是"不接硬件就能跑
//! 全部测试"的支点。

pub mod hid_transport;
pub mod vgn_mouse;

use anyhow::Result;

/// 包住一次到设备的字节往返。
pub trait Transport {
    /// 发一帧、收一帧。
    ///
    /// 入参和返回值都是**不含 Report ID 的帧**——Report ID 是这条通路的性质，
    /// 由实现自己补上和剥掉。这样驱动里的下标与 `docs/protocol.md` 直接对得上，
    /// 不必到处 +1（那是文档点名的"最容易一次性写错的地方"）。
    ///
    /// 失败即"这条 Endpoint 这一次没读到"：超时、设备已拔、通道被厂商上位机占着，
    /// 对调用方是同一件事。
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
    /// 设备正在充电。
    pub charging: bool,
    /// 电池电压，毫伏。
    pub voltage_mv: u16,
}
