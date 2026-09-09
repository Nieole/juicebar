# 07: 厂商上位机暂停

**What to build:** 用户打开 VGN HUB 时，juicebar 自动让出 HID 通道，并且在界面上显示"暂停中"
而不是"失联"——后者会让用户以为设备出了问题。

这不是防御性设计：键盘 dongle 的 feature 报文是一块**保存最近一次应答的共享缓冲区**，两个程序
同时发命令会互相覆盖对方的应答，双方都读到错数据。HUB 同样驱动有线设备，所以两条 HID Endpoint
都要停。

**Blocked by:** 04

**Status:** ready-for-agent

- [ ] 按 `vendor_hub_processes` 配置检测进程
- [ ] 检测到时暂停 Wired 与 Dongle24G
- [ ] Ble 在暂停期间照常更新（它是本地属性读取，不参与竞争）
- [ ] `status` 里暂停态与失联态可区分
- [ ] 暂停期间保留最后读数而不是清空
- [ ] `pause_when_vendor_hub_running = false` 时不暂停
