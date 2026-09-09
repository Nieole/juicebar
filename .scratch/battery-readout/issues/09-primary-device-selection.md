# 09: Primary Device 选择

**What to build:** 默认盯住当前电量最低的那台设备，用户不用手动切换就能看见风险。但"最低"必须
有意义——一个几天前的蓝牙缓存值不该抢走这个位置。

**Blocked by:** 06

**Status:** ready-for-agent

- [ ] `primary = "lowest"` 时选出电量最低的 Device
- [ ] 只有新鲜且可信的 Reading 参与比较；陈旧、Unknown、失联的都不参与
- [ ] 正在充电的 Device 照常参与比较
- [ ] 全部不可信时保持上次的选择，不来回跳
- [ ] `primary = "<设备 id>"` 时钉死该 Device
- [ ] `status` 标出当前 Primary Device
