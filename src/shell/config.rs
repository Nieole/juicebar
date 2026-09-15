//! 配置文件这一头：启动时读它（没有就先写一份草稿，首次运行），每一格时钟之前看一眼它变没变、变了重读；以及
//! 配置那一格的动作落到哪（用系统默认程序打开它；扫一遍本机；把补过空块的全文写回去）。
//!
//! 读的结果原样递进内核（[`crate::tray::config::Event`]），沿用哪一份、挂不挂告警、补不补空块都是内核的事。
//!
//! **"变没变"看的是文件的修改时间与大小**（parking lot Q250）：一次 `metadata`，不读内容，每秒一次也觉不出来；
//! 它在消息循环这个线程上，不碰取数线程，一次取数不因它慢一分。样子变了、而且连着两格不再变（写入方多半写完了）
//! 才读、才解析；读的那一刻文件被别人占着，不算读过，下一格再试。
//!
//! **写回之后不另外告诉内核，也不改记下的样子**：下一格看得出文件变了，照常等它停下来、重读，写回的那一份走
//! 重读那条路进内核——与用户手改的一样。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

use crate::config::{Config, TraySetting, pin_primary, write_draft, write_tray};
use crate::config::{NewDevice, add_device, register_ble, unregister_ble};
use crate::hid::HidInfo;
use crate::primary::PrimaryRule;
use crate::tray::config::{Action, Event, Scan};

use super::App;

/// 配置文件：它在哪，上一次读的时候、上一格看的时候各是什么样子。
pub(super) struct ConfigFile {
    path: PathBuf,
    /// 上一次读的时候文件的样子；那时文件不在（读不到它的属性）就是 `None`。
    read: Option<Stamp>,
    /// 上一格看到的样子。这一格与它一样，才算样子停下来了。
    last_look: Option<Stamp>,
    /// 取数线程扫到的、还在等配置文件静下来才交进内核的那一份（[`scanned`]）。
    scanned: Option<Vec<HidInfo>>,
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
        // 先记样子再读：两步之间它若又变了，之后几格看得出来，再读一次。
        let seen = stamp(&path);
        let config = Config::load(&path)?;
        let file = Self {
            path,
            read: seen,
            last_look: seen,
            scanned: None,
        };
        Ok((file, config, first_run))
    }

    /// 看一眼配置文件变没变：变了、而且样子停下来了，就重读一遍，交回要递进内核的事件；否则是 `None`。
    ///
    /// - **样子还在变就先不读**：编辑器存盘是"先清空再写"，正好撞上那一瞬会读到空文件或者半截文件。多等一格。
    /// - **文件被别人占着不算读过**：写入方还没放手时读，得到的是共享冲突，不是一份坏配置；不交事件、不记样子，
    ///   下一格再试，不然文件之后不再变，这一次改动就一直不生效。
    /// - **别的读不了也记下样子**：同一份坏文件不重复读、不重复记日志，下一次存盘才再读。
    pub(super) fn reread_if_changed(&mut self) -> Option<Event> {
        let now = stamp(&self.path);
        let settled = now == self.last_look;
        self.last_look = now;
        if now == self.read || !settled {
            return None;
        }
        let loaded = Config::load(&self.path);
        if loaded.as_ref().is_err_and(is_busy) {
            return None;
        }
        self.read = now;
        Some(Event::Reloaded(loaded.map_err(|e| format!("{e:#}"))))
    }

    /// 扫到的在等着、而配置文件静下来了，就读全文，交回要递进内核的那一个事件；否则是 `None`，下一格再试。
    ///
    /// "静下来"是**此刻的样子就是上一次重读时的样子**（[`Self::reread_if_changed`] 只在样子停下来之后才记它），读完
    /// 再核一遍：用户正在存盘、程序刚写回还没重读，都不读——照一份半截的文件补，写回去就把用户后半截的内容盖没了。
    /// 读的那一刻被别人占着也不算，下一格再试。
    fn scan_if_at_rest(&mut self) -> Option<Event> {
        if self.scanned.is_none() || stamp(&self.path) != self.read {
            return None;
        }
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("读不到配置 {}", self.path.display()));
        if text.as_ref().is_err_and(is_busy) || stamp(&self.path) != self.read {
            return None;
        }
        let collections = self.scanned.take()?;
        let scan = text
            .map(|text| Scan { collections, text })
            .map_err(|e| format!("{e:#}"));
        Some(Event::Scanned(scan))
    }

    /// 把配置文件整个换成这份全文（自动补空块写回）。不改记下的样子，理由见模块文档。
    fn write(&self, text: &str) -> Result<()> {
        std::fs::write(&self.path, text)
            .with_context(|| format!("写不进配置 {}", self.path.display()))
    }

    /// 把 `primary` 写成菜单里选的那一条：读此刻文件的全文，照配置那道缝格式保留地只改那一格（[`pin_primary`]），真改了
    /// 才写。不改记下的样子，理由见模块文档：下一格看得出文件变了，照常重读。
    fn write_primary(&self, rule: &PrimaryRule) -> Result<()> {
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("读不到配置 {}", self.path.display()))?;
        let written = pin_primary(&text, rule)?;
        if written.changed {
            self.write(&written.text)?;
        }
        Ok(())
    }

    /// 把 `[tray]` 里这几个键写成菜单里点的取值：读此刻文件的全文，照配置那道缝格式保留地只改这几个键（[`write_tray`]），
    /// 真改了才写。不改记下的样子，理由见模块文档。
    fn write_tray(&self, settings: &[TraySetting]) -> Result<()> {
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("读不到配置 {}", self.path.display()))?;
        let written = write_tray(&text, settings)?;
        if written.changed {
            self.write(&written.text)?;
        }
        Ok(())
    }

    /// 读此刻文件的全文，照配置那道缝改一处（`edit` 交回改好的全文；本来就是那样时交回 `None`，一个字节都不写），有改动才
    /// 写。菜单"登记设备"里点的登记、解除与新建走这里。不改记下的样子，理由见模块文档：下一格看得出文件变了，照常重读。
    fn rewrite(&self, edit: impl FnOnce(&str) -> Result<Option<String>>) -> Result<()> {
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("读不到配置 {}", self.path.display()))?;
        if let Some(written) = edit(&text)? {
            self.write(&written)?;
        }
        Ok(())
    }
}

