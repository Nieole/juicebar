# 成品不带命令行：只有托盘一个程序，诊断工具只留在示例程序里

`juicebar.exe` 只有一个样子：托盘。带不带参数启动都是它；`status` 与 `config-refresh` 两条子命令删掉。做协议逆向用的
诊断工具（`scan`、`caps`、`probe`，含 `probe --listen`）只留在 `examples/` 下，从源码树里跑，不进发布构建，只调用库的
公开面（resident-tray spec「成品形态」）。

命令行原来的活都有了去处：`status` 那几行变成右键菜单里每台 Device 一行与悬停提示，排版用例迁到托盘内核那一层
（`tests/tray_hover.rs`、`tests/tray_device_row.rs`）；`config-refresh` 的首次运行草稿由托盘启动时写（resident-tray 票 09），
补空块由托盘在本机插拔时自动做（票 10），扫描与所写不一致那句提醒进日志（parking lot Q273）；从 `scan` 的输出里抄 MAC
由菜单"登记设备"接手（票 11、12）。

## Considered Options

**两个 exe**：托盘带要求管理员权限的程序清单（ADR-0007），另编一个普通权限的命令行程序，留着 `status` 与
`config-refresh`。被否：

- **普通权限的命令行读不了 2.4G。**非管理员写 dongle 时 Windows 把写静默吞掉，那一行说"读超时"（`docs/gaps.md` 第一条）
  ——命令行 `status` 恰好在它要读的那台设备上复现程序清单要消灭的那个误导人的症状。要它读得准就得开管理员终端，那是
  spec 的 Problem Statement 第一条（"想看电量，得开一个管理员终端敲命令"），而托盘带着清单已经把这件事做了。
- **它要的每一样托盘都已经给了**，留着它就是同一轮结果的第二份排版：悬停提示立过"措辞与命令行那一行一个字不差"的
  规矩，两边各有一张陈旧标注的表，改一边要记得另一边；为了"命令行输出一个字不变"，Primary 候选不看来路那个缺陷
  （parking lot Q154）一直修不了。
- **真要终端的只剩做协议逆向的开发者**（spec 用户故事 50），他手上有源码树；诊断工具本来就挪成了示例程序
  （resident-tray 票 02）。

**一个 exe、保留子命令**（过渡期的样子）：程序是窗口程序（双击不闪黑框），子命令只能借父终端印字——终端不等它结束、
拿不到退出码，而且跟着提权（parking lot Q160）。过渡期能忍，成品不值得。

## Consequences

- 发布构建里没有在终端里看电量的办法，也没有给脚本用的出口（输出、退出码）；看电量靠托盘的悬停提示与右键菜单。
  要一个脚本出口是一个新决定。
- `clap` 只剩示例程序在用，挪进 `[dev-dependencies]`；`AttachConsole` 连同它要的 `Win32_System_Console` feature 一起删掉
  （parking lot Q132、Q160）。
- 诊断示例程序不带清单（`build.rs` 只嵌进 bin）：走 2.4G 的 `caps`、`probe` 要在管理员终端里跑，否则照旧是"读超时"
  （ADR-0007）。
- 两样东西只有命令行那一行印过，成品里不再看得到：**电压**与**"未充电"**——菜单行与悬停提示不写它们
  （`src/tray/hover.rs`）。`Ble` 那一级的"多久前"从取得时刻算到此刻，不再照印 Windows 当时报的缓存年龄（托盘常驻，
  那个数过一分钟就不对了）。"Primary Device 保持上次的选择"那句交代搬进了悬停提示的末尾（parking lot Q342）。
- 用户看得见的文字与文档不再叫人跑 `juicebar` 的子命令：`git grep -nE 'juicebar (scan|caps|probe|status|config-refresh)'`
  排掉 `docs/protocol.md` 的实测记录与 `.scratch/` 之后为零。
- 翻案面：要一个命令行出口，得另立一个 bin（或者子命令加 `AttachConsole` 那一套），并重新决定它不提权时读不了 2.4G
  该说什么。托盘内核交出的"这一轮的结果"是结构化的（`crate::round`），另一份排版从它排，不必动内核。
