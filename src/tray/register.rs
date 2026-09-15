//! 登记设备：本机扫到、还没登记的蓝牙设备，登记到已有的 Device 上；解除某台 Device 的蓝牙登记（spec「菜单」"登记设备 ›"，
//! 结构照设计稿，`tests/menu_baselines/register*.txt`）。
//!
//! 子菜单里是几组，组与组之间一条分隔线，哪一组没有就不列；一组都没有时只有一行灰字。
//!
//! **点了之后只交出写回**（[`on_command`]）：外壳读此刻文件的全文、照配置那道缝格式保留地写（`crate::config::register_ble`），
//! 写完靠重读生效——手上那一份配置不当场改（parking lot Q312）。

use crate::bluetooth::BleBattery;
use crate::clock::Timestamp;
use crate::config::Config;
use crate::endpoints::EndpointKind;

use super::Tray;
use super::menu::{Entry, Item, Kind};

/// "登记设备"子菜单那一行的字。
const REGISTER: &str = "登记设备";

/// 一台未登记的蓝牙设备子菜单里，"登记到"那一项的字。
const REGISTER_TO: &str = "登记到";

/// "解除蓝牙登记"子菜单那一行的字。
const UNREGISTER: &str = "解除蓝牙登记";

/// 子菜单里一组都没有时的那一行灰字。
const NOTHING_TO_REGISTER: &str = "没有可登记的设备（蓝牙设备要先在 Windows 里配对）";

/// 登记设备这一块收的事件。
#[derive(Debug)]
pub enum Event {
    /// 外壳扫了一遍本机的蓝牙设备：扫到的那些（一次 [`crate::bluetooth::enumerate`]），扫不了时是完整原因。
    Scanned(Result<Vec<BleBattery>, String>),
}

/// 登记设备这一块的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 扫一遍本机的蓝牙设备（一次 [`crate::bluetooth::enumerate`]），扫到的交回来（[`Event::Scanned`]）。
    Scan,
}

/// "登记设备"里点得到的一项（随 [`super::menu::Command::Register`] 进来）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// "登记到"里点了一台：把本机扫到的这个蓝牙地址登记到它上面。
    RegisterBle {
        /// 登记到哪一台：它的 id。
        device_id: String,
        /// 本机扫到的那个蓝牙地址，照 Windows 报的写法。
        address: String,
    },
    /// "解除蓝牙登记"里点了一台：删掉它的蓝牙登记。
    UnregisterBle {
        /// 解除哪一台：它的 id。
        device_id: String,
        /// 菜单上它右列写着的那个地址（点的那一刻配置里登记的）：只进日志，登记错了照着日志还登记得回去。
        address: String,
    },
}

/// 登记设备这一块手上的东西。
#[derive(Default)]
pub(super) struct Register {
    /// 上一遍扫到的本机蓝牙设备；还没扫回来过是空的。
    found: Vec<BleBattery>,
    /// 上一次要扫一遍的那一刻；还没要过是 `None`。
    asked_at: Option<Timestamp>,
    /// 要了的那一遍还没交回来。
    scanning: bool,
    /// 上一次扫不了时记进日志的那个原因；上一遍扫成了是 `None`。同一个原因接连扫不了只记一次。
    failed: Option<String>,
}

impl Register {
    /// 时钟走到了（启动那一刻也算一格）：还没要过扫描，或者离上一次要已经过了 `poll_interval_bluetooth`，就要一遍；外头还有
    /// 一遍没交回来就不要——它与取数排在同一条队上，一次 HID 超时就是三秒，不该在队里排出一串扫描
    /// （parking lot Q310）。一遍扫描与 Ble 那一级每次取数读的是同一种东西——Windows 攒的设备属性——所以借它的节奏。
    pub(super) fn on_tick(
        &mut self,
        config: &Config,
        now: Timestamp,
        out: &mut Vec<super::Action>,
    ) {
        let interval = config.general.poll_interval_bluetooth;
        let due = self.asked_at.is_none_or(|asked| {
            now.as_unix_secs() >= asked.as_unix_secs().saturating_add(interval)
        });
        if due && !self.scanning {
            self.asked_at = Some(now);
            self.scanning = true;
            out.push(super::Action::Register(Action::Scan));
        }
    }

    /// 登记设备那一侧的事：扫到了就换上这一遍；扫不了就照上一遍扫到的列，完整原因记进日志——同一个原因接连扫不了只记
    /// 一次，每 10 秒一行同样的话是噪音。
    pub(super) fn on_event(&mut self, event: Event, out: &mut Vec<super::Action>) {
        // 交回来了（扫到了，或者扫不了），到了间隔就能再要。
        self.scanning = false;
        match event {
            Event::Scanned(Ok(found)) => {
                self.found = found;
                self.failed = None;
            }
            Event::Scanned(Err(reason)) => {
                if self.failed.as_ref() != Some(&reason) {
                    out.push(super::Action::Log(format!(
                        "扫不了本机的蓝牙设备，登记设备里照上一遍扫到的列 —— {reason}"
                    )));
                    self.failed = Some(reason);
                }
            }
        }
    }
}

