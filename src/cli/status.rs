//! `juicebar status` —— 一行一个 Device，打印当前电量。
//!
//! 这是取数链路的收口：配置 → 驱动 → 设备 → 一行看得懂的字。托盘将来显示的是
//! 同一组信息。

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::config::{Config, Device, HidEndpoint};
use crate::hid::{self, HidInfo};
use crate::sources::hid_transport::{FeatureReportTransport, OutputReportTransport};
use crate::sources::{Reading, ReportKind, Transport, driver_for};

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let path = match config_path {
        Some(path) => path,
        None => default_config_path()?,
    };
    if !path.exists() {
        bail!(
            "没找到配置 {}。仓库里的 config.example.toml 是一份带注释的样例，\
             抄一份过去改改即可。",
            path.display()
        );
    }
    let config = Config::load(&path)?;
    if config.devices.is_empty() {
        println!("配置 {} 里一个 Device 都没有。", path.display());
        return Ok(());
    }

    // 枚举一次，全部 Device 共用：一次枚举要打开本机每一条 HID collection。
    let collections = hid::enumerate()?;
    for device in &config.devices {
        let line = match read(device, &collections) {
            Ok(reading) => render(&reading),
            // 一台读不到不该拖累别的 Device，把原因印在它自己那一行上。
            Err(e) => format!("读不到 —— {e:#}"),
        };
        println!("{}  {line}", device.name);
    }
    Ok(())
}

/// 取一次数。
fn read(device: &Device, collections: &[HidInfo]) -> Result<Reading> {
    // 配置里可以出现本次编译还没实现的驱动名，那该是这一行写着"尚未实现"。
    let driver =
        driver_for(&device.driver).ok_or_else(|| anyhow!("驱动 {} 尚未实现", device.driver))?;
    let endpoint = device
        .dongle_24g
        .as_ref()
        .ok_or_else(|| anyhow!("没有配置 [device.wireless_24g]"))?;
    let transport = open_transport(driver.report_kind(), collections, endpoint)?;
    driver.read_battery(transport.as_ref())
}

/// 按驱动声明的报文种类，找到配置指名的那条 collection 并在上面打开一条 Transport。
///
/// 两种报文各有各的实现，也各有各的筛选条件——这是驱动那一侧唯一看不见、也不该看见
/// 的差异。
fn open_transport(
    report_kind: ReportKind,
    collections: &[HidInfo],
    endpoint: &HidEndpoint,
) -> Result<Box<dyn Transport>> {
    match report_kind {
        ReportKind::OutputAndInput { read_timeout_ms } => {
            let collection = find_output_collection(collections, endpoint)?;
            Ok(Box::new(OutputReportTransport::open(
                collection,
                endpoint.report_id,
                read_timeout_ms,
            )?))
        }
        ReportKind::Feature => {
            let collection = find_feature_collection(collections, endpoint)?;
            Ok(Box::new(FeatureReportTransport::open(
                collection,
                endpoint.report_id,
            )?))
        }
    }
}

/// 在本机枚举到的 collection 里找配置指名的那一条，**限于能发输出报文的**。
///
/// 名字里的 output 是要紧的：走 feature 报文的通道 `out` 为 0（键盘的 vendor
/// collection 实测就是 `in:0 out:0 feat:65`），会被这里筛掉——那条路要用
/// [`find_feature_collection`]，否则报出来的是一句误导人的"设备没插"。
fn find_output_collection<'a>(
    collections: &'a [HidInfo],
    endpoint: &HidEndpoint,
) -> Result<&'a HidInfo> {
    // out 为 0 的通道发不出命令，不是它。
    find_collection(collections, endpoint, "输出报文", |c| c.output_len > 0)
}

/// 同上，**限于能发 feature 报文的**。键盘的 vendor collection 走的就是这条。
fn find_feature_collection<'a>(
    collections: &'a [HidInfo],
    endpoint: &HidEndpoint,
) -> Result<&'a HidInfo> {
    find_collection(collections, endpoint, "feature 报文", |c| {
        c.feature_len > 0
    })
}

/// 认 collection 用 VID/PID 加 usage page / usage，外加一条"这条通路发得出命令吗"。
///
/// `kind_label` 只进错误信息，是给人读的那半句。
fn find_collection<'a>(
    collections: &'a [HidInfo],
    endpoint: &HidEndpoint,
    kind_label: &str,
    can_send: impl Fn(&HidInfo) -> bool,
) -> Result<&'a HidInfo> {
    collections
        .iter()
        .find(|c| {
            c.vid == endpoint.vid
                && c.pid == endpoint.pid
                && c.usage_page == endpoint.usage_page
                && c.usage == endpoint.usage
                && can_send(c)
        })
        .ok_or_else(|| {
            anyhow!(
                "本机没有 VID {:04X} PID {:04X} UP {:04X} U {:04X} 这条能发 {kind_label} 的通路（设备没插？）",
                endpoint.vid,
                endpoint.pid,
                endpoint.usage_page,
                endpoint.usage
            )
        })
}

/// 电量后面那句 Reported Level 不是啰嗦：项目里还有一个由电压查表算出的
/// Derived Level，两个数可能不一致，而按来源挑哪一个尚未实现。在此之前把来源
/// 写明白，好过让人以为这就是最终口径。
fn render(reading: &Reading) -> String {
    // 键盘那一位至今没有实测样本，spec 明写「键盘暂不显示充电态」，所以说不上来时
    // 这一格整个不印——印"未充电"就是替设备做了一个没人验过的断言。
    let charging = match reading.charging {
        Some(true) => "  充电中",
        Some(false) => "  未充电",
        None => "",
    };
    // 键盘的回包里根本没有电压这一项，那一格同样整个不印。印一个 0 mV 会被当成读数。
    let voltage = match reading.voltage_mv {
        Some(mv) => format!("  {mv} mV"),
        None => String::new(),
    };
    format!(
        "{}%（Reported Level）{charging}{voltage}",
        reading.reported_level,
    )
}

fn default_config_path() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| anyhow!("读不到环境变量 APPDATA，请用 --config 指定配置路径"))?;
    Ok(Path::new(&appdata).join("juicebar").join("config.toml"))
}
