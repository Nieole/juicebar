//! 配置：配置文件读到了、读坏了，往配置里写一处改动。
//!
//! **本票还没有**：配置在启动时读一次（外壳读，交给 [`Tray::new`]），运行中不变。这一块是留好的位置：
//! 票 09 在这里收"配置读到了 / 读坏了"（[`Event`]）、换掉内核手上那一份、给菜单顶上那一行；票 06 起
//! 往配置里写一处改动（钉死 Primary Device、`[tray]` 的一个键、登记一台设备）的动作也住这里
//! （[`Action`]），外壳写文件的那一段在 `shell/config.rs`。

use super::Tray;

/// 配置这一块收的事件。本票没有。
#[derive(Debug)]
pub enum Event {}

/// 配置这一块的动作。本票没有。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {}

impl Tray {
    /// 配置那一侧的事。
    pub(super) fn on_config(&mut self, event: Event, _out: &mut Vec<super::Action>) {
        match event {}
    }
}
