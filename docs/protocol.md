# 设备电量协议

这份文档记录 juicebar 读电量所依赖的全部协议细节。**代码可以重写，这些数字重挖一次要一小时**，所以任何实测修正都往这里写回。

出处：VGN HUB **4.0.6** 的 `resources/app.asar`（Electron 应用，JS 明文）。不是抓包得来的，因此存在版本漂移风险——固件或上位机更新后命令号可能变。

---

## 0. 两条数据来源的根本差异

| | 2.4G | 蓝牙 |
|---|---|---|
| 取数方式 | 主动向设备发查询包 | 读 Windows 缓存的设备属性 |
| 新鲜度 | 实时（当场问出来的） | **可能过期几个月** |
| 成本 | 走无线链路，会打扰设备 | 纯本地内存读，几微秒 |
| 通用性 | 每个品牌一套私有协议 | 所有 BLE 设备通用 |

**这个差异必须反映到界面上**：两种来源的数字可信度不是一个量级，混在一起显示而不加区分就是误导。

---

## 1. VGN 2.4G —— 鼠标（Dragonfly 3 Master+）

证据链完整，HUB 配置与注册表 usage page 完全对得上，可直接实现。

### 设备定位

| 项 | 值 |
|---|---|
| dongle VID/PID | `0x391D` / `0x1A05` |
| 序列号 | `541505796617` |
| collection | `MI_01&Col05`，Usage Page `0xFF02` Usage `0x0002` |
| Report ID | **8** |
| HUB 内部标识 | `customDeviceType: "vgn-y2"`，`productName: "VGN USB Receiver"` |
| 固件版本 | REV_0300（实测，与 HUB 逆出的协议同代） |

`juicebar scan` 实测该 collection 是 `in:17 out:17`——16 字节帧加 1 字节 Report ID，
与逆出的帧格式严丝合缝。

同一个 dongle 还有一条历史枚举记录 `VID_373B&PID_11D9`（REV 0301，序列号相同），是固件升级前的身份，**不必支持**。

### 帧格式（16 字节）

```
下标  0    1..3   4      5..14      15
     cmd   0      len    payload    crc
```

不带参数时 `len` 与 `payload` 全为 0。构造时下标 15 先填占位 `0xEF`，随后被 crc 覆盖。

### 校验和

```
sum = (data[0] + data[1] + ... + data[14]) mod 256      # 注意：不含下标 15
crc = (0x55 - sum) mod 256
data[15] = (crc - report_id) mod 256                     # report_id = 8
```

Rust 侧全部用 `wrapping_sub` / `wrapping_add`，原始 JS 依赖 `Uint8Array` 赋值的截断行为。

### 命令表

```
1   EncryptionData          14  GetCurrentConfig
2   PCDriverStatus          15  SetCurrentConfig
3   DeviceOnLine            16  ReadCIDMID
4   BatteryLevel  ★         17  EnterMTKMode
5   DongleEnterPair         18  ReadVersionID
6   GetPairState            20  Set4KDongleRGB
7   WriteFlashData          21  Get4KDongleRGBValue
8   ReadFlashData           22  SetLongRangeMode
9   ClearSetting            23  GetLongRangeMode
10  StatusChanged           24  SetDongleRGBBarMode
11  SetDeviceVidPid         25  GetDongleRGBBarMode
12  SetDeviceDescriptorStr  29  GetDongleVersion
13  EnterUsbUpdateMode      176 MusicColorful
```

**只发 3 和 4。** 其余命令里有 `EnterUsbUpdateMode`(13)、`ClearSetting`(9)、`SetDeviceVidPid`(11) 这类会改设备状态的，绝不能误发——这也是不做盲试探测的理由。

### 查询时序

HUB 的做法是先问在线再问电量，照抄：

1. 发 `cmd 3`（DeviceOnLine）→ 响应 `[5] > 0` 表示在线，`[6][7][8]` 是设备无线地址的第 3/2/1 字节（**倒序**，拼成 addr 时要反过来）。
2. 在线才发 `cmd 4`（BatteryLevel）。
3. 离线时 HUB 用 1500ms 间隔重试；juicebar 按共识改为 60 秒定频，不做快速重试。

### 响应解析（cmd 4）

```
[0]     cmd 回显（应为 4，不是则丢弃）
[5]     level     固件自报的百分比
[6]     charging  == 1 表示充电中
[7..8]  voltage   毫伏，大端（(data[7] << 8) | data[8]）
```

### 电压换算表 —— 为什么不用 `[5]`

HUB **丢弃固件自报的 `level`**，改用电压查表。21 档，每档 5%：

```
3050 3420 3480 3540 3600 3660 3720 3760 3800 3840 3880
3920 3940 3960 3980 4000 4020 4040 4060 4080 4110
 0%   5%  10%  15%  20%  25%  30%  35%  40%  45%  50%
 55%  60%  65%  70%  75%  80%  85%  90%  95% 100%
```

