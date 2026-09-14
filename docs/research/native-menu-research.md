# 原生弹出菜单能画什么

这份文档回答一个问题：Windows 原生弹出菜单（`TrackPopupMenu`，不带 GUI 框架，Rust `windows` crate 0.62）能画出什么、画不出什么。它决定托盘右键菜单能在多大程度上照着样稿 `.scratch/tray-design/icon-review.html` 做。查证日期 2026-09-10。

置信度标记：

- **文档写明**：learn.microsoft.com 上的 Win32 文档原文这么写。其中 Vista 时期的归档页会注明"归档"。
- **源码可见**：知名开源实现的源码里能看到这样做，或有这样的注释。这只说明他们这么做了、认为有效，不等于微软承诺。Wine 是对 user32 的重新实现，它的行为是在模仿 Windows，引用时单独说明。
- **推断**：由上面两类证据推出来，没有直接证据。
- **实机**：原型在一台真机上把菜单弹出来、截图、逐像素核过。只对那台机器的 Windows 版本与显示缩放成立，见第 8 节开头。

调研时没有做实机实验（调研阶段不写代码）。2026-09-14 menu-as-designed 票 01 的原型在实机上验证了其中一批，结论在第 8 节；被查实或被推翻的推断，在原处加了"实机"的注。凡是"推断"的条目，都列在文末"查不实的"一节。

源码版本（下文只写路径和行号，都指这些提交）：

| 仓库 | 提交 |
|---|---|
| tauri-apps/muda | `d00ab36630112bebb919de53e22976ca2bc36800`（Cargo.toml 0.19.3） |
| tauri-apps/tao | `27fe28c73ace307eb0b1d1abf457508410f1340e` |
| tauri-apps/tray-icon | `0114db53d67ac98173b33d0233cd4ce63baca3fe` |
| notepad-plus-plus/notepad-plus-plus | `2f50e44ffe9aa607a0e50e1f2ab143e0daed1391` |
| ozone10/darkmodelib | `25298e8a92ed0fac8e17b3e6878df5bbd7a5d699` |
| wxWidgets/wxWidgets | `684420c9fa985be2d3a87b30103c8f771e1f8798` |
| wine-mirror/wine | `913e31f201d344223bdf3d13a50a41af35893d12` |
| winsiderss/systeminformer | `2755b144231e202a8434580592fcca72399217dd` |
| microsoft/PowerToys | `7dd034ff294c11661301c83d8d92b0f14b1bc87f` |

permalink 格式是 `https://github.com/<仓库>/blob/<提交>/<路径>#L<起>-L<止>`。

---

## 1. 标准字符串项（非 owner-draw）

**一项显示两行：做不到。** MENUITEMINFO 的 fType 里没有和多行有关的取值，MFT_STRING 只说 dwTypeData 是 "a null-terminated string"（文档写明，https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-menuiteminfow）。文档没说字符串里的换行符会怎样。Wine 画菜单文字一律带 `DT_SINGLELINE`（源码可见，wine `dlls/win32u/menu.c` L2666-L2668、L2706-L2716）。微软演示"仿系统外观的 owner-draw 菜单"时，文字也用 `DrawThemeText(..., DT_SINGLELINE | DT_LEFT | uAccel, ...)` 画（文档写明，归档 https://learn.microsoft.com/en-us/previous-versions/bb756890(v=msdn.10)）。所以系统画的标准项是单行（推断）。样稿里设备行"名字 + 下面一行小灰字"，标准项做不出来。**实机（8.1）**：字符串里的 `\n`、`\r\n` 不换行、也不画出来，两行字接成一行，项高不变。

**右对齐列：`\t` 给的不是右对齐，`\a` 才是。** 资源语法文档说，`\t` "inserts a tab in the string and is used to align text in columns"，`\a` "aligns all text that follows it flush right to the menu bar or pop-up menu"（文档写明，https://learn.microsoft.com/en-us/windows/win32/menurc/menuitem-statement）。About Menus 只说快捷键文字 "appears to the right of the menu item name, after a backslash and tab character (\t)"（文档写明，https://learn.microsoft.com/en-us/windows/win32/menurc/about-menus）。`\a` 是资源编译器的转义，用 InsertMenuItem 在运行时建菜单时它对应哪个字符，文档没写。Wine 在运行时把 `\t` 后面的文字从同一个 x（该菜单的 tab 列）起**左对齐**，把 `\b`（0x08）后面的文字**右对齐**到该列（源码可见，wine `menu.c` L2686-L2716）。据此：`\t` 得到的是"各项从同一起点开始的第二列"，真要右对齐应在字符串里放 0x08（推断，依据 Wine；实机未验证）。**实机（8.1）：这条推断被推翻。**Windows 11 上 `\t` 后面的文字贴右对齐，不是 Wine 那样左对齐；0x08 画出来与 `\t` 逐字节相同。

**分组标题（小灰字）：没有这种项类型。** fType 的全部取值是 MFT_BITMAP、MFT_MENUBARBREAK、MFT_MENUBREAK、MFT_OWNERDRAW、MFT_RADIOCHECK、MFT_RIGHTJUSTIFY、MFT_RIGHTORDER、MFT_SEPARATOR、MFT_STRING，没有标题（文档写明，MENUITEMINFOW 同上）。主题的 MENU 类部件里也没有标题部件（文档写明，https://learn.microsoft.com/en-us/windows/win32/controls/parts-and-states）。能凑合的是一个 MFS_GRAYED 的字符串项：灰、点不了。但主题给 MENU_POPUPITEM 定义了 MPI_DISABLEDHOT 状态（文档写明，Parts and States 同上），也就是灰项在鼠标经过时仍有一种"悬停"外观（推断）。这和样稿里静态的"托盘上画哪一台"标题不完全一样。**实机（8.5）**：灰项被选中时画出与正常项相同的悬停底色，字仍是灰的。

