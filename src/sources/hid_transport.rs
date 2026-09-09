//! Transport 落到真 HID 上的实现。
//!
//! 目前只有 output+input 那条路（VGN 鼠标走的）。走 feature 报文的那条是另一个
//! 实现——键盘的 vendor collection 输入输出长度都是 0，只有 feature 报文，
//! 见 `docs/protocol.md` 第 2 节。

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
