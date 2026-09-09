//! 状态文件：上次已知的读数，一个 Device 一条，跨重启活下来。
//!
//! 为什么非要落盘：`status` 是一次性命令，当场枚举、当场取数、印一行就退出。设备收进
//! 抽屉或者关了机，这一趟一条 Endpoint 都读不到，而进程刚启动内存里什么都没有——"上次
//! 是多少电"只可能来自磁盘。
//!
//! **它只放运行时缓存，不放用户可见的配置选择**（`docs/adr/0003` 划的界）：Primary 的
//! 手动选择回写 `config.toml` 本身，理由是单一事实来源；而上次已知的读数不是用户做的
//! 选择，把它写进用户手写的那份文件里只会让那份文件多一段没人该读的噪音。两份文件因此
//! 都各有各的形态：配置要保住注释（走 `toml_edit`），这一份**程序自己写自己读、用户不该
//! 编辑**，所以既不需要格式保留也不需要注释，`toml` + serde 就够。
//!
//! **历史值必须会过期。**这个模块最要紧的一句话不是"读得回来"，而是它读回来的东西
//! 不能伪装成现状：超过 `very_stale_after` 的记录在[读回来那一步][LastKnown::reading_for]
//! 就被丢掉（陈旧判定那一侧不管这件事，见 parking lot Q29），活下来的那些由
//! `cli::status::render` 明确标注成上次已知值。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::config::General;
use crate::endpoints::{EndpointKind, EndpointReading};
use crate::sources::Reading;

/// 这一行的数字是这一趟取到的，还是从状态文件里拿出来的上次已知值。
///
/// 为什么这件事非要单独一维、不能由陈旧判定顶替：**一份十秒前取到的读数是新鲜的，而同一份
/// 读数在设备失联之后拿出来就不是现状了**——它有多新和设备此刻在不在，是两个独立的事实。
/// 只看阈值，那一行会给一只已经收进抽屉的鼠标印一个不带任何标注的 95%，而那正是这张票
/// 要防的假的安全感（票面："这个历史值不能伪装成现状"）。
///
/// 住在这个模块而不是 `cli::status`：它说的是"这个数从哪儿来的"，而"哪儿"是这个模块。
/// `render` 只把它印出来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// 这一趟真的从设备上取到的。
    JustRead,
    /// 设备这一趟失联，数字来自状态文件——**上次已知值**，不是现状。
    LastKnown,
}

/// 状态文件的内存形态：每个 Device 一条上次已知的读数，外加上次选出来的那台 Primary Device。
///
/// 两样东西住在一处，因为它们是同一类：**都是从读数推出来的运行时缓存，不是用户做的选择**
/// （`docs/adr/0003` 划的就是这条界）。
///
/// Device 那一头键用 `id` 而不是 name：`id` 是配置里用来互相引用的那个稳定标识。它仍然是
/// 用户可改的自由文本，改了 id 会让旧记录变成孤儿（丢一次历史值，可以接受），但**不会让 A 的
/// 历史值配给 B**——那才是不可接受的那一种错。孤儿记录不清理，理由见 [`Self::record`]。
#[derive(Debug, Clone, Default)]
pub struct LastKnown {
    last_primary: Option<String>,
    devices: BTreeMap<String, Record>,
}

impl LastKnown {
    /// 从磁盘读一份状态文件。**读不到、或者读到的东西坏了，都当成没有已知值。**
    ///
    /// 不返回 `Result`，与 `Config::load` 正相反：一份读不动的配置意味着用户写错了东西、
    /// 该当场告诉他；而这个文件是程序自己写的缓存，读不动只可能是第一次运行、有人删了它、
    /// 或者上一次写到一半断了电。三种情形里没有一种是用户该处置的，也没有一种该让
    /// `status` 印不出那几行——票面第 5 条要的就是这句话。
    pub fn load(path: &Path) -> Self {
        Self::parse(&std::fs::read_to_string(path).unwrap_or_default())
    }

    /// 解析一段状态文件文本。**解析不动就当成没有已知值。**
    ///
    /// 不返回 `Result`：调用方对"文件坏了"唯一能做的事就是当它不存在，而这个文件在这一趟
    /// 的末尾会被整份重写——损坏因此自愈，没有一个需要用户处置的错误状态。空文件、TOML
    /// 语法坏了、字段类型不对都落在这里。**字段缺失不算损坏**：那正是
    /// `taken_at: None` 落盘的样子。
    pub fn parse(text: &str) -> Self {
        let file = toml::from_str::<StateFile>(text).unwrap_or_default();
        Self {
            last_primary: file.last_primary,
            devices: file.last_known,
        }
    }

