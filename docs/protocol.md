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

> **实测后 juicebar 不照抄这个时序，只发 cmd 4。**
>
> 实测 cmd 3 回包（CRC 均验通），走 dongle 与走有线两条路 `[5]` 不同：
>
> ```
> 2.4G dongle（未充电）: 08 | 03 00 00 00 01 00 35 D4 97 …   data[5] = 00
> 有线本体（充电中）    : 08 | 03 00 00 00 01 01 35 D4 97 …   data[5] = 01
> ```
>
> 当时鼠标经 dongle 明明在线（同一时刻 cmd 4 答得好好的），`[5]` 却是 `00`，所以文档原来那句
> "`[5] > 0` 表示在线"至少不准确。`[5]` 更像是**充电/供电标志**——它的取值和 cmd 4 里
> `[6]` 的 charging 位在两次实测中完全同步（都是 `00` → `01`）。样本只有两组，不下定论。
>
> 地址位是对的：两条路 `[6][7][8]` 都是 `35 D4 97`，倒序拼出 `97:D4:35`。
>
> 既然 cmd 4 本身回不回有效帧就已经蕴含了在线与否，juicebar **直接拿 cmd 4 当在线判据**，
> 超时即离线。少一次空中往返，也少一个偏移存疑的字段。
>
> **超时不能用 1000ms。** 实测 cmd 4 在 1000ms 内超时过，改 3000ms 后每次都首读命中；而
> cmd 3 在 1000ms 内秒回。合理解释是 cmd 3 由 dongle 本地作答，cmd 4 要走一次到鼠标的
> 空中往返。驱动的读超时取 **3000ms**。

### 响应解析（cmd 4）

```
[0]     cmd 回显（应为 4，不是则丢弃）
[5]     level     固件自报的百分比
[6]     charging  == 1 表示充电中
[7..8]  voltage   毫伏，大端（(data[7] << 8) | data[8]）
```

### ⚠ 有意偏离 HUB：`level == 0` 不当作 100%

HUB 原文是：

```js
battery.level = result[1] == 0 ? 100 : result[1];
```

**juicebar 不照抄这一行。**照抄的后果是：键盘电量真正读到 0 时，托盘显示满电——不崩溃、不报警、
看着完全合理，而且偏偏发生在最该提醒充电的时刻。这与本工具存在的全部意义直接冲突。

判断依据：响应里 `[0]` 已经是独立的就绪标志（`[0] == 0` 时 HUB 自己会重发），所以 `[1]` 上那条
`0 → 100` 更像是第二道兜底，含义是"这个字段还没填好"，而不是"电量是 100"。

因此 juicebar 把 `level == 0` 一律当作**未知**处理，按第 6 节的原则：保留上次已知值并标记为陈旧，
或显示 `?`，**唯独不显示 100**。

两种可能都不会让这个选择更差：若设备确实会在低电时回 0，那更不能显示 100；若它只在未就绪时回 0，
那与 `[0]` 的就绪标志重复，当作未知也没有任何损失。

> 写在这里是为了防止后来者对着 `app.asar` 核对时，误以为这是实现错误而"改回去"。

### 实测记录（2026-09-09，电池由用户确认为满电 100%）

```
发送  [id 08] 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49
回包  08 04 00 00 00 02 5F 00 10 3E 00 00 00 00 00 00 9A
```

字段逐个对上，多次重跑字节级稳定：`[5]`=0x5F=95、`[6]`=00 未充电、`[7..8]`=0x103E=4158 mV
（另有 0x103A=4154 mV，±4mV 的抖动符合 ADC 特征）。

**最硬的证据是回包也带 CRC 且算法相同**：`0x55 − sum − reportId` 验算，`0xAF→0x9E`、
`0xB3→0x9A` 全部吻合，cmd 3 的回包同样验通（`0xA4→0xA9`）。帧结构是被证实的，不是推测的。

### 固件 level 与电压查表，哪个可信？—— 结论已修正为「高段固件更可信」

一度记为"固件低报、查表对"，**那个结论作废**：它依据的"电池此刻是 100%"是用户的口头判断，
而后续实测表明当时鼠标其实只有 95%。

真正的证据来自一次**受控充电**前后的对比（中间实测到 `charging=1` 的真实充电过程）：

