//! 通知那一格的动作落到哪：从托盘图标上弹一条系统通知。
//!
//! 本票还没有谁弹通知；票 08（低电）、09（首次运行）、10（自动补了配置）、13（第二个实例来敲门）弹的
//! 都走这里，所以这一段先写好，免得几张票各写一遍。

use crate::tray::notify::Notice;

use super::App;

pub(super) fn execute(notice: &Notice, app: &mut App) {
    app.icon.balloon(notice);
}