**看起来灰、但仍能点：标准项做不到。** MFS_GRAYED 和 MFS_DISABLED 同为 0x3，文档对两者的描述一字不差："Disables the menu item and grays it so that it cannot be selected"（文档写明，MENUITEMINFOW）。老的 EnableMenuItem 语义里"disabled"恰好相反："A disabled item looks just like an enabled item"，同样点不了（文档写明，About Menus）。没有"灰而可点"的标准状态，要这种效果只能 owner-draw 自己画灰字。另外，MFS_DEFAULT 让项 "displayed in bold"，是标准项里唯一能改字重的状态，一个菜单只能有一项（文档写明，MENUITEMINFOW）。

## 2. hbmpItem 放 32bpp 位图

**逐像素 alpha：认，而且必须是预乘的。** 归档的 Vista 文档写明，主题菜单会对同时满足以下条件的位图做 alpha 混合："The bitmap is a 32bpp DIB section." "The DIB section has BI_RGB compression." "The bitmap contains pre-multiplied alpha pixels." "The bitmap is stored in hbmpChecked, hbmpUnchecked, or hbmpItem fields." 并注明 "MFT_BITMAP items do not support PARGB32 bitmaps."（文档写明，归档 https://learn.microsoft.com/en-us/previous-versions/bb757020(v=msdn.10)）。同一篇给了两种转换样例：WIC 转 `GUID_WICPixelFormat32bppPBGRA`，或 BufferedPaint + `DrawIconEx` 后对没有 alpha 的图标补预乘。深色菜单下是否同样认 alpha，没有文档（推断为认，因为走的是同一套 user32 绘制；未验证）。**实机（8.6）：认。**强制深色、强制浅色下，预乘位图的半透明像素都逐像素按 alpha 混合到菜单底色上；自顶向下（高度为负）的 DIB section 行序也照常。

**尺寸：会把项撑大（Wine 如此，Windows 推断）。** 文档没写 hbmpItem 的尺寸上限或缩放规则。Wine 里项高取"位图高 + 2"与文字高的较大者，并且全菜单的文字起点右移到"本菜单最宽的那张位图"的宽度（`menu->textOffset = max(...)`）（源码可见，wine `menu.c` L2073-L2090、L2670-L2671）。据此 32×32 和 100×32 都会原样画出，把该项撑高，并把整列文字推右（推断，未实机验证）。文档里唯一的宽位图示例是 MF_BITMAP 纯位图项（宽为 `SM_CXMENUCHECK × 5`），不是"文字 + hbmpItem"的情形（文档写明，"Adding Lines and Graphs to a Menu"，https://learn.microsoft.com/en-us/windows/win32/menurc/using-menus）。**实机（8.4）**：原样画、不缩放，项高与文字列都会被撑开，但规则与 Wine 不同：位图与勾共用一格，文字只右移"最宽那张位图的宽 − 16"。

**圆点与位图并排：不设 MNS_CHECKORBMP 时应当并排（推断）。** MNS_CHECKORBMP 的原文是："The same space is reserved for the check mark and the bitmap. If the check mark is drawn, the bitmap is not. All checkmarks and bitmaps are aligned. Used for menus where some items use checkmarks and some use bitmaps."（文档写明，https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-menuinfo）。反过来读，不设它时勾（圆点）和位图各占一格。Wine 正是这样：不设时位图画在 `4 + SM_CXMENUCHECK` 处，即勾的右边；设了则画在勾的位置，项被勾选时不画位图（源码可见，wine `menu.c` L2577-L2586、L2637）。主题的 MENU_POPUPCHECKBACKGROUND 另有一个 MCB_BITMAP 状态（文档写明，Parts and States），说明主题渲染对"带位图的勾选项"另有画法，具体效果没有文档（推断）。MFT_RADIOCHECK 只在 hbmpChecked 为 NULL 时才画成圆点（文档写明，MENUITEMINFOW）。**实机（8.3）：这条推断被推翻，不并排。**不设 MNS_CHECKORBMP 时，位图就画在勾那一格里，挂了位图的项勾选后圆点（或勾）根本不画；MCB_BITMAP 那种勾选底框也没看到。

**MNS_CHECKORBMP 改变什么：** 就是上面那段原文——勾和位图共用一格、勾了就不画位图、所有勾和位图左对齐。归档 Vista 文档的样例在给菜单加图标时总是设 `MNS_CHECKORBMP` 并清掉 `MNS_NOCHECK`，理由是 "to make the menu look good in the Windows Classic color scheme"（文档写明，bb757020）。样稿最里层"圆点 + 预览小图 + 文字"的布局，要的恰恰是**不设**它。**实机（8.3）**：不设它也并排不了；设了它，当前项用圆点换掉自己的位图，与原文一致。

**HBMMENU_CALLBACK**（让 owner 在 WM_MEASUREITEM / WM_DRAWITEM 里画位图）会让菜单失去视觉样式：bb757020 把 "Using HBMMENU_CALLBACK to defer bitmap rendering" 列在会关掉主题菜单的情形里（文档写明）。