| | 充电前 | 充电后 |
|---|---|---|
| 电压 | 4155 mV | 4190 mV |
| 固件 `[5]` | 95 | **100** |
| 电压查表 | 100（clamp） | 100（clamp） |

电池确实增加了电量，**固件 level 跟上了，查表没有**——因为表顶 4110 就是 100%，4155 与 4190
都越界、都被 clamp 成同一个值。在表顶这一段，**固件 level 携带的信息比查表多**。

能确定的：固件 `[5]` 不是死值，会随真实充放电变化，方向也正确。
不能确定的：它的绝对精度，以及中低电量段两者孰优。

**"HUB 也显示 100%" 不能作为佐证。** HUB 的取数逻辑（app.asar 原文）是：

```js
factLevel = battery.level;              // 固件自报
isBatVol  = battery.voltage > 0;
if (isBatVol) {
  factLevel = voltageToLevel(battery.voltage, battery.charging);   // 只要有电压就覆盖掉固件值
}
displayLevel = calculationBattery(lastLevel, factLevel, sec);
```

而 `voltageToLevel` 在表顶是硬 clamp：

```js
if (voltage > 4110) { return charging ? 99 : 100; }
```

所以 4155 mV 时 HUB **必然**显示 100%，与电池真实状态无关。用 HUB 的显示去验证查表是循环论证。

**给实现的建议**：不要无条件照抄"丢弃固件 level、只用查表"。至少在电压越过表顶（> 4110 mV）时
应改用固件 `[5]`，否则 95% 和 100% 会被压成同一个数。HUB 为什么坚持用查表尚不清楚，可能是为了
规避某些型号固件 level 不准——接入新设备时要重新评估。

### `voltageToLevel` 完整算法（实现时照此）

```
if voltage > 4110:  return 99 if charging else 100
找到第一个 voltages[i] > voltage，记作 index
if index == 0:      level = 0
else:
    interval = (voltages[index] - voltages[index-1]) / 5
    level    = (voltage - voltages[index-1]) / interval + (index-1) * 5
if level == 0 or level == 15:  level += 1        # HUB 的怪癖，原样保留
level = round(level)
```

注意 `charging` 会参与判定：充电且越过表顶时返回 **99** 而不是 100，避免充电中就显示满电。

### `calculationBattery` —— HUB 的"平滑"其实是变化率限幅

```js
if (sec > 1800)      level = factLevel;      // 距上次读数超过 30 分钟，直接采信
else if (sec < 60)   level = lastLevel;      // 不足 60 秒，完全不更新
else {
  theoryMax = min(lastLevel + 0.028 * sec, 100);   // 最快充电速率
  theoryMin = max(lastLevel - 0.014 * sec, 0);     // 最快放电速率
  level = factLevel > theoryMax ? theoryMax
        : factLevel < theoryMin ? theoryMin
        : lastLevel;                                // ← 落在区间内时保持旧值不动
}
```

上限 0.028%/秒（约 1.68%/分钟）、下限 0.014%/秒（约 0.84%/分钟），缓存按设备无线地址写进
localStorage。

**注意最后那个分支**：真实值落在合理区间内时，HUB 返回的是 `lastLevel` 而不是 `factLevel`——
也就是说只要变化"不异常"，显示值就**不更新**。这解释了为什么 HUB 的数字看起来很稳，也解释了
共识里"juicebar 只查表、不平滑"的必要性：这套限幅会让数字显著滞后于真实下降，而尽早知道该
充电正是本工具存在的意义。

### 实测记录补充（充电后，拔线静置）

```
回包  08 | 04 00 00 00 02 64 00 10 5E 00 … 75
                        level=0x64=100  charging=0  voltage=0x105E=4190 mV
```

### 有线模式（实测）—— 会多枚举一个设备，且**能读电量**

插上 USB 线后鼠标不是切换模式，而是多出一个设备（与键盘同构）：

| | VID/PID | REV |
|---|---|---|
| 2.4G dongle | `0x391D` / `0x1A05` | 0300 |
| 有线本体 | `0x391D` / `0x1005` | 0303 |

> `1005` 与 `1A05` 只差一个字符，写代码时极易混淆，务必小心。

两者 collection 布局完全相同（`mi_01&col05`、UP:FF02、in/out 17）。实测：

```
有线 cmd 4： 08 | 04 00 00 00 02 5F 01 10 8B 00 … 4C
                          level=95  charging=1  voltage=0x108B=4235mV
```

