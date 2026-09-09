# 09: Primary Device 选择

**What to build:** 默认盯住当前电量最低的那台设备，用户不用手动切换就能看见风险。但"最低"必须
有意义——一个几天前的蓝牙缓存值不该抢走这个位置。

**Blocked by:** 06

**Status:** resolved

- [x] `primary = "lowest"` 时选出电量最低的 Device
- [x] 只有新鲜且可信的 Reading 参与比较；陈旧、Unknown、失联的都不参与
- [x] 正在充电的 Device 照常参与比较
- [x] 全部不可信时保持上次的选择，不来回跳 —— 规则已实现并测试；**"上次"存哪里待票 08 接线**（见下）
- [x] `primary = "<设备 id>"` 时钉死该 Device
- [x] `status` 标出当前 Primary Device

**第 4 条只挣到了规则那一半。** 「保持上次的选择」的规则实现了、也测了
（`primary::select` 收一个 `previous`，`Selection::HeldOver`），但**"上次"存在哪里不是本票的
活**：`status` 是一次性命令，手上没有"上次"，所以 `run()` 恒传 `None` ——
`HeldOver` 在今天的成品二进制里还走不到，而 `config.example.toml` 已经把这条行为写给用户了。

那份记忆归**票 08 的状态文件**，不归 `config.toml`（回写会把 `"lowest"` 这条规则本身洗掉）。
理由与给票 08 / 票 11 的交代见 parking lot **Q41**。**接线的那一步没有主**：票 08 正在并行
实现，它的票面上只有"上次已知读数"，没有"上次选出的 Primary Device id"。编排者得把这一格
派给票 08，否则这条验收框对用户始终是空的。