## 3. 弹出菜单的深色

**公开 API：文档里没有。** 微软的 "Support Dark and Light themes in Win32 apps"（2026-06-16 更新）只给两件事：用 UISettings 判断是否深色，用 `DWMWA_USE_IMMERSIVE_DARK_MODE` 让标题栏变深；并说明该文 "does not cover specifics of how to repaint and render your app UI using a Dark mode color set"（文档写明，https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-windows-themes）。MENUINFO、MENUITEMINFO、TrackPopupMenuEx 的文档里都没有深色相关的字段或标志（文档写明，见前述链接）。WindowsAppSDK 仓库 2025-06-19 开的 "Feature Request: API to set dark mode for Win32 controls" 仍是 open（https://github.com/microsoft/WindowsAppSDK/issues/5543；这是 issue 不是文档，只作旁证）。"查到的文档里没有"不能证明"不存在"，但在能查的范围内确实没有。

**未公开路线：uxtheme.dll 按序号取函数。** 几个开源实现对序号的认定一致（源码可见）：

| 序号 | 函数 |
|---|---|
| 104 | `RefreshImmersiveColorPolicyState()` |
| 132 | `ShouldAppsUseDarkMode()` |
| 133 | `AllowDarkModeForWindow(HWND, bool)` |
| 135 | 1809（17763）是 `AllowDarkModeForApp(bool)`；1903（18362）起是 `SetPreferredAppMode(PreferredAppMode)`，枚举 `Default=0, AllowDark=1, ForceDark=2, ForceLight=3, Max=4` |
| 136 | `FlushMenuThemes()` |
| 137 | `IsDarkModeAllowedForWindow(HWND)` |
| 138 | `ShouldSystemUseDarkMode()` |

出处：notepad-plus-plus `PowerEditor/src/DarkMode/DarkMode.cpp` L32-L37、L88-L98、L303-L315；wxWidgets `src/msw/darkmode.cpp` L249-L253；ysc3839/win32-darkmode `win32-darkmode/DarkMode.h`（https://github.com/ysc3839/win32-darkmode/blob/cc26549b65b25d6f3168a80238792545bd401271/win32-darkmode/DarkMode.h）。

**适用版本。** Notepad++ 只在这些 build 上启用：17763、18362、18363、19041–19044、大于 19044 的 Win10，以及所有 ≥22000 的 Win11（源码可见，`DarkMode.cpp` L256-L266）。wxWidgets 的接口文档写的是 "Windows 10 versions later than v1809 (which includes Windows 10 LTSC 2019) and all Windows 11 versions"（源码可见，wx `interface/wx/app.h` L1429-L1435）。

**能否不看用户设置，强制深或强制浅：能（源码可见，第三方文档与实际用法）。** wxWidgets 把 `DarkMode_Always` 映射到 ForceDark，文档写 "force dark mode regardless of the system mode"；把 `DarkMode_Never` 映射到 ForceLight，写 "likewise force light mode"；`DarkMode_Auto` 映射到 AllowDark（wx `interface/wx/app.h` L1422-L1452，`src/msw/darkmode.cpp` L258-L280）。Notepad++ 和 darkmodelib 打开深色时调 `SetPreferredAppMode(ForceDark)`，紧接着 `FlushMenuThemes()`；关闭时回到 `Default`（notepad-plus-plus `DarkMode.cpp` L181-L193、L337-L342；darkmodelib `src/DmlibWinApi.cpp` L251-L270、L459-L460）。System Informer 给这几个值起名 `PreferredAppModeDisabled / DarkOnDark / DarkAlways`，自己用 DarkAlways（systeminformer `phlib/theme.c` L230-L241、L773）。运行中切换模式后要调 `FlushMenuThemes()` 让菜单主题刷新，这是 Notepad++ 和 darkmodelib 的做法（源码可见，同上）。**实机（8.7、8.8）**：强制深、强制浅都压得住应用模式；切完调 `FlushMenuThemes()`，下一次弹出就是新的深浅，不调则不变。高对比度下菜单主题整个关掉（源码可见，wx `src/msw/darkmode.cpp` L689 注释 "Menu theme is turned off in high contrast mode."）。

**AllowDark 时跟哪个设置：大概率是"应用模式"（AppsUseLightTheme），没有直接证实。** 证据都是间接的：

- uxtheme 里有两个分开的判断函数，132 叫 `ShouldAppsUseDarkMode`，138 叫 `ShouldSystemUseDarkMode`（源码可见，notepad-plus-plus `DarkMode.cpp` L88-L98）。
- wxWidgets 把 AllowDark 写成 "Follow the global setting."，返回 `ShouldAppsUseDarkMode()`（源码可见，wx `src/msw/darkmode.cpp` L266-L269）。
- tao 用读注册表 `HKCU\...\Themes\Personalize\AppsUseLightTheme` 来代替 `ShouldAppsUseDarkMode`，并注释该函数 "may return incorrect values on Windows 11"（源码可见，tao `src/platform_impl/windows/dark_mode.rs` L204-L240）。
- System Informer 把 AllowDark 叫 "DarkOnDark"（源码可见，同上）。

这些都指向"AllowDark 下跟随 `ShouldAppsUseDarkMode` 的结果"；而它对应 AppsUseLightTheme，是按函数名和 tao 的替代做法推出来的（推断）。**实机（8.7）：查实，跟应用模式。**Windows 模式都是深色时，应用模式深色则菜单深、应用模式浅色则菜单浅。