**有线本体照常响应 cmd 4，能读到完整的 level / charging / voltage。**这与共识里"有线模式只识别
在线并充电中、不读电量"的前提不同——那条共识假设有线路径拿不到电量，实测表明至少鼠标可以。
是否利用这一点由使用方决定，协议层如实记录。

同时，**鼠标接上线后 2.4G dongle 那条会超时**（cmd 4 三次全无回包）：设备不再经 2.4G 传数据。
所以插线时 dongle 侧的"离线"是预期行为，不是故障；判断在线要看有线本体是否枚举出来。

### 充电位校准（实测）

同一只鼠标，只改变 USB 线这一个变量：

```
拔线（经 dongle）: 08 | 04 00 00 00 02 5F 00 10 3E …   charging=0  voltage=4158 mV
插线（经有线）    : 08 | 04 00 00 00 02 5F 01 10 8B …   charging=1  voltage=4235 mV
```

`[6]` 的充电位如实翻转，**charging 字段就此确认**。电压同步升高也符合物理预期（充电电压高于
静置电压），等于给电压字段追加了一道独立佐证。

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

### 帧格式（实测修正） —— **必须带校验和**

之前逆向漏掉了校验和，导致发出去的帧全部被设备静默丢弃。`app.asar` 里 `RyServe.sendCmd` 的原文：

```js
async sendCmd(cmdContent, isRead = 0) {
  let cmd = new Uint8Array(cmdContent.length + 1);   // 7 字节命令 + 1 字节 CRC
  cmd.set(cmdContent, 0);
  cmd[cmd.length - 1] = await this.getCrc(cmd);
  while (cmd.length < 64) { cmd = new Uint8Array([...cmd, 0]); }  // 补零到 64
  return this.sendHidBuffer(cmd, isRead);
}
getCrc(data) { var cs = 0; for (var i = 0; i < data.length; i++) cs += data[i]; return 255 - (cs & 255); }
```

所以正确的帧是：

```
7 字节命令 + 1 字节 CRC，然后补零到 64 字节
crc = (255 - (sum(前 7 字节) & 255)) & 255      # 求和时末位那个 CRC 槽还是 0
```

**和鼠标那套完全不同**（鼠标是 `0x55 - sum - reportId`，16 字节帧），两条协议不要互相套用。

发送落到 Windows 上：`HidD_SetFeature`，缓冲区 = `[0x00]` + 64 字节 = 65 字节（Report ID 为 0，
`juicebar caps` 实测 `mi_02` 的 feature 报文不带编号）。随后立刻 `HidD_GetFeature` 读回，
对应 HUB 的 `sendFeatureReport` / `receiveFeatureReport`。**通道选 `mi_02` 是对的。**

### 命令表（`RyCmd`，实测自 app.asar）

原文档写"`0x87` 应是读电量"是**错的**。真实取值：

```
GetBatterLevel      130 (0x82)      SetLowBatter          2
GetDeviceInfo       143 (0x8F)      SetDeviceReset        1
GetReturnRate       131 (0x83)      SetReturnRate         3
GetConfig           132 (0x84)      SetConfig             4
GetCloseLight       133 (0x85)      SetCloseLight         5
GetKeyDebouncing    134 (0x86)      SetKeyDebouncing      6
GetKeyOption        137 (0x89)      SetKeyOption          9
GetSleepTime        145 (0x91)      SetSleepTime         17
GetMagneticAxisTravel 229           SetMagneticAxisTravel 101
```

**规律：`Get* == Set* + 128`，高位置 1 表示读。**照这个规律，`0x87`(135) 是 `Set#7` 的读命令，
与电量无关；原文档把它当成读电量是双重错误（既非电量命令、又没带校验和）。

判断一条命令安不安全，看它落在 `Set*` 那一列就是写命令，**不要发**。

### 2.4G dongle 的电量命令是 `0xF7`，不是 `0x82`

HUB 按 `productName.includes("2.4G")` 判断走的是 dongle 还是有线本体，dongle 走
`getDongleData()`：

```js
async getDongleData() {
  await this.sleep(100);
  const data = Uint8Array.of(247, 0, 0, 0, 0, 0, 0);        // 0xF7
  const ret = await this.sendCmd(data, this.isRead);
  if (result[0] == 0) { return await this.getDongleData(); } // 未就绪，重试
  this.deviceInfo.battery.level    = result[1] == 0 ? 100 : result[1];
  this.deviceInfo.battery.charging = result[9] == 0 ? false : true;
  ...
}
```