（上排 21 个电压对应下排 21 个百分比，档间线性插值。）

HUB 在此之上还做了一层跨时间的平滑，缓存写进 localStorage，所以会出现 `93.524` 这种小数。**juicebar 按共识只查表、不平滑**——平滑会让数字滞后于真实下降，而这个工具存在的意义正是尽早知道该充电了。

`[5]` 的实际精度**待实测**：HUB 特意绕开它，八成有原因，但也可能只是历史包袱。

---

## 2. VGN 2.4G —— 键盘（Neon75）

### 设备定位

| 项 | 值 |
|---|---|
| dongle VID/PID | `0x3151` / `0x5038` |
| collection | `MI_02`，Usage Page `0xFFFF` Usage `0x0002` |
| 通道类型 | **feature 报文**，长度 65（1 字节 Report ID + 64 字节数据） |
| Report ID | 0 |
| HUB 内部标识 | `customDeviceType: "rongyuan"`，`productName: "VGN Neon75 Dongle"` |
| 固件版本 | REV_0801（实测） |

### collection 是怎么定下来的

HUB 配置写的是 `path: "mi_01&col01"`，但那条 collection 的 usage 是 `0x000C/0x0001`（消费控制），
不是私有通道。`juicebar scan` 实测把这个疑点解决了——该 dongle 上两条 vendor 通道是：

```
mi_01&col05    UP:FFFF U:0001  in:32  out:0   feat:0     ← 只能收，发不了命令
mi_02          UP:FFFF U:0002  in:0   out:0   feat:65    ← 唯一能发命令的通道
```

**键盘走的是 feature 报文，不是 output 报文。** `mi_01&col05` 的 `out` 为 0，根本发不出去；
`mi_02` 输入输出都是 0，只有 65 字节的 feature 报文。所以键盘侧要用
`HidD_SetFeature` / `HidD_GetFeature`，和鼠标那条 output/input 的路径完全不同。

### 帧格式

发 7 字节：

```
87 00 00 01 00 02 00
```

`0x87` = 135。同族的 `getVersion()` 发 `82 01 00 01 00 06 00`，`0x82` = 读，`0x04` = 写——所以字节 0 是操作码，`0x87` 应是"读电量"。字节 5 疑似长度（电量给 2，版本给 6）。

实际报文长度大于 7（响应在下标 8/9），命令应会被补零到 collection 的报文长度。**具体长度待实测。**

### 响应解析

```
[8]           level     百分比
[9] 高 4 位   charging  == 1 表示充电中
```

---

## 3. HID 读写的两个坑

**发送**：`WriteFile` / `HidD_SetOutputReport` 的缓冲区第 0 字节必须是 Report ID，之后才是上面说的帧内容。鼠标是 `[0x08, cmd, ...]` 共 17 字节；键盘 Report ID 为 0 时首字节填 `0x00`。缓冲区长度必须**正好等于** collection 的输出报文长度（`HidP_GetCaps` 的 `OutputReportByteLength`），不足补零。

**接收**：`ReadFile` 返回的缓冲区第 0 字节同样是 Report ID，**因此上文所有下标都要 +1**。这是最容易一次性写错的地方。

**feature 报文是第三条路**：`HidD_SetFeature` / `HidD_GetFeature` 内部是同步 IOCTL，
句柄不能带 `FILE_FLAG_OVERLAPPED`，要单独开一个同步句柄。缓冲区第 0 字节同样是 Report ID，
长度必须正好等于 `FeatureReportByteLength`。VGN 键盘走的就是这条。

**独占**：VGN HUB 在跑时会和我们抢同一条通道，响应可能串台。按共识——检测到 HUB 进程就暂停 2.4G 轮询。手工实测时先退出 HUB。

---

## 4. 蓝牙 —— 通用，任何 BLE 设备

不需要 BLE GATT、不需要 WinRT、不需要管理员。三步：

```
1. CM_Get_Device_ID_List_SizeW / CM_Get_Device_ID_ListW
     pszFilter = "BTHLE"
     ulFlags   = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT   (0x1 | 0x100)

2. CM_Locate_DevNodeW(&dev_inst, id, 0)

3. CM_Get_DevNode_PropertyW(dev_inst, &key, &type, buf, &size, 0)
```

要读的属性：

