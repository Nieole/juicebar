//! 启动那一格落到 Windows 上：单实例——一个具名互斥体，抢不到就敲老实例的门、自己退出；开机自启——弹出菜单之前问系统
//! 那个计划任务在不在，点了之后建或删它（`scheduled_task.rs`）。
//!
//! 敲了门弹什么、"开机自启"勾不勾、点了建还是删，都是内核的事（`crate::tray::launch`）；这里只问 Windows、办 Windows。

use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
use windows::core::{Owned, PCWSTR, w};

use crate::tray::Event;
use crate::tray::launch::{Action, Event as LaunchEvent};

use super::{App, WINDOW_CLASS, WM_KNOCK, feed, scheduled_task};

/// 单实例的互斥体叫什么。`Local\`：一个登录会话里一个托盘——托盘图标本来就只画在自己的会话里，第二个实例也只找得到
/// 自己会话里的那扇窗口去敲门。
const MUTEX_NAME: PCWSTR = w!("Local\\juicebar-tray");

/// 第二个实例最多找几回老实例的那扇窗口。两次启动挨得很近时，老实例抢到了名字、窗口还没建好。
const KNOCK_TRIES: u32 = 50;
/// 每回之间等多久：五十回合起来五秒，够老实例读完配置、建好窗口。
const KNOCK_WAIT: Duration = Duration::from_millis(100);

/// 抢这个会话里唯一那一个托盘的名分：抢到了交回那个互斥体（活到托盘退出，丢掉它就放手）；已经有一个在跑，就敲它的门、
/// 交回 `None`，这个实例随即退出。
pub(super) fn claim_or_knock() -> Result<Option<Owned<HANDLE>>> {
    // SAFETY: 建或打开一个具名互斥体，不要初始所有权：它只占一个名字，不用来互斥谁。紧跟着读它留下的错误码——名字已经
    // 被占着时它照样交回句柄，只在错误码里说一声。句柄只有这里握着，交给 Owned 收回。
    let (mutex, existed) = unsafe {
        let mutex = CreateMutexW(None, false, MUTEX_NAME).context("建不了单实例的互斥体")?;
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        (Owned::new(mutex), existed)
    };
    if !existed {
        return Ok(Some(mutex));
    }
    knock();
    Ok(None)
}

/// 敲老实例的门：找到它那扇窗口，投一条 [`WM_KNOCK`] 就走，不等它弹完。
///
/// 找不到（它还没建好窗口，或者正在退出）就隔一会儿再找；找够了还没有就不敲了——那一次用户看不到那句话，但托盘里照样
/// 只有一个图标。
fn knock() {
    for _ in 0..KNOCK_TRIES {
        // SAFETY: 按窗口类名找一扇顶层窗口，找到了往它投一条消息。
        let knocked = unsafe {
            FindWindowW(WINDOW_CLASS, PCWSTR::null())
                .and_then(|hwnd| PostMessageW(Some(hwnd), WM_KNOCK, WPARAM(0), LPARAM(0)))
        };
        if knocked.is_ok() {
            return;
        }
        sleep(KNOCK_WAIT);
    }
}

/// 老实例这一头：窗口收到了 [`WM_KNOCK`]，递进内核。
pub(super) fn knocked() {
    feed(Event::Launch(LaunchEvent::Knocked));
}

/// 弹出菜单之前问一次系统，开机自启的那个计划任务在不在，把答复递进内核：菜单上勾不勾照它（`crate::tray::launch`）。
pub(super) fn ask_autostart() {
    let found = scheduled_task::exists().map_err(|e| format!("{e:#}"));
    feed(Event::Launch(LaunchEvent::AutostartAsked(found)));
}

/// 启动那一格的动作：建或删那个计划任务，办成了、没办成都记一行日志——建成了的那一行写着任务指向哪个 exe。
///
/// 没办成不另外告诉内核（parking lot Q323）：菜单上勾不勾，下一次弹出之前照系统再问，照实就是。
pub(super) fn execute(action: Action, app: &mut App) {
    let task = scheduled_task::TASK_NAME;
    let line = match action {
        Action::EnableAutostart => match scheduled_task::register() {
            Ok(exe) => format!(
                "开机自启已打开 —— 计划任务「{task}」在登录时以最高权限启动 {}",
                exe.display()
            ),
            Err(e) => format!("没能打开开机自启 —— {e:#}"),
        },
        Action::DisableAutostart => match scheduled_task::delete() {
            Ok(()) => format!("开机自启已关掉 —— 删掉了计划任务「{task}」"),
            Err(e) => format!("没能关掉开机自启 —— {e:#}"),
        },
    };
    app.log.write(&line);
}
