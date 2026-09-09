//! `juicebar status` —— 一行一个 Device，打印当前电量。
//!
//! 这是取数链路的收口：配置 → 驱动 → 设备 → 一行看得懂的字。托盘将来显示的是
//! 同一组信息。

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::cli::{config_refresh, resolve_config_path};
use crate::config::{Config, Device};
use crate::endpoints::{EndpointKind, EndpointReading, Endpoints, HidEndpoints};
use crate::sources::{Reading, Transport, vgn_mouse};

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    if !path.exists() {
        // 首次运行：先给一份扫出来的草稿，而不是打发用户去抄样例。草稿里扫不到的
        // Endpoint 是注释掉的占位，得他自己看一眼，所以这一趟到此为止、不接着取数。
        return config_refresh::bootstrap(&path);
    }
    let config = Config::load(&path)?;
    if config.devices.is_empty() {
        println!("配置 {} 里一个 Device 都没有。", path.display());
        return Ok(());
    }

    // 枚举一次，全部 Device 共用：一次枚举要打开本机每一条 HID collection。
    let endpoints = HidEndpoints::enumerate()?;
    for device in &config.devices {
        let line = match read(device, &endpoints) {
            Ok(reading) => render(&reading),
            // 一台读不到不该拖累别的 Device，把原因印在它自己那一行上。
            Err(e) => format!("读不到 —— {e:#}"),
        };
        println!("{}  {line}", device.name);
    }
    Ok(())
}

/// 取一次数：按 [`EndpointKind::PRIORITY`] 依次试在场的每一条 Endpoint，遇第一个
/// 成功即停。
///
/// 靠前的一条不行就**在同一次调用里**接着试下一条，不等下一个轮询周期——拔线那一刻
/// `Wired` 直接从枚举里消失（瞬时不在场而不是一次三秒的超时），降级几乎不要钱。
/// 在场的 Endpoint 全都没读到，才算失联。
///
/// `pub` 是为了让"拔线 → 同周期降级"这条路径能在 `tests/endpoints.rs` 里被驱动：
/// 集成测试只看得见 pub 接口，而这条路径在真机上拔一次线才复现一次。
pub fn read(device: &Device, endpoints: &dyn Endpoints) -> Result<EndpointReading> {
    let present = endpoints.present(device);
    let mut failures = Vec::new();

    for kind in EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| present.contains(kind))
    {
        // 挑哪个**驱动**不在这个函数里做，在 `read_with_driver` 里；这里只挑 Endpoint。
        let attempt = match kind {
            // Wired 与 Dongle24G 取数都是一次 HID 往返，要经过协议驱动。票 05 的
            // Ble 不是——它读的是 Windows 属性，不经过驱动。这里**不写通配分支**，
            // 好让加上第三种的那一天编译器把人指到这一行来。
            EndpointKind::Wired | EndpointKind::Dongle24G => endpoints
                .open_transport(device, kind)
                .and_then(|transport| read_with_driver(device, kind, transport.as_ref())),
        };
        match attempt {
            Ok(reading) => {
                return Ok(EndpointReading {
                    endpoint: kind,
                    reading,
                });
            }
            // 试过就记下原因，接着试下一条。全都失败时这些原因合起来就是失联的解释。
            Err(e) => failures.push(format!("{kind}: {e:#}")),
        }
    }

    Err(lost_reason(device, &failures))
}

/// 一条 Endpoint 都没读到时印出去的那句话。
///
/// 分两种说法，因为它们要用户做的事完全不同：一条都不在场多半是设备没插或配置写错，
/// 而在场却读不到才是设备那头出了事。
///
/// "不在场"那句**不能只说"设备没插"**：本机枚举得到、却没有输出报文的通路同样算不
/// 在场（键盘的 vendor collection 实测就是 `in:0 out:0 feat:65`），那时设备好端端插
/// 着，只是这条通路还没被接上。所以话要说到"枚举不到能发输出报文的通路"为止，
/// 把没插当成一个问句而不是结论。
fn lost_reason(device: &Device, failures: &[String]) -> anyhow::Error {
    if !failures.is_empty() {
        return anyhow!("全部 Endpoint 都没读到 —— {}", failures.join("；"));
    }
    let configured: Vec<String> = EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| kind.config_in(device).is_some())
        .map(|kind| kind.to_string())
        .collect();
    if configured.is_empty() {
        return anyhow!("这个 Device 一条 Endpoint 都没配置");
    }
    anyhow!(
        "配置的 Endpoint（{}）一条都不在场：本机枚举不到它们能发输出报文的通路（设备没插？）",
        configured.join("、")
    )
}

/// 按配置里的 `driver` 挑协议驱动，取一次数。
///
/// **驱动分发只在这个函数里做，[`read`] 里不再做。**下面那几行是从 `read` 里原样
/// 搬过来的，一个字没改：`read` 现在只管 Endpoint 的优先级与降级，两维正交
/// ——加一个驱动改这里，加一条 Endpoint 改 `read`。合并时若看到 `read` 里还留着
/// 驱动分发，那是"搬家"和"改写"撞在了一起，正确的归宿是这里。
///
/// `_endpoint` 现在没人用：一个驱动要覆盖该协议家族的所有 Endpoint 种类，内部按
/// 种类分支（键盘的 Dongle24G 与 Wired 命令不同；鼠标两者相同，分支自然退化），
/// 所以这个参数是留给驱动那一维的，不是留给 `read` 的。
fn read_with_driver(
    device: &Device,
    _endpoint: EndpointKind,
    transport: &dyn Transport,
) -> Result<Reading> {
    // 配置里可以出现本次编译还没实现的驱动名，那该是这一行写着"尚未实现"。
    if device.driver != "vgn_mouse" {
        bail!("驱动 {} 尚未实现", device.driver);
    }
    vgn_mouse::read_battery(transport)
}

/// 电量后面那句 Reported Level 不是啰嗦：项目里还有一个由电压查表算出的
/// Derived Level，两个数可能不一致，而按来源挑哪一个尚未实现。在此之前把来源
/// 写明白，好过让人以为这就是最终口径。
///
/// 末尾那句"来自 X"是另一种来源：同一个 Device 的几条 Endpoint 在不同时刻各自可用，
/// 不写出来就看不出这一行是插着线读的还是走 2.4G 读的——而"插着线"恰恰是那个最容易
/// 被误报成离线的时刻。
///
/// `pub` 与 [`read`] 同理：这一行印成什么样是 `status` 的外部行为。
pub fn render(sourced: &EndpointReading) -> String {
    let EndpointReading { endpoint, reading } = sourced;
    format!(
        "{}%（Reported Level）  {}  {} mV  来自 {endpoint}",
        reading.reported_level,
        if reading.charging {
            "充电中"
        } else {
            "未充电"
        },
        reading.voltage_mv
    )
}
