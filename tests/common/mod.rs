//! 测试之间共用的东西：假实现与实测夹具。
//!
//! **测试怎么组织**（这份是先例，后面的票照这个来）：
//!
//! - 测试都放在 `tests/` 下的集成测试里，一个被测模块一个文件。集成测试只看得见
//!   `pub` 接口，测不到私有函数——spec 要的"只测外部行为"因此由编译器守住，
//!   而不是靠自觉。`src/lib.rs` 的存在就是为了这个。
//! - 共用的东西放在这个目录：`common::fixtures` 是实测字节，`common` 本身放假实现。
//!   `tests/common/` 不会被当成独立的测试目标编译，用 `mod common;` 引进来即可。
//! - 用例名用 `CONTEXT.md` 的词（Device / Endpoint / Dongle24G / Reading /
//!   Reported Level …），读起来就是一句关于外部行为的话。

// 每个测试二进制只用得到这里的一部分，没用到的那些不是死代码。
#![allow(dead_code)]

pub mod fixtures;

use std::cell::RefCell;
use std::collections::VecDeque;

use anyhow::{Result, bail};
use juicebar::sources::Transport;

/// 假 Transport：按脚本交回预先排好的帧，并记下每一次发出去的帧。
///
/// 有了它，驱动的全部行为都能在不接任何硬件的情况下断言——往设备发了哪些字节，
/// 收到某一帧时产出什么 Reading。
pub struct FakeTransport {
    report_id: u8,
    responses: RefCell<VecDeque<Vec<u8>>>,
    sent: RefCell<Vec<Vec<u8>>>,
}

impl FakeTransport {
    /// 回包按给定顺序交出，用完之后再 `exchange` 就是一次失败（相当于超时）。
    pub fn new(report_id: u8, responses: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            report_id,
            responses: RefCell::new(responses.into_iter().collect()),
            sent: RefCell::new(Vec::new()),
        }
    }

    /// 至今发出去的每一帧，按顺序。
    pub fn sent(&self) -> Vec<Vec<u8>> {
        self.sent.borrow().clone()
    }
}

impl Transport for FakeTransport {
    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.sent.borrow_mut().push(request.to_vec());
        let next = self.responses.borrow_mut().pop_front();
        match next {
            Some(response) => Ok(response),
            None => bail!(
                "假 Transport 的回包用完了：第 {} 次 exchange 没有脚本",
                self.sent.borrow().len()
            ),
        }
    }

    fn report_id(&self) -> u8 {
        self.report_id
    }
}
