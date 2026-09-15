//! 开机自启的那个计划任务：建、删、问在不在（ADR-0007）。走任务计划程序的 COM 接口（`ITaskService`），不起
//! `schtasks.exe`（parking lot Q320）。
//!
//! 任务的样子（[`register`]）：叫 [`TASK_NAME`]，放在任务计划程序库的根下；此刻这个用户登录时触发；以此刻这个用户、交互式
//! 令牌（只在他登录着的会话里跑）、**最高权限**运行——程序清单要求管理员，从这里启动就不弹 UAC；动作是启动此刻这个 exe。
//! 另有三项改掉任务计划程序的缺省值，不改它们，开机自启会悄悄走样：
//!
//! - 用电池时照样启动、拔了电源也不停：缺省是用电池时不启动，笔记本上开机自启就静悄悄地不生效；
//! - 不设运行时限：缺省 72 小时后结束任务，常驻的托盘三天后就没了；
//! - 优先级 4（普通）：缺省 7 是低于普通，连带 I/O 也是低优先级，与手动双击起来的那一个不一样。
//!
//! 每一次调用都在消息循环那个线程上，COM 在那里初始化成单线程套间（[`Com`]，`super::run`）。

use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, VARIANT_FALSE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::TaskScheduler::{
    IExecAction, ILogonTrigger, ITaskFolder, ITaskService, TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE,
    TASK_LOGON_INTERACTIVE_TOKEN, TASK_RUNLEVEL_HIGHEST, TASK_TRIGGER_LOGON, TaskScheduler,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, Interface};

/// 那个计划任务叫什么（任务计划程序库的根下）。
pub(super) const TASK_NAME: &str = "juicebar";

/// 任务的优先级：4 是普通（任务计划程序的缺省 7 是低于普通）。
const NORMAL_PRIORITY: i32 = 4;

/// 消息循环那个线程上的 COM（单线程套间）：活到托盘退出，初始化成了的，丢掉时反初始化。
///
/// 初始化不成不让托盘起不来：只有开机自启用得到它，问、建、删那时各自报错（记日志、菜单不勾），别的照常。
pub(super) struct Com {
    /// 这个线程上的 COM 初始化成了没有：成了才在丢掉时反初始化。
    initialized: bool,
}

impl Com {
    pub(super) fn init() -> Self {
        // SAFETY: 在这个线程上初始化 COM。
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        Self { initialized }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: 与上面那一次成了的初始化配对；`drop` 只走一次。
            unsafe { CoUninitialize() };
        }
    }
}

/// 那个任务在不在。
pub(super) fn exists() -> Result<bool> {
    let root = root(&connect()?)?;
    // SAFETY: 按名字取一个已登记的任务；取回来的接口随即丢掉。
    match unsafe { root.GetTask(&BSTR::from(TASK_NAME)) } {
        Ok(_) => Ok(true),
        Err(e) if is_missing(&e) => Ok(false),
        Err(e) => Err(e).with_context(|| format!("问不出计划任务「{TASK_NAME}」在不在")),
    }
}

/// 建那个任务（已经有了就照此刻的样子换掉），交回它指向的 exe。
pub(super) fn register() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("认不出此刻这个 exe 在哪")?;
    let user = current_user()?;
    let service = connect()?;
    let root = root(&service)?;
    define_and_register(&service, &root, &user, &exe)
        .with_context(|| format!("建不了计划任务「{TASK_NAME}」"))?;
    Ok(exe)
}

/// 删那个任务；本来就没有，就当删过了。
pub(super) fn delete() -> Result<()> {
    let root = root(&connect()?)?;
    // SAFETY: 按名字删一个已登记的任务。
    match unsafe { root.DeleteTask(&BSTR::from(TASK_NAME), 0) } {
        Ok(()) => Ok(()),
        Err(e) if is_missing(&e) => Ok(()),
        Err(e) => Err(e).with_context(|| format!("删不掉计划任务「{TASK_NAME}」")),
    }
}

