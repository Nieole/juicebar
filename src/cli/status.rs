//! `juicebar status` —— 一行一个 Device，打印当前电量。
//!
//! 这是取数链路的收口：配置 → 驱动 → 设备 → 一行看得懂的字。托盘将来显示的是
//! 同一组信息。

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::bluetooth::{self, BleBattery};
use crate::cli::{config_refresh, resolve_config_path};
use crate::config::{Config, Device};
use crate::endpoints::{EndpointKind, EndpointReading, Endpoints, SystemEndpoints};
use crate::sources::{Reading, Transport, driver_for};

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    if !path.exists() {
        // 首次运行：先给一份扫出来的草稿，而不是打发用户去抄样例。草稿里扫不到的
        // Endpoint 是注释掉的占位，得他自己看一眼，所以这一趟到此为止、不接着取数。
        return config_refresh::bootstrap(&path);
    }
    let config = Config::load(&path)?;

    // 枚举一次，全部 Device 共用：一次枚举要打开本机每一条 HID collection。
    let endpoints = SystemEndpoints::enumerate()?;
    // 一个 Device 都没配不是就此收摊：**那正是最需要下面那段未登记名单的时刻**（本机扫到的
    // 每一台 BLE 设备都还没登记，而 MAC 就在那几行里等着抄）。所以这里不再提前 return。
    if config.devices.is_empty() {
        println!("配置 {} 里一个 Device 都没有。", path.display());
    }
    for device in &config.devices {
        let line = match read(device, &endpoints) {
            Ok(reading) => render(&reading),
            // 一台读不到不该拖累别的 Device，把原因印在它自己那一行上。
            Err(e) => format!("读不到 —— {e:#}"),
        };
        println!("{}  {line}", device.name);
    }

    // 未登记的 BLE 设备是**另一条输出维度**，不属于上面任何一行：它们是本机扫得到、
    // 而配置里没有对应 `[[device]]` 的那些。缺省不印。
    let unregistered = unregistered_ble_lines(&config, endpoints.scanned_ble());
    if !unregistered.is_empty() {
        println!();
        println!("未登记的 BLE 设备：");
        for line in unregistered {
            println!("  {line}");
        }
    }
    Ok(())
}

/// 本机扫到、而配置里没有登记的那些 BLE 设备，一台一行。
///
/// **名字里是"未登记"而不是"Unknown"**：`CONTEXT.md` 的 Unknown 说的是"一份 Reading 的电量
/// 字段无法采信"，是读数的状态；这里说的是"这台设备配置里没写"，两回事。配置键叫
/// `show_unknown_ble` 是用户看得见的名字、已经定了，代码这一侧不跟着漂。
///
/// 关着时是空的——那是缺省，因为一台机器上的 BLE 设备多数跟键鼠无关（耳机、手机、手环），
/// 全列出来只会把两行有用的埋掉。打开它的用处是接新设备：想登记的那台就在这几行里，
/// MAC 可以直接抄进 `[device.bluetooth]`。
///
/// **"登记过没有"看的是 MAC 对不对得上**，不是名字：`friendly_name` 虽然是唯一可靠的
/// 产品标识，但同型号两台会重名，而配置里认设备本来就是靠 MAC。认不出来的症状是同一台
/// 设备既在上面那一行、又在这一段里出现一次，像是本机有两只鼠标。
///
/// `pub` 与 [`read`] / [`render`] 同理：印出哪几台是 `status` 的外部行为。而"本机有哪些
/// BLE 设备"那一步要真去问 Windows，只有把这一半留成纯函数它才断言得到。
pub fn unregistered_ble_lines(config: &Config, scanned: &[BleBattery]) -> Vec<String> {
    if !config.general.show_unknown_ble {
        return Vec::new();
    }
    scanned
        .iter()
        .filter(|found| !is_registered(config, found))
        .map(|found| {
            let name = if found.friendly_name.is_empty() {
                "(无名)"
            } else {
                found.friendly_name.as_str()
            };
            // 电量那一格：`scanned_ble()` 交出来的快照来自 `bluetooth::enumerate`，它就地
            // 滤掉了没有电量属性的设备，所以实际上走不到 `None` 那一支。留着它是因为类型要
            // 交代一句，而**交代的话得是实话**：那种情形是"这台设备答不出电量"，不是"0%"。
            //
            // "（Reported Level）"这半句与 `render` 里那一段字面相同，**故意不抽成共用
            // 函数**：票 03 此刻正在改 `render` 里显示哪个百分比那一段（Reported 还是
            // Derived），抽一个两边共用的东西正好撞在它手上。等它落地，那时才知道该抽的
            // 是什么形状。
            let level = match found.level {
                Some(level) => format!("{level}%（Reported Level）"),
                None => "没有电量属性".to_string(),
            };
            // 不印"来自 Ble"：这一整段都是 BLE 设备，说一遍就够。留下的是"多久以前"
            // ——`scan` 的提示里那句话对这里同样成立：那一列才是它的真实可信度。
            format!(
                "{name}  {level}  {}  MAC {}",
                bluetooth::age_text(found.age_secs),
                found.address
            )
        })
        .collect()
}