    /// 这个 Device 上次已知的读数，**超过 `very_stale_after` 的一律不交出去**。
    ///
    /// 丢弃在这一步而不是交给 `crate::staleness`（parking lot Q29 给本票的原话）：那一侧
    /// 对两条 HID 根本没有第二档，一份存了三天的有线读数在那里只会得到一句"已陈旧"，
    /// 而这张票要的是它**压根不出现**。边界与陈旧判定同一侧：整整一天前的还在，多一秒
    /// 就没有（"超过"才算，配置注释的原话）。
    ///
    /// 收 `&General` 而不是一个裸 `u64`，是因为那个结构里有两个长得一样的阈值
    /// （`stale_after` / `very_stale_after`），而把 3600 秒当成丢弃期限会让每一份
    /// 隔夜的历史值悄悄消失。类型不让调用方选错。
    ///
    /// **取得时刻说不出来的记录一样会过期**，按它在文件里躺了多久算（[`Record::stored_at`]）。
    /// `taken_at` 缺席是那台 BLE 设备根本没有更新时间戳，那种记录算不出"这个数是多久前的"
    /// ——只按取得时刻判，它**永远**不过期，一个六个月前的 50% 可以一直挂在那儿，而这张票的
    /// 标题句是"这个历史值必须会过期"。两个时刻里**取得时刻优先**：它必然不晚于写盘时刻，
    /// 所以拿它判是两者中更严的那一侧（parking lot Q38）。
    ///
    /// 丢弃与呈现因此各用各的时刻，这不是重复：丢弃问的是"这条记录该不该还在"，而那一行
    /// 印给用户的是"这个数是什么时候的"——后者对这种记录恰恰**答不出来**，`render` 照旧
    /// 印"无时间戳"，写盘时刻一个字都不露出去。
    ///
    /// 票 07（厂商上位机暂停）读的就是这个方法：暂停期间保留最后读数而不是清空，而
    /// `status` 刚启动时内存里没有"最后读数"，它只能来自这里。
    pub fn reading_for(
        &self,
        device_id: &str,
        general: &General,
        now: Timestamp,
    ) -> Option<EndpointReading> {
        let record = self.devices.get(device_id)?;
        let reading = record.to_reading(now)?;
        let dated_at = reading
            .taken_at
            .unwrap_or_else(|| Timestamp::from_unix_secs(record.stored_at));
        (now.secs_since(dated_at) <= general.very_stale_after).then_some(reading)
    }

    /// 记下这个 Device 这一次读到的读数，盖掉上一条。
    ///
    /// **只喂这一趟真读到的读数**，不要把 [`Self::reading_for`] 交出来的历史值再喂回来：
    /// 那会刷新写盘时刻，于是一条说不出取得时刻的记录被无限续命——正是上面那条规则要拦的
    /// 东西。`cli::status::read_or_last_known` 因此只在取数成功那一支里调它。
    ///
    /// `now` 就是写盘时刻。收一个值而不是去问时钟，与这条链路上其余每一处同一条规矩
    /// （parking lot Q27）：一趟只有一个"当下"。
    pub fn record(&mut self, device_id: &str, reading: &EndpointReading, now: Timestamp) {
        self.devices
            .insert(device_id.to_string(), Record::from_reading(reading, now));
    }

    /// 上一趟按规则选出来的那台 Primary Device 的 `id`。
    ///
    /// **这一格不在票 08 的票面上**，是编排者按票 09 的 Q41 派进来的：那张票的第 4 条验收框
    /// 要"全都不可信时保持上次的选择，不来回跳"，而 `status` 是一次性命令——枚举、取数、
    /// 印几行、退出，它手上没有"上次"。
    ///
    /// 为什么家在这里而不是 `config.toml`：`primary` 那一项只有**一个格子**，`"lowest"`
    /// （一条规则）和一个 Device id（一个具体选择）共用它。把自动选出来的 id 回写过去，
    /// `primary = "lowest"` 就变成 `primary = "dragonfly3"`——那条规则不见了，工具从此永久
    /// 钉在那一台上，而用户从没要求过。ADR-0003 里"程序自己管的字段（`primary`…）"说的是
    /// 托盘菜单里的**手动**切换（回写那一头是 `config::pin_primary`），那是一个用户做出的
    /// 选择；自动选出来的这个 id 不是。
    ///
    /// **它不会过期**，与上次已知的读数不同：一份读数的价值随时间衰减，而"上次选的是谁"是一个
    /// 没有年龄的事实——它要答的是"别来回跳"，那个问题不因为过了一天就变。真正让它失效的是
    /// 那台设备从配置里消失，而那件事由选择规则拿候选名单一比就知道，不需要一个期限。
    ///
    /// 交 `&str` 而不是拷一份 `String`：调用方（`primary::select` 的 `previous` 参数）
    /// 收的就是 `Option<&str>`。
    pub fn last_primary(&self) -> Option<&str> {
        self.last_primary.as_deref()
    }