**AllowDark 时 owner 窗口是否必须 `AllowDarkModeForWindow`：未查实。** win32-darkmode 和 tao 都对窗口调了它（tao `dark_mode.rs` L140）。微软 Q&A 上有人报告只在 WM_NCCREATE 里调 `SetPreferredAppMode(AllowDark)` 就够让弹出菜单跟着变（二手来源，唯一证据：https://learn.microsoft.com/en-us/answers/questions/893697/thrilled-to-have-dwmwa-use-immersive-dark-mode-for）。用 ForceDark 时应当无关（推断）。**实机（8.9）：弹出菜单不需要。**AllowDark + FlushMenuThemes 时不调它，菜单照样跟应用模式变深；调了（本机返回 false）结果不变。

## 4. owner-draw 菜单项（MFT_OWNERDRAW）

**能画什么：任意。** "An application can completely control the appearance of a menu item by using an owner-drawn item."（文档写明，About Menus）。尺寸由应用在 WM_MEASUREITEM 里填 itemWidth / itemHeight（文档写明，Using Menus），所以两行的设备行、标题行、任意颜色都能画。约束有三条：

- 只能画在 rcItem 之内，系统"does not clip menu items"（文档写明，https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-drawitemstruct）。
- owner-draw 项不能用 `&` 定助记键，要自己处理 WM_MENUCHAR（文档写明，Using Menus "Owner-Drawn Menus and the WM_MENUCHAR Message"）。
- 分隔符也可以 owner-draw：微软样例对所有项（含 MFT_SEPARATOR）加 MFT_OWNERDRAW，在 WM_DRAWITEM 里自己画分隔线（文档写明，归档 https://learn.microsoft.com/en-us/previous-versions/bb756947(v=msdn.10)）。

**代价：系统主题外观要自己重做。** 归档 Vista 文档："Owner-draw menus can be used in Windows Vista, but the menus will not be visually styled."，并把 MFT_OWNERDRAW 列为会关掉主题菜单的情形之一（文档写明，bb757020）。要看起来像系统菜单，得自己 `OpenThemeData(hwnd, VSCLASS_MENU)`，再按 MENU_POPUPBACKGROUND → MENU_POPUPGUTTER → MENU_POPUPSEPARATOR → MENU_POPUPITEM → MENU_POPUPCHECKBACKGROUND → MENU_POPUPCHECK 分层 `DrawThemeBackground`，最后 `DrawThemeText`（文档写明，bb756890）。深色更麻烦：wxWidgets 注释说 `DarkMode::Menu` 主题只定义了 MENU_POPUPBACKGROUND、MENU_POPUPSUBMENU、MENU_POPUPGUTTER 三个部件，选中背景要从 `DarkMode_ImmersiveStart::Menu` 取 MENU_POPUPITEM（源码可见，wx `src/msw/menuitem.cpp` L873-L880）。这些主题类名本身也是未公开的。

**MENUINFO.hbrBack：能设弹出菜单底色。** 文档只写 "A handle to the brush to be used for the menu's background."（文档写明，MENUINFO）。wxWidgets 用 `MIM_BACKGROUND | MIM_APPLYTOSUBMENUS` 设背景刷，并注明只在支持 owner-draw 时才这样做，因为标准项的文字颜色改不了，"it's better to leave everything white than to make it unreadable"（源码可见，wx `src/msw/menu.cpp` L960-L985）。也就是说 hbrBack 改得了底色，但标准项的文字仍按主题色画，深底配深字会看不清（推断）。

**仍由系统画、或要另想办法的部分：**

- **子菜单箭头**：系统在应用画完 WM_DRAWITEM 之后自己再画一遍。Wine 的注释："Windows will leave all drawing to the application except for the popup-menu arrow. Windows always draws that itself, after the menu owner has finished drawing."（源码可见，wine `menu.c` L2466-L2474，这是 Wine 对 Windows 行为的实验记录；Wine 自己在 L2498-L2502 照做）。wxWidgets 深色下自己用 MENU_POPUPSUBMENU 画箭头，再 `ExcludeClipRect` 把那块挡住，"Prevent Windows from drawing its default arrow over ours."（源码可见，wx `src/msw/menuitem.cpp` L941-L960）。主题里有 MENU_POPUPSUBMENU 部件，状态 MSM_NORMAL / MSM_DISABLED（文档写明，Parts and States）。
- **边框**：主题部件 MENU_POPUPBORDERS（文档写明，Parts and States）。Win11 上可用 `DWMWA_BORDER_COLOR` 设边框颜色，或设 `DWMWA_COLOR_NONE` 不画边框，"supported starting with Windows 11 Build 22000"（文档写明，https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute）。wxWidgets 对菜单窗口这样设，颜色取自主题的 MENU_POPUPBORDERS（源码可见，wx `src/msw/darkmode.cpp` L1293-L1331）。菜单窗口的 HWND 可以从 WM_ENTERIDLE 拿：wParam 为 MSGF_MENU 时，lParam 是 "window containing the displayed menu"（文档写明，https://learn.microsoft.com/en-us/windows/win32/dlgbox/wm-enteridle）。Win10 没有这个 DWM 属性，边框能否改色见文末。
- **Win11 圆角与阴影**：wxWidgets 注释 "Fix menu rounded corners in Windows 11 which are turned off if menu is owner-drawn and we're not in dark mode"；修法是对菜单窗口设 `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUNDSMALL`，并去掉窗口类的 `CS_DROPSHADOW`（源码可见，wx `src/msw/window.cpp` L4407-L4416，`src/msw/darkmode.cpp` L1293-L1318）。微软文档把 `DWMWCP_ROUNDSMALL` 作为"让自定义菜单跟标准菜单一样用小圆角"的示例值（文档写明，https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-rounded-corners Example 4）。按 wx 的注释，uxtheme 深色菜单下 owner-draw 菜单仍保留圆角（源码可见；原因不明）。
- **键盘提示下划线**：不是系统画的。DRAWITEMSTRUCT 用 ODS_NOACCEL 告诉你这次 "drawn without the keyboard accelerator cues"，应用自己决定加不加 DT_HIDEPREFIX（文档写明，DRAWITEMSTRUCT；用法见 bb756890 样例）。完全可控。
- **分隔符**：不 owner-draw 时系统按主题 MENU_POPUPSEPARATOR 画；owner-draw 时自己画（见上）。
- **项之外的留白**：弹出菜单四周几个像素不属于任何项，由 hbrBack 或主题背景填（推断）。
- **rcItem 比你量出来的宽**：Wine 注释说系统给 owner-draw 项的矩形 = WM_MEASUREITEM 返回的尺寸 + 勾的宽度 + 箭头的宽度（源码可见，wine `menu.c` L2466-L2472，属 Wine 的实验记录）。

