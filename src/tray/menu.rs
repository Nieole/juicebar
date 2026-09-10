//! 菜单：右键菜单上有什么，点了之后内核做什么。
//!
//! **菜单模型在内核里**，外壳只把它画成原生菜单（`shell/menu.rs`）：它在弹出之前来问
//! [`Tray::menu`]，拿到的是此刻的样子。本票只有"退出"；设备行与"托盘上画哪一台"归票 06，"图标样式"
//! 与"菜单显示"归票 07，"打开配置文件"归票 09，"开机自启"归票 13（spec「菜单」写着自上而下的次序）。

use super::Tray;

/// 右键菜单，自上而下。
pub struct Menu {
    pub entries: Vec<Entry>,
}

/// 菜单上的一项。
pub struct Entry {
    /// 这一项写着什么。
    pub text: String,
    /// 点了它，内核收到什么。
    pub command: Command,
}

/// 菜单上点得到的一项——菜单这一块收的事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// 退出。
    Quit,
}

/// 菜单这一块的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 退出：外壳拿掉图标、停下取数线程、结束消息循环。
    Quit,
}

impl Tray {
    /// 此刻的右键菜单。外壳在弹出菜单之前来问。
    pub fn menu(&self) -> Menu {
        Menu {
            entries: vec![Entry {
                text: "退出".to_string(),
                command: Command::Quit,
            }],
        }
    }

    /// 点了菜单上的某一项。
    pub(super) fn on_menu(&mut self, command: Command, out: &mut Vec<super::Action>) {
        match command {
            // 退出之后内核什么都不再做（`Tray::handle` 的第一句）：外壳停计时器、停取数线程之间还可能
            // 有一两个事件落进来，它们不该再排出一次取数。
            Command::Quit => {
                self.quit = true;
                out.push(super::Action::Menu(Action::Quit));
            }
        }
    }
}
