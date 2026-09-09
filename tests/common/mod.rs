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
use std::rc::Rc;

use anyhow::{Result, bail};
use juicebar::config::Device;
use juicebar::endpoints::{EndpointKind, Endpoints};
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

/// 假枚举接缝：按脚本说哪些 Endpoint 在场，并给每一条备一个假 Transport。
///
/// 有了它，"拔线 → Wired 从枚举里消失 → 同周期降级到 Dongle24G"这条路径不用真的
/// 拔一次线就能断言——那是实测确认过、也最容易写错的行为。
///
/// 它**故意按给定的顺序原样交出在场的 Endpoint**，不替谁排序：优先级住在取数那一步，
/// 不住在这里，所以用例可以把顺序倒过来，验证取数没有在偷懒地信任枚举给的顺序。
pub struct FakeEndpoints {
    present: Vec<(EndpointKind, Rc<FakeTransport>)>,
}

impl FakeEndpoints {
    /// 在场的 Endpoint，每一条带一份回包脚本。脚本为空即这条 Endpoint 读不到
    /// （相当于超时）。
    ///
    /// `report_id` 是每条假 Transport 用的报文编号——它参与鼠标的校验和运算，
    /// 所以断言"发出去的帧与实测逐字节相同"时它必须是真的那一个。
    pub fn new(
        report_id: u8,
        present: impl IntoIterator<Item = (EndpointKind, Vec<Vec<u8>>)>,
    ) -> Self {
        Self {
            present: present
                .into_iter()
                .map(|(kind, responses)| (kind, Rc::new(FakeTransport::new(report_id, responses))))
                .collect(),
        }
    }

    /// 某条 Endpoint 上那个假 Transport，用来断言往它发了哪些帧——
    /// "Wired 在场时不去打扰 Dongle24G"就靠它的 `sent()` 是空的。
    pub fn transport(&self, endpoint: EndpointKind) -> Rc<FakeTransport> {
        self.find(endpoint)
            .unwrap_or_else(|| panic!("{endpoint} 不在这份假枚举里"))
    }

    fn find(&self, endpoint: EndpointKind) -> Option<Rc<FakeTransport>> {
        self.present
            .iter()
            .find(|(kind, _)| *kind == endpoint)
            .map(|(_, transport)| transport.clone())
    }
}

impl Endpoints for FakeEndpoints {
    fn present(&self, _device: &Device) -> Vec<EndpointKind> {
        self.present.iter().map(|(kind, _)| *kind).collect()
    }

    fn open_transport(
        &self,
        _device: &Device,
        endpoint: EndpointKind,
    ) -> Result<Box<dyn Transport>> {
        match self.find(endpoint) {
            Some(transport) => Ok(Box::new(SharedTransport(transport))),
            None => bail!("{endpoint} 不在场"),
        }
    }
}

/// 交出去一份、自己手里还留着一份的 [`FakeTransport`]。
///
/// `open_transport` 要把 Transport 的所有权交出去，而用例还得回头问它发了哪些帧，
/// 所以两边共享同一个。孤儿规则不允许直接给 `Rc<FakeTransport>` 实现 `Transport`，
/// 套一层就够了。
struct SharedTransport(Rc<FakeTransport>);

impl Transport for SharedTransport {
    fn exchange(&self, request: &[u8]) -> Result<Vec<u8>> {
        self.0.exchange(request)
    }

    fn report_id(&self) -> u8 {
        self.0.report_id()
    }
}

/// 用例里那台键盘：只有 Dongle24G 一条 Endpoint（有线本体要拨机身开关才枚举得出来），
/// 实测的 VID/PID。
///
/// **故意不写 `level_source`**，好让"缺省是 auto"和"键盘没有电压所以 auto 退化成用
/// 固件自报值"这两件事在同一条用例里一起被走到。
pub fn keyboard_with_dongle_endpoint() -> Device {
    juicebar::config::Config::parse(
        r#"
        [[device]]
        id = "neon75"
        name = "VGN Neon75"
        driver = "vgn_keyboard"

          [device.wireless_24g]
          vid = 0x3151
          pid = 0x5038
          usage_page = 0xFFFF
          usage = 0x0002
          report_id = 0
        "#,
    )
    .expect("用例里的配置应当解析得动")
    .devices
    .remove(0)
}

/// 用例里那只鼠标：两条 HID Endpoint 都配齐了，实测的 VID/PID。
pub fn mouse_with_both_endpoints() -> Device {
    juicebar::config::Config::parse(
        r#"
        [[device]]
        id = "dragonfly3"
        name = "Dragonfly 3 Master+"
        driver = "vgn_mouse"

          [device.wired]
          vid = 0x391D
          pid = 0x1005
          usage_page = 0xFF02
          usage = 0x0002
          report_id = 8

          [device.wireless_24g]
          vid = 0x391D
          pid = 0x1A05
          usage_page = 0xFF02
          usage = 0x0002
          report_id = 8
        "#,
    )
    .expect("用例里的配置应当解析得动")
    .devices
    .remove(0)
}
