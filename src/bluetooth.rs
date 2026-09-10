//! 蓝牙电量：读 Windows 已经缓存好的设备属性。
//!
//! 不需要 BLE GATT、不需要 WinRT、不需要管理员。细节见 `docs/protocol.md` 第 4 节。
//!
//! 最要紧的一点：这些值是缓存，设备离线也照样返回旧值且没有任何提示。实测见过
//! 2025 年 3 月的读数被当成"当前电量"。所以每次读电量都必须同时读更新时间戳。

use anyhow::{Result, bail};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT, CM_Get_DevNode_PropertyW,
    CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW, CM_LOCATE_DEVNODE_NORMAL,
    CM_Locate_DevNodeW, CR_SUCCESS,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_FriendlyName, DEVPROPTYPE};
use windows::Win32::Foundation::{DEVPROPKEY, FILETIME};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::core::{GUID, PCWSTR};

/// 电量。`{104EA319-6EE2-4701-BD47-8DDBF425BBE5}` PID 2，类型 BYTE，值域 0-100。
const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// 电量最后更新时间，FILETIME。未文档化，但这是判断读数新不新鲜的唯一依据。
const DEVPKEY_BLUETOOTH_BATTERY_UPDATED: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 7,
};

/// 实测只在真正连着的设备上出现，可当在线标志用。未文档化。
const DEVPKEY_BLUETOOTH_CONNECTED: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x995ef0b0_7eb3_4a8b_b9ce_068bb3f4af69),
    pid: 9,
};

/// MAC。同一 GUID 下的 VID/PID（pid 7/8）在容器节点上读不到——实测全是空，
/// 它们挂在 GATT 服务的子节点上。身份识别本来就靠 MAC + FriendlyName，不去追。
const DEVPKEY_BLUETOOTH_ADDRESS: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a),
    pid: 1,
};

/// 一台 BLE 设备的电量读数。
#[derive(Debug, Clone)]
pub struct BleBattery {
    /// 重读单台设备时的入口，目前只有 enumerate 在用
    #[allow(dead_code)]
    pub instance_id: String,
    /// 唯一可靠的产品标识——BLE 的 Device Information Service 只缓存了芯片型号，帮不上忙。
    pub friendly_name: String,
    /// MAC，唯一可靠的实例标识，配置文件里用它来认设备。
    pub address: String,
    pub level: Option<u8>,
    /// 读数距今多少秒。None 表示这台设备根本没有更新时间戳。
    pub age_secs: Option<u64>,
    pub connected: bool,
}

impl BleBattery {
    /// 这台设备的 [`age_text`]。留着这个方法是因为示例程序 `scan` 在用它。
    pub fn age_text(&self) -> String {
        age_text(self.age_secs)
    }
}

/// 把"多久以前"渲染成人能读的形式。
///
/// 抽成自由函数是因为 `status` 那一行手上只有秒数：一份读数经枚举接缝交出来时只带走了
/// 电量和这个秒数，没有整台 [`BleBattery`]（它不是 `Copy`）。两处印的是同一件事，
/// 措辞得是同一份——`scan` 说"多久前那一列才是它的真实可信度"，`status` 那一行不该
/// 换个说法。
///
/// 名字里没有 BLE，也不该有：`status` 现在**三条 Endpoint 都印"多久前"**，两条 HID 那一格
/// 的秒数由取得时刻与当下相减得来（见 `cli::status::source_of`）。这个函数只管把一个秒数
/// 说成人话，不管那个秒数是谁的。
pub fn age_text(age_secs: Option<u64>) -> String {
    match age_secs {
        None => "无时间戳".into(),
        Some(s) if s < 90 => format!("{s} 秒前"),
        Some(s) if s < 5400 => format!("{} 分钟前", s / 60),
        Some(s) if s < 172800 => format!("{} 小时前", s / 3600),
        Some(s) => format!("{} 天前", s / 86400),
    }
}