## 5. muda 在 Windows 上实际做了什么

**(a) 深色：只管窗口菜单栏，不管弹出菜单。** `MenuTheme` 的文档注释就是 "The window menu bar theme"（源码可见，muda `src/items/menu.rs` L662-L671）。实现是子类化窗口，截获未公开的 `WM_UAHDRAWMENU`（0x91）和 `WM_UAHDRAWMENUITEM`（0x92）自己画菜单栏，文件头注明移植自 ppsspp 的 UAHMenuBar.cpp 和 win32-darkmode（muda `src/platform_impl/windows/dark_menu_bar.rs` L5、L22-L23、L140-L266；`src/platform_impl/windows/mod.rs` L1074-L1100）。深色判断用 uxtheme 序号 132 和 137（`dark_menu_bar.rs` L270-L310）。全仓库没有 `SetPreferredAppMode`、`AllowDarkModeForWindow`、`FlushMenuThemes`（源码可见，全仓 grep 无结果）。右键菜单只是 `SetForegroundWindow` 后 `TrackPopupMenu(hmenu, TPM_LEFTALIGN | TPM_RETURNCMD, ...)`（`mod.rs` L906-L935）。所以 muda 的弹出菜单深不深取决于宿主进程：tao 在深色时调 `SetPreferredAppMode(AllowDark)`，否则 `Default`，并对窗口调 `AllowDarkModeForWindow`（源码可见，tao `src/platform_impl/windows/dark_mode.rs` L38-L90、L140）。tray-icon 仓库里没有任何深色相关代码（源码可见，全仓 grep 无结果）。

**(b) 菜单项图标：固定 16×16 的 hbmpItem。** `WinIcon::to_hbitmap` 建一张**固定 16×16**、32bpp、BI_RGB 的 DIB section，用 `DrawIconEx(..., DI_NORMAL)` 把 HICON 画进去（muda `src/platform_impl/windows/icon.rs` L68-L115），再以 `MIIM_BITMAP` + `hbmpItem` 挂到项上（`mod.rs` L628、L938-L943）。不做显式预乘，不用 WIC，不设 MNS_CHECKORBMP，不 owner-draw，尺寸不随 DPI 变（源码可见）。

## 6. Win11 任务栏与通知区的系统菜单跟哪个设置

微软引入这两个设置时写明：选 Light 后 "all system UI will now be light. This includes the taskbar, Start menu, Action Center, touch keyboard, and more"，并把颜色设置分成 "Windows mode and app mode"（文档写明，Windows Insider 官方公告 build 18282，2018-11-14，https://blogs.windows.com/windows-insider/2018/11/14/announcing-windows-10-insider-preview-build-18282/；这是 Win10 时期的公告）。现行支持页只说深色模式 "is for the Start menu, taskbar, and action center"（文档写明，https://support.microsoft.com/en-us/windows/change-colors-in-windows-d26ef4d6-819a-581c-1581-493cfcc005fe）。PowerToys 给托盘图标挑深浅版本时读的是 `SystemUsesLightTheme`（源码可见，PowerToys `src/common/Themes/theme_helpers.cpp` L12-L19，`src/runner/tray_icon.cpp` L440），说明微软自家的托盘应用按"Windows 模式"判断任务栏底色。

Win11 任务栏自己的右键菜单、系统托盘图标（音量、网络等）的右键菜单具体跟哪个设置，没找到一手证据。按上面的材料推断跟 Windows 模式，即 SystemUsesLightTheme（推断，未查实）。

对 juicebar 的含义：自家托盘菜单是自己进程画的，跟的是自己设的 PreferredAppMode。若用 AllowDark，按第 3 节的推断会跟应用模式，在"深色任务栏 + 浅色应用"时会与任务栏不一致。要与任务栏一致，得自己读 `SystemUsesLightTheme`（未公开的注册表值），再选 ForceDark 或 ForceLight（推断）。**实机（8.7、8.8）**：AllowDark 跟应用模式已查实；"深色任务栏、浅色应用"下 ForceDark 压得住，菜单与深色应用时逐字节相同。任务栏自己的菜单跟哪个设置仍未查实。

