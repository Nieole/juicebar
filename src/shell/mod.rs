//! Win32 外壳：把事件喂给托盘内核（`crate::tray`），把它交出来的动作执行出去。
//!
//! **这里不做决定**：该不该取数、画什么、存不存、记什么，都在内核里、都有用例。外壳不做自动测试
//! （spec Testing Decisions），所以它要薄到一眼看完——每一段都只是"把这个事件递进去"或者"把这个
//! 动作落到 Windows 上"，靠票面那张手工验收清单守着。
//!
//! 两个线程：这一个跑消息循环（一扇看不见的窗口收计时器、托盘图标的回调、资源管理器重启的广播、设备插拔），
//! 另一个取数（`worker.rs`）——一次取数可能要等几秒超时，放在这里会让右键菜单跟着卡住。
//!
//! 动作按内核的分格分派，每一格的执行在它自己的文件里（`cadence.rs`、`round.rs`、`menu.rs`、
//! `notify.rs`、`config.rs`）；这个文件只路由（[`route`]），与内核那一层同一个规矩。

mod cadence;
mod config;
mod devices;
mod icon;
mod log;
mod look;
mod menu;
mod menu_theme;
mod notify;
mod register;
mod round;
mod worker;

use std::cell::{Cell, RefCell};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, KillTimer, MB_ICONERROR, MB_OK, MSG, MSGFLT_ALLOW, MessageBoxW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL, SetTimer, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_DEVICECHANGE, WM_RBUTTONUP, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::cli::{default_config_path, state_path_beside_config};
use crate::clock::{Clock, SystemClock};
use crate::state::LastKnown;
use crate::tray::{Action, Event, Tray};

/// 托盘图标的回调消息：鼠标在图标上做了什么，在 `lParam` 里。
const WM_TRAY: u32 = WM_APP + 1;
/// 取数线程交回了东西（一次取数的结果，或者一次写状态文件的结果），去它的通道里取。
const WM_REPORT: u32 = WM_APP + 2;
/// 那个每秒一格的计时器。
const TICK_TIMER: usize = 1;

/// 外壳手上的全部东西。只活在消息循环那个线程上（[`APP`]）。
struct App {
    hwnd: HWND,
    tray: Tray,
    icon: icon::TrayIcon,
    worker: worker::Worker,
    log: log::LogFile,
    config: config::ConfigFile,
    /// 让菜单的深浅跟任务栏的那两个函数，启动时取一次。
    uxtheme: menu_theme::UxTheme,
}

thread_local! {
    /// 窗口过程要够得着外壳，而窗口过程是 Windows 回调进来的一个自由函数。
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// 资源管理器重启后广播的那条消息（`TaskbarCreated`）的编号，运行时才注册得出来。
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

/// 跑托盘，直到用户点"退出"。
pub fn run() -> Result<()> {
    // 配置与状态文件的位置照命令行那一套（`crate::cli`），票 14 收掉命令行时一起搬过来。
    let config_path = default_config_path()?;
    let mut log = log::LogFile::beside(&config_path);
    let state_path = state_path_beside_config(&config_path);
    let (config_file, config, first_run) = config::ConfigFile::open(config_path)?;
    let last_known = LastKnown::load(&state_path);
    let (uxtheme, menu_theming) = menu_theme::UxTheme::load();
    let look = look::detect(menu_theming);

    let hwnd = create_window()?;
    // 活到这个函数返回，消息循环结束之后才注销（`devices.rs`）。
    let _devices = devices::DeviceWatch::register(hwnd, &mut log);
    let (tray, actions) = Tray::new(config, &last_known, look, SystemClock.now());
    log.write("托盘启动");
    let mut app = App {
        hwnd,
        tray,
        icon: icon::TrayIcon::new(hwnd, WM_TRAY),
        worker: worker::Worker::start(hwnd, WM_REPORT, state_path, last_known),
        log,
        config: config_file,
        uxtheme,
    };
    for action in actions {
        route(&mut app, action);
    }
    // 首次运行那一条排在起手那一批之后：图标先进了通知区，通知才弹得出来。
    if let Some(event) = first_run {
        for action in app.tray.handle(Event::Config(event)) {
            route(&mut app, action);
        }
    }
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    // SAFETY: hwnd 是上面刚建的那扇窗口；消息循环是标准写法。
    unsafe {
        SetTimer(Some(hwnd), TICK_TIMER, 1_000, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    APP.with(|cell| cell.borrow_mut().take());
    Ok(())
}

/// 托盘起不来时说一句：程序是窗口程序，没有控制台可印，返回一个错误等于什么都没说。
pub fn report_fatal(error: &anyhow::Error) {
    let text = HSTRING::from(format!("juicebar 起不来：{error:#}"));
    // SAFETY: 一个没有父窗口的消息框。
    unsafe {
        MessageBoxW(None, &text, w!("juicebar"), MB_OK | MB_ICONERROR);
    }
}

/// 用系统默认程序打开这个文件，与在资源管理器里双击它一样：菜单上的"打开配置文件"（`config.rs`），点一条告警打开
/// 日志（`menu.rs`）。
fn open_in_default_program(hwnd: HWND, path: &Path) -> Result<()> {
    let file = HSTRING::from(path.as_os_str());
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
            path.display(),
            windows::core::Error::from_thread()
        ));
    }
    Ok(())
}

