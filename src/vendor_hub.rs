//! 厂商上位机这一维：它在跑的时候，两条 HID Endpoint 让开。
//!
//! **为什么让开不是防御性设计**：键盘 dongle 的 feature 报文是一块保存最近一次应答的
//! 共享缓冲区，两个程序同时发命令会互相覆盖对方的应答，双方都读到错数据（`docs/protocol.md`
//! 记着的那次实测：`HidD_GetFeature` 照样返回成功、照样交回缓冲区里的陈旧残留且不报错）。
//! HUB 同样驱动有线设备，所以 `Wired` 和 `Dongle24G` 都要停；`Ble` 是纯本地属性读取，
//! 不存在竞争，照常。
//!
//! **这是这个项目的第四条接缝**，形状照 [`crate::clock`] 抄：一次进程枚举要碰真系统，
//! 而这个仓库的用例不接硬件也跑得通。接缝之下只传一个**值**（[`VendorHub`]），不传
//! `&dyn Processes`——问系统只在 `cli::status::run` 的顶上发生一次，往下全是纯函数
//! （parking lot Q27 定的规矩：一次 `status` 只该问一次系统）。
//!
//! 名字比对留在这一侧、不进接缝，是为了让那件最容易写错的事（大小写）被用例守住：
//! 认不出来的症状是"暂停永远不触发"，而它和"HUB 没在跑"长得一模一样。

use anyhow::{Context, Result};

use crate::config::General;
use crate::endpoints::EndpointKind;

/// 这一次撞见的那个厂商上位机进程。**它存在即"取数要让开"**。
///
/// 拿一个结构而不是一个 `bool`：那一行印给用户的话里要有进程名——"暂停中"本身不足以让
/// 用户做任何事，而"VGN VHUB.exe 正在运行"是他关掉它的依据。
///
/// 它是个**值**，不是一条通向系统的路：造出来之后再没人能拿它去问第二次
/// （理由与 [`crate::clock::Timestamp`] 同一条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorHub {
    process: String,
}

impl VendorHub {
    /// 问一次本机：配置的那几个进程名里有没有一个正在跑。
    ///
    /// **开关关着时连进程都不枚举**（票面第 6 条）：一次进程枚举不便宜，而开关关着就
    /// 意味着用户说了"别管这件事"。所以那一句提前 return 在问接缝**之前**。
    ///
    /// **名字怎么比对**：取路径末段（用户可能贴了一个完整路径进来），ASCII 大小写不敏感。
    /// 这不是宽松，理由与 `config::BluetoothEndpoint::matches` 那一条字面相同——写法对不上
    /// 的症状是**暂停永远不触发**，而它和"HUB 没在跑"长得一模一样，没有任何东西会指向那个
    /// 大小写。不做的是补 `.exe`：那一步会让"名单里写了什么"和"比的是什么"漂开
    /// （parking lot Q33）。
    ///
    /// 交出去的是**本机报的那个写法**，不是配置里写的那个：用户要去任务管理器里找的是
    /// 前者。
    ///
    /// 外层 `Result` 是"问不出来"（一次 Win32 失败），与"没在跑"不是一回事：调用方对前者
    /// 该说一句话，对后者什么都不必说。
    pub fn detect(general: &General, processes: &dyn Processes) -> Result<Option<Self>> {
        if !general.pause_when_vendor_hub_running {
            return Ok(None);
        }
        let running = processes
            .running()
            .context("枚举不出本机在跑哪些进程，认不出厂商上位机")?;
        // 外层遍历**配置的名单**而不是本机那几百个进程：撞见谁因此只由用户写下的顺序
        // 决定，不由进程枚举的次序决定——同一台机器上跑着两个上位机时，那一行印出来的
        // 名字才不会一次一个样。
        Ok(general
            .vendor_hub_processes
            .iter()
            .find_map(|configured| {
                running
                    .iter()
                    .find(|name| same_process(configured, name))
                    .cloned()
            })
            .map(|process| Self { process }))
    }

    /// 撞见的那个进程名，本机报的写法。
    pub fn process(&self) -> &str {
        &self.process
    }

    /// 这个上位机在跑时，这条 Endpoint 让不让开。
    ///
    /// 两条 HID 让开，`Ble` 照常（票面第 3 条）。`Ble` 不参与那个竞争——它读的是 Windows
    /// 攒的属性缓存，压根不往设备发字节（`CONTEXT.md` 的 `Ble` 条目）；而 HUB 同样驱动
    /// 有线设备，所以 `Wired` 也得停，不是只停 `Dongle24G`。
    ///
    /// 这份知识住在这个模块而不是 [`EndpointKind`] 上：它说的不是"这条 Endpoint 是什么"，
    /// 而是"这个上位机会不会跟它抢"。哪天多一个只抢蓝牙的上位机，改的是这里。
    ///
    /// 对 [`EndpointKind`] **穷举、不写通配分支**：加第四种 Endpoint 时编译器会把人指到
    /// 这一行来问"它跟 HUB 抢不抢"，而一个 `_` 会让新来的悄悄按 `Ble` 那一支放行。
    ///
    /// 收 `&self` 而它一个字段都没用到，是因为**"有一个上位机在跑"是这个判断的前提**：
    /// 没撞见上位机时压根不该问这个问题，而一个关联函数会让那种调用看着像合法的。
    pub fn pauses(&self, endpoint: EndpointKind) -> bool {
        match endpoint {
            EndpointKind::Wired | EndpointKind::Dongle24G => true,
            EndpointKind::Ble => false,
        }
    }
}

