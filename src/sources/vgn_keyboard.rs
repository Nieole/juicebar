//! VGN 键盘（Neon75）的 2.4G 私有协议。细节见 `docs/protocol.md` 第 2 节。
//!
//! **和鼠标那套没有一处相同**：帧 64 字节而不是 16、校验和是 `255 − (sum & 255)`
//! 而不是 `0x55 − sum − report_id`、命令码是 dongle 专有的 `0xF7` 而不是 `4`、
//! 回包 `[0]` 是就绪标志而不是 cmd 回显。两条协议不要互相套用。

use anyhow::{Result, bail};

use crate::sources::{Driver, Reading, ReportKind, Transport, require_frame_len};

/// VGN 键盘这一族的驱动。配置里写 `driver = "vgn_keyboard"` 取到的就是它。
pub struct VgnKeyboard;

impl Driver for VgnKeyboard {
    fn report_kind(&self) -> ReportKind {
        ReportKind::Feature
    }

    fn read_battery(&self, transport: &dyn Transport) -> Result<Reading> {
        read_battery(transport)
    }
}

/// 一帧的长度，不含 Report ID。命令字节 + 1 字节校验和，其余补零到这个长度。
pub const FRAME_LEN: usize = 64;

/// getDongleData。dongle 专有，不在 `RyCmd` 表里。
///
/// **不是 `0x82`。**`0x82`（`GetBatterLevel`）是**有线本体**那条路的命令，dongle 上
/// 不适用；HUB 按 `productName.includes("2.4G")` 分流到 `getDongleData()` 才发这一条。
const CMD_GET_DONGLE_DATA: [u8; 7] = [0xF7, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

/// 回包 `[0]`：dongle 还没准备好，把命令再发一遍。
const NOT_READY: u8 = 0;

/// 回包 `[0]`：这一帧是本次请求的应答。实测恒为 `0x01`。
const READY: u8 = 1;

/// 就绪标志为 0 时最多试几次。
///
/// HUB 的 `getDongleData()` 是**无界递归**重试，这里要个上界：一条读不到的 Endpoint
/// 该及时让位给下一条，而不是把这一轮轮询卡死在自己身上。
///
/// **这个数是实测调出来的，不是拍的。**键盘闲置一段时间后，dongle 会连着好几次都答
/// 未就绪——本票落地时实测过：一台刚才还答得好好的键盘，隔一阵子头 6 次一次性轮询
/// 全是 `[0] == 0`，之后才转为就绪并稳定下来。取 10 次（约 900 ms）是因为它仍远小于
/// Dongle24G 的 60 秒轮询间隔，也和鼠标那条 3000 ms 读超时同一个数量级——两者等的是
/// 同一件事：一次到设备的空中往返。**上界仍然可能不够**（见 parking lot Q10）。
const READY_ATTEMPTS: u32 = 10;

/// 两次尝试之间等多久。HUB 每次发 `0xF7` 之前都 `sleep(100)`。
///
/// 不等就重发没有意义：dongle 侧那块 feature 缓冲区保存的是"最近一次应答"，
/// 立刻回读只会把同一帧再读一遍。**首发之前不等**——键盘醒着的时候首读就已就绪，
/// 那 100 ms 是白搭进去的延迟。
const RETRY_DELAY_MS: u64 = 100;

/// 取一次电量。
pub fn read_battery(transport: &dyn Transport) -> Result<Reading> {
    let request = build_frame(&CMD_GET_DONGLE_DATA)?;
    for attempt in 0..READY_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
        }
        let response = transport.exchange(&request)?;
        if let Some(reading) = parse_dongle_data(&response)? {
            return Ok(reading);
        }
    }
    bail!("dongle 连着 {READY_ATTEMPTS} 次都报未就绪，这一次没有可采信的读数")
}

/// 解析 `0xF7` 的回包。`Ok(None)` 是"dongle 还没准备好，把命令再发一遍"。
fn parse_dongle_data(frame: &[u8]) -> Result<Option<Reading>> {
    require_frame_len(frame, FRAME_LEN)?;
    // `[0]` 是就绪标志，不是 cmd 回显。未就绪的帧里 `[1]` 照样躺着一个像模像样的
    // 百分比，采信它就是凭空发明一个读数。
    if frame[0] == NOT_READY {
        return Ok(None);
    }
    // 到这里比 HUB 严：它只判 `result[0] == 0`，非零一律当就绪。这条通路上的 feature
    // 缓冲区保存的是"最近一次应答"，而 `0xF7` 的回包里**没有 cmd 回显可以对**——
    // 就绪标志是唯一能认出"这不是本次的应答"的字节，所以它得是个真正的标志位而不是
    // "非零即真"。实测抓到过一帧别的命令的残留 `F4 01 F4 01 …`，放行它就读出 1%。
    if frame[0] != READY {
        bail!(
            "回包的就绪标志是 {:#04X}，既不是 {NOT_READY:#04X}（未就绪）也不是 \
             {READY:#04X}（就绪）—— 这一帧不是本次请求的应答",
            frame[0]
        );
    }
    Ok(Some(Reading {
        reported_level: frame[1],
        // 充电位在 `[9]`，但**至今没有实测样本**：实测期间电池一直满电，插线只亮了很短
        // 的红灯就转绿，从没抓到过 `!= 0`（`docs/protocol.md` 第 7 节仍把它挂在待实测
        // 清单上）。spec 因此明写「键盘暂不显示充电态」——所以这里既不交 `true` 也不交
        // `false`，而是说"不知道"。等哪天电量掉下来复测过，这里换成 `Some(frame[9] != 0)`。
        charging: None,
        // 键盘的回包里没有电压。见 `Reading::voltage_mv` 那条文档注释。
        voltage_mv: None,
    }))
}

/// 拼一帧命令：命令字节原样在前，校验字节紧跟其后，再补零到 [`FRAME_LEN`]。
///
/// 命令长到放不下校验字节就是**构造错误**，绝不把一帧没有校验和的发出去：那样的帧
/// 会被设备**静默丢弃**，而 `HidD_SetFeature` 照样返回成功、`HidD_GetFeature` 照样
/// 交回缓冲区里的陈旧残留且不报错——整条键盘链路曾经就这样"看起来在工作"地全错。
///
/// 之所以是 `pub`：键盘这一族还有别的命令要发（有线本体的 `0x82`、初始化的 `0xF6`），
/// 而校验和这一份实现在仓库里只该有一处。鼠标那边的 `apply_checksum` 是同一个理由。
pub fn build_frame(command: &[u8]) -> Result<[u8; FRAME_LEN]> {
    if command.len() >= FRAME_LEN {
        bail!(
            "{} 字节的命令后面放不下校验字节，一帧总共只有 {FRAME_LEN} 字节",
            command.len()
        );
    }
    let mut frame = [0u8; FRAME_LEN];
    frame[..command.len()].copy_from_slice(command);
    frame[command.len()] = checksum(command);
    Ok(frame)
}

/// `crc = 255 − (sum(命令字节) & 255)`。
///
/// 原文 `RyServe.getCrc` 求和时把末位那个校验槽也算了进去，但它当时还是 0，
/// 所以等价于只对命令字节求和。求和按 u8 回绕：原始 JS 依赖 `Uint8Array` 的截断。
fn checksum(command: &[u8]) -> u8 {
    let sum = command.iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
    255 - sum
}