`0x82`（`GetBatterLevel`）是**有线本体**那条路走的命令，dongle 上不适用。`0xF7` 不在 `RyCmd`
表内，是 dongle 专有的。

有线本体那条 HUB 的解析是 `level = result[1]`、`voltage = result[3]`，而 **charging 直接硬编码为
`true`**（`charging = dongle == true ? false : true`）——即"有线即视为充电中"，并不读设备上报位。

### 响应解析（cmd 0xF7，dongle）

```
[0]   就绪标志   == 0 表示还没准备好，重发；非 0 才是有效响应
[1]   level      百分比，**0 要当作 100**（HUB 原文 `result[1] == 0 ? 100 : result[1]`）
[9]   charging   != 0 表示充电中
```

### 实测记录（2026-09-09，电池由用户确认为满电 100%）

```
发送  [id 00] F7 00 00 00 00 00 00 08  00 …(补零到 64)
回读  00 | 01 64 00 00 01 01 01 00 00 00 …
            ↑  ↑                    ↑
      就绪=1  level=0x64=100%   charging=0
```

三次重跑一致，`level` 与用户确认的真值 100% 吻合。**`[1]` 是电量这一点由 HUB 源码作证**
（`battery.level = result[1]`），不再是靠数值巧合推断的。

### 插线充电时 dongle 照常工作（实测，且与鼠标行为不同）

键盘保持 2.4G 档、同时插上 USB 线充电，dongle 侧用正确帧读到的结果与拔线时**完全一致**：

```
拔线：00 | 01 64 00 00 01 01 01 00 00 …
插线：00 | 01 64 00 00 01 01 01 00 00 …     ← 逐字节相同
```

两条结论：

1. **此前"插线后 dongle 归零/失联"的说法彻底作废**——那是无效帧读到陈旧缓冲区的假象。
   用正确帧时，插线完全不影响 dongle 读数。
2. `[9]` 的 charging 位仍为 `0`。实测时电池已满（充电指示灯插上后仅短暂红灯即转绿），
   所以这**很可能是正确读数**而非字段无效，但在电量真正偏低时复测之前不能确认。

**键盘与鼠标在这里行为不同，驱动要分别对待：**

| | 插线且模式开关在 2.4G | 有线本体是否枚举 | dongle 侧 |
|---|---|---|---|
| 键盘 Neon75 | 仅充电，数据仍走 2.4G | 否（需把开关拨到有线） | **照常应答** |
| 鼠标 Dragonfly 3 | 自动切到有线传数据 | 是（`391D:1005`） | **超时** |

所以"插线中"的判定不能一套逻辑通吃：鼠标看有线本体是否枚举，键盘得看 charging 位
（而该位尚未在真实充电场景下验证）。

### 初始化序列（备查，目前不发也能读到）

```js
async dongleInit() {
  const data1 = Uint8Array.of(247, 0, 0, 0, 0, 0, 0);   // 0xF7
  const ret = await this.sendCmd(data1, this.isRead);
  if (result[3] == 1) return false;
  const data2 = Uint8Array.of(246, 10, 0, 0, 0, 0, 0);  // 0xF6 0x0A
  await this.sendCmd(data2, this.isRead);
}
```

实测只发 `0xF7` 就能拿到有效响应，`0xF6` 那步暂时没有必要。

### 作废：曾经的「被动状态块」结论

排查过程中一度认为 `mi_02` 是一块不需要发命令的被动状态块——**那是错的**，成因是当时发出去的
帧没有校验和、全被设备丢弃，于是 `GetFeature` 每次读到的都是缓冲区里的**陈旧残留**，看起来
就像"发什么都一样"。

由此得出的几条结论一并作废，**不要引用**：

- ~~"有线本体 `502F` 状态块全零，所以有线读不到电量"~~ —— 那次读也是无效帧，需用正确 CRC 重测
- ~~"插着 USB 线时固件停止上报电量"~~ —— 同上，伪相关
- ~~"这块 blob 随设备在线状态变化"~~ —— 变化其实来自 dongle 重新枚举与 HUB 的残留数据

