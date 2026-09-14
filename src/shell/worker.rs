//! 取数线程：一次取数，以及状态文件。
//!
//! 一次取数可能要等几秒（一条 HID 超时就是三秒），放在消息循环那个线程上，右键菜单就跟着卡住。所以
//! 它在这里，一台一台排着做；做完一次，把结果交回去（[`Report::Fetched`]），再敲一下窗口让那边来取。
//!
//! **状态文件归这个线程**：取数那一层（`crate::readout::read_or_last_known`）每读到一份就当场记进手上那一份
//! 状态、读不到时从里面取上次已知值，所以那一份得跟着取数住。内核只说什么时候存、记下哪一台是 Primary
//! Device（[`crate::tray::round::SaveState`]），存也排在这里，与取数同一条队——不会有两个线程同时碰它。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use anyhow::anyhow;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::clock::{Clock, SystemClock};
use crate::endpoints::SystemEndpoints;
use crate::readout::{self, NoReading};
use crate::round::{InHand, PauseCheck};
use crate::state::LastKnown;
use crate::tray::cadence::FetchRequest;
use crate::tray::round::SaveState;
use crate::tray::{Fetched, warnings};
use crate::vendor_hub::SystemProcesses;

/// 取数线程交回来的东西。
pub(super) enum Report {
    /// 一次取数有了结果，装好箱，原样递进内核（[`crate::tray::Event::Fetched`]）。
    Fetched(Box<Fetched>),
    /// 告警那一块的事，原样递进内核（[`crate::tray::Event::Warnings`]）。今天只有写了一次状态文件的结果
    /// （[`warnings::Event::StateSaved`]）——写成了也递：挂着的那条告警要等某一次写成了才摘得掉，而写没写成
    /// 只有这里知道。
    Warnings(warnings::Event),
}

/// 排给取数线程的活。
enum Job {
    Fetch(FetchRequest),
    Save(SaveState),
}

/// 取数线程的这一头。
pub(super) struct Worker {
    jobs: Option<Sender<Job>>,
    reports: Receiver<Report>,
    thread: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
}

impl Worker {
    /// 起一个取数线程。交回东西时往 `hwnd` 投一条 `wake`。
    pub(super) fn start(hwnd: HWND, wake: u32, state_path: PathBuf, last_known: LastKnown) -> Self {
        let (jobs, inbox) = mpsc::channel();
        let (outbox, reports) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        // 窗口句柄不能跨线程传（它是个裸指针），传它的数值；投消息本身是线程安全的。
        let hwnd = hwnd.0 as usize;
        let thread = {
            let stopping = Arc::clone(&stopping);
            std::thread::spawn(move || {
                let tell = |report: Report| {
                    if outbox.send(report).is_ok() {
                        // SAFETY: 往那扇窗口投一条消息；窗口没了投递失败，无妨。
                        let _ = unsafe {
                            PostMessageW(Some(HWND(hwnd as *mut _)), wake, WPARAM(0), LPARAM(0))
                        };
                    }
                };
                run(&inbox, &tell, &stopping, &state_path, last_known);
            })
        };
        Self {
            jobs: Some(jobs),
            reports,
            thread: Some(thread),
            stopping,
        }
    }

    /// 排一次取数。
    pub(super) fn fetch(&self, request: FetchRequest) {
        self.send(Job::Fetch(request));
    }

    /// 排一次存状态文件。
    pub(super) fn save(&self, save: SaveState) {
        self.send(Job::Save(save));
    }

    /// 取数线程交回来的下一样东西，没有了就是 `None`。
    pub(super) fn next_report(&self) -> Option<Report> {
        self.reports.try_recv().ok()
    }

    /// 停下：排着的取数一次都不再做，排着的存盘照做，等它把手上那一样做完。
    ///
    /// 等是有意的：它可能正在写状态文件，进程这时退出，那份文件就可能只写了一半——下次启动读回来是
    /// 坏的，就当没有上次已知值（`LastKnown::load`），而那正是"重启不等于失忆"要防的事。代价是图标
    /// 拿掉之后，进程最多再活一次取数的工夫（几秒）。
    pub(super) fn stop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn send(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }
}

/// 取数线程本身：一样一样做排过来的活，直到那一头关掉。
fn run(
    inbox: &Receiver<Job>,
    tell: &dyn Fn(Report),
    stopping: &AtomicBool,
    state_path: &std::path::Path,
    mut last_known: LastKnown,
) {
    for job in inbox {
        match job {
            Job::Fetch(_) if stopping.load(Ordering::SeqCst) => {}
            Job::Fetch(request) => {
                tell(Report::Fetched(Box::new(fetch(&request, &mut last_known))))
            }
            Job::Save(save) => {
                // 选出了 Primary Device 才记；`None` 是那一格不动（`SaveState::remember_primary`）。
                if let Some(id) = &save.remember_primary {
                    last_known.remember_primary(id);
                }
                let outcome = last_known.save(state_path).map_err(|e| format!("{e:#}"));
                tell(Report::Warnings(warnings::Event::StateSaved(outcome)));
            }
        }
    }
}

/// 对一台 Device 做一次取数，照命令行 `status` 那一套：先问一次本机进程（暂停检测），枚举一次本机，取一个
/// "当下"，取数——读不到就退到上次已知值。
fn fetch(request: &FetchRequest, last_known: &mut LastKnown) -> Fetched {
    let pause = PauseCheck::detect(&request.general, &SystemProcesses);
    let endpoints = SystemEndpoints::enumerate();
    let at = SystemClock.now();
    let outcome = match endpoints {
        Ok(endpoints) => readout::read_or_last_known_with_reason(
            &request.device,
            &endpoints,
            pause.paused_by(),
            last_known,
            &request.general,
            at,
        ),
        // 连本机有什么都问不出来，这一次取数就没法去读：原因照实写，照样退到上次已知值。
        Err(e) => readout::fall_back_to_last_known(
            &request.device,
            NoReading::Failed(anyhow!("读不到 —— 枚举不了本机的设备：{e:#}")),
            last_known,
            &request.general,
            at,
        ),
    };
    Fetched {
        device_id: request.device.id.clone(),
        at,
        in_hand: InHand::from(outcome.row),
        fell_back_because: outcome.fell_back_because,
        warning: pause.warning(),
    }
}
