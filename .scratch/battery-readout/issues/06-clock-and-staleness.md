# 06: Clock 接缝与陈旧判定

**What to build:** 用户能一眼看出某个读数是刚问出来的还是很久以前的。蓝牙缓存可能过期几个月，
在最需要它的时候给出假的安全感——这正是这个工具要防的事。

**Blocked by:** 05

**Status:** ready-for-agent

- [ ] 引入 Clock 接缝，陈旧判定不依赖真实时间流逝，测试无需 sleep
- [ ] HID Endpoint 的陈旧阈值 = 3 × 该 Endpoint 的轮询间隔，自动推导，不设配置项
- [ ] Ble 使用配置里的 `stale_after` / `very_stale_after`
- [ ] 超过 `very_stale_after` 的 Ble 读数只显示日期，不显示百分比
- [ ] `status` 每行显示"多久前"
- [ ] 陈旧的读数被明确标注，与新鲜的可区分
