//! 轮询节奏：时钟走到了 → 该对哪几台 Device 做一次取数。
//!
//! 本票是**统一节奏**：每台都按 `poll_interval_24g`，从它上一次取数的"当下"起算。各走各的节奏（按它
//! 上一次拿到读数的那条 Endpoint 的间隔，spec「轮询节奏」）归票 05，改的是 [`Cadence::on_fetched`]
//! 里取哪个间隔。
//!
//! 一次取数本身不在这里：这一块只交出"对这一台做一次取数"（[`Action::Fetch`]），读由取数那一层在外壳的
//! 取数线程上做，暂停检测也在那一侧、每次取数之前问一次。

use std::collections::{BTreeMap, BTreeSet};

use crate::clock::Timestamp;
use crate::config::{Config, Device, General};

use super::Fetched;

/// 轮询节奏这一块的动作。
#[derive(Debug)]
pub enum Action {
    /// 对这台 Device 做一次取数，结果以 [`super::Event::Fetched`] 回来。
    Fetch(FetchRequest),
}

/// 对一台 Device 做一次取数要的东西：那台 Device 与此刻的 `[general]`（暂停名单、陈旧阈值都在里面）。
///
/// 连同配置一起交出去，而不是只交一个 id：配置归内核（票 09 起它会在运行中换掉），取数线程手上不留
/// 一份会过时的。
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

    /// 某台的一次取数有了结果：从那一次取数的"当下"起，隔 `poll_interval_24g` 再问它。
    pub fn on_fetched(&mut self, config: &Config, fetched: &Fetched) {
        self.in_flight.remove(&fetched.device_id);
        let interval = config.general.poll_interval_24g;
        let next = Timestamp::from_unix_secs(fetched.at.as_unix_secs().saturating_add(interval));
        self.due.insert(fetched.device_id.clone(), next);
    }
}
