//! 本机设备的插拔：HID 设备接口出现或消失时，系统给托盘那扇窗口发一条 `WM_DEVICECHANGE`，外壳把它递进内核
//! （[`config::Event::DevicesChanged`]）。扫不扫、什么时候扫、扫到了补不补，都是内核的事（`crate::tray::config`）。
//!
//! **登记的是 HID 这一类设备接口**（`RegisterDeviceNotificationW`），而不是只等系统广播给顶层窗口的"设备树变了"
//! （`DBT_DEVNODES_CHANGED`）：后者 U 盘、声卡一动都来，说不出变的是什么；登记来的只在 HID 设备接口出现
//! （`DBT_DEVICEARRIVAL`）或消失（`DBT_DEVICEREMOVECOMPLETE`）时来，也不管窗口是不是顶层窗口。一根线插上来几条
//! collection 就来几次，合并成一遍扫描在内核里（parking lot Q270）。

use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetHidGuid;
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_DEVICEINTERFACE,
    DEV_BROADCAST_DEVICEINTERFACE_W, DEV_BROADCAST_HDR, DEVICE_NOTIFY_WINDOW_HANDLE, HDEVNOTIFY,
    RegisterDeviceNotificationW, UnregisterDeviceNotification,
};

use crate::tray::{Event, config};

use super::log::LogFile;

/// 托盘那扇窗口在系统那里登记的"HID 设备接口来了、走了告诉我"。丢掉它就注销。
pub(super) struct DeviceWatch(Option<HDEVNOTIFY>);

impl DeviceWatch {
    /// 登记。**登记不上也照常起来**：插拔时不再自动补空块（启动时那一遍照旧），原因记进日志——为一件顺带的事让
    /// 托盘起不来，不值。
    pub(super) fn register(hwnd: HWND, log: &mut LogFile) -> Self {
        let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            // SAFETY: 只问一个 GUID。
            dbcc_classguid: unsafe { HidD_GetHidGuid() },
            ..Default::default()
        };
        // SAFETY: filter 是一份填好的 DEV_BROADCAST_DEVICEINTERFACE_W，调用期间一直活着；hwnd 是托盘那扇窗口。
        let handle = unsafe {
            RegisterDeviceNotificationW(
                HANDLE(hwnd.0),
                (&raw const filter).cast(),
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        };
        match handle {
            Ok(handle) => Self(Some(handle)),
            Err(e) => {
                log.write(&format!(
                    "登记不了设备插拔的通知，插上设备时不会自动补空块（启动时照旧补） —— {e}"
                ));
                Self(None)
            }
        }
    }
}

impl Drop for DeviceWatch {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            // SAFETY: 注销上面登记来的那一个；`drop` 只走一次。
            let _ = unsafe { UnregisterDeviceNotification(handle) };
        }
    }
}

/// 窗口收到了 `WM_DEVICECHANGE`：是 HID 设备接口来了或走了，就递进内核。
///
/// 回 `TRUE`，与缺省处理一样：这条消息里只有几种"可不可以"的询问看回的是什么，`TRUE` 是准许。
pub(super) fn changed(wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if is_hid_interface(wparam, lparam) {
        super::feed(Event::Config(config::Event::DevicesChanged));
    }
    LRESULT(1)
}

/// 这一条说的是一个设备接口来了或走了。登记的只有 HID 这一类，所以设备接口就是 HID 的。
///
/// 要看 `lParam` 那份头：U 盘的分区（`DBT_DEVTYP_VOLUME`）来了、走了，系统也广播给每一扇顶层窗口，事件是同样的两种。
fn is_hid_interface(wparam: WPARAM, lparam: LPARAM) -> bool {
    if !matches!(
        wparam.0 as u32,
        DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE
    ) || lparam.0 == 0
    {
        return false;
    }
    // SAFETY: 这两种事件的 lParam 指向一份以 DEV_BROADCAST_HDR 开头的结构，处理这条消息期间有效。
    let header = unsafe { &*(lparam.0 as *const DEV_BROADCAST_HDR) };
    header.dbch_devicetype == DBT_DEVTYP_DEVICEINTERFACE
}
