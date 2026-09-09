//! Transport 落到真 HID 上的实现，一种报文一个。
//!
//! output+input 那条是 VGN 鼠标走的；feature 那条是 VGN 键盘走的——它的 vendor
//! collection 输入输出长度都是 0（实测 `in:0 out:0 feat:65`），只有 feature 报文。
//! 见 `docs/protocol.md` 第 2、3 节。走哪一条由驱动的 `report_kind()` 说了算。

use anyhow::{Result, bail};

use crate::hid::{HidHandle, HidInfo};
use crate::sources::Transport;

/// 发一份输出报文、等一份输入报文。
pub struct OutputReportTransport {
    handle: HidHandle,
    report_id: u8,
    read_timeout_ms: u32,
}

impl OutputReportTransport {
    /// 打开一条 collection。超时由调用方给——它是协议的性质（鼠标 3000 ms），
    /// 不是 HID 的性质。
    pub fn open(info: &HidInfo, report_id: u8, read_timeout_ms: u32) -> Result<Self> {
        Ok(Self {
            handle: HidHandle::open(info)?,
            report_id,
            read_timeout_ms,
        })
    }
}

impl Transport for OutputReportTransport {
    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.handle.write_report(self.report_id, request)?;
        let mut buf = self.handle.read_report(self.read_timeout_ms)?;

        // 读写缓冲区的第 0 字节都是 Report ID。剥掉它，驱动看到的下标就与
        // docs/protocol.md 里的下标一致。
        let Some(&id) = buf.first() else {
            bail!("回包是空的");
        };
        if id != self.report_id {
            bail!(
                "回包是 Report ID {id:#04X} 的报文，不是这条通路的 {:#04X}",
                self.report_id
            );
        }
        Ok(buf.split_off(1))
    }

    fn report_id(&self) -> u8 {
        self.report_id
    }
}

/// 发一份 feature 报文，随即把它读回来。
///
/// 没有超时这一项：`HidD_SetFeature` / `HidD_GetFeature` 是同步 IOCTL，等多久由驱动
/// 说了算，调用方给不了。句柄也因此要单独用不带 `FILE_FLAG_OVERLAPPED` 的方式打开。
///
/// **这条通路上"调用成功"什么都不说明。**校验和错了的帧会被设备静默丢弃，而
/// `HidD_SetFeature` 照样返回成功、`HidD_GetFeature` 照样交回缓冲区里的陈旧残留且
/// 不报错。认不认这一帧，只能由驱动去看回读的内容——整条键盘链路曾经就栽在这里。
pub struct FeatureReportTransport {
    handle: HidHandle,
    report_id: u8,
}

impl FeatureReportTransport {
    /// 打开一条 collection。
    pub fn open(info: &HidInfo, report_id: u8) -> Result<Self> {
        Ok(Self {
            handle: HidHandle::open_sync(info)?,
            report_id,
        })
    }
}

impl Transport for FeatureReportTransport {
    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.handle.set_feature(self.report_id, request)?;
        let mut buf = self.handle.get_feature(self.report_id)?;

        // feature 缓冲区的第 0 字节同样是 Report ID。剥掉它，驱动看到的下标就与
        // docs/protocol.md 里的下标一致。
        let Some(&id) = buf.first() else {
            bail!("回读是空的");
        };
        if id != self.report_id {
            bail!(
                "回读是 Report ID {id:#04X} 的报文，不是这条通路的 {:#04X}",
                self.report_id
            );
        }
        Ok(buf.split_off(1))
    }

    fn report_id(&self) -> u8 {
        self.report_id
    }
}
