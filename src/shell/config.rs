//! 配置文件这一头：启动时读它（没有就先写一份草稿，首次运行），每一格时钟之前看一眼它变没变、变了重读；以及
//! 配置那一格的动作落到哪（用系统默认程序打开它）。
//!
//! 读的结果原样递进内核（[`crate::tray::config::Event`]），沿用哪一份、挂不挂告警都是内核的事。
//!
//! **"变没变"看的是文件的修改时间与大小**（parking lot Q250）：一次 `metadata`，不读内容，每秒一次也觉不出来；
//! 它在消息循环这个线程上，不碰取数线程，一次取数不因它慢一分。变了才读、才解析。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, anyhow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

use crate::cli::config_refresh::write_draft;
use crate::config::Config;
use crate::tray::config::{Action, Event};

use super::App;

/// 配置文件：它在哪，上一次读的时候长什么样。
pub(super) struct ConfigFile {
    path: PathBuf,
    /// 上一次读的时候，文件的修改时间与大小；那时文件不在（读不到它的属性）就是 `None`。
    seen: Option<Stamp>,
}

/// 文件的修改时间与大小：两样都没变，就当它没变。
type Stamp = (SystemTime, u64);

impl ConfigFile {
    /// 启动：没有配置文件就照现有的草稿生成写一份，然后读它。交回配置文件这一头、读到的配置，以及要递进内核的
    /// 首次运行那一个事件（不是首次运行就没有）。
    ///
    /// 草稿写不进、或者配置读不了，托盘就起不来（`super::report_fatal` 说一句）：启动时还没有上一份读好的可沿用
    /// （parking lot Q251）。
    pub(super) fn open(path: PathBuf) -> Result<(Self, Config, Option<Event>)> {
        let first_run = if path.exists() {
            None
        } else {
            write_draft(&path)?;
            Some(Event::DraftWritten)
        };
        // 先记样子再读：两步之间它若又变了，下一格看得出来，再读一次。
        let seen = stamp(&path);
        let config = Config::load(&path)?;
        Ok((Self { path, seen }, config, first_run))
    }

    /// 看一眼配置文件变没变：没变是 `None`；变了就重读一遍，交回要递进内核的事件。
    ///
    /// 读不了的那一次也记下样子：同一份坏文件不重复读、不重复记日志，下一次存盘才再读。
    pub(super) fn look(&mut self) -> Option<Event> {
        let now = stamp(&self.path);
        if now == self.seen {
            return None;
        }
        self.seen = now;
        Some(Event::Reloaded(
            Config::load(&self.path).map_err(|e| format!("{e:#}")),
        ))
    }

    /// 用系统默认程序打开配置文件，与在资源管理器里双击它一样。
    fn open_in_default_program(&self, hwnd: HWND) -> Result<()> {
        let file = HSTRING::from(self.path.as_os_str());
        // SAFETY: 一个以 NUL 结尾的路径，其余参数为空；缺省动词（与双击一样）。
        let result = unsafe {
            ShellExecuteW(
                Some(hwnd),
                PCWSTR::null(),
                &file,
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // 大于 32 才是打开了（ShellExecuteW 的老规矩）。
        if result.0 as usize <= 32 {
            return Err(anyhow!(
                "打不开 {}：{}",
                self.path.display(),
                windows::core::Error::from_thread()
            ));
        }
        Ok(())
    }
}

/// 文件此刻的样子；读不到它的属性（文件不在）就是 `None`。
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::OpenFile => {
            if let Err(e) = app.config.open_in_default_program(app.hwnd) {
                app.log.write(&format!("{e:#}"));
            }
        }
    }
}
