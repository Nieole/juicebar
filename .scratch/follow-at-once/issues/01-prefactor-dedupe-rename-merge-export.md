# 01: prefactor —— 四处重复收成一处、十三条用例改名、两份导出脚本合一

**What to build:** 行为一个字不变的收拾，排在这个努力最前面：后面几张票要动的正是这几处，先收拾干净，它们就在
干净的地方写。四件事（spec「小尾巴」，parking lot Q202、Q232、Q348、Q282）：

1. "低不低电""算不算充电中"只在一处判：一轮里每台 Device 的状态（`DeviceState`）上加这两个方法，`Level` 加
   `percent()`；图标状态、低电通知、悬停提示都来问它，不再各写一遍 `percent < 阈值`、`charging_now(..) == Some(true)`、
   `Reported(p) | Derived(p)`。
2. "已暂停（X 正在运行）"只在一处定：收成上位机（`VendorHub`）上一个方法，短原因与悬停提示都来问它。
3. 配置用例里十三条 `config_refresh_*` 改成 `auto_fill_*` 前缀——名字里说的是一条已不存在的命令（ADR-0008）。"fill fills"
   撞车的几条换个动词。`config::refresh` 函数名与 `Refreshed` / `RefreshNote` **不动**（59 处，不是纯改名）。
4. 两份导出脚本合成一份 `tests/export_baselines.mjs`：起一次浏览器分别打开设计稿的两个锚点，各自的 JSON 写进各自的
   目录；任一像素认不出调色板就两个目录都不写。基准文件、比对它们的用例、设计稿页底的两个锚点都不动，
   `preview-gray.txt` 留在原处。

**Blocked by:** None —— can start immediately

**Status:** ready-for-agent

- [ ] `DeviceState` 上有"低不低电""算不算充电中"两个方法，`Level` 上有 `percent()`；图标状态、通知、悬停提示三处都改成
      问它，仓库里不再有第二处 `percent < 阈值` 的比较与第二处 `charging_now(..) == Some(true)` 的判断
- [ ] "已暂停（… 正在运行）"这句在仓库里只出现一次（`VendorHub` 的方法里），短原因与悬停提示的用例一个字不改仍然全绿
- [ ] `tests/config.rs` 里不再有 `config_refresh_` 开头的用例，十三条各有一个 `auto_fill_` 开头、读得通的名字；用例正文不动
- [ ] `tests/icon_baselines/export.mjs` 与 `tests/menu_baselines/export.mjs` 删掉，`tests/export_baselines.mjs` 一次导两种；
      两份脚本头上"改了设计稿两份都要跑"那句随之消失，`coding-standards.md` 或别处若提到两份脚本一并改口
- [ ] 跑一次 `node tests/export_baselines.mjs`（要有 Chrome 或 Edge 的机器），两个基准目录 `git diff` 为空
- [ ] 现有用例一条不删、一条不改断言（改名除外），全绿
- [ ] gate 三条全绿（`cargo test` 只能在 Windows 上跑）
