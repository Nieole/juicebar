//! 通知：弹一条系统通知。
//!
//! **本票还没有要弹的通知**，这一块是留好的位置：票 08（低电通知）在 [`Notify::on_fetched`] 里判"跌破
//! 了没有、提醒过没有"，状态加在 [`Notify`] 上；票 09（首次运行生成了草稿）、票 10（自动补了配置）、
//! 票 13（第二个实例来敲门）也弹通知。它们弹的都是同一种东西（[`Notice`]），外壳弹它的那一段
//! （`shell/notify.rs`）也只写一遍——几张票各自再写一遍，就会在同一处撞上。

use crate::config::Config;

use super::Fetched;

/// 一条系统通知：标题一行，正文一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// 通知这一块记着的东西。本票没有。
#[derive(Default)]
pub struct Notify {}

impl Notify {
    /// 某台 Device 的一次取数有了结果。本票什么都不做（低电通知归票 08）。
    pub fn on_fetched(
        &mut self,
        _config: &Config,
        _fetched: &Fetched,
        _out: &mut Vec<super::Action>,
    ) {
    }
}
