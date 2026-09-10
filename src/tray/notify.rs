//! 通知：弹一条系统通知。
//!
//! 今天弹的是低电通知（票 08，spec「低电通知」）：[`Notify::on_fetched`] 判"跌破了没有、提醒过没有"，
//! "提醒过"记在 [`Notify`] 上。票 09（首次运行生成了草稿）、票 10（自动补了配置）、票 13（第二个实例来
//! 敲门）也弹通知。它们弹的都是同一种东西（[`Notice`]），外壳弹它的那一段（`shell/notify.rs`）也只写
//! 一遍——几张票各自再写一遍，就会在同一处撞上。

use std::collections::BTreeSet;

use crate::config::{Config, Device};
use crate::round::DeviceState;
use crate::sources::level::Level;
use crate::state::Provenance;

use super::Fetched;

/// 一条系统通知：标题一行，正文一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// 通知这一块记着的东西：为哪几台提醒过低电。
#[derive(Default)]
pub struct Notify {
    /// 为它弹过低电通知、之后还没重新武装的那几台 Device 的 id。
    ///
    /// **只记在内存里**（spec「低电通知」）：程序重启算新的开始，仍然低电就再提醒一次。所以它不进状态
    /// 文件——那里记的是跨重启要活下来的东西（上次已知值、上一轮的 Primary Device）。
    reminded: BTreeSet<String>,
}

impl Notify {
    /// 某台 Device 的一次取数有了结果：读到的电量低于它自己的阈值、而此前没为它提醒过，就弹一条低电
    /// 通知；读到不低于阈值、或者读到充电中，重新武装（spec「低电通知」）。
    ///
    /// - **任何一台已登记的 Device 都算**，不只是 Primary Device：钉死了某一台时，不在图标上的那台最
    ///   容易突然没电（spec 用户故事 19）。
    /// - **只有这一次取数真读到的那一份算数**。拿上次已知值顶上的不是新消息：不提醒，也不重新武装——
    ///   图标照样按低电画，那是「一轮」那一块的事。交不出数的（取数失败、暂停）什么都没读到，同理。所以
    ///   一台时读得到、时读不到的设备，不会一次次地提醒。
    /// - **真读到的就算，不问它多旧、为谁让开过**：一份很旧的 `Ble` 缓存、上位机在跑时 `Ble` 顶上来的
    ///   那一份，都是这一次取数从系统里读到的（`CONTEXT.md`「上次已知值」分的正是这两种来路）。低电压过
    ///   Stale 的理由在这里一样成立：读不到新数的设备，最常见的原因就是没电了（parking lot Q200）。
    /// - **充电中压过低电**：用户已经在充了，不提醒；它同时重新武装，拔了线再跌破时再提醒一次。与图标
    ///   状态的优先顺序一致；上次已知值里记着的"充电中"不算（[`Provenance::charging_now`]）。
    /// - **电量 Unknown 什么都不算**：它不等于 0%（`CONTEXT.md`「Unknown」），也不等于回升了。
    ///
    /// 阈值与电量取自 [`DeviceState::assess`]——这一轮判图标状态用的正是它，所以"这台低不低电"，通知与
    /// 图标答的是同一个答案（[`DeviceState::low_battery`] 上写着它为什么交出来）。"低于"阈值才算，刚好
    /// 等于还不算（`CONTEXT.md`「图标状态」）。
    pub fn on_fetched(&mut self, config: &Config, fetched: &Fetched, out: &mut Vec<super::Action>) {
        let Some(device) = config
            .devices
            .iter()
            .find(|device| device.id == fetched.device_id)
        else {
            return;
        };
        let state = DeviceState::assess(device, &fetched.in_hand, &config.general, fetched.at);
        let Some((row, _)) = state.reading() else {
            return;
        };
        // 对来路穷举：加第三种来路时，编译器会把人指到这里来问一句"它算不算读到"。
        match row.provenance {
            Provenance::JustRead => {}
            Provenance::LastKnown => return,
        }
        if row.provenance.charging_now(row.reading.reading.charging) == Some(true) {
            self.reminded.remove(&device.id);
            return;
        }
        match state.level() {
            Some(level @ (Level::Reported(percent) | Level::Derived(percent)))
                if percent < state.low_battery =>
            {
                if self.reminded.insert(device.id.clone()) {
                    out.push(super::Action::Notify(low_battery(
                        device,
                        level,
                        state.low_battery,
                    )));
                }
            }
            Some(Level::Reported(_) | Level::Derived(_)) => {
                self.reminded.remove(&device.id);
            }
            // 有读数就有电量，`None` 走不到；写出来只为穷举。
            Some(Level::Unknown) | None => {}
        }
    }
}

/// 一条低电通知：标题说是哪一台，正文说电量多少、低于多少。
///
/// 电量那一段用 [`Level`] 的 `Display`，与悬停提示那一行一字不差（"15%（Reported Level）"）：两个来源
/// 要一路带到界面上（`crate::sources::level`），同一个用户读的同一件事也不该有两种说法。阈值写出来，
/// 是因为它可以单台覆盖：用户得看得出起作用的是哪一个（parking lot Q201）。
fn low_battery(device: &Device, level: Level, threshold: u8) -> Notice {
    Notice {
        title: format!("{} 电量低", device.name),
        body: format!("{level}，低于低电量阈值 {threshold}%"),
    }
}
