# 04: Wired Endpoint 与优先级降级

**What to build:** 用户给鼠标插上线充电时，`status` 照常显示电量而不是"离线"；拔线的那一刻
数据源无缝切回 2.4G，不空窗。

插线会让鼠标**额外枚举出一个设备**（与 dongle 并存，不是替换），同时原来的 2.4G 通道超时。
如果不处理，充电中的鼠标就没有任何可用数据源。

**Blocked by:** 01

**Status:** resolved

- [x] 引入枚举接缝，回答"当前哪些 Endpoint 在场"，可在测试中替换
- [x] 鼠标 Wired Endpoint 可取数（与 dongle 同协议、同命令）
- [x] 取数按 `Wired > Dongle24G` 依次尝试，遇第一个成功即停
- [x] 靠前的 Endpoint 失败时在**同一周期内**立刻降级，不等下一轮
- [x] 场景测试：Wired 在场 → 用 Wired；Wired 从枚举中消失 → 同周期降级到 Dongle24G
- [x] Wired 在场时不去打扰 Dongle24G（不白发无线包）
- [x] `status` 标出这一行数据来自哪条 Endpoint
- [x] 全部 Endpoint 失败才算失联

实现落在 `src/endpoints.rs`（接缝）与 `src/cli/status.rs`（合成），用例在 `tests/status.rs`。
`Wired` 那条路只有场景测试跑过：实测时鼠标没插线，本机枚举不到 `391D:1005`，
所以真机上验到的是 `Dongle24G` 那一级和"来自 X"这个标注。岔路记在 parking lot Q16–Q19。
