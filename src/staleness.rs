//! 陈旧判定：一份读数的取得时刻距今已久到不该再当作现状了吗。
//!
//! `CONTEXT.md` 的 **Stale** 逐字是：「形容一份 Reading 的取得时刻距今已久到不该再当作
//! 现状。**只有 Ble 的 Reading 会陈旧到有实际影响。**」这个模块就是那句话的实现，
//! 全是纯函数——环境（"现在几点"）在 `crate::clock` 那一侧问完，这里只收一个
//! [`Timestamp`]。
//!
//! 为什么值得单独一个模块：`status` 是一次性命令，HID 读数永远是刚取的，所以 3 × 轮询
//! 间隔那一档在今天的程序里没有任何端到端可观察的效果。把它写成纯函数，它今天就是对的、
//! 今天就验得到，将来接上常驻轮询也不用改一个字。

use crate::clock::Timestamp;
use crate::config::General;
use crate::endpoints::{EndpointKind, EndpointReading};

/// 一份读数还算不算现状。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// 新鲜：这个数可以当作现在的电量。
    Fresh,
    /// 陈旧：数字照印，但要明确标注出来，别让人当成现状。
    Stale,
    /// 陈旧到不该再显示数字：只说这个数是哪一天的。
    ///
    /// **只有 `Ble` 到得了这一档**（`CONTEXT.md`：「只有 Ble 的 Reading 会陈旧到有实际
    /// 影响」）。它与 [`Self::Stale`] 的差别不是程度而是**做法**：一个几个月前的百分比
    /// 印出来就是一句假话，而"这个数是 3 月 15 日的"是当时唯一还成立的事实。
    VeryStale,
}

/// 一条 Endpoint 的陈旧阈值。
///
/// 分成"取阈值"和"下判定"两步，是因为阈值的来源逐 Endpoint 不同（HID 从轮询间隔推导，
/// `Ble` 从配置读），而判定本身对三级都是同一句话。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalenessPolicy {
    /// 取得时刻距今超过这么多秒即 [`Freshness::Stale`]。
    pub stale_after_secs: u64,
    /// 超过这么多秒即 [`Freshness::VeryStale`]。`None` = 这条 Endpoint 没有这一档。
    ///
    /// 两条 HID 是 `None`：那一档的表现是"只显示日期"，而 HID 手上只有 3 × 轮询间隔
    /// 一个数，拿它当第二档意味着一份 91 秒前、间隔 30 秒的有线读数会立刻只剩一个日期
    /// ——而它其实是一分半钟前当场问出来的。`config.example.toml` 也写明那两个阈值
    /// 只对 Ble 生效（parking lot Q29）。
    pub very_stale_after_secs: Option<u64>,
}

/// HID 的陈旧阈值是轮询间隔的几倍。
///
/// 3 倍而不是 1 倍：错过一次轮询是常事（设备忙、一次超时），就地判成陈旧会让那一格
/// 不停地闪。连错三次才是"这条通路真的不说话了"。
const HID_STALE_AFTER_POLL_INTERVALS: u64 = 3;

impl StalenessPolicy {
    /// 这条 Endpoint 的阈值。
    ///
    /// 对枚举穷举、不写通配分支：加第四种 Endpoint 时编译器会把人指到这里来，
    /// 而不是让新来的悄悄继承别人的阈值（那是一条会静默说谎的路）。
    pub fn for_endpoint(endpoint: EndpointKind, general: &General) -> Self {
        match endpoint {
            // 两条 HID 是当场往返，新鲜度**由各自的轮询间隔推导**，不设配置项：改了
            // 间隔阈值自动跟着走，不会出现"改了间隔忘了改阈值"的不一致
            // （`config.example.toml` 的注释写的就是这条理由）。
            EndpointKind::Wired => Self::from_poll_interval(general.poll_interval_wired),
            EndpointKind::Dongle24G => Self::from_poll_interval(general.poll_interval_24g),
            // `Ble` 读的是 Windows 攒的缓存，可能过期几个月——轮询得多勤也不会让缓存里
            // 的数字变新，所以这一级的阈值只能由用户给（`poll_interval_bluetooth`
            // 因此不参与这里）。
            EndpointKind::Ble => Self {
                stale_after_secs: general.stale_after,
                very_stale_after_secs: Some(general.very_stale_after),
            },
        }
    }

    fn from_poll_interval(poll_interval_secs: u64) -> Self {
        Self {
            stale_after_secs: poll_interval_secs.saturating_mul(HID_STALE_AFTER_POLL_INTERVALS),
            very_stale_after_secs: None,
        }
    }

    /// 取得时刻 + 当下 → 这份读数还算不算现状。纯函数。
    ///
    /// `taken_at` 为 `None` 是"说不出这个数是什么时候的"（`bluetooth.rs` 的原话：这台
    /// 设备根本没有更新时间戳）。那一种落在 [`Freshness::Stale`]：说它新鲜就是替设备
    /// 编了一个它没给的时间戳，而 [`Freshness::VeryStale`] 也不对——那一档要印的是日期，
    /// 而这里连日期都说不出来。
    pub fn verdict(&self, taken_at: Option<Timestamp>, now: Timestamp) -> Freshness {
        let Some(taken_at) = taken_at else {
            return Freshness::Stale;
        };
        let age_secs = now.secs_since(taken_at);
        if self
            .very_stale_after_secs
            .is_some_and(|threshold| age_secs > threshold)
        {
            Freshness::VeryStale
        } else if age_secs > self.stale_after_secs {
            Freshness::Stale
        } else {
            Freshness::Fresh
        }
    }
}

/// 一份读数在某个"当下"看上去有多旧。陈旧判定的成品。
///
/// 判定与呈现分开：这一步把"当下"用掉，`cli::status::render` 收到的就只是已经定下来的
/// 事实，于是它退回成一个纯粹的排版函数（不问配置、不问时钟）。票 09 的 Primary 选择
/// 要的也是这个值——"只有新鲜且可信的参与比较"里的前一半就是 [`Self::freshness`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Staleness {
    /// 取得时刻距今多少秒——"多久前"那一列印的就是它。
    ///
    /// `None` = 说不出这个数是什么时候的（那台 BLE 设备没有更新时间戳）。
    /// 它与 `Some(0)` 是两回事，`bluetooth::age_text` 也把两者印成不同的话。
    pub age_secs: Option<u64>,
    /// 这份读数还算不算现状。
    pub freshness: Freshness,
}

impl Staleness {
    /// 一份读数 + 配置里的阈值 + 当下 → 它有多旧。纯函数。
    pub fn assess(reading: &EndpointReading, general: &General, now: Timestamp) -> Self {
        Self {
            age_secs: reading.taken_at.map(|taken_at| now.secs_since(taken_at)),
            freshness: StalenessPolicy::for_endpoint(reading.endpoint, general)
                .verdict(reading.taken_at, now),
        }
    }
}
