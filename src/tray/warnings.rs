//! 告警：此刻还挂着的那几条（`CONTEXT.md`「告警」）——不属于任何一台 Device、而用户该知道的一件没办成的
//! 事，挂到那件事下一次办成为止，不随「一轮」来去（spec「告警」，定了 parking lot Q162）。
//!
//! 每件会挂告警的事，每办一次都把结果交到这里：没办成就挂上（同一件事已经挂着，换成这一次的原因），并把
//! 完整原因写进日志；办成了就摘掉。
//!
//! | 那件事 | 在哪儿办 | 结果怎么到这里 |
//! |---|---|---|
//! | 问本机在跑哪些进程 | 取数线程，每次取数之前 | [`Fetched::warning`]，经 [`Warnings::on_fetched`] |
//! | 把读数写进状态文件 | 取数线程，照 [`SaveState`] 写 | [`Event::StateSaved`]，经 [`Warnings::on_event`] |
//! | 读配置文件 | 外壳，每一格时钟之前看一眼变没变、变了重读 | [`Reloaded`]，经 `Tray::on_config` |
//!
//! **挂着哪几条、按什么次序，只在这里定**：这一轮的结果（[`Round::warnings`]）收的、菜单顶上画的（票 06），
//! 都是 [`Warnings::hanging`]。
//!
//! **加一种告警，挂告警的这一处只多一种**：`Matter` 多一个变体、`matter_of` 多一支（编译器会指过来）。措辞不在
//! 这里，照旧加在 [`Warning`] 上（多一个变体连同它那一句）；然后在办那件事的地方，没办成调 `Warnings::raise`、
//! 办成了调 `Warnings::clear`——"配置读不了"（票 09）就是 `on_config` 里读坏、读好那两支。
//! 两个入口不对称是有意的：没办成带着原因（一条 [`Warning`]），办成了除了"是哪件事"什么都不必说。
//!
//! [`SaveState`]: crate::tray::round::Action::SaveState
//! [`Reloaded`]: crate::tray::config::Event::Reloaded
//! [`Round::warnings`]: crate::round::Round::warnings

use std::collections::BTreeMap;

use crate::round::Warning;

use super::{Action, Fetched, Tray};

/// 告警挂在哪一件事上。一件事至多挂一条。
///
/// **声明的次序就是挂着的告警排出来的次序**（[`Warnings::hanging`]，菜单顶上自上而下），不是出事的先后：
/// 菜单顶上几行的位置不该随哪件事先出错而跳（parking lot Q222）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Matter {
    /// 问本机在跑哪些进程（[`Warning::ProcessesUnknown`]）。
    Processes,
    /// 把读数写进状态文件（[`Warning::StateNotSaved`]）。
    StateFile,
    /// 读配置文件（[`Warning::ConfigUnreadable`]）。
    ConfigFile,
}

/// 告警这一块收的事件：会挂告警、而结果不跟着一次取数来的那几件事。
#[derive(Debug)]
pub enum Event {
    /// 外壳照内核的吩咐（[`SaveState`]）写了一次状态文件：写成了是 `Ok`，写不进是完整原因（`{:#}` 排好的
    /// 错误链）。
    ///
    /// **写成了也要交**：不然挂着的那一条永远摘不掉（parking lot Q220）。
    ///
    /// [`SaveState`]: crate::tray::round::Action::SaveState
    StateSaved(Result<(), String>),
}

/// 此刻还挂着的告警，一件事一格。
///
/// **只记在内存里**：重启算新的开始，那几件事启动之后各自再办一次，没办成的再挂上。
#[derive(Default)]
pub struct Warnings {
    hanging: BTreeMap<Matter, Warning>,
}

impl Warnings {
    /// 某台 Device 的一次取数有了结果：取数之前问本机进程没问出来，就挂上；问出来了就摘掉——哪一台的取数
    /// 问的都算，问的是本机。
    ///
    /// 暂停开关关着时根本不去问（`VendorHub::detect` 的第一行），交回来的也是"没有告警"，同样摘掉：那时
    /// 该不该暂停已经不是问题，还挂着"这一轮不暂停"只是一句过时的话。
    pub fn on_fetched(&mut self, fetched: &Fetched, out: &mut Vec<Action>) {
        match &fetched.warning {
            Some(warning) => self.raise(warning.clone(), out),
            None => self.clear(Matter::Processes),
        }
    }

    /// 告警这一块自己收的事件。
    pub fn on_event(&mut self, event: Event, out: &mut Vec<Action>) {
        match event {
            Event::StateSaved(Err(reason)) => self.raise(Warning::StateNotSaved(reason), out),
            Event::StateSaved(Ok(())) => self.clear(Matter::StateFile),
        }
    }

    /// 此刻还挂着的全部告警，按 `Matter` 声明的次序。
    pub fn hanging(&self) -> Vec<Warning> {
        self.hanging.values().cloned().collect()
    }

    /// 那件事没办成：挂上这一条——同一件事已经挂着，换成这一次的原因——并把完整原因写进日志。
    ///
    /// **每次都写**，挂着的时候又发生一次也写：日志里要看得出它出了几次、每次为什么。反过来，挂着本身不写
    /// ——每一轮都把挂着的记一行，只会把真的发生淹掉。
    pub(super) fn raise(&mut self, warning: Warning, out: &mut Vec<Action>) {
        out.push(Action::Log(warning.to_string()));
        self.hanging.insert(matter_of(&warning), warning);
    }

    /// 那件事办成了：摘掉挂在它上面的那一条，没挂着就什么都不做。不写日志：办成是常态。
    pub(super) fn clear(&mut self, matter: Matter) {
        self.hanging.remove(&matter);
    }
}

/// 一条告警挂在哪一件事上。对 [`Warning`] 穷举：加一种告警时编译器会把人指到这里来。
fn matter_of(warning: &Warning) -> Matter {
    match warning {
        Warning::ProcessesUnknown(_) => Matter::Processes,
        Warning::StateNotSaved(_) => Matter::StateFile,
        Warning::ConfigUnreadable(_) => Matter::ConfigFile,
    }
}

impl Tray {
    /// 此刻还挂着的告警，按 `Matter` 声明的次序。
    ///
    /// 这一轮的结果（[`Round::warnings`]）带的就是这几条；菜单顶上那几行（票 06）画的也是它们。
    ///
    /// [`Round::warnings`]: crate::round::Round::warnings
    pub fn warnings(&self) -> Vec<Warning> {
        self.warnings.hanging()
    }
}