/// 这个错误说的是"没有这个任务"：问的时候就是不在，删的时候就是删过了。
fn is_missing(error: &windows::core::Error) -> bool {
    error.code() == ERROR_FILE_NOT_FOUND.to_hresult()
}

/// 新建一份任务定义，照模块文档里的样子填好，登记到根下（同名的换掉）。
fn define_and_register(
    service: &ITaskService,
    root: &ITaskFolder,
    user: &BSTR,
    exe: &Path,
) -> windows::core::Result<()> {
    // SAFETY: 照任务计划程序 2.0 的写法逐项填一份任务定义；取出来的每个接口由 windows crate 的包装配对释放。
    unsafe {
        let task = service.NewTask(0)?;
        task.RegistrationInfo()?.SetDescription(&BSTR::from(
            "juicebar 的开机自启：登录时以最高权限启动托盘，开机不弹 UAC。在托盘菜单里取消\"开机自启\"就会删掉它。",
        ))?;
        let principal = task.Principal()?;
        principal.SetUserId(user)?;
        principal.SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN)?;
        principal.SetRunLevel(TASK_RUNLEVEL_HIGHEST)?;
        let trigger: ILogonTrigger = task.Triggers()?.Create(TASK_TRIGGER_LOGON)?.cast()?;
        trigger.SetUserId(user)?;
        let action: IExecAction = task.Actions()?.Create(TASK_ACTION_EXEC)?.cast()?;
        action.SetPath(&quoted(exe))?;
        let settings = task.Settings()?;
        settings.SetDisallowStartIfOnBatteries(VARIANT_FALSE)?;
        settings.SetStopIfGoingOnBatteries(VARIANT_FALSE)?;
        settings.SetExecutionTimeLimit(&BSTR::from("PT0S"))?;
        settings.SetPriority(NORMAL_PRIORITY)?;
        root.RegisterTaskDefinition(
            &BSTR::from(TASK_NAME),
            &task,
            TASK_CREATE_OR_UPDATE.0,
            &VARIANT::default(),
            &VARIANT::default(),
            TASK_LOGON_INTERACTIVE_TOKEN,
            &VARIANT::default(),
        )?;
    }
    Ok(())
}

/// 连上本机的任务计划程序，用此刻这个用户的身份。
fn connect() -> Result<ITaskService> {
    // SAFETY: 起任务计划程序的进程内 COM 对象；Connect 的四个参数都空着，就是本机、此刻这个用户。
    unsafe {
        let service: ITaskService = CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
            .context("起不来任务计划程序的 COM 对象")?;
        service
            .Connect(
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
                &VARIANT::default(),
            )
            .context("连不上本机的任务计划程序")?;
        Ok(service)
    }
}

/// 任务计划程序库的根。
fn root(service: &ITaskService) -> Result<ITaskFolder> {
    // SAFETY: 取根文件夹。
    unsafe { service.GetFolder(&BSTR::from("\\")) }.context("打不开任务计划程序库的根")
}

/// 此刻登录的这个用户，写成 `域\用户名`（本机账户的"域"是计算机名）：任务只在他登录时触发、只以他的身份跑。
fn current_user() -> Result<BSTR> {
    let domain = std::env::var_os("USERDOMAIN").context("认不出此刻是哪个用户：没有 USERDOMAIN")?;
    let name = std::env::var_os("USERNAME").context("认不出此刻是哪个用户：没有 USERNAME")?;
    let wide: Vec<u16> = domain
        .encode_wide()
        .chain(once(u16::from(b'\\')))
        .chain(name.encode_wide())
        .collect();
    Ok(BSTR::from_wide(&wide))
}

/// exe 的路径，两头加上引号：路径里有空格时（`Program Files`），任务计划程序照样认得出整段是程序。
fn quoted(exe: &Path) -> BSTR {
    let quote = u16::from(b'"');
    let wide: Vec<u16> = once(quote)
        .chain(exe.as_os_str().encode_wide())
        .chain(once(quote))
        .collect();
    BSTR::from_wide(&wide)
}
