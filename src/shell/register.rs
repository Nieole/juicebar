//! 登记设备那一格的动作落到哪：扫一遍本机的蓝牙设备，排在取数线程上（`worker.rs`）。什么时候扫、扫到了列谁、点了写回
//! 什么，都是内核的事（`crate::tray::register`）；写回配置文件走配置那一格（`config.rs`）。

use crate::tray::register::Action;

use super::App;

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::Scan => app.worker.scan_ble(),
    }
}