/// "登记设备"里点了一项：记一行日志说点了什么，交给外壳写回。写不写得进只有外壳知道，写不进由它再记一行。
pub(super) fn on_command(command: Command, out: &mut Vec<super::Action>) {
    match command {
        Command::RegisterBle { device_id, address } => {
            out.push(super::Action::Log(format!(
                "登记设备 —— 把蓝牙地址 {address} 登记到 {device_id}"
            )));
            out.push(super::Action::Config(super::config::Action::RegisterBle {
                device_id,
                address,
            }));
        }
        Command::UnregisterBle { device_id, address } => {
            out.push(super::Action::Log(format!(
                "登记设备 —— 解除 {device_id} 的蓝牙登记（它登记的是 {address}）"
            )));
            out.push(super::Action::Config(
                super::config::Action::UnregisterBle { device_id },
            ));
        }
    }
}

impl Tray {
    /// 一级菜单里的"登记设备 ›"，连同它子菜单里的几项。
    pub(super) fn register_submenu(&self) -> Item {
        let mut groups: Vec<Vec<Item>> = Vec::new();
        let unregistered = unregistered(&self.config, &self.register.found);
        if !unregistered.is_empty() {
            groups.push(unregistered);
        }
        // 票 12：认得但没登记的 HID 设备那一组排在这里。
        groups.extend(unregister(&self.config).map(|item| vec![item]));
        let items = if groups.is_empty() {
            vec![Item::Entry(Entry {
                grayed: true,
                ..Entry::new(Kind::Normal, NOTHING_TO_REGISTER)
            })]
        } else {
            groups.join(&Item::Separator)
        };
        Item::Entry(Entry::new(Kind::Submenu(items), REGISTER))
    }
}

/// 未登记的蓝牙设备那一组：本机扫到、带电量属性、地址不在任何 Device 里的，每一台一个子菜单，右列写扫到它的那条通路
/// （Ble），里面是"登记到 ›"——**只列还没有蓝牙地址的 Device**，已经有地址的不列：换地址是先解除、再登记，一点不会悄悄
/// 覆盖旧地址。
fn unregistered(config: &Config, found: &[BleBattery]) -> Vec<Item> {
    found
        .iter()
        // 带电量属性的才列：登记上了，Ble 那一级读的正是这个属性。
        .filter(|found| found.level.is_some() && !is_registered(config, found))
        .map(|found| {
            let devices: Vec<Item> = config
                .devices
                .iter()
                .filter(|device| device.bluetooth.is_none())
                .map(|device| {
                    Item::Entry(Entry {
                        command: Some(super::menu::Command::Register(Box::new(
                            Command::RegisterBle {
                                device_id: device.id.clone(),
                                address: found.address.clone(),
                            },
                        ))),
                        ..Entry::new(Kind::Normal, device.name.clone())
                    })
                })
                .collect();
            // 一台都登记不上（都已经有地址了）就变灰、不带子菜单（设计稿，parking lot Q284）。
            let register_to = if devices.is_empty() {
                Entry {
                    grayed: true,
                    ..Entry::new(Kind::Normal, REGISTER_TO)
                }
            } else {
                Entry::new(Kind::Submenu(devices), REGISTER_TO)
            };
            // 票 12："新建一台 Device"跟在"登记到"后面。
            Item::Entry(Entry {
                right: Some(EndpointKind::Ble.to_string()),
                ..Entry::new(
                    Kind::Submenu(vec![Item::Entry(register_to)]),
                    found.friendly_name.clone(),
                )
            })
        })
        .collect()
}

/// "解除蓝牙登记 ›"：有蓝牙地址的每台 Device 一项，右列写它登记的地址（照配置里的写法），登记错了看得出是哪个。一台
/// 都没有就没有这一组。
fn unregister(config: &Config) -> Option<Item> {
    let bound: Vec<Item> = config
        .devices
        .iter()
        .filter_map(|device| {
            let bluetooth = device.bluetooth.as_ref()?;
            Some(Item::Entry(Entry {
                right: Some(bluetooth.address.clone()),
                command: Some(super::menu::Command::Register(Box::new(
                    Command::UnregisterBle {
                        device_id: device.id.clone(),
                        address: bluetooth.address.clone(),
                    },
                ))),
                ..Entry::new(Kind::Normal, device.name.clone())
            }))
        })
        .collect();
    (!bound.is_empty()).then(|| Item::Entry(Entry::new(Kind::Submenu(bound), UNREGISTER)))
}

/// 这台扫到的蓝牙设备已经登记在某台 Device 上了吗：看地址对不对得上，写法不同也算
/// （[`crate::config::BluetoothEndpoint::matches`]）。登记过的那台已经是一级里的一行设备行，这里再列一遍就是同一台说在两处。
fn is_registered(config: &Config, found: &BleBattery) -> bool {
    config
        .devices
        .iter()
        .filter_map(|device| device.bluetooth.as_ref())
        .any(|bluetooth| bluetooth.matches(&found.address))
}
