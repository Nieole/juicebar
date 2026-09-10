//! 一轮那一格的动作落到哪：图标与悬停提示交给托盘图标，存状态文件排给取数线程（状态归它，见
//! `worker.rs`）。

use crate::tray::round::Action;

use super::App;

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::DrawIcon(request) => {
            if let Err(e) = app.icon.draw(&request) {
                app.log.write(&format!("画不出托盘图标：{e:#}"));
            }
        }
        Action::Tooltip(text) => app.icon.set_tip(&text),
        Action::SaveState(save) => app.worker.save(save),
    }
}
