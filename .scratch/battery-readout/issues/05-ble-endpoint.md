# 05: Ble Endpoint 接入

**What to build:** 两条 HID 通路都不可用时（设备关机、收起来了、接收器拔了），`status` 退到
蓝牙缓存兜底，并明确标出这个数来自蓝牙——用户需要能分辨"当场问出来的"和"系统攒的缓存"。

蓝牙那条已有实现，本票是把它接成第三级 Endpoint，而不是重写。

**Blocked by:** 04

**Status:** resolved

- [x] Ble 成为优先级最低的第三级 Endpoint
- [x] Ble 不经过协议驱动（它是系统属性读取，配置里的 driver 字段只作用于 HID）
- [x] 前两级都失败时退到 Ble
- [x] `status` 明确标出来源是蓝牙，与 2.4G/有线可区分
- [x] 配置里未登记的 BLE 设备是否显示，受 `show_unknown_ble` 控制
