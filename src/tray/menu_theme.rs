//! 菜单深浅（ADR-0006）：哪些 Windows 让菜单的深浅跟得上任务栏，以及菜单此刻实际是深是浅。
//!
//! 菜单跟的是任务栏（"Windows 模式"），不是应用模式，好让菜单、托盘图标、图标样式子菜单里的预览是同一套调色。
//! Windows 没有让弹出菜单变深的公开接口，只有 uxtheme.dll 里按序号取的两个未公开函数：序号 135 设首选模式，
//! 序号 136 刷新菜单主题（`docs/research/native-menu-research.md` 第 3 节）。微软随时可以改掉它们，所以只在
//! 认得的版本上用。这里定哪些版本认得、认得时序号 135 是哪一个函数（[`app_mode_call`]）；外壳照它去取、去调
//! （`shell/menu_theme.rs`），取没取到随 [`Look`] 交回来（[`MenuTheming`]）。

use crate::icon::Theme;

use super::{Action, Look, Tray};

/// 菜单的深浅跟不跟得上任务栏：外壳启动时问一次 Windows，随 [`Look`] 交进来，运行中不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuTheming {
    /// 跟得上：认得这个版本，那两个函数也取到了，菜单是任务栏的深浅。1809 上是个例外——那一版只能"允许
    /// 深色"，菜单跟的是应用模式（parking lot Q260）。
    FollowsTaskbar,
    /// 认不得这个版本的 Windows：一个未公开函数都不调，菜单是系统缺省的浅色。
    UnknownWindows,
    /// 认得这个版本（`build` 是它的 build 号），却取不到那两个函数：同样是浅色，启动时记一条日志。
    FunctionsMissing { build: u32 },
}

/// 认得的版本上，uxtheme 序号 135 是哪一个函数。两种签名不一样，调错了不会报错，只会做错事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppModeCall {
    /// Windows 10 1809：`AllowDarkModeForApp(bool)`。它只能"允许深色"——允许了，菜单跟的是应用模式——
    /// 强制不了深色，也强制不了浅色（parking lot Q260）。
    AllowDarkModeForApp,
    /// 1903 起：`SetPreferredAppMode(mode)`，强制深色是 2、强制浅色是 3。
    SetPreferredAppMode,
}

/// 这个 build 认不认得；认得时序号 135 是哪一个函数。认不得（`None`）就一个未公开函数都不调。
///
/// 照 Notepad++ 的白名单（`DarkMode.cpp` 的 `CheckBuildNumber`，调研第 3 节）：1809（17763）、1903 与 1909
/// （18362、18363）、2004 起（19041 及以后，含全部 Windows 11）。之间的预览版 build 不认：序号 135 就是在
/// 那几个预览版里换的签名。
///
/// 收的是 build 号，不是版本名：注册表里的 `ProductName` 在 Windows 11 上照样可能写着"Windows 10"。
pub fn app_mode_call(build: u32) -> Option<AppModeCall> {
    match build {
        17763 => Some(AppModeCall::AllowDarkModeForApp),
        18362 | 18363 | 19041.. => Some(AppModeCall::SetPreferredAppMode),
        _ => None,
    }
}

impl Tray {
    /// 菜单此刻实际是深是浅：跟得上任务栏时就是任务栏的深浅，跟不上时是系统缺省的浅色。图标样式子菜单里的
    /// 预览按它选调色（`resident-tray` 票 07），好让预览放在菜单上就是它在托盘上的样子。
    ///
    /// **"此刻"是内核手上那份任务栏深浅的此刻**：外壳每次弹出菜单之前另读一次任务栏，按那一刻强制菜单深浅；
    /// 而内核手上的那一份（「一轮」画图标用的 [`Look`]）今天还是启动时的。运行中切了深浅，要等票 07 把
    /// "深浅变了"喂进来换掉它，这里才跟上（parking lot Q261）。
    pub fn menu_theme(&self) -> Theme {
        let look = self.round.look();
        match look.menu_theming {
            MenuTheming::FollowsTaskbar => look.theme,
            MenuTheming::UnknownWindows | MenuTheming::FunctionsMissing { .. } => Theme::Light,
        }
    }
}

/// 启动：跟不上任务栏是因为取不到那两个函数时，记一条日志——菜单为什么是浅的，托盘上没有别处说。
pub(super) fn on_start(look: Look, out: &mut Vec<Action>) {
    if let MenuTheming::FunctionsMissing { build } = look.menu_theming {
        out.push(Action::Log(format!("菜单跟不上任务栏的深浅，退回浅色 —— 认得这个版本的 Windows（build {build}），却从 uxtheme.dll 取不到那两个函数（序号 135、136）")));
    }
}