/// 配置里写的名字和本机报的进程名是不是同一个程序。
///
/// 路径末段 + ASCII 大小写不敏感。分隔符两种都认（`\` 与 `/`），因为一个从别处贴进来的
/// 路径两种都可能。
fn same_process(configured: &str, running: &str) -> bool {
    file_name(configured).eq_ignore_ascii_case(file_name(running))
}

/// 一个路径的末段。本身就是个裸文件名时原样交回。
fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// 进程接缝：唯一被允许去问"本机在跑哪些进程"的地方。
///
/// 它交出的是**名字的清单**，而不是"这几个名字里有没有在跑的"：名字怎么比对
/// （大小写、路径）是这个项目自己的判断，留在接缝这一侧才被用例守得住。一份假实现
/// 因此没法靠自己的比对规则替被测逻辑作答。
pub trait Processes {
    /// 本机此刻在跑的那些进程的可执行文件名（`VGN VHUB.exe` 这种），不含路径。
    ///
    /// 顺序不表达任何东西——它是本机枚举的次序。谁被撞见由配置里的名单顺序决定
    /// （见 [`VendorHub::detect`]）。
    fn running(&self) -> Result<Vec<String>>;
}

/// 落到真机上的那一份：一次 `CreateToolhelp32Snapshot` 遍历。
///
/// 选 Toolhelp 而不是 `EnumProcesses`：后者交出的是一串 PID，名字还得逐个
/// `OpenProcess` + `QueryFullProcessImageNameW` 去问，而**对系统进程那一步会失败**
/// ——一份缺了几十项的名单，症状恰好是"暂停有时不触发"。Toolhelp 一次快照就把名字带出来，
/// 而且不需要任何额外权限。
///
/// 与 `hid::enumerate` / `bluetooth::enumerate` 同一形态：**取一次快照、当场读完、
/// 交出一份纯数据**。快照里的进程可能在这之后就退出了，那不要紧——这一次要的就是"刚才那
/// 一瞬间在跑什么"。
pub struct SystemProcesses;

impl Processes for SystemProcesses {
    fn running(&self) -> Result<Vec<String>> {
        use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES};
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        };

        // 第二个参数只在快照模块/堆时才用得上；进程快照要的是"全机"，那就是 0。
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .context("拿不到进程快照（CreateToolhelp32Snapshot）")?;
        // 遍历包在一个闭包里、句柄在外面关，与 `hid::describe` 同一形状：中途失败照样要还句柄。
        let walk = (|| -> Result<Vec<String>> {
            let mut names = Vec::new();
            let mut entry = PROCESSENTRY32W {
                // 这一格不填，Toolhelp 一律失败（`ERROR_BAD_LENGTH`）。它是这套 API 唯一的坑。
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            unsafe {
                // **第一条取不到就是调用失败，不是"本机没有进程"**：快照里永远至少有
                // `System`。把它咽成一份空名单，症状恰好是"暂停永远不触发"——而那和
                // "HUB 没在跑"长得一模一样。所以它往上传，由 `run` 印一句
                // （parking lot Q34：认不出来就按没在跑走，但要出声）。
                Process32FirstW(snapshot, &mut entry)
                    .context("读不出快照里的第一条进程（Process32FirstW）")?;
                loop {
                    names.push(exe_name(&entry.szExeFile));
                    match Process32NextW(snapshot, &mut entry) {
                        Ok(()) => {}
                        // 走到底了：这是唯一一种"失败"其实是正常收尾的情形。
                        Err(e) if e.code() == ERROR_NO_MORE_FILES.into() => break,
                        // 中途真失败就整次作废，**不交一份缺项的名单**：缺了哪几项无从得知，
                        // 而缺的正好是 HUB 那一项时，暂停会时灵时不灵（`Cargo.toml` 里选
                        // Toolhelp 而不是 EnumProcesses 的理由，逐字就是这一句）。
                        Err(e) => {
                            return Err(anyhow::Error::new(e)
                                .context("遍历进程快照中途失败（Process32NextW）"));
                        }
                    }
                }
            }
            Ok(names)
        })();
        // 快照的句柄照旧要还。关不掉只会漏一个句柄，不因此改变这一次的结论
        // （`hid.rs` 里每一处 `CloseHandle` 同样处置）。
        unsafe {
            let _ = CloseHandle(snapshot);
        }
        walk
    }
}

/// `PROCESSENTRY32W::szExeFile` 那个定长宽字符数组里的那个名字。
///
/// 到第一个 NUL 为止。`from_utf16_lossy` 与 `hid.rs` / `bluetooth.rs` 里读 Windows 字符串
/// 那几处同一条：一个名字里的坏字节不该让整份名单失败。
fn exe_name(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