fn now_filetime() -> u64 {
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn split_multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// 列出某个枚举器（如 "BTHLE"）下当前在册的全部设备实例 ID。
pub fn instance_ids(enumerator: &str) -> Result<Vec<String>> {
    let filter = wide(enumerator);
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    unsafe {
        for _ in 0..8 {
            let mut len: u32 = 0;
            let cr = CM_Get_Device_ID_List_SizeW(&mut len, PCWSTR(filter.as_ptr()), flags);
            if cr != CR_SUCCESS {
                bail!("CM_Get_Device_ID_List_SizeW 失败: {cr:?}");
            }
            let mut buf = vec![0u16; len as usize];
            let cr = CM_Get_Device_ID_ListW(PCWSTR(filter.as_ptr()), &mut buf, flags);
            if cr == CR_SUCCESS {
                return Ok(split_multi_sz(&buf));
            }
        }
        bail!("设备列表反复变化，放弃枚举")
    }
}

fn locate(instance_id: &str) -> Option<u32> {
    let id = wide(instance_id);
    let mut dev_inst: u32 = 0;
    let cr =
        unsafe { CM_Locate_DevNodeW(&mut dev_inst, PCWSTR(id.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) };
    (cr == CR_SUCCESS).then_some(dev_inst)
}

/// 读一个属性的原始字节。属性不存在是常态（多数设备没有电量），返回 None。
fn raw_property(dev_inst: u32, key: &DEVPROPKEY) -> Option<Vec<u8>> {
    unsafe {
        let mut ty = DEVPROPTYPE(0);
        let mut size: u32 = 0;
        // 先探长度：属性不存在时这一次就会失败，不必再来第二次。
        let _ = CM_Get_DevNode_PropertyW(dev_inst, key, &mut ty, None, &mut size, 0);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let cr =
            CM_Get_DevNode_PropertyW(dev_inst, key, &mut ty, Some(buf.as_mut_ptr()), &mut size, 0);
        if cr != CR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }
}

fn prop_u8(dev_inst: u32, key: &DEVPROPKEY) -> Option<u8> {
    raw_property(dev_inst, key)?.first().copied()
}

fn prop_string(dev_inst: u32, key: &DEVPROPKEY) -> Option<String> {
    let b = raw_property(dev_inst, key)?;
    let u: Vec<u16> = b
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&u))
}

fn prop_filetime(dev_inst: u32, key: &DEVPROPKEY) -> Option<u64> {
    let b = raw_property(dev_inst, key)?;
    if b.len() < size_of::<FILETIME>() {
        return None;
    }
    let lo = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64;
    let hi = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as u64;
    Some((hi << 32) | lo)
}

/// 读一台 BLE 设备的电量。没有电量属性的设备返回 None。
pub fn read_one(instance_id: &str) -> Option<BleBattery> {
    let dev_inst = locate(instance_id)?;
    let level = prop_u8(dev_inst, &DEVPKEY_BLUETOOTH_BATTERY);
    let updated = prop_filetime(dev_inst, &DEVPKEY_BLUETOOTH_BATTERY_UPDATED);
    let now = now_filetime();

    Some(BleBattery {
        instance_id: instance_id.to_string(),
        friendly_name: prop_string(dev_inst, &DEVPKEY_Device_FriendlyName).unwrap_or_default(),
        address: prop_string(dev_inst, &DEVPKEY_BLUETOOTH_ADDRESS).unwrap_or_default(),
        level,
        // FILETIME 是 100ns 单位。时钟回拨时相减会下溢，saturating 处理成 0。
        age_secs: updated.map(|t| now.saturating_sub(t) / 10_000_000),
        connected: raw_property(dev_inst, &DEVPKEY_BLUETOOTH_CONNECTED).is_some(),
    })
}

/// 列出本机全部带电量属性的 BLE 设备。
///
/// 只扫 `BTHLE`：经典蓝牙（`BTHENUM`）设备一律没有这个属性，实测耳机手机全无。
pub fn enumerate() -> Result<Vec<BleBattery>> {
    Ok(instance_ids("BTHLE")?
        .iter()
        .filter_map(|id| read_one(id))
        .filter(|d| d.level.is_some())
        .collect())
}