## 7. 带子菜单的标准项，`\t` 后的文字还显示吗

没有文档提到。Wine 对所有弹出菜单项（不区分是否 MF_POPUP）都画 `\t` 后的文字，起点是本菜单的 tab 列；算宽度时 tab 列排在箭头宽度之前，箭头单独画在最右侧的箭头宽度里（源码可见，wine `menu.c` L2087-L2090、L2645-L2651、L2686-L2716）。所以在 Wine 里它会显示在箭头左边，不重叠。真实 Windows 是否一样：推断会显示，未查实。另要注意 `\t` 后是左对齐的列（见第 1 节），样稿里"当前值紧贴箭头"的右对齐效果要靠 0x08（推断）。**实机（8.2）**：显示，贴右对齐，紧挨箭头；`\t` 本身就贴右，不必用 0x08。

## 8. 实机验证（2026-09-14）

**环境**：Windows 11 25H2，build 26200.9445（注册表 `ProductName` 写的是 "Windows 10 Pro"，以 build 号为准）；显示缩放 100%（96 DPI）。深浅设置看了两种：本机常态的"Windows 模式深色、应用模式深色"，以及用户手动切过去、看完又切回的"Windows 模式深色、应用模式浅色"（下称混合模式）。**以下结论只对这台机器、这一档缩放成立**：125% 以上的缩放、Windows 10 都没看过。

**原型**：`prototype/native-menu` 分支上的 `examples/menu_proto.rs`（menu-as-designed 票 01，不进 master）。它用普通菜单项（`InsertMenuItemW`，不 owner-draw）建出一级、"托盘上画哪一台"、"图标样式"第二层，以及"画法"与"灰状态"两个最里层。预览由 `juicebar::icon::render` 按 16、32 像素画，电量 57，转成预乘过的 BGRA，放进 32 位、BI_RGB、自顶向下（高度为负）的 DIB section，挂到 `hbmpItem` 上；"灰状态"那一组是 Stale、暂停、取数失败三张并排，间距照设计稿的 max(2, round(S / 6))。弹出前调 `SetPreferredAppMode`（序号 135）设强制深或强制浅，再调 `FlushMenuThemes`（136）。选中与展开子菜单，只对本进程本线程的 `#32768` 菜单窗口发 `MN_SELECTITEM`（0x1E5）、`MN_OPENHIERARCHY`（0x1E3）这两个未公开的菜单窗口消息，没有动鼠标，也没有模拟输入。每一帧截菜单窗口的外接矩形；再用 `MN_GETHMENU` 与 `GetMenuItemRect` 取每一项的矩形，在截图里找到预览位图落在哪，逐像素核半透明像素。

**截图**在本文件旁边的 `native-menu-research/` 目录。菜单外面那一圈是原型自己的底板窗口，不是桌面：深色 `#0F2433`、浅色 `#CFE0EE`（设计稿级联菜单的底色），`switch` 场景是灰 `#808080`。

**量出来的外观（100%）**：一行高 22 像素，分隔线 9 像素；没有位图时，文字起点距项左缘 35 像素，勾从 10 像素处起；菜单底色深 `#2C2C2C`、浅 `#F9F9F9`；悬停底色深 `#353535`、浅 `#F0F0F0`；菜单文字是 ClearType 子像素抗锯齿。

### 8.1 右列：`\t` 与 0x08 画出来一样，都贴右对齐——用 `\t`

- `名字\t值` 的值**贴右对齐**：同一张菜单里长短不一的值，右缘对齐，不是 Wine 那样从同一个 x 左起（`dark-probe.png` 最后四行；`dark-style-dir-16.png` 第二层的"D 数字+底条 / 粗块 / 满格符号……"）。
- `名字\x08值`（资源语法 `\a` 编出来的那个字符）与 `\t` 画出来**完全一样**：深浅两种 × 16、32 像素 × 每场 7 帧，共 28 对截图逐字节相同。
- 所以贴近设计稿（`.acc{margin-left:auto}` 那种贴右）的，就是普通项自带的这一列。两种一样，用文档写着的 `\t`。
- **一项只有一个右列。**一个字符串里放两个分隔符（`\t…\t`、`\x08…\x08`、`\t…\x08`），中间那一段不画，最后一段也不再贴右（`dark-probe.png` 第 2–4 行）。设备行"名字 ｜ 中段 ｜ 右列"的中段因此只能接在名字后面、写进左边那段文字里，中段不成列（`dark-style-dir-16.png` 一级的"Dragonfly 3 Master+  来自 Dongle24G，0 秒前……57%"）。
- 换行：`\n`、`\r\n` 都不换行，也不画出来，两行字接成一行，项高不变（`dark-probe.png` 第 5、6 行）。

### 8.2 带子菜单的项：右列照样显示，紧挨 ›

"托盘上画哪一台\t自动（电量最低）"、第二层六行"画法\tD 数字+底条"等：值贴右，紧挨在箭头左边，箭头照画（`dark-which.png`、`dark-style-dir-16.png`）。同一张菜单里没有子菜单的项，值的右缘与有子菜单的项对齐，箭头那一栏空着（一级的"57%""取数失败"与"自动（电量最低）"）。一级"图标样式"不写右列，就只有箭头。

### 8.3 圆点与预览位图：不并排（推翻第 2 节的推断）

