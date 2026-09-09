# 10: 配置自举与 config-refresh

**What to build:** 首次运行的用户拿到一份能直接用的配置草稿，而不必从零手写 VID/PID。草稿要
诚实地写出"这一块现在扫不到、你插线后跑某某命令补全"——用户不该在不知情的情况下缺一整条 Endpoint。

Wired Endpoint 只在插线时才枚举得到，自举那一刻通常缺席。键盘更麻烦：要把机身模式开关拨到
有线档才会枚举出来，仅插线是不够的。

**Blocked by:** 04

**Status:** resolved

- [x] 首次运行且无配置时生成带注释的草稿，能猜的先猜、猜不动的留空并注明
- [x] 扫不到的 `[device.wired]` 以**注释掉的占位**形式写入，并写明补全办法
- [x] `config-refresh` 命令扫描当前在场的 Endpoint 并填充空着的块
- [x] `config-refresh` 只填空缺，不动已有内容
- [x] 草稿里 `level_source` 显式写出，便于用户看见这个开关的存在