| DEVPROPKEY | 类型 | 含义 |
|---|---|---|
| `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` PID **2** | BYTE | **电量 0-100**（`DEVPKEY_Bluetooth_Battery`） |
| 同 GUID PID **7** | FILETIME | **电量最后更新时间**，判新鲜度用（未文档化） |
| 同 GUID PID 3 | BOOLEAN | 实测全设备恒为 False，含义不明，不用 |
| `{995EF0B0-7EB3-4A8B-B9CE-068BB3F4AF69}` PID 9 | — | 只在真正连着的设备上出现，可当在线标志 |
| `DEVPKEY_Device_FriendlyName` | STRING | 产品名，**唯一可靠的产品标识** |
| `DEVPKEY_Bluetooth_DeviceAddress` | STRING | MAC，唯一的实例标识 |
| ~~`DEVPKEY_Bluetooth_DeviceVID` / `DevicePID`~~ | UINT16 | **实测在容器节点上读不到**，全是空。它们挂在 GATT 服务的子节点上。身份识别本来就靠 MAC + FriendlyName，不必去追 |

### ⚠️ 值是缓存的，会骗人

设备离线也照样返回旧值，**没有任何提示**。实测样本：

- Dragonfly 读出 95%，时间戳是 8/30（当时它正走 2.4G）
- FUN60 读出 51%，时间戳是 **2025 年 3 月**

所以读电量必须同时读 PID 7 的时间戳。共识定的阈值：**超 1 小时标灰，超 24 小时只显示日期不显示百分比**。

### 不适用范围

经典蓝牙（`BTHENUM\`）设备一律没有这个属性，实测耳机手机全无。这条路只对 BLE (`BTHLE\`) 有效——VGN 三模键鼠走的正是纯 BLE (HOGP + BAS)，够用。

### 备用路径（暂不实现）

两台设备都暴露了标准 Battery Service `0x180F`，所以 WinRT `BluetoothLEDevice::FromIdAsync` + `GattCharacteristic(0x2A19)` 能主动读实时值。设备接口 ID 现成放在 `{3B2CE006-5E61-4FDE-BAB8-9B8AAC9B26DF}` PID 8。但这条要异步运行时、要设备当前真连着蓝牙，复杂度高一个数量级，**只在 DEVPKEY 值过期且确有需要时才考虑**。

---

## 5. 设备身份对照表

2.4G 与蓝牙是两套完全独立的身份，**没有任何字段能自动缝合**，只能靠配置文件手写。

| | Neon75 键盘 | Dragonfly 3 Master+ |
|---|---|---|
| 2.4G dongle VID/PID | `0x3151` / `0x5038` | `0x391D` / `0x1A05` |
| 蓝牙 VID/PID | `0x3151` / `0x5027` | `0x3554` / `0xF533` |
| 蓝牙 MAC | `f4ee2553b27e` | `e452430072a9` |
| 蓝牙 FriendlyName | `VGN Neon75` | `Dragonfly 3 Master+` |
| 主控芯片 | Panchip `pan1080xa3` | Nordic `nrf54l05` |
| 容器类别 | Input.Keyboard | Input.Mouse |

**键盘两侧 VID 相同（`0x3151`）纯属巧合，鼠标就对不上**——dongle 和设备自己的 BLE 射频是两颗不同的芯片，各有各的 VID。别把"VID 相同"写成合并规则。

**VID 不能用来认品牌**：`0x3151` 底下还挂着同一台机器上的 M7W RGB BT1 和 FUN60 BT-1（同 OUI `F4:EE:25`），大概率同厂代工。

BLE 的 Device Information Service 只缓存了 Manufacturer 和 Model 两个字符串，而且是**芯片型号不是产品型号**，对识别产品没帮助。序列号（`0x2A25`）Windows 根本没缓存。

---

## 6. 合理性校验（防固件漂移）

协议是从特定版本的上位机逆出来的，**厂商推个新固件就可能失效**。驱动读到数之后必须校验，失败就报"读取异常"，**绝不显示一个荒谬的数字**：

- `level` 落在 0..=100
- `voltage` 落在 3050..=4110 mV（超出说明帧结构变了，不是电池坏了）
- 响应的 cmd 回显与请求一致
- 校验和对得上

一个诚实说"我不知道"的工具，比一个自信地显示 200% 的工具有用得多。

---

## 7. 待实测清单

- [x] ~~键盘的 vendor collection~~ —— 是 `MI_02`，且走 feature 报文（`juicebar scan` 实测）
- [x] ~~键盘 collection 的报文长度~~ —— feature 65 字节
- [ ] 键盘发 `87 00 00 01 00 02 00` 后，回读的 65 字节里电量落在哪个下标（协议说数据区 [8]，
      加上 Report ID 应是缓冲区 [9]，待 `probe --feature` 验证）
- [ ] 鼠标固件 `[5]` 的 level 与电压查表值差多少
- [ ] 型号疑点：截图显示 "Neon75 Ultra"，但 HUB 设备表里没有 Ultra 键盘（带 Ultra 的都是鼠标），蓝牙 FriendlyName 是 "VGN Neon75"
- [ ] 有线模式下两台设备各以什么 VID/PID 出现（只需识别在线，不读电量）