/// 建那扇看不见的窗口：托盘图标的回调只投给一扇窗口。
///
/// **不是只收消息的窗口**（`HWND_MESSAGE`）：那种窗口收不到广播，资源管理器重启时的 `TaskbarCreated`
/// 就到不了，图标加不回来。它从不显示。
fn create_window() -> Result<HWND> {
    // SAFETY: 注册一个窗口类、建一扇窗口，全是标准用法；类名是静态的宽字符串。
    unsafe {
        let instance = GetModuleHandleW(None).context("拿不到模块句柄")?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("juicebar-tray"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(anyhow!(
                "注册不了窗口类：{}",
                windows::core::Error::from_thread()
            ));
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("juicebar-tray"),
            w!("juicebar"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .context("建不了托盘的窗口")?;
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        TASKBAR_CREATED.with(|cell| cell.set(taskbar_created));
        // 程序以管理员身份跑，而资源管理器不是：权限低的进程发来的消息默认被挡掉（UIPI），那两条得放行。
        let _ = ChangeWindowMessageFilterEx(hwnd, taskbar_created, MSGFLT_ALLOW, None);
        let _ = ChangeWindowMessageFilterEx(hwnd, WM_TRAY, MSGFLT_ALLOW, None);
        Ok(hwnd)
    }
}

/// 窗口过程：每一种消息只做一件事，递进内核或者交给对应的那一格。
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // 每一格也顺手取一遍取数线程交回来的东西：哪次敲门时外壳正借给别人，那一样就等到这里。
        WM_TIMER => {
            take_reports();
            reread_config_if_changed();
            feed(Event::Tick(SystemClock.now()));
        }
        WM_REPORT => take_reports(),
        // 老式回调（没有设 NOTIFYICON_VERSION_4）：鼠标消息本身就在 lParam 里。
        WM_TRAY if matches!(lparam.0 as u32, WM_RBUTTONUP | WM_CONTEXTMENU) => {
            if let Some(command) = menu::popup(hwnd) {
                feed(Event::Menu(command));
            }
        }
        WM_DESTROY => {
            // SAFETY: 结束这个线程的消息循环。
            unsafe { PostQuitMessage(0) };
        }
        // 本机插上或拔掉了设备（`devices.rs`）。
        WM_DEVICECHANGE => return devices::changed(wparam, lparam),
        m if m != 0 && m == TASKBAR_CREATED.with(Cell::get) => {
            with_app(|app| app.icon.add_again());
        }
        // SAFETY: 其余的照 Windows 的缺省处理。
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// 看一眼配置文件变没变，变了就把重读的结果递进内核。排在这一格的时钟之前：这一格要是排出取数，带的就是
/// 新读好的那一份。
fn reread_config_if_changed() {
    if let Some(event) = with_app(|app| app.config.reread_if_changed()).flatten() {
        feed(Event::Config(event));
    }
    // 扫到的在等配置文件静下来（`config.rs` 的 `scanned`）：重读之后再问一次。
    config::hand_over_scan();
}

/// 取数线程交回来的东西，一样一样取出来，递进内核。
fn take_reports() {
    while let Some(report) = with_app(|app| app.worker.next_report()).flatten() {
        match report {
            worker::Report::Fetched(fetched) => feed(Event::Fetched(fetched)),
            worker::Report::Warnings(event) => feed(Event::Warnings(event)),
            worker::Report::Scanned(scan) => config::scanned(scan),
            worker::Report::BleScanned(found) => feed(Event::Register(
                crate::tray::register::Event::Scanned(found),
            )),
        }
    }
}

/// 把一个事件递进内核，把交出来的动作一个一个执行掉。
fn feed(event: Event) {
    let Some(actions) = with_app(|app| app.tray.handle(event)) else {
        return;
    };
    for action in actions {
        with_app(|app| route(app, action));
    }
}

/// 按内核的分格，把一个动作交给执行它的那一格。
fn route(app: &mut App, action: Action) {
    match action {
        Action::Cadence(action) => cadence::execute(action, app),
        Action::Round(action) => round::execute(action, app),
        Action::Menu(action) => menu::execute(action, app),
        Action::Notify(notice) => notify::execute(&notice, app),
        Action::Config(action) => config::execute(action, app),
        Action::Log(line) => app.log.write(&line),
        Action::Register(action) => register::execute(action, app),
    }
}

/// 借外壳用一下。
///
/// 借不到就不做：窗口过程可能在一次借用还没还的时候被重入（弹菜单、销毁窗口时 Windows 会同步派发
/// 消息）。那一刻落进来的一个计时器格子丢了也无妨，下一秒还有；而 `RefCell` 重借一次就是整个程序崩掉。
fn with_app<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

impl App {
    /// 退出：停计时器、拿掉图标、等取数线程把手上那一次做完（它会把最后一次该存的状态文件写完）、
    /// 销毁窗口，消息循环随之结束。
    fn quit(&mut self) {
        // SAFETY: 停掉自己窗口上的计时器、销毁自己的窗口。
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TICK_TIMER);
        }
        self.icon.remove();
        self.worker.stop();
        self.log.write("退出");
        // SAFETY: 同上。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