    /// 记下这一趟选出来的 Primary Device。
    ///
    /// **只有"设"没有"清"**，这是有意的：这一格存在的全部理由是"全都不可信时保持上次的
    /// 选择"，而一个能把它清空的方法恰好能让那句话落空——选不出来的那一趟正是最需要上次那个
    /// 值的时候。哪一趟该调它由选择规则那一侧定（选出了一台真的 Primary 才算），这里只管存。
    pub fn remember_primary(&mut self, device_id: &str) {
        self.last_primary = Some(device_id.to_string());
    }

    /// 整份写回磁盘。
    ///
    /// **整份重写而不是改一条**：这个文件里没有需要保住的东西——没有注释、没有用户手写的
    /// 值、没有格式（那是 `config.toml` 那一侧的事，`docs/adr/0003`），所以格式保留式的
    /// 原地编辑在这里只是多一份代价。整份重写还顺带让上一趟留下的损坏自愈。
    ///
    /// **写失败要交给调用方，不在这里咽掉。**读不到当成没有已知值是对的（那是缓存的常态），
    /// 而写不进去是环境出了事（目录没了、盘满了、权限没了），一声不吭地丢掉每一次读数
    /// 会让"重启不等于失忆"这句承诺永久失效而没有任何症状。怎么处置由 `cli::status`
    /// 决定——那几行已经印出去了，它选择在 stderr 上说一句而不是让整条命令失败。
    pub fn save(&self, path: &Path) -> Result<()> {
        // 序列化先做完再碰文件。反过来（先开文件再序列化）意味着序列化失败时文件已经被
        // 截断了——那一趟不但没记下新读数，还把每个 Device 的历史值一起抹掉，而症状是
        // "下次启动就是失忆"，一声不吭。
        let text = self.to_toml()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("建不了目录 {}", dir.display()))?;
        }
        std::fs::write(path, text).with_context(|| format!("写不进状态文件 {}", path.display()))
    }

    /// 整份状态文件的文本形态。
    ///
    /// 序列化一个只有整数、布尔和短字符串的表**大概**不会失败，而这个方法照样交
    /// `Result`：把它咽成一个空字符串，[`Self::save`] 就会拿那个空字符串去覆盖文件，
    /// 于是一次"不会发生"的失败变成一次静默的全量删除。`Result` 是这条路上唯一不会把
    /// 失败翻译成数据丢失的形状。
    pub fn to_toml(&self) -> Result<String> {
        let file = StateFile {
            last_primary: self.last_primary.clone(),
            last_known: self.devices.clone(),
        };
        toml::to_string(&file).context("状态文件序列化失败")
    }
}

/// 状态文件的磁盘形态。
///
/// 套一层具名结构而不是直接序列化那个 map，是为了让文件里有一个说明自己是什么的表头
/// （`[last_known.<id>]`）：这份文件将来会被人打开看一眼，哪怕他不该编辑它。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StateFile {
    /// 上一趟按规则选出来的那台 Primary Device 的 `id`，缺席 = 没有"上次"。
    ///
    /// **声明在那张表前面不是随意的**：TOML 里顶层的裸键必须排在任何表之前，而 serde 按
    /// 字段声明顺序序列化。挪到后面，写出来的文件就是一份 TOML 都解析不动的东西。
    ///
    /// 键名是 `last_primary` 而不是 `primary`：后者在 `config.toml` 里已经有主，而它那一格
    /// 装的是**规则或者一个钉死的 id**，与这里的"上次自动选出来的是谁"不是一回事。两份文件
    /// 里同名而不同义，正是票 09 的 Q41 点名要防的那种混淆。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_primary: Option<String>,
    /// 每个 Device 一条，键是 `id`。
    #[serde(default)]
    last_known: BTreeMap<String, Record>,
}

