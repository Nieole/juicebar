//! 轮询节奏那一格的动作落到哪：排给取数线程。

use crate::tray::cadence::Action;

use super::App;

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::Fetch(request) => app.worker.fetch(request),
    }
}
