//! 轮询节奏：时钟走到了 → 该对哪几台 Device 做一次取数。
//!
//! 每台 Device 各走各的节奏（spec「轮询节奏」）：按它**上一次取数拿到读数的那条 Endpoint** 的间隔再问
//! ——鼠标走 2.4G 就隔 `poll_interval_24g`，退到 Ble 就隔 `poll_interval_bluetooth`；还没读到过的，按它
//! 配置里优先级最高的那条（`interval`）。间隔从那一次取数的"当下"起算。一台有了结果只排它自己的下一次：
//! 那个结果开始的新一轮（`CONTEXT.md`「一轮」）不让别的哪一台被多问一次。
//!
//! 间隔取自 [`staleness::poll_interval`]，两条 HID 陈旧阈值的那"1 倍"也取自它：节奏与陈旧判定不各算各的。
//!
//! 一次取数本身不在这里：这一块只交出"对这一台做一次取数"（[`Action::Fetch`]），读由取数那一层在外壳的
//! 取数线程上做，暂停检测也在那一侧、每次取数之前问一次。

use std::collections::{BTreeMap, BTreeSet};

use crate::clock::Timestamp;
use crate::config::{Config, Device, General};
use crate::endpoints::EndpointKind;
use crate::round::InHand;
use crate::staleness;

use super::Fetched;

/// 轮询节奏这一块的动作。
#[derive(Debug)]
pub enum Action {
    /// 对这台 Device 做一次取数，结果以 [`super::Event::Fetched`] 回来。
    Fetch(FetchRequest),
}

/// 对一台 Device 做一次取数要的东西：那台 Device 与此刻的 `[general]`（暂停名单、陈旧阈值都在里面）。
///
/// 连同配置一起交出去，而不是只交一个 id：配置归内核，运行中会换掉（配置文件变了、读好了，`super::config`），
/// 取数线程手上不留一份会过时的。
#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub device: Device,
    pub general: General,
}

/// 每台 Device 下一次该什么时候取数。
///
/// 表里没有的那一台就是"现在就该问"：刚启动时每一台都是，配置里新出现的那一台也是。
#[derive(Default)]
pub struct Cadence {
    due: BTreeMap<String, Timestamp>,
    /// 已经交出去、结果还没回来的那几台。取数线程一台一台排着做，这时再排一次只会让它在后面堆起来。
    in_flight: BTreeSet<String>,
}

impl Cadence {
    /// 时钟走到了：到点、且没有一次取数正在路上的每一台，各做一次取数。次序即配置里的书写顺序。
    pub fn on_tick(&mut self, config: &Config, now: Timestamp, out: &mut Vec<super::Action>) {
        for device in &config.devices {
            let due = self.due.get(&device.id).is_none_or(|due| *due <= now);
            if due && !self.in_flight.contains(&device.id) {
                self.in_flight.insert(device.id.clone());
                out.push(super::Action::Cadence(Action::Fetch(FetchRequest {
                    device: device.clone(),
                    general: config.general.clone(),
                })));
            }
        }
    }

    /// 某台的一次取数有了结果：从那一次取数的"当下"起，隔 `interval` 再问它。只排它自己这一台。
    pub fn on_fetched(&mut self, config: &Config, fetched: &Fetched) {
        self.in_flight.remove(&fetched.device_id);
        let next = Timestamp::from_unix_secs(
            fetched
                .at
                .as_unix_secs()
                .saturating_add(interval(config, fetched)),
        );
        self.due.insert(fetched.device_id.clone(), next);
    }
}

/// 这一次取数之后，隔多少秒再问这一台。
///
/// **按手上那一份读数来自的那条 Endpoint**：这一次读到了，就是这一次的那条；没读到、退到了上次已知值，就是
/// 那份上次已知值来自的那条——它正是这台上一次拿到读数的那条，状态文件里带过来的也算（parking lot Q171）。
///
/// 手上什么都没有（取数失败或者暂停，又退不到上次已知值），就当它还没读到过：按配置里优先级最高的那条
/// （[`EndpointKind::PRIORITY`]）。一条 Endpoint 都没配的，没有"最高的那条"可按——问它什么都碰不到，在
/// 配置变之前每一次都是同一句取数失败——按三个间隔里最长的那个（parking lot Q170）。
///
/// **它按的是哪条读到了，不是哪几条被问了。**一次取数把在场的 Endpoint 按优先级挨个试：接收器插着、2.4G
/// 没答话、Ble 从 Windows 缓存里答了的那一台，按 Ble 的间隔再问，而每一次都先往 2.4G 发一遍查询、等满
/// 一次超时。"问得不比该问的勤"在这一种上不成立；要它成立，得让取数那一层交回这一次问过哪几条。
fn interval(config: &Config, fetched: &Fetched) -> u64 {
    let general = &config.general;
    let endpoint = match &fetched.in_hand {
        InHand::Reading(row)
        | InHand::FellBack {
            last_known: row, ..
        } => Some(row.reading.endpoint),
        InHand::NoReading(_) | InHand::NoKnownValue => config
            .devices
            .iter()
            .find(|device| device.id == fetched.device_id)
            .and_then(|device| {
                EndpointKind::PRIORITY
                    .into_iter()
                    .find(|kind| kind.is_configured_in(device))
            }),
    };
    match endpoint {
        Some(endpoint) => staleness::poll_interval(endpoint, general),
        None => EndpointKind::PRIORITY
            .into_iter()
            .map(|kind| staleness::poll_interval(kind, general))
            .fold(0, u64::max),
    }
}
