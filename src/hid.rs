//! HID 设备的枚举与读写。
//!
//! 枚举走 Config Manager 而不是 SetupAPI：`CM_Get_Device_Interface_ListW` 一次调用就拿到
//! 全部接口路径，不用维护 `HDEVINFO` 句柄的生命周期，也和 `bluetooth.rs` 那边取数的方式一致。
//!
//! 关于报文下标的坑见 `docs/protocol.md` 第 3 节：读写缓冲区的第 0 字节都是 Report ID，
//! 协议文档里所有下标都是不含 Report ID 的，落到这里要 +1。

use anyhow::{Result, anyhow, bail};
use std::ffi::c_void;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_ListW, CR_SUCCESS,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetFeature,
    HidD_GetHidGuid, HidD_GetManufacturerString, HidD_GetPreparsedData, HidD_GetProductString,
    HidD_SetFeature, HidP_GetCaps,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile, WriteFile,
};
use windows::Win32::System::IO::{CancelIo, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::PCWSTR;

/// 一条 HID collection 的静态信息。一个物理设备通常暴露多条。
#[derive(Debug, Clone)]
pub struct HidInfo {
    /// 设备接口路径，形如 \\?\hid#vid_391d&pid_1a05&mi_01&col05#...
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub version: u16,
    pub usage_page: u16,
    pub usage: u16,
    /// 含 Report ID 在内的报文长度，0 表示该方向没有报文
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    pub product: String,
    pub manufacturer: String,
}

impl HidInfo {
    /// 是否是厂商自定义页。私有协议通道一定落在这个区间。
    pub fn is_vendor_defined(&self) -> bool {
        self.usage_page >= 0xFF00
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把双 NUL 结尾的宽字符多字符串切成一条条。
fn split_multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// 列出本机全部 HID 接口路径。
fn interface_paths() -> Result<Vec<String>> {
    unsafe {
        let guid = HidD_GetHidGuid();

        // 尺寸可能在两次调用之间变化（设备热插拔），失败就重试几轮。
        for _ in 0..8 {
            let mut len: u32 = 0;
            let cr = CM_Get_Device_Interface_List_SizeW(
                &mut len,
                &guid,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            );
            if cr != CR_SUCCESS {
                bail!("CM_Get_Device_Interface_List_SizeW 失败: {cr:?}");
            }
            let mut buf = vec![0u16; len as usize];
            let cr = CM_Get_Device_Interface_ListW(
                &guid,
                PCWSTR::null(),
                &mut buf,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            );
            if cr == CR_SUCCESS {
                return Ok(split_multi_sz(&buf));
            }
        }
        bail!("HID 接口列表反复变化，放弃枚举")
    }
}

/// 以只查询方式打开：dwDesiredAccess = 0。
///
/// 这一步不能要读写权限——键盘、鼠标的标准 collection 被系统独占打开着，
/// 要权限会直接失败，而我们只是想看看它的 VID/PID 和 usage。
fn open_for_query(path: &str) -> Result<HANDLE> {
    unsafe {
        let h = CreateFileW(
            PCWSTR(wide(path).as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )?;
        Ok(h)
    }
}

fn hid_string(h: HANDLE, f: unsafe fn(HANDLE, *mut c_void, u32) -> bool) -> String {
    let mut buf = [0u16; 128];
    unsafe {
        let ok = f(h, buf.as_mut_ptr() as *mut c_void, (buf.len() * 2) as u32);
        if !ok {
            return String::new();
        }
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 读一条 collection 的完整信息。设备可能刚被拔掉，失败返回 None 而不是中断枚举。
fn describe(path: &str) -> Option<HidInfo> {
    let h = open_for_query(path).ok()?;
    let info = (|| -> Option<HidInfo> {
        unsafe {
            let mut attrs = HIDD_ATTRIBUTES {
                Size: size_of::<HIDD_ATTRIBUTES>() as u32,
                ..Default::default()
            };
            if !HidD_GetAttributes(h, &mut attrs) {
                return None;
            }

            let mut preparsed = Default::default();
            if !HidD_GetPreparsedData(h, &mut preparsed) {
                return None;
            }
            let mut caps = HIDP_CAPS::default();
            let status = HidP_GetCaps(preparsed, &mut caps);
            let _ = HidD_FreePreparsedData(preparsed);
            if status.is_err() {
                return None;
            }

            Some(HidInfo {
                path: path.to_string(),
                vid: attrs.VendorID,
                pid: attrs.ProductID,
                version: attrs.VersionNumber,
                usage_page: caps.UsagePage,
                usage: caps.Usage,
                input_len: caps.InputReportByteLength,
                output_len: caps.OutputReportByteLength,
                feature_len: caps.FeatureReportByteLength,
                product: hid_string(h, HidD_GetProductString),
                manufacturer: hid_string(h, HidD_GetManufacturerString),
            })
        }
    })();
    unsafe {
        let _ = CloseHandle(h);
    }
    info
}

/// 枚举本机全部 HID collection。
pub fn enumerate() -> Result<Vec<HidInfo>> {
    Ok(interface_paths()?
        .iter()
        .filter_map(|p| describe(p))
        .collect())
}

/// 一条打开着的、可读写的 HID collection。
pub struct HidHandle {
    handle: HANDLE,
    pub info: HidInfo,
}

impl HidHandle {
    /// 以读写方式打开，用于 input/output 报文。私有协议通道通常没被系统独占，
    /// 能开成功；开不成的最常见原因是厂商上位机正在跑。
    pub fn open(info: &HidInfo) -> Result<Self> {
        Self::open_with(info, FILE_FLAG_OVERLAPPED)
    }

    /// 同步方式打开，用于 feature 报文。
    ///
    /// `HidD_GetFeature` / `HidD_SetFeature` 内部是同步 IOCTL，句柄上带了
    /// `FILE_FLAG_OVERLAPPED` 反而会出问题，所以这条路单独开一个不带异步标志的句柄。
    pub fn open_sync(info: &HidInfo) -> Result<Self> {
        Self::open_with(info, FILE_FLAGS_AND_ATTRIBUTES(0))
    }

    fn open_with(info: &HidInfo, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Result<Self> {
        unsafe {
            let handle = CreateFileW(
                PCWSTR(wide(&info.path).as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
            .map_err(|e| anyhow!("打开 {} 失败（厂商上位机是否正在运行？）: {e}", info.path))?;
            Ok(Self {
                handle,
                info: info.clone(),
            })
        }
    }

    /// 发一份 feature 报文。VGN 键盘走的是这条路，不是 output 报文。
    ///
    /// 缓冲区第 0 字节同样是 Report ID，补零到 FeatureReportByteLength。
    pub fn set_feature(&self, report_id: u8, payload: &[u8]) -> Result<()> {
        let len = self.info.feature_len as usize;
        if len == 0 {
            bail!("该 collection 没有 feature 报文");
        }
        if payload.len() + 1 > len {
            bail!("帧长 {} 超过 feature 报文长度 {}", payload.len() + 1, len);
        }
        let mut buf = vec![0u8; len];
        buf[0] = report_id;
        buf[1..1 + payload.len()].copy_from_slice(payload);
        unsafe {
            if !HidD_SetFeature(self.handle, buf.as_mut_ptr() as *mut c_void, len as u32) {
                bail!("HidD_SetFeature 失败: {}", std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// 收一份 feature 报文，返回含 Report ID 的原始缓冲区。
    pub fn get_feature(&self, report_id: u8) -> Result<Vec<u8>> {
        let len = self.info.feature_len as usize;
        if len == 0 {
            bail!("该 collection 没有 feature 报文");
        }
        let mut buf = vec![0u8; len];
        buf[0] = report_id;
        unsafe {
            if !HidD_GetFeature(self.handle, buf.as_mut_ptr() as *mut c_void, len as u32) {
                bail!("HidD_GetFeature 失败: {}", std::io::Error::last_os_error());
            }
        }
        Ok(buf)
    }

    /// 发一份输出报文。
    ///
    /// `payload` 是不含 Report ID 的帧内容，本函数负责在前面补上 Report ID
    /// 并补零到 OutputReportByteLength。
    pub fn write_report(&self, report_id: u8, payload: &[u8]) -> Result<()> {
        let len = self.info.output_len as usize;
        if len == 0 {
            bail!("该 collection 没有输出报文");
        }
        if payload.len() + 1 > len {
            bail!("帧长 {} 超过输出报文长度 {}", payload.len() + 1, len);
        }
        let mut buf = vec![0u8; len];
        buf[0] = report_id;
        buf[1..1 + payload.len()].copy_from_slice(payload);

        unsafe {
            let event = CreateEventW(None, true, false, PCWSTR::null())?;
            let mut ov = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            let mut written: u32 = 0;
            let r = WriteFile(self.handle, Some(&buf), Some(&mut written), Some(&mut ov));
            let out = match r {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    if WaitForSingleObject(event, 1000) != WAIT_OBJECT_0 {
                        let _ = CancelIo(self.handle);
                        Err(anyhow!("写超时"))
                    } else {
                        GetOverlappedResult(self.handle, &ov, &mut written, false)
                            .map_err(|e| anyhow!("写失败: {e}"))
                    }
                }
                Err(e) => Err(anyhow!("写失败: {e}")),
            };
            let _ = CloseHandle(event);
            out
        }
    }

    /// 收一份输入报文，返回含 Report ID 的原始缓冲区。
    pub fn read_report(&self, timeout_ms: u32) -> Result<Vec<u8>> {
        let len = self.info.input_len as usize;
        if len == 0 {
            bail!("该 collection 没有输入报文");
        }
        let mut buf = vec![0u8; len];

        unsafe {
            let event = CreateEventW(None, true, false, PCWSTR::null())?;
            let mut ov = OVERLAPPED {
                hEvent: event,
                ..Default::default()
            };
            let mut read: u32 = 0;
            let r = ReadFile(self.handle, Some(&mut buf), Some(&mut read), Some(&mut ov));
            let out = match r {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    if WaitForSingleObject(event, timeout_ms) != WAIT_OBJECT_0 {
                        let _ = CancelIo(self.handle);
                        Err(anyhow!("读超时（{timeout_ms}ms 内没有回包）"))
                    } else {
                        GetOverlappedResult(self.handle, &ov, &mut read, false)
                            .map_err(|e| anyhow!("读失败: {e}"))
                    }
                }
                Err(e) => Err(anyhow!("读失败: {e}")),
            };
            let _ = CloseHandle(event);
            out?;
            buf.truncate(read as usize);
            Ok(buf)
        }
    }
}

impl Drop for HidHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
