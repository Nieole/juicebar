//! 配置：配置文件读到了、读坏了，首次运行，打开配置文件（spec「配置的读与写」「首次运行」）。往配置里写一处
//! 改动的动作（钉死 Primary Device、`[tray]` 的一个键、登记一台设备，票 06、10、11 起）以后也住这里
//! （[`Action`]），外壳那一段在 `shell/config.rs`。
//!
//! **配置归内核**：启动时外壳读好一份交给 [`Tray::new`]；之后外壳每一格时钟之前看一眼配置文件变没变，变了
//! 就重读，把读的结果交进来（[`Event::Reloaded`]）。读好了就换掉手上那一份，下一次取数带的就是它（轮询节奏
//! 每次都拿此刻的配置排取数）；读不了就沿用上一份读好的，挂上"配置读不了"（[`Warning::ConfigUnreadable`]，
//! 完整原因随之进日志），挂到下一次读好为止（`super::warnings`）。
//!
//! **写回之后不必另外告诉内核**：程序自己往配置文件里写了一处，外壳下一格照样看得见它变了，重读的结果走
//! 同一条路进来——用户手改的与程序写回的，内核分不出、也不必分。
//!
//! 换掉配置**不重开一轮**：图标与悬停提示照新配置排，要等下一次取数有结果（parking lot Q252）；各台下一次
//! 什么时候问，也还是上一次取数有结果时按旧配置算定的那一刻（parking lot Q172）。

use crate::config::Config;
use crate::round::Warning;

use super::Tray;
use super::menu::OPEN_CONFIG_FILE;
use super::notify::Notice;
use super::warnings::Matter;

/// 配置这一块收的事件。
#[derive(Debug)]
pub enum Event {
    /// 外壳看到配置文件变了，重读了一遍：读好了是那一份，读不了是完整原因（`{:#}` 排好的错误链）。
    ///
    /// 文件不见了也算读不了：运行中配置文件被删掉，多半是编辑器存盘的那一瞬，不是首次运行（parking lot Q253）。
    Reloaded(Result<Config, String>),
    /// 首次运行：启动时没有配置文件，外壳照现有的草稿生成写了一份，交给 [`Tray::new`] 的就是它。
    DraftWritten,
}

/// 配置这一块的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 用系统默认程序打开配置文件（菜单上的"打开配置文件"）。
    OpenFile,
}

impl Tray {
    /// 配置那一侧的事。
    pub(super) fn on_config(&mut self, event: Event, out: &mut Vec<super::Action>) {
        match event {
            Event::Reloaded(Ok(config)) => {
                self.config = config;
                self.warnings.clear(Matter::ConfigFile);
            }
            Event::Reloaded(Err(reason)) => {
                self.warnings.raise(Warning::ConfigUnreadable(reason), out);
            }
            Event::DraftWritten => out.push(super::Action::Notify(draft_notice())),
        }
    }
}

/// 首次运行生成了草稿之后弹的那一条：草稿里注释掉的块是程序猜不动、或者此刻扫不到的东西，用户该看一眼。
fn draft_notice() -> Notice {
    Notice {
        title: "已生成配置".to_string(),
        body: format!("右键托盘图标，点\"{OPEN_CONFIG_FILE}\"看一眼"),
    }
}