教训记在这里：**`HidD_SetFeature` 返回成功只代表驱动收下了，不代表设备认这一帧。**校验和错误
的帧会被静默丢弃，而 `GetFeature` 照样返回陈旧数据、不会报错。判断一条私有协议通不通，不能看
API 返回值，只能看回读内容是否随命令**有意义地**变化。

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
- `voltage` 落在 **3050..=4350** mV

  > **上界原本写的是 4110，那是个会误杀好帧的 bug。**满电锂电静置电压本就在 4.15V 上下，
  > 实测刚充满的鼠标稳定读到 4154–4158 mV，而那些帧的 CRC 全部验通、帧结构完全正确。
  > 照 4110 的上界，**每一次充完电都会被判成"帧结构变了"**——这不是边缘情况，是常态。
  >
  > 上界取 4350 而不是 4200：**充电时电压比静置更高**，实测插线充电中的鼠标稳定读到
  > 4231–4235 mV。锂电充电终止电压标称 4.20V ±1%，4350 给出足够余量又仍能识别出帧错乱。
  >
  > 与之配套：电压查表对 **> 4110 的读数 clamp 到 100%**。两处必须一起改，只改一处会自相
  > 矛盾（要么把满电当异常，要么查表越界）。
- 响应的 cmd 回显与请求一致
- 校验和对得上

一个诚实说"我不知道"的工具，比一个自信地显示 200% 的工具有用得多。

---

## 7. 待实测清单

- [x] ~~键盘的 vendor collection~~ —— 是 `MI_02`，且走 feature 报文（`juicebar scan` 实测）
- [x] ~~键盘 collection 的报文长度~~ —— feature 65 字节
- [x] ~~键盘回读里电量落在哪个下标~~ —— 逆向出的 `[8]/[9]` 是错的。真相是帧缺校验和导致命令被
      丢弃；补上 CRC 并改用 dongle 的 `0xF7` 后，`[1]`=level、`[9]`=charging，**已由 HUB 源码佐证**
- [x] ~~鼠标固件 `[5]` 的 level 与电压查表值差多少~~ —— 满电段固件低报 5 个点（固件 95%，
      真值与查表均为 100%），**查表对、固件错**
- [x] ~~有线模式下键盘以什么 VID/PID 出现~~ —— `0x3151/0x502F` REV_0501。**要把机身的模式开关
      拨到有线档才枚举得出来**：仅插线而开关还在 2.4G 档时它根本不出现，数据仍走 2.4G、dongle
      照常应答（第 2 节「插线充电时 dongle 照常工作」）。原先这一行还写着"状态块全零，读不到
      电量"，那是**无效帧造成的假象**（第 2 节「作废：曾经的『被动状态块』结论」），已删——
      有线本体到底读不读得到电量仍未实测，见下面那一条
- [x] ~~鼠标 `charging` 位~~ —— 已校准，插线时 `[6]` 由 `00` 翻为 `01`
- [ ] 键盘 `charging` 位（`[9]`）待验证 —— 实测期间电池一直满电、插线只亮了很短的红灯就转绿，
      没抓到 `charging != 0` 的样本
- [ ] 键盘**有线本体 `502F`** 用正确帧重测（`0x82` + CRC `0x7D`）—— 需要把键盘模式开关拨到
      有线，`502F` 才会枚举出来；仅插线而开关在 2.4G 档时它不出现
- [ ] 键盘 `[4][5][6]` 三个 `01` 的含义（HUB 只用了 `[0]`/`[1]`/`[9]`）
- [x] ~~有线模式下鼠标以什么 VID/PID 出现~~ —— `0x391D/0x1005` REV_0303，且**能读电量**
      （见第 1 节），同时 dongle 侧转为超时
- [ ] 电压查表中段（50%、20% 附近）准不准 —— 目前只知道**表顶那一段查表不可用**（会把 95%
      clamp 成 100%），中低段两者孰优完全未知
- [ ] 键盘 dongle 响应 `[3]` 的含义 —— 充电那一轮之后由 `00` 变为 `01` 并保持；
      `dongleInit()` 里 `result[3] == 1` 被当作失败条件，值得留意
- [ ] 键盘状态块 `[5][6][7]` 三个标志位各自的含义（在线？充电？通道？）
- [x] ~~偶发跳变 `00 F4 01 F4 01 …`~~ —— 已解释：那是缓冲区里另一条命令的陈旧残留，
      小端 0x01F4=500 落在回报率/睡眠/防抖的取值范围，符合 HUB 拉设备信息时留下的数据
