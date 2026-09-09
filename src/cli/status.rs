//! `juicebar status` —— 一行一个 Device，打印当前电量。
//!
//! 这是取数链路的收口：配置 → 驱动 → 设备 → 一行看得懂的字。托盘将来显示的是
//! 同一组信息。

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};

use crate::bluetooth::{self, BleBattery};
use crate::clock::{Clock, SystemClock, Timestamp};
use crate::config::{Config, Device};
use crate::endpoints::{EndpointKind, EndpointReading, Endpoints, SystemEndpoints};
use crate::sources::{Reading, Transport, driver_for};
use crate::staleness::{Freshness, Staleness};

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

    // 枚举一次，全部 Device 共用：一次枚举要打开本机每一条 HID collection。
    let endpoints = SystemEndpoints::enumerate()?;
    let clock = SystemClock;
    // 一个 Device 都没配不是就此收摊：**那正是最需要下面那段未登记名单的时刻**（本机扫到的
    // 每一台 BLE 设备都还没登记，而 MAC 就在那几行里等着抄）。所以这里不再提前 return。
    if config.devices.is_empty() {
        println!("配置 {} 里一个 Device 都没有。", path.display());
    }
    for device in &config.devices {
        // 一个 Device 一个"当下"，取数与陈旧判定共用它。几台设备依次读下来会花掉真实
        // 时间，所以这个值在循环里取而不在循环外——但**一台设备只取一次**。
        let now = clock.now();
        let line = match read(device, &endpoints, now) {
            Ok(reading) => {
                let staleness = Staleness::assess(&reading, &config.general, now);
                render(&reading, &staleness)
            }
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
///
/// `now` 收的是一个**值**而不是 `&dyn Clock`：整条链路只在 [`run`] 里问一次"现在几点"，
/// 那一个"当下"再被取数与陈旧判定共用。问两次不是浪费而是**错的**——几条 Endpoint 依次
/// 试下来会花掉真实时间（一次鼠标超时就是三秒），两条 HID 各超时一次再落到 Ble，用第二个
/// "当下"去判第一个"当下"盖的时间戳，那一行印出来就不是"0 秒前"而是"6 秒前"。
/// 接缝本身（`crate::clock::Clock`）因此只出现在 [`run`] 的顶上，这一层往下全是纯函数。
pub fn read(device: &Device, endpoints: &dyn Endpoints, now: Timestamp) -> Result<EndpointReading> {
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
                .map(|reading| EndpointReading::from_hid(kind, reading, now)),
            // Ble 不是往返：它读的是 Windows 攒的属性缓存，**不经过协议驱动**
            // （所以这一支根本不碰 `read_with_driver`——配置里的 `driver` 只作用于
            // HID），也没有 Transport 可开。接缝那一侧直接交成品，因为只有它知道
            // 那份缓存有多旧。这里仍**不写通配分支**：加第四种时编译器还会把人指到
            // 这一行来。
            EndpointKind::Ble => endpoints.read_ble(device, now),
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
pub fn render(sourced: &EndpointReading, staleness: &Staleness) -> String {
    let EndpointReading {
        endpoint: _,
        reading,
        cache_age_secs: _,
        taken_at,
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
    // 陈旧到不该再显示数字的那一档，把电量、充电态、电压**整段换掉**：一个几个月前的
    // 百分比印出来就是一句假话，而"充电中"是同一句假话的另一半（几个月前在充电，
    // 现在呢？）。换上去的是那时唯一还成立的事实——这个数是哪一天的。
    // 百分比为什么不见了由 [`stale_marker`] 那半句交代。
    let level = format!(
        "{}%（Reported Level）{charging}{voltage}",
        reading.reported_level
    );
    // 对 `Freshness` **穷举、不写通配分支**：加一档时编译器会把人指到这一行来
    // （parking lot Q29 承诺的"翻案是加一个变体，波及 render 的一个 match"靠的正是这个，
    // 一个 `_` 就会让那句承诺落空）。
    let measured = match (staleness.freshness, taken_at) {
        (Freshness::VeryStale, Some(taken_at)) => format!("{taken_at} 的读数"),
        // VeryStale 而说不出取得时刻走不到：那一档要算出年龄才判得出来，而算得出年龄就
        // 有时刻。留着这一支是因为类型要交代一句，而**交代的话得是实话**：说不出是哪天的
        // 就只能照常印数字，由末尾那个陈旧标注去拦。
        (Freshness::VeryStale, None) => level,
        (Freshness::Fresh | Freshness::Stale, _) => level,
    };
    format!(
        "{measured}  {}{}",
        source_of(sourced, staleness),
        stale_marker(staleness.freshness),
    )
}

/// 陈旧标注：新鲜的读数什么都不加，另两档都以"已陈旧"开头。
///
/// 两档共用同一个词是有意的：`CONTEXT.md` 只给了一个 **Stale**，而用户要分辨的也只有
/// "能不能当现状"这一件事。给第二档另造一个词（"极旧"？"失效"？）只会多一个
/// `CONTEXT.md` 里没有的说法，而 Stale 那一条的 _Avoid_ 恰恰就是"过期、失效"。
///
/// 第二档多的半句是**交代那个百分比去哪了**：前面那一段已经换成了一个日期，不说一句
/// 用户会以为读数没读到——而它读到了，只是旧得不该再当数字报出来。
fn stale_marker(freshness: Freshness) -> &'static str {
    match freshness {
        Freshness::Fresh => "",
        Freshness::Stale => "  已陈旧",
        Freshness::VeryStale => "  已陈旧，不显示百分比",
    }
}

/// 来源那一段：这个数是从哪条 Endpoint 上来的，以及它有多旧。
///
/// `Ble` 与另两级的差别不是装饰。Wired 与 Dongle24G 是当场问出来的，`Ble` 读的是
/// Windows 攒的缓存，可能是几个月前的（`CONTEXT.md`：「数据取自 Windows 缓存而非当场问
/// 设备，因此天然可能陈旧」）。只印一个"来自 Ble"，用户分不出"刚问出来的 62%"和"三月份
/// 那个 62%"——而 spec 的第 6 条 user story 要的正是这个分辨。
///
/// **"多久前"三级都印**，不只蓝牙那一级：`status` 今天是一次性命令，HID 那一格恒为
/// "0 秒前"，但常驻轮询之后"这一行是几秒前还是几分钟前取的"才是真正要分辨的事。
/// 只有"（Windows 缓存，…）"那半句是 Ble 独有的——它说的是这个数**不是当场问出来的**。
///
/// **那个秒数两级各有各的来源，这不是重复而是两个不同的事实。**`Ble` 印的是
/// [`EndpointReading::cache_age_secs`]，也就是 **Windows 自己报的**那份缓存的年龄
/// ——它**不经过 Clock**（parking lot Q21 给本票的原话：「它是 Windows 报的秒数、不经过
/// Clock」；那份时间戳是系统攒的，本机时钟只能用来算差值，而 `bluetooth.rs` 已经算过了）。
/// 两条 HID 没有"缓存年龄"这回事，它们的秒数只能由取得时刻与当下相减得来，那正是
/// [`Staleness::age_secs`]。
///
/// **这里只排版，不判定**：阈值怎么定、够不够陈旧，都在 `crate::staleness` 里判完了。
fn source_of(sourced: &EndpointReading, staleness: &Staleness) -> String {
    let endpoint = sourced.endpoint;
    match endpoint {
        EndpointKind::Wired | EndpointKind::Dongle24G => {
            format!(
                "来自 {endpoint}（{}）",
                bluetooth::age_text(staleness.age_secs)
            )
        }
        EndpointKind::Ble => format!(
            "来自 {endpoint}（Windows 缓存，{}）",
            bluetooth::age_text(sourced.cache_age_secs)
        ),
    }
}

fn default_config_path() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| anyhow!("读不到环境变量 APPDATA，请用 --config 指定配置路径"))?;
    Ok(Path::new(&appdata).join("juicebar").join("config.toml"))
}
