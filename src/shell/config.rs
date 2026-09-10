//! 配置那一格的动作落到哪。本票还没有（写配置文件、打开配置文件归票 06、09 起）。

use crate::tray::config::Action;

use super::App;

pub(super) fn execute(action: Action, _app: &mut App) {
    match action {}
}