- 不设 `MNS_CHECKORBMP` 时，**预览位图就画在勾那一格里**（左上角在项内 (6, 3)），勾选的项**不画圆点**；不带 `MFT_RADIOCHECK` 的勾同样不画。深浅一样：`dark-style-dir-16.png`、`light-style-dir-16.png` 里当前值"D 数字+底条"前面没有圆点，`dark-style-gray-16.png` 里的"灰数字与灰符号两种"也没有。位图透明处露出来的是纯底色，圆点不是被压在位图下面；主题 MCB_BITMAP 那种勾选底框也没看到。
- 同一张菜单里有的项有位图、有的没有（`dark-probe-zoom.png`，放大 4 倍）：位图那一格的宽取最宽的那张，窄的位图贴右；没有位图的项，圆点画在同一格里，与窄位图同一个中心。勾与位图始终共用一格，有位图就不画勾。
- 设了 `MNS_CHECKORBMP`：当前项画圆点、不画它自己的预览，其余项照画预览，位置不变（`dark-checkorbmp-dir.png`）。这就是文档那句 "If the check mark is drawn, the bitmap is not"。
- `MFS_DEFAULT`（加粗）与位图可以同时有（`dark-probe-zoom.png` 末行）。
- **对设计稿与实现的含义**：spec「画法」一节"不设那个菜单样式，好让圆点与预览并排（原型确认）"，与设计稿最里一层的"圆点 + 预览 + 文字"，**普通项画不出来**。普通项里画得出的"标出当前选项"有三种：①设 `MNS_CHECKORBMP`，当前项用圆点换掉自己的预览（实机，有截图）；②不画圆点，当前项加粗（`MFS_DEFAULT`，一张菜单只能有一项；加粗与位图同在一项是实机看过的，"当前项加粗"的整张菜单没截）；③把圆点画进预览位图自己的左边一格（位图逐像素照画是实机查实的，所以画得出；这一种原型没截，属推断）。选哪种归设计稿（menu-as-designed 票 06）与用户审稿，见 parking lot Q210；预览那一边（`resident-tray` 票 07）照定下来的那一种做。

### 8.4 32 像素的预览、三张并排的宽预览：项撑高，那一张菜单的文字整列右移

- 位图原样画、不缩放，左上角在项内 (6, 3)。**项高 = max(22, 位图高 + 6)**：16 像素的项仍是 22，32 像素的项是 38。
- **文字起点 = 35 + max(0, 这张菜单里最宽那张位图的宽 − 16)**：16 像素单张是 35（不动）；54×16 的三张并排是 73；32 像素单张是 51；106×32 的三张并排是 125（`dark-style-dir-16.png`、`dark-style-gray-16.png`、`dark-style-dir-32.png`、`dark-style-gray-32.png`）。菜单跟着变宽同样的量（画法从 16 到 32：项宽 276 → 292；灰状态：272 → 324）。
- 只影响挂了位图的那一张菜单，一级与第二层不变。右列"默认""16、20 像素不显示数字"照样贴右，在撑高的行里垂直居中。
- 这是 100% 缩放的菜单里挂一张 32 像素位图，不是 200% 缩放下的真样子：那时字号、行高、勾那一格都会跟着变，比例没看过（parking lot Q212）。

### 8.5 变灰的项悬停：有悬停底色，字仍是灰的

- 对灰掉的设备行发 `MN_SELECTITEM`，让它进入选中态：画出与正常项**同一种**悬停底色（深 `#353535`、浅 `#F0F0F0`），文字保持灰色，不变亮（`dark-gray-hover.png`、`light-gray-hover.png`）。没有用真鼠标去停（原型不动用户的鼠标）；`MN_SELECTITEM` 走的是菜单自己的选中，推断与鼠标经过时画法相同（parking lot Q211）。
- 用方向键选中时，画的是一圈圆角焦点框，没有底色（`dark-keyboard-focus.png`）。

### 8.6 强制深色下，预乘位图的半透明边缘：照常

- 每张预览的半透明像素（D 底条与 C 圆环没被电量盖住的那一段，α = 56），拿去比"预乘值 + 底色 × (255 − α) / 255"：强制深色、强制浅色、混合模式下的强制深色，16 与 32 像素，单张与三张并排，**最大差都是 0**；若当成不透明直接画，差 34–41。没有黑边，也没有发白。
- 自顶向下的 DIB section 行序照常，不颠倒（`dark-preview-32-zoom.png`：四张 32 像素预览放大 4 倍，底条在数字下面，圆环轨道那一段是半透明的）。

### 8.7 强制深浅之后刷新菜单主题：下一次弹出就生效；不刷新就不生效

同一个进程里连着弹九次（原型的 `switch` 场景），深色应用与混合模式各跑一遍：

| 这次弹出之前做了什么 | 深色应用下的菜单 | 混合模式下的菜单 |
|---|---|---|
| ForceDark + Flush | 深 | 深（`mixed-forcedark.png`） |
| ForceLight + Flush | 浅 | 浅 |
| ForceDark，不调 Flush | **仍是浅**（`switch-forcedark-noflush.png`） | **仍是浅** |
| 只补一次 Flush | 深 | 深 |
| Default + Flush | 浅 | 浅 |
| AllowDark + Flush，不调 AllowDarkModeForWindow | 深（`dark-allowdark.png`） | **浅**（`mixed-allowdark.png`） |
| 对 owner 窗口调 AllowDarkModeForWindow(true)（返回 false），再 AllowDark + Flush | 深 | 浅 |
| ForceLight + Flush | 浅 | 浅 |
| ForceDark + Flush | 深 | 深 |

