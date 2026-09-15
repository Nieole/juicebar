//! 启动：程序是怎么被启动起来的那两件事——再启动一次时来敲门的第二个实例，与菜单上的"开机自启"（spec「开机自启、
//! 单实例、日志」，ADR-0007）。
//!
//! **单实例在外壳里**（`shell/launch.rs`）：一个具名互斥体，抢不到就找到老实例的那扇窗口、敲一下门，自己退出。老实例的
//! 外壳把敲门递进来（[`Event::Knocked`]），这里弹一条"已经在托盘里了"：用户再双击一次，看到的是一句话，而不是托盘里
//! 冒出第二个一样的图标、两份轮询抢同一条通路（spec 用户故事 5）。
//!
//! **开机自启以系统里那个计划任务为准**（ADR-0007）：开没开不另存一份，配置里没有、状态文件里也没有。外壳每次弹出菜单
//! 之前问一次系统那个任务在不在（[`Event::AutostartAsked`]），这里只记着这一次的答复，菜单照它勾或不勾；点了之后交出
//! "建"或"删"（[`Action`]，`super::menu` 的 `on_menu`），由外壳去办。**缺省不开**：内核自己从不建它，只有用户点了才建。

use super::menu::{Command, Entry, Item, Kind};
use super::notify::Notice;

/// 启动这一块记着的东西。
#[derive(Default)]
pub(super) struct Launch {
    /// 外壳上一次问系统时，开机自启的那个计划任务在不在。还没问过是 `false`：缺省不开。
    autostart: bool,
}

/// 启动这一块收的事件。
#[derive(Debug)]
pub enum Event {
    /// 又启动了一次：第二个实例发现已经有一个在跑，敲了老实例的门，自己退出了。
    Knocked,
    /// 外壳弹出菜单之前问了系统：开机自启的那个计划任务在（`true`）还是不在（`false`）；问不出来是完整原因。
    AutostartAsked(Result<bool, String>),
}

/// 启动这一块的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 打开开机自启：建那个计划任务——登录时触发、以最高权限运行、指向此刻这个 exe；已经有了就照此刻的样子换掉。
    EnableAutostart,
    /// 关掉开机自启：删那个计划任务；本来就没有，就当删过了。
    DisableAutostart,
}

impl Launch {
    /// 启动那一侧的事。
    pub(super) fn on_event(&mut self, event: Event, out: &mut Vec<super::Action>) {
        match event {
            Event::Knocked => out.push(super::Action::Notify(already_running_notice())),
            Event::AutostartAsked(Ok(found)) => self.autostart = found,
            // 问不出来就照"不在"画：点一下最坏也只是把已有的任务照原样再建一遍（parking lot Q322）。
            Event::AutostartAsked(Err(reason)) => {
                self.autostart = false;
                out.push(super::Action::Log(format!(
                    "问不出开机自启的计划任务在不在，菜单上照没开画 —— {reason}"
                )));
            }
        }
    }

    /// 菜单上"开机自启"那一项：系统里那个任务在，就勾上；点了是勾着就关、没勾就开。
    pub(super) fn autostart_entry(&self) -> Item {
        let command = if self.autostart {
            Command::DisableAutostart
        } else {
            Command::EnableAutostart
        };
        Item::Entry(Entry {
            checked: self.autostart,
            command: Some(command),
            ..Entry::new(Kind::Check, "开机自启")
        })
    }
}

/// 第二个实例来敲门之后弹的那一条：它已经退出了，用户要找的就是托盘里这一个。
fn already_running_notice() -> Notice {
    Notice {
        title: "已经在托盘里了".to_string(),
        body: "juicebar 已经在运行，不用再启动一次：右键托盘图标就能用".to_string(),
    }
}
