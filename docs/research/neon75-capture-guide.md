# VGN Neon75 2.4G 协议抓包与逆向调试指引

本文档面向需要在真机上排查并收口 **VGN Neon75（VID 0x3151 PID 0x5038 / 有线 0x502F）** 2.4G 电量读取协议的开发者。

---

## 1. 背景与核心疑问

在当前代码库与真机实测中，Neon75 存在一个关键存疑点（详见 `docs/gaps.md` 与 `docs/protocol.md` 第 2/7 节）：
- **现象**：向 `MI_02`（Feature 报文 65 字节）发送 `0xF7` 查询帧后，回读首字节的就绪位恒为 `0`（`00 00 62 00 00 01 01 01 ...`，其中第 0 字节为 Report ID `0x00`，第 1 字节为就绪位 `0x00`，第 2 字节为电量 `0x62` = 98%）。
- **疑问**：
  1. 为什么同一时刻 VGN Hub 能够正常显示电量？
  2. 缓冲区里的 `0x62` 到底是 Dongle 自身在空中维护的最新电量，还是 VGN Hub 之前通信后遗留在接收器里的静态缓存？
  3. VGN Hub 在查询电量前，是否向其他通道（例如 `MI_00` 标准键盘通道、`MI_01&col05` 纯输入通道）发送了未被逆向出的“唤醒/握手序列”？

为了彻底收口上述问题，需要借助**动态 Diff 抓包**与 **Wireshark 底层 USB 抓包**。

---

## 2. 方案 A：使用 `neon75_probe diff-watch` 动态差分抓包（最轻量推荐）

无需安装 Wireshark 即可抓取 HID 共享内存变化。

### 操作步骤

1. **开启终端管理员权限**：
   以管理员身份打开 PowerShell / Windows Terminal。
2. **启动差分监听器**：
   ```powershell
   cargo run --example neon75_probe -- diff-watch
   ```
   此时程序会打印初始的 65 字节缓冲区内容，并以 100ms 间隔监控。
3. **配合 VGN Hub 触发通信**：
   - 启动 VGN Hub 软件；
   - 在 Hub 中点击键盘设备，切换页面或点击“刷新/读取设备配置”；
   - 观察终端输出：一旦 Hub 发送命令并引起接收器缓冲区变化，终端会**立即高亮输出变化的字节下标及旧值 -> 新值**。
4. **监听纯输入中断通道**（排查是否有异步回包）：
   ```powershell
   cargo run --example neon75_probe -- diff-watch --listen-input
   ```
   在键盘上按键、切换旋钮或在 Hub 里操作，观察 `MI_01&col05`（32 字节输入通道）是否有报文产生。

---

## 3. 方案 B：使用 Wireshark + USBPcap 进行底层 USB 流量抓包

当需要查看完整的 USB 控制传输（URB Control Transfer）或非 HID 通道通信时，使用 USBPcap。

### 准备环境
1. 安装 [Wireshark](https://www.wireshark.org/)，安装过程中务必勾选 **Install USBPcap**。
2. 重启电脑以加载 USBPcap 驱动。

### 抓包实操步骤

1. **确定 USBPcap 接口**：
   - 打开 Wireshark，在网卡列表中能看到 `USBPcap1`、`USBPcap2` 等接口；
   - 找到键盘 2.4G 接收器所在的那个 USB 根集线器（可在设备管理器中按“依连接排序设备”查看）。
2. **设置 Wireshark 捕获过滤器**：
   - 双击对应的 `USBPcap` 接口开始抓包。
3. **设置 Wireshark 显示过滤器 (Display Filter)**：
   过滤目标设备（VID `0x3151`，PID `0x5038`）：
   ```wireshark
   usb.idVendor == 0x3151 && usb.idProduct == 0x5038
   ```
   如果抓包时 USB 描述符没有被重新读取，Wireshark 可能未解析出 VID/PID，可先拔插一次接收器，或按设备的 USB 地址过滤：
   ```wireshark
   usb.device_address == <设备地址>
   ```
4. **过滤 HID 类特定请求（Get_Report / Set_Report）**：
   ```wireshark
   usb.transfer_type == 0x02 && (usbhid.setup.bRequest == 0x01 || usbhid.setup.bRequest == 0x09)
   ```
   - `0x01` 为 `Get_Report`
   - `0x09` 为 `Set_Report`
5. **触发 Hub 通信并保存抓包文件**：
   - 打开 VGN Hub，等待其读出 100% 或电量数据；
   - 在 Wireshark 中停止抓包，保存为 `neon75-capture.pcapng`；
   - 重点检查在 `Set_Report` 之前，Hub 是否先发送了 Control 传输或访问了其他 Interface（`Interface 0`、`1`、`2`）。

---

## 4. 方案 C：断电/开机实验（验证静态残留还是动态同步）

`docs/gaps.md` 中提出的免抓包定性实验：

```powershell
cargo run --example neon75_probe -- verify-offline
```

该命令会自动引导以下步骤：
1. **第一阶段**：键盘开机且在 2.4G 档，采样当前基线；
2. **第二阶段**：提示关闭键盘电源（或切到蓝牙/有线），程序持续监控 30 秒，确认关机状态下缓冲区是否有跳变；
3. **第三阶段**：提示重新开启键盘电源，连续尝试发送 `F7` 并探测就绪位是否恢复为 `1`。

---

## 5. 常用调试命令速查

| 目的 | 命令 |
|---|---|
| 查看当前 HID 设备与电量属性 | `cargo run --example scan` |
| 查看 Neon75 各通道 Report ID | `cargo run --example caps -- --vid 3151 --pid 5038` |
| 自动连发 F7 探测就绪翻转 | `cargo run --example neon75_probe -- query --retries 20` |
| 动态差分捕获（配合 Hub） | `cargo run --example neon75_probe -- diff-watch` |
| 发送初始化序列 `F6 0A` | `cargo run --example neon75_probe -- send f6` |
| 发送自定义十六进制帧 | `cargo run --example neon75_probe -- send "F7 00 00 00 00 00 00"` |
| 通用 Probe 调试与解析 | `cargo run --example probe -- --vid 3151 --pid 5038 --usage-page ffff --usage 0002 --feature --vgn-keyboard-crc f7 --reads 5` |