- 切了模式**必须**调 `FlushMenuThemes`。调了，下一次 `TrackPopupMenuEx` 就是新的深浅，不必重启进程；只调 `SetPreferredAppMode`、不刷新，已经弹过菜单的进程下一次弹出不变。
- 深、浅两种菜单各自逐字节相同：`mixed-forcedark.png` 与 `dark-allowdark.png` 是同一份内容，`mixed-allowdark.png` 与 `switch-forcedark-noflush.png` 是同一份内容。

### 8.8 深色任务栏、浅色应用：强制深色压得住

混合模式下（`SystemUsesLightTheme=0`、`AppsUseLightTheme=1`，用户手动切的），ForceDark + Flush 的菜单是深色 `#2C2C2C`（`mixed-forcedark.png`）。整条级联连同预览截出来，与深色应用时**逐字节相同**（`mixed-style-gray-16.png` 与 `dark-style-gray-16.png` 是同一份内容），半透明像素最大差 0。同一时刻 AllowDark、Default 都是浅色。ADR-0006 否掉"交给系统（AllowDark）"的理由——深色任务栏、浅色应用时菜单会是浅的——在实机上成立。

### 8.9 顺带查实

- **AllowDark 跟应用模式**（`AppsUseLightTheme`），不跟 Windows 模式（8.7 的表）。第 3 节那条推断查实。
- **AllowDark 下不必调 `AllowDarkModeForWindow`**：不调，菜单照样跟应用模式变深；调了（本机返回 false）结果不变。第 3 节那条"未查实"就此查实。
- 强制浅色在深色应用下同样压得住（8.7 的表）。
- uxtheme 序号 133、135、136 在 26200 上都取得到，行为与第 3 节的表一致。

---

## 查不实的

下面这些原先没拿到一手证据，或只有 Wine 与二手来源。2026-09-14 的原型在实机上（Windows 11 25H2 build 26200.9445，100% 缩放）验证了其中十条，结论写在各条里，详见第 8 节；没标"实机"的仍未查实。

1. 标准项字符串里的换行符怎样显示。**实机查实（8.1）**：`\n`、`\r\n` 都不换行，也不画出来，两行字接成一行。
2. 运行时字符串里的 0x08 在 Win10/11 主题菜单里是否右对齐。**实机查实（8.1，只看了 Win11）**：右对齐，与 `\t` 画出来逐字节相同。
3. `\t` 后的列在主题菜单里是否左对齐。**实机推翻（8.1）**：不是左对齐，是贴右对齐。
4. hbmpItem 为 32×32、100×32 时项高和文字列偏移。**实机查实（8.4，量的是 32×32 与 106×32）**：原样画、不缩放；项高 = max(22, 位图高 + 6)；这张菜单的文字起点 = 35 + max(0, 最宽那张位图的宽 − 16)。
5. 不设 MNS_CHECKORBMP 时，MFT_RADIOCHECK + MFS_CHECKED 的圆点和 hbmpItem 是否并排、深色下是否正常；MCB_BITMAP 状态的实际画法。**实机推翻（8.3）**：不并排。位图画在勾那一格，挂了位图的项勾选后不画圆点；深色、浅色一样；没看到任何勾选底框。
6. 深色菜单下 PARGB32 位图的 alpha 是否照常。**实机查实（8.6）**：照常，逐像素对得上。
7. `SetPreferredAppMode(AllowDark)` 下菜单跟 AppsUseLightTheme 还是 SystemUsesLightTheme。**实机查实（8.7、8.9）**：跟 AppsUseLightTheme。
8. AllowDark 时 owner 窗口是否必须调 `AllowDarkModeForWindow`。**实机查实（8.9）**：弹出菜单不需要。
9. Win11 任务栏右键菜单、系统托盘图标右键菜单跟哪个设置。（仍未查实：原型只看得见自己进程的菜单。）
10. 带子菜单的项上 `\t` 后文字是否显示、是否紧挨箭头。**实机查实（8.2）**：显示，贴右，紧挨箭头。
11. MFS_GRAYED 的项在悬停时有没有高亮。**实机查实（8.5）**：有，与正常项同一种悬停底色，字仍是灰的（用 `MN_SELECTITEM` 选中，没用真鼠标）。
12. Win10 深色下 owner-draw 菜单的边框能否改色。Win10 没有 `DWMWA_BORDER_COLOR`；有一条二手回答说边框来自菜单窗口（类名 `#32768`）的 `WS_EX_DLGMODALFRAME`，可去掉（唯一证据，https://learn.microsoft.com/en-us/answers/questions/649537/how-to-change-color-or-remove-pop-menu-border-in-m）。（仍未查实：本机是 Win11，ADR-0006 也不做 owner-draw。）
13. owner-draw 对整张菜单的影响范围。归档 Vista 文档一处说 owner-draw 的菜单 "will not be visually styled"，另一处又说混用时 "users will see a mix of the standard and owner-draw rendering methods"（都在 bb757020）。Win10/11 上它对背景、边框的具体影响，除了 wx 关于 Win11 圆角的那条注释，没找到一手描述。（仍未查实：ADR-0006 不做 owner-draw，原型没碰。）
14. 以上所有 uxtheme 序号函数都是未公开的，微软可以在任何一次更新里改掉。Notepad++ 按 build 号白名单启用正是为此。2026-09-14 在 build 26200 上实机看过：133、135、136 三个序号都还在，行为照第 3 节的表。