/// 一条记录：够复原成一行的最小事实，加上一个只给过期判定用的写盘时刻。
///
/// 与 [`EndpointReading`] 不是镜像：少了 `cache_age_secs`（理由在 [`Self::to_reading`]），
/// 多了 [`Self::stored_at`]（理由在它自己身上）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    /// 这份读数是从哪条 Endpoint 上取到的，写的是配置里的块名（`wireless_24g`）。
    ///
    /// 落盘的是 [`EndpointKind::config_key`]，读回来走
    /// [`EndpointKind::from_config_key`]——两面都住在那个枚举上，这里只搬字符串。
    endpoint: String,
    /// 设备固件自己上报的电量百分比。
    reported_level: u8,
    /// 缺席 = 这条协议说不出充电与否，不是"没在充电"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    charging: Option<bool>,
    /// 缺席 = 这条协议不给电压，不是"0 mV"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voltage_mv: Option<u16>,
    /// 取得时刻，Unix 纪元秒。
    ///
    /// **缺席 = 说不出这个数是什么时候的**（那台 BLE 设备没有更新时间戳），
    /// 不是"新鲜"。一个十位数的十进制整数就是 `Timestamp` 存在的形式
    /// （`clock.rs` 说的"它是读数要落到磁盘上的形式"）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    taken_at: Option<u64>,
    /// **这一条记录是什么时候写下来的**，Unix 纪元秒。程序自己的时刻，不是设备说的。
    ///
    /// 它存在只为一件事：让 `taken_at` 缺席的记录也会过期。那一种说不出"这个数是多久前
    /// 的"，只按取得时刻判就永远过不了期。这个数程序**一定**说得出来，所以它不是 `Option`。
    ///
    /// **不印给用户看。**它答的是"这条记录躺了多久"，而那一行要说的是"这个数是什么时候
    /// 的"——两个不同的问题，把写盘时刻印成后者就是替设备编一个它没给的时间戳，正是
    /// `taken_at` 那个 `Option` 一路守着不做的事。
    ///
    /// 缺席时取 0，也就是纪元：那种记录**立刻过期**。这个缺省不是一个看着合理的假数字
    /// （1970 不像任何一份读数的时刻），而是失败方向上保守的那一侧——一条说不清自己什么
    /// 时候写下的记录，宁可丢掉也不要拿出来当上次已知值。
    #[serde(default)]
    stored_at: u64,
}

impl Record {
    fn from_reading(reading: &EndpointReading, now: Timestamp) -> Self {
        Self {
            endpoint: reading.endpoint.config_key().to_string(),
            reported_level: reading.reading.reported_level,
            charging: reading.reading.charging,
            voltage_mv: reading.reading.voltage_mv,
            taken_at: reading.taken_at.map(Timestamp::as_unix_secs),
            stored_at: now.as_unix_secs(),
        }
    }

    /// 复原成一份读数。认不出 `endpoint` 就当没有这条记录。
    ///
    /// **`cache_age_secs` 是在这里重算出来的，不是存回来的。**那个字段答的是"这份缓存
    /// 多久之前更新过"，而它是一个**相对量**：写盘那一刻 Windows 报的"5 分钟之前"，
    /// 三小时之后就不成立了。照原样存回来，一份放了三天的缓存会一直自称五分钟前的
    /// ——那正是票 06 拒绝"每次推算取得时刻"时点名的错（parking lot Q26），方向相反而已。
    /// 存绝对的取得时刻、年龄由它和当下减出来，两侧就不会漂：那个百分比描述的时刻没变，
    /// 而它离现在确实又远了三小时。
    ///
    /// 只有 `Ble` 有这一项（`EndpointReading::cache_age_secs` 的原话：两条 HID 是当场
    /// 往返，根本没有缓存这回事），所以另两级恒为 `None`——重算不能顺手给它们编一个。
    fn to_reading(&self, now: Timestamp) -> Option<EndpointReading> {
        let endpoint = EndpointKind::from_config_key(&self.endpoint)?;
        let taken_at = self.taken_at.map(Timestamp::from_unix_secs);
        Some(EndpointReading {
            endpoint,
            reading: Reading {
                reported_level: self.reported_level,
                charging: self.charging,
                voltage_mv: self.voltage_mv,
            },
            cache_age_secs: match endpoint {
                EndpointKind::Wired | EndpointKind::Dongle24G => None,
                EndpointKind::Ble => taken_at.map(|taken_at| now.secs_since(taken_at)),
            },
            taken_at,
        })
    }
}