/// 文件此刻的样子；读不到它的属性（文件不在）就是 `None`。
fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// 这一次读不了，是因为文件正被别人占着（共享冲突、锁冲突），而不是文件本身有问题。
fn is_busy(error: &anyhow::Error) -> bool {
    let busy = [ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION].map(|code| code.0 as i32);
    error
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .and_then(std::io::Error::raw_os_error)
        .is_some_and(|code| busy.contains(&code))
}

pub(super) fn execute(action: Action, app: &mut App) {
    match action {
        Action::OpenFile => {
            // 用系统默认程序打开，与在资源管理器里双击它一样。
            if let Err(e) = super::open_in_default_program(app.hwnd, &app.config.path) {
                app.log.write(&format!("{e:#}"));
            }
        }
        Action::Scan => app.worker.scan(),
        Action::Write(text) => {
            if let Err(e) = app.config.write(&text) {
                app.log.write(&format!("{e:#}"));
            }
        }
        Action::WritePrimary(rule) => {
            if let Err(e) = app.config.write_primary(&rule) {
                app.log
                    .write(&format!("没能把托盘上画哪一台写回配置 —— {e:#}"));
            }
        }
        Action::WriteTray(settings) => {
            if let Err(e) = app.config.write_tray(&settings) {
                app.log
                    .write(&format!("没能把图标样式与菜单显示写回配置 —— {e:#}"));
            }
        }
        Action::RegisterBle { device_id, address } => {
            if let Err(e) = app
                .config
                .rewrite(|text| register_ble(text, &device_id, &address))
            {
                app.log.write(&format!(
                    "没能把蓝牙地址 {address} 登记到 {device_id} —— {e:#}"
                ));
            }
        }
        Action::UnregisterBle { device_id } => {
            if let Err(e) = app.config.rewrite(|text| unregister_ble(text, &device_id)) {
                app.log
                    .write(&format!("没能解除 {device_id} 的蓝牙登记 —— {e:#}"));
            }
        }
        Action::NewDevice(new) => {
            if let Err(e) = app.config.rewrite(|text| add_device(text, &new)) {
                let which = match &new {
                    NewDevice::Ble { name, .. } => name,
                    NewDevice::Hid { known_id, .. } => known_id,
                };
                app.log
                    .write(&format!("没能把 {which} 新建成一台 Device —— {e:#}"));
            }
        }
    }
}

/// 取数线程扫完了一遍本机（[`Action::Scan`]）。
///
/// 枚举不了就当场把原因递进内核（[`Event::Scanned`]），不必读文件。扫到了先存着，等配置文件静下来才读全文交出去
/// （[`hand_over_scan`]）：内核照那份全文补、交出写回，外壳当场就写（[`execute`]），读与写之间只隔内核里的几次解析。
/// 在取数线程上随枚举一起读就早了——那一份在通道里等着的工夫，用户存下的改动会被写回的那一份盖掉。
///
/// **这里借外壳不会落空**：它由 `take_reports` 在刚借外壳取出这份报告之后调用，同一个调用栈、中间不派发消息。它要是
/// 落了空，内核那一遍扫描就永远交不回来（`crate::tray::config` 的 `Scans`），之后插拔只记一笔、不再扫。
pub(super) fn scanned(collections: Result<Vec<HidInfo>, String>) {
    match collections {
        Err(reason) => super::feed(crate::tray::Event::Config(Event::Scanned(Err(reason)))),
        Ok(collections) => {
            super::with_app(|app| app.config.scanned = Some(collections));
            hand_over_scan();
        }
    }
}

/// 扫到的在等着、配置文件又静下来了，就读全文、递进内核。扫描刚交回来时问一次，之后每一格重读配置之后再问。
///
/// 取出那一份与递进内核是同一个调用栈上的两次借用；借不到外壳（窗口过程被重入）时那一份原样留着，下一格再问。
pub(super) fn hand_over_scan() {
    if let Some(event) = super::with_app(|app| app.config.scan_if_at_rest()).flatten() {
        super::feed(crate::tray::Event::Config(event));
    }
}