/// 这台扫到的设备已经登记在某个 Device 的 `[device.bluetooth]` 里了吗。
fn is_registered(config: &Config, found: &BleBattery) -> bool {
    config.devices.iter().any(|device| {
        device
            .bluetooth
            .as_ref()
            .is_some_and(|configured| configured.matches(&found.address))
    })
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
            // Wired 与 Dongle24G 取数都是一次 HID 往返，要经过协议驱动。
            EndpointKind::Wired | EndpointKind::Dongle24G => endpoints
                .open_transport(device, kind)
                .and_then(|transport| read_with_driver(device, kind, transport.as_ref()))
                .map(|reading| EndpointReading::from_hid(kind, reading)),
            // Ble 不是往返：它读的是 Windows 攒的属性缓存，**不经过协议驱动**
            // （所以这一支根本不碰 `read_with_driver`——配置里的 `driver` 只作用于
            // HID），也没有 Transport 可开。接缝那一侧直接交成品，因为只有它知道
            // 那份缓存有多旧。这里仍**不写通配分支**：加第四种时编译器还会把人指到
            // 这一行来。
            EndpointKind::Ble => endpoints.read_ble(device),
        };
        match attempt {
            Ok(reading) => return Ok(reading),
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
/// "不在场"那句**不能只说"设备没插"**：本机枚举得到、却发不出这个协议要的那种报文的
/// 通路同样算不在场（键盘的 vendor collection 实测就是 `in:0 out:0 feat:65`），那时设备
/// 好端端插着，只是这条通路还没被接上。所以话要说到"枚举不到它们"为止，把没插当成一个
/// 问句而不是结论。
///
/// 三条 Endpoint 之后这句话不再点名"输出报文"：报文种类现在是**驱动**那一维的事
/// （键盘走 feature 报文），而 `Ble` 压根不是 HID——它不在场是因为本机的 BLE 设备里没有
/// 那个地址。一句话要同时对三种成立，就只能说到"枚举不到"，把三个可能的原因摆成问句。
fn lost_reason(device: &Device, failures: &[String]) -> anyhow::Error {
    if !failures.is_empty() {
        return anyhow!("全部 Endpoint 都没读到 —— {}", failures.join("；"));
    }
    // 问的是 `is_configured_in` 而不是 `hid_config_in`：后者对 Ble 恒为 None，拿它筛会把
    // 蓝牙从这份名单里漏掉，而这份名单就是要告诉用户"你配了这几条"。
    let configured: Vec<String> = EndpointKind::PRIORITY
        .into_iter()
        .filter(|kind| kind.is_configured_in(device))
        .map(|kind| kind.to_string())
        .collect();
    if configured.is_empty() {
        return anyhow!("这个 Device 一条 Endpoint 都没配置");
    }
    anyhow!(
        "配置的 Endpoint（{}）一条都不在场：本机枚举不到它们（设备没插？蓝牙没配对？跑 `juicebar scan` 看本机有什么）",
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
    let driver =
        driver_for(&device.driver).ok_or_else(|| anyhow!("驱动 {} 尚未实现", device.driver))?;
    driver.read_battery(transport)
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
    let EndpointReading {
        endpoint,
        reading,
        cache_age_secs,
    } = sourced;
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
        "{}%（Reported Level）{charging}{voltage}  {}",
        reading.reported_level,
        source_of(*endpoint, *cache_age_secs),
    )
}

/// 来源那一段：这个数是从哪条 Endpoint 上来的，以及——只有蓝牙才有的——它有多旧。
///
/// `Ble` 与另两级的差别不是装饰。Wired 与 Dongle24G 是当场问出来的，`Ble` 读的是
/// Windows 攒的缓存，可能是几个月前的（`CONTEXT.md`：「数据取自 Windows 缓存而非当场问
/// 设备，因此天然可能陈旧」）。只印一个"来自 Ble"，用户分不出"刚问出来的 62%"和"三月份
/// 那个 62%"——而 spec 的第 6 条 user story 要的正是这个分辨。
///
/// **这里只标注，不判定。**阈值（`stale_after` / `very_stale_after`）、变灰、只显示日期，
/// 连同为了可测而要引入的 Clock 接缝，全是票 06 的活。
fn source_of(endpoint: EndpointKind, cache_age_secs: Option<u64>) -> String {
    match endpoint {
        EndpointKind::Wired | EndpointKind::Dongle24G => format!("来自 {endpoint}"),
        EndpointKind::Ble => format!(
            "来自 {endpoint}（Windows 缓存，{}）",
            bluetooth::age_text(cache_age_secs)
        ),
    }
}
