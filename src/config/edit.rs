//! 格式保留的原地编辑：`config-refresh` 的补全，`primary` 的回写，与 `[tray]` 的回写。
//!
//! 几样都走 `toml_edit` 而不是 serde 的序列化——后者会把整份文件重写一遍、洗掉全部
//! 注释，而这份配置的价值有一半在注释里（见 `docs/adr/0003`）。几样也都是纯函数：
//! 文本进、文本出，落盘那一步在调用方。
//!
//! **改回写机制只改这一个文件**，`toml_edit` 那几个坑（从别的文档克隆过来的表带着
//! 别人的位置、`[general]` 缺席时建表要钉位置）因此集中在一处。

use anyhow::{Context, Result, anyhow};

use crate::config::draft::bluetooth_block;
use crate::config::draft::endpoint_block;
use crate::config::known_devices::{is_present, recognise, same_scanned_identity};
use crate::config::tray::Literal;
use crate::config::{Config, HidEndpoint, TraySetting};
use crate::endpoints::EndpointKind;
use crate::hid::HidInfo;
use crate::primary::PrimaryRule;

// ---------------------------------------------------------------
// config-refresh：把当时在场、而配置里空着的 Endpoint 块补上
// ---------------------------------------------------------------

/// 一次 [`refresh`] 的结果。
///
/// 文本、做过的事、没做的事分成三样交出去：命令行把后两样印出来，托盘拿做过的事弹通知、记日志。ADR-0003
/// 划死的边界是"只补空缺，扫描结果与用户所写不一致时只提醒"——那句提醒就住在 [`Self::notes`] 里，
/// 它是这条边界唯一的出口。
pub struct Refreshed {
    /// 补全之后的全文。什么都没补时与入参**逐字节相同**。
    pub text: String,
    /// 这一次补上了哪些块。
    pub filled: Vec<Fill>,
    /// 没补的地方和为什么，一条一句人话。它不改文件，只解释。
    pub notes: Vec<String>,
}

/// [`refresh`] 补上的一块：哪台 Device 的哪条 Endpoint，补的是哪组身份。
///
/// 交的是结构而不只是一句话：托盘的通知说"补了哪台的哪条"，用的是 Device 的名字与 Endpoint 的种类；日志与命令行
/// 要的是那一整句（`Display`），身份的四个字段、没实测过的那一项都在里面。
#[derive(Debug, Clone)]
pub struct Fill {
    /// 补在哪一台 Device 上：它的 id。
    pub device_id: String,
    /// 那台 Device 的名字。
    pub device_name: String,
    /// 补上的是哪一条 Endpoint。
    pub endpoint: EndpointKind,
    /// 补上的那组身份。
    identity: HidEndpoint,
    /// 这组身份里没实测过的部分，没有就是空串（身份表逐台写着）。
    caveat: &'static str,
}

impl std::fmt::Display for Fill {
    /// "dragonfly3：补上了 Wired（VID 391D PID 1005 UP FF02 U 0002）。"，身份里有没实测过的部分就接在句号后面。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}：补上了 {}（{}）。{}",
            self.device_id,
            self.endpoint,
            describe(&self.identity),
            // 注意事项在草稿里是分行写的（一行放不下），这里要压成一句。
            self.caveat.replace('\n', "")
        )
    }
}

/// 把当时在场、而配置里空着的 Endpoint 块补进这份 TOML，**只补空缺**。
///
/// 走 `toml_edit` 而不是 serde 的序列化：后者会把整份文件重写一遍，注释全没了，而这份配置
/// 的价值有一半在注释里（见 `docs/adr/0003`）。原地插入的代价是插进去的块落在该 Device
/// 已有的几块之后，而不是紧挨着草稿留下的那段注释掉的占位——占位于是变成一段历史记录，
/// 草稿里那句"留着或删掉都行"说的就是这件事。
///
/// 与 [`draft`] 一样是纯函数：入参是文本和一次枚举的结果，出参是文本。真正落盘的那一步
/// 在 `cli::config_refresh` 里。
///
/// [`draft`]: fn@crate::config::draft
pub fn refresh(text: &str, collections: &[HidInfo]) -> Result<Refreshed> {
    // 读用 serde（要的是"哪一块空着"这个类型化的问题），写用 toml_edit（要的是保住注释）。
    // 同一段文本解析两遍，是这两种需求各自的代价，不是重复劳动。
    let config = Config::parse(text)?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("配置解析失败（toml_edit）")?;
    let mut filled = Vec::new();
    let mut notes = Vec::new();

    for (index, device) in config.devices.iter().enumerate() {
        let recognised = recognise(device);
        let missing: Vec<EndpointKind> = EndpointKind::PRIORITY
            .into_iter()
            .filter(|kind| !kind.is_configured_in(device))
            .collect();

        // ADR-0003 那半句："扫描结果与用户所写不一致时只在命令行提醒"。
        //
        // 不一致指的是**本机此刻在场的身份**和用户写下的那一条对不上——不是"用户写的那条
        // 现在不在场"：鼠标没插线时 Wired 当然不在场，那是常态，每次都印一句就成了噪音，
        // 而噪音会把真正该看的这一句一起淹掉。`report_id` 不参与比较，因为枚举问不出它，
        // 拿它说"扫描结果不一致"是没有依据的。
        if let Some(known) = recognised {
            for kind in EndpointKind::PRIORITY {
                let Some(configured) = kind.hid_config_in(device) else {
                    continue;
                };
                let Some(identity) = known.identity(kind) else {
                    continue;
                };
                if !same_scanned_identity(configured, identity) && is_present(identity, collections)
                {
                    notes.push(format!(
                        "{}：{kind} 你写的是 {}，而本机此刻在场的是 {}（我记下的这台设备的 \
                         {kind} 身份）。没有动它——你写过的值一概不改。",
                        device.id,
                        describe(configured),
                        describe(identity)
                    ));
                }
            }
        }

        if missing.is_empty() {
            continue;
        }
        let Some(known) = recognised else {
            notes.push(format!(
                "{}：认不出这是哪台设备（它已配好的 Endpoint 身份不在程序认得的那张表里），\
                 所以猜不出它缺的 {} 该填什么，得手填。",
                device.id,
                missing
                    .iter()
                    .map(EndpointKind::to_string)
                    .collect::<Vec<_>>()
                    .join("、")
            ));
            continue;
        };

        for kind in missing {
            // Ble 的地址不在身份表里、也猜不出来，但**空着不能不声不响**——那正是这张票
            // 一直在守的"用户不该在不知情的情况下缺一整条 Endpoint"。
            let Some(identity) = known.identity(kind) else {
                notes.push(format!(
                    "{}：{kind} 空着，而它的地址程序猜不出来（BLE 射频是另一颗芯片，\
                     与 dongle 之间没有能缝合的字段）。{}",
                    device.id,
                    known.absent_hint(kind)
                ));
                continue;
            };
            if !is_present(identity, collections) {
                notes.push(format!(
                    "{}：{kind} 现在不在场（本机枚举不到 {}），没有填。{}",
                    device.id,
                    describe(identity),
                    known.absent_hint(kind)
                ));
                continue;
            }
            insert_block(&mut doc, index, kind, &endpoint_block(kind, identity))?;
            // 身份里有没实测过的部分就一并说出来。`is_present` 核对的只有 VID/PID/usage
            // 四项，`report_id` 是枚举问不出来的——那一项若是抄来的猜测，用户有权知道
            // 自己刚拿到的是一个没人验过的值。
            filled.push(Fill {
                device_id: device.id.clone(),
                device_name: device.name.clone(),
                endpoint: kind,
                identity: identity.clone(),
                caveat: known.caveat(kind),
            });
        }
    }

    Ok(Refreshed {
        text: doc.to_string(),
        filled,
        notes,
    })
}

/// 一条 Endpoint 身份的人话写法，用在命令行的提醒里。
///
/// 四个字段都印出来：鼠标的有线本体是 `391D:1005`、dongle 是 `391D:1A05`，只差一个字符，
/// 只印一半的话用户根本看不出程序说的是哪一条。
fn describe(endpoint: &HidEndpoint) -> String {
    format!(
        "VID {:04X} PID {:04X} UP {:04X} U {:04X}",
        endpoint.vid, endpoint.pid, endpoint.usage_page, endpoint.usage
    )
}

/// 把一条 Endpoint 的块插进第 `index` 个 `[[device]]` 里：`rendered` 是那一块渲染好的文本。
///
/// 块是**先渲染成文本再解析回来**的：`toml_edit` 保留解析时看到的原始写法，于是
/// `vid = 0x391D` 落到文件里仍然是十六进制，而不是被规范化成 14621。手工构造
/// `Formatted<i64>` 再改 repr 能得到同样的结果，但那要跟 `toml_edit` 的内部表示打交道，
/// 而这里已经有一个"块该长什么样"的唯一写法（[`endpoint_block`]、蓝牙那一块是 [`bluetooth_block`]），
/// 复用它同时也保证了补出来的块和草稿里写的块逐字一致。
fn insert_block(
    doc: &mut toml_edit::DocumentMut,
    index: usize,
    kind: EndpointKind,
    rendered: &str,
) -> Result<()> {
    let snippet = rendered
        .parse::<toml_edit::DocumentMut>()
        .context("渲染出来的 Endpoint 块自己解析不动，这是程序的错，不是配置的错")?;
    let mut block = snippet["device"][kind.config_key()]
        .as_table()
        .ok_or_else(|| anyhow!("渲染出来的 Endpoint 块不是一张表"))?
        .clone();
    // 前面空一行、缩进两格，和这份配置里其余的 Endpoint 块对齐。
    block.decor_mut().set_prefix("\n  ");
    // **这一行不能省。**`toml_edit` 输出时把整份文档的表拉平、按各表解析时记下的
    // "文档内位置"排序；位置为 `None` 的表继承前一张表的位置，于是自然落在它父表的后面。
    // 而上面那张表是从**另一份文档**（那个片段）里克隆出来的，身上带着片段里的位置 1
    // ——搬进主文档就会排到第一个 `[[device]]` 的位置上去。
    //
    // 后果不是排版难看：TOML 里 `[device.wired]` 属于它前面最近的那个 `[[device]]`，
    // 于是键盘的有线身份会挂到鼠标头上，文件照样解析得动，错误直到取数时才以一句
    // "读不到"的面目出现。`tests/config.rs` 里那条
    // `config_refresh_puts_the_block_under_the_right_device` 就是抓这个的。
    block.set_position(None);

    doc["device"]
        .as_array_of_tables_mut()
        .and_then(|tables| tables.get_mut(index))
        .ok_or_else(|| anyhow!("配置里找不到第 {index} 个 [[device]]"))?
        .insert(kind.config_key(), toml_edit::Item::Table(block));
    Ok(())
}

// ---------------------------------------------------------------
// primary 回写：把用户当场做出的那个选择落进 config.toml
// ---------------------------------------------------------------

/// 一次 [`pin_primary`] 的结果。
///
/// 文本和"这一次到底改没改"分成两样交出去，与 [`Refreshed`] 同一个理由：**落盘那一步不该
/// 自己再判一遍**。`config-refresh` 靠 `filled` 是不是空的来决定写不写文件——没改动却重写
/// 一遍会白白改掉文件的修改时间，也让人以为程序动过它；而回写只动一格，没有一个"补了几处"
/// 的清单可以借，所以那件事在这里是一个具名字段。
#[derive(Debug)]
pub struct Pinned {
    /// 回写之后的全文。`primary` 本来就是这一条时与入参**逐字节相同**。
    pub text: String,
    /// 这一次真的改了 `primary` 吗。`false` = 本来就是这一条，一个字节都不必写。
    pub changed: bool,
}

/// 把 `primary` 写成用户在菜单里选的那一条——钉死某一台（`"<id>"`），或者切回自动（`"lowest"`）——文件里别的
/// 一个字节都不动。
///
/// 这是 ADR-0003 那句"运行时选择回写 `config.toml`"的**能力**那一半。与 [`draft`] 和
/// [`refresh`] 一样是纯函数：文本进、文本出，落盘另放。触发它的是托盘菜单里的"托盘上画哪一台"
/// （`crate::tray::config::Action::WritePrimary`），落盘在外壳（`shell/config.rs`）：读全文、调这个函数、
/// [`Pinned::changed`] 为真才写。
///
/// **只回写用户主动选的那一条。**按 `"lowest"` 自动选出来的 id 绝不写到这里来：`primary`
/// 只有一个格子，那条规则和一个具体 id 共用它，把自动选出的 id 写回去，`primary = "lowest"`
/// 就变成 `primary = "dragonfly3"`——规则不见了，工具从此永久钉在那一台上，而用户从没要求过
/// （票 09 的 parking lot Q41）。自动选出来的那个 id 有自己的家，见
/// [`crate::state::LastKnown::remember_primary`]。
///
/// **这一格是程序唯一允许覆盖的、用户写过的值。**ADR-0003 把它点了名："只补空缺和程序
/// 自己管的字段（`primary`、`config-refresh` 填充的 Endpoint 块）"。用户在菜单里点一下
/// 就是要改这一格，此时"不动用户写过的值"反而意味着那一下点了没用。别的每一处仍然一概不动，
/// `tests/config.rs::primary_writeback_touches_nothing_else_the_user_wrote` 逐行守着这句话。
///
/// [`draft`]: fn@crate::config::draft
pub fn pin_primary(text: &str, wanted: &PrimaryRule) -> Result<Pinned> {
    // 读用 serde、写用 toml_edit，同一段文本解析两遍，与 [`refresh`] 同一条理由：
    // "这个 id 在册吗、现在钉的是谁"是类型化的问题，"保住注释"是文档树的问题。
    let config = Config::parse(text)?;

    // 不在册的 id **一个字节都不写**。写下去的话，下一次启动 `primary::select` 交回的是
    // `Selection::PinnedNotFound`：一行都没标，命令行上多一句"配置里 primary 钉的 id 不在
    // 登记的 Device 里"。让程序自己写出那种配置，等于替用户造一个他没犯的错——而菜单只
    // 可能把在册的设备列出来，所以走到这里的一个不在册的 id 是**调用方的 bug**。
    if let PrimaryRule::Pinned(device_id) = wanted
        && !config.devices.iter().any(|device| &device.id == device_id)
    {
        return Err(anyhow!(
            "primary 钉不到 \"{device_id}\" 上：配置里没有这个 id 的 Device"
        ));
    }

    // 本来就是这一条就什么都不做，理由见 [`Pinned`]。`[general]` 整节没写、缺省就是自动时也算：
    // 切回自动不该凭空多出一节来。
    if config.general.primary == *wanted {
        return Ok(Pinned {
            text: text.to_string(),
            changed: false,
        });
    }

    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("配置解析失败（toml_edit）")?;

    // `[general]` 整节缺席时（缺席时每一项都取缺省值，那是一份跑得动的配置）先把它建出来，
    // 而且**建成一张真正的表**：不建的话 `doc["general"]["primary"]` 会自动生出一个
    // `general = { primary = "…" }` 的行内表。那个东西解析得动，但它是个陷阱——用户下次
    // 想加一项 `low_battery`，顺手写一个 `[general]` 段就撞上 TOML 的"键重复"，整份配置
    // 从此读不动，而他会以为是自己写错了。
    //
    // 位置钉在 0，让它排在所有 `[[device]]` 前面，和草稿写出来的次序一致：`toml_edit` 的
    // 解析器给表编号是**从 1 开始**的（`current_position += 1` 在赋值之前），所以 0 严格
    // 小于文件里任何一张表。留 `None` 反而危险——那种表继承前一张表的位置（Q46 记的
    // 就是这个坑），会落到某个 `[[device]]` 中间去。
    if !doc.contains_key("general") {
        let mut general = toml_edit::Table::new();
        general.set_position(Some(0));
        doc.insert("general", toml_edit::Item::Table(general));
    }

    // 只换那个字符串**里面的内容**，把它的 decor（前面那个空格、后面那句行尾注释）原样
    // 搬过来。`doc[..] = toml_edit::value(..)` 那种写法换掉的是整个值，而行尾那句注释正住
    // 在旧值的 decor 里；键上方那句住在**键**的 decor 里、不受影响——于是两句注释里只有
    // 一句会消失，而那种半条命的错最难在肉眼下发现。
    let slot = &mut doc["general"]["primary"];
    let mut chosen = toml_edit::Value::from(wanted.config_value());
    if let Some(previous) = slot.as_value() {
        *chosen.decor_mut() = previous.decor().clone();
    }
    *slot = toml_edit::Item::Value(chosen);
    let text = doc.to_string();

    // **自检**：这份写完的文本，拿启动时那条读法读回来必须真的是这一条。它守的是这次回写
    // 唯一的承诺——"重启之后生效的就是用户刚点的那一个"——而它用的正是重启时会走的那段
    // 代码，不是把那条规则在这里抄第二遍。
    //
    // 眼下唯一会落进这里的是一台 id 恰好叫 `lowest` 的 Device：那个词先被当成"电量最低的
    // 那个"这条规则（`primary::PrimaryRule` 的 `Deserialize`），于是写下去的东西读回来是
    // 一条规则、不是这一台。照样写就是最难发现的那一种错——那条规则多数时候恰好也选中它。
    // `config.example.toml` 早就写着这件事："id 恰好叫 lowest 的 Device 钉不住，换个 id"。
    let written =
        Config::parse(&text).context("回写之后的配置自己解析不动了，这是程序的错，不是配置的错")?;
    if written.general.primary != *wanted {
        return Err(anyhow!(
            "primary 写不成 \"{}\"：写进去再读回来不是这一个选择（lowest 是规则的写法）",
            wanted.config_value()
        ));
    }

    Ok(Pinned {
        text,
        changed: true,
    })
}

// ---------------------------------------------------------------
// [tray] 回写：把菜单里点的那一项落进 config.toml
// ---------------------------------------------------------------

/// 一次 [`write_tray`] 的结果，与 [`Pinned`] 同一个形状、同一个理由：落盘那一步不该自己再判一遍改没改。
#[derive(Debug)]
pub struct TrayWritten {
    /// 回写之后的全文。每一项本来就这样写着时与入参**逐字节相同**。
    pub text: String,
    /// 这一次真的改了什么吗。`false` = 一个字节都不必写。
    pub changed: bool,
}

/// 把 `[tray]` 里这几个键写成菜单里点的取值，文件里别的一个字节都不动。菜单里点一项是一个键；"恢复默认"是图标样式
/// 那六个（[`TraySetting::icon_defaults`]）。
///
/// ADR-0005 让这八项"在菜单里改，点一下立刻生效并按 ADR-0003 格式保留地写回"，所以它们与 `primary` 一样是程序
/// 允许覆盖的、用户写过的值。与 [`pin_primary`] 一样是纯函数，落盘在外壳（`shell/config.rs`）。
///
/// 每个键怎么写：
/// - **写着的正是这个取值**：不动。
/// - **写着别的**（连认不出的笔误也算）：只换那个值，前后的空白与行尾注释原样搬过来，理由同 [`pin_primary`]。
/// - **没写**：要的是缺省值就不动——不写生效的本来就是它，凭空多出一行、一张表只会让人以为程序动过它；要的不是
///   缺省值，就补在 `[tray]` 最后；整张 `[tray]` 都没有时建一张真表（不是行内表，理由同 [`pin_primary`] 建
///   `[general]`），排在 `[general]` 后面、所有 `[[device]]` 前面。
///
/// `tray` 写成了一张表以外的东西时写不进去，交回错误，一个字节都不写。
pub fn write_tray(text: &str, settings: &[TraySetting]) -> Result<TrayWritten> {
    // 先照启动时那条读法读一遍：配置此刻写坏了（别处的错），就别往里写，免得回写之后那条自检把用户的错说成程序的错。
    Config::parse(text)?;
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("配置解析失败（toml_edit）")?;
    let mut changed = false;
    for &setting in settings {
        changed |= write_tray_key(&mut doc, setting)?;
    }
    if !changed {
        return Ok(TrayWritten {
            text: text.to_string(),
            changed: false,
        });
    }
    let text = doc.to_string();

    // 自检，理由同 [`pin_primary`]：写完的文本拿启动时那条读法读回来，每一项都得真的是菜单里点的那一个。
    let written =
        Config::parse(&text).context("回写之后的配置自己解析不动了，这是程序的错，不是配置的错")?;
    if let Some(missed) = settings
        .iter()
        .find(|setting| !setting.is_in(&written.tray))
    {
        return Err(anyhow!(
            "[tray] 里的 {} 写不成：写进去再读回来不是菜单里点的那一个",
            missed.key()
        ));
    }
    Ok(TrayWritten {
        text,
        changed: true,
    })
}

/// 把一个键写进文档树，交回改没改。
fn write_tray_key(doc: &mut toml_edit::DocumentMut, setting: TraySetting) -> Result<bool> {
    let key = setting.key();
    let Some(tray) = doc.get_mut("tray") else {
        if setting.is_default() {
            return Ok(false);
        }
        // 与 `[general]` 同一个位置编号：输出时按编号稳定排序，而这张新表挂在文档最后，同号时就紧跟在 `[general]`
        // 后面、排在编号更大的 `[[device]]` 前面。没有 `[general]` 就取 0，排在所有表前面（理由见 [`pin_primary`]）。
        let position = doc
            .get("general")
            .and_then(toml_edit::Item::as_table)
            .and_then(toml_edit::Table::position)
            .unwrap_or(0);
        let mut table = toml_edit::Table::new();
        table.set_position(Some(position));
        table.insert(key, toml_edit::Item::Value(literal_value(setting)));
        doc.insert("tray", toml_edit::Item::Table(table));
        return Ok(true);
    };
    let table = tray
        .as_table_like_mut()
        .ok_or_else(|| anyhow!("[tray] 不是一张表，{key} 写不进去"))?;
    let Some(slot) = table.get_mut(key) else {
        if setting.is_default() {
            return Ok(false);
        }
        table.insert(key, toml_edit::Item::Value(literal_value(setting)));
        return Ok(true);
    };
    let already = match setting.literal() {
        Literal::Word(word) => slot.as_str() == Some(word),
        Literal::Switch(on) => slot.as_bool() == Some(on),
    };
    if already {
        return Ok(false);
    }
    let mut value = literal_value(setting);
    if let Some(previous) = slot.as_value() {
        *value.decor_mut() = previous.decor().clone();
    }
    *slot = toml_edit::Item::Value(value);
    Ok(true)
}

/// 这一项的取值做成一个 TOML 值。
fn literal_value(setting: TraySetting) -> toml_edit::Value {
    match setting.literal() {
        Literal::Word(word) => toml_edit::Value::from(word),
        Literal::Switch(on) => toml_edit::Value::from(on),
    }
}

// ---------------------------------------------------------------
// 蓝牙登记与解除：菜单"登记设备"里点的那一下
// ---------------------------------------------------------------

/// 把本机扫到的一个蓝牙地址登记到这台 Device 上：往它的 `[[device]]` 里补一块 `[device.bluetooth]`，文件里别的一个
/// 字节都不动。交回写好的全文；这台本来就登记着这个地址时是 `None`，一个字节都不必写（理由同 [`Pinned`]）。
///
/// 触发它的是托盘菜单"登记设备"里点的"登记到"（`crate::tray::config::Action::RegisterBle`），落盘在外壳
/// （`shell/config.rs`）：读此刻文件的全文、调这个函数、有新文本才写。与 [`pin_primary`] 一样是纯函数。
///
/// **只补空缺，一点不覆盖**（ADR-0003）：这台已经登记着别的地址，就拒绝、一个字节不写——换地址是先解除
/// （[`unregister_ble`]）、再登记，每一步都看得见。菜单的"登记到"本来只列还没有蓝牙地址的 Device，所以走到这里的
/// 多半是托盘还没重读到用户刚改过的配置。同一个地址已经登记在另一台上也拒绝：两台说的是同一台设备的电量。
pub fn register_ble(text: &str, device_id: &str, address: &str) -> Result<Option<String>> {
    // 读用 serde、写用 toml_edit，理由同 [`refresh`]。
    let config = Config::parse(text)?;
    let index = device_index(&config, device_id)?;
    if !address.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err(anyhow!(
            "蓝牙地址 \"{address}\" 里没有地址：登记不进 {device_id}"
        ));
    }
    if let Some(bluetooth) = &config.devices[index].bluetooth {
        if bluetooth.matches(address) {
            return Ok(None);
        }
        return Err(anyhow!(
            "{device_id} 已经登记着蓝牙地址 {}，不覆盖它：要换，先解除 {device_id} 的蓝牙登记，再登记 {address}",
            bluetooth.address
        ));
    }
    if let Some(other) = config.device_with_bluetooth_address(address) {
        return Err(anyhow!(
            "蓝牙地址 {address} 已经登记在 {} 上，不再登记到 {device_id}",
            other.id
        ));
    }

    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("配置解析失败（toml_edit）")?;
    insert_block(
        &mut doc,
        index,
        EndpointKind::Ble,
        &bluetooth_block(address),
    )?;
    let text = doc.to_string();

    // **自检**，与 [`pin_primary`] 同一个道理：拿启动时那条读法读回来，这一台登记的必须就是这个地址，一台不多一台不少。
    let written =
        Config::parse(&text).context("登记之后的配置自己解析不动了，这是程序的错，不是配置的错")?;
    let landed = written
        .devices
        .get(index)
        .filter(|device| device.id == device_id)
        .and_then(|device| device.bluetooth.as_ref())
        .is_some_and(|bluetooth| bluetooth.matches(address));
    if !landed || written.devices.len() != config.devices.len() {
        return Err(anyhow!(
            "蓝牙地址 {address} 登记不进 {device_id}：写进去再读回来不是这一台的地址"
        ));
    }
    Ok(Some(text))
}

/// 配置里 id 是 `device_id` 的那一台排第几。不在册就说出来：菜单只列在册的 Device，走到这里多半是托盘还没重读到
/// 用户刚改过的配置。
fn device_index(config: &Config, device_id: &str) -> Result<usize> {
    config
        .devices
        .iter()
        .position(|device| device.id == device_id)
        .ok_or_else(|| anyhow!("配置里没有 id 是 \"{device_id}\" 的 Device"))
}

/// 解除这台 Device 的蓝牙登记：删掉它的 `[device.bluetooth]` 那一块（连同块名正上方的那一行空行，见 [`block_lines`]），
/// 文件里别的一个字节都不动。交回写好的全文；这台
/// 本来就没有蓝牙地址时是 `None`（托盘还没重读到上一次解除时又点了一下），一个字节都不必写。
///
/// 触发它的是托盘菜单"登记设备 › 解除蓝牙登记"里点的那一台（`crate::tray::config::Action::UnregisterBle`），落盘在外壳，
/// 与 [`register_ble`] 同一条路。
///
/// **删的是行，而且只删块名与块里那几行键**：写在块上面的注释一句不丢（ADR-0003）。`toml_edit` 的删表做不到这一点
/// ——块名上面的注释住在那张表的 decor 里，表删了它就跟着没了，而用户照草稿的说明自己去掉 `#`、填上地址时，那段说明
/// 正住在那里。所以拿 `toml_edit` 解析时记下的位置（span）找出那几行，在原文上删掉（[`block_lines`]）。
pub fn unregister_ble(text: &str, device_id: &str) -> Result<Option<String>> {
    let config = Config::parse(text)?;
    let index = device_index(&config, device_id)?;
    if config.devices[index].bluetooth.is_none() {
        return Ok(None);
    }
    let parsed = toml_edit::Document::parse(text).context("配置解析失败（toml_edit）")?;
    let removed = block_lines(&parsed, text, index)?;
    let kept: String = text
        .split_inclusive('\n')
        .enumerate()
        .filter(|(line, _)| !removed.contains(line))
        .map(|(_, content)| content)
        .collect();

    // **自检**，与 [`register_ble`] 同一个道理：读回来这一台没有蓝牙地址了，一台不多一台不少。
    let written =
        Config::parse(&kept).context("解除之后的配置自己解析不动了，这是程序的错，不是配置的错")?;
    let gone = written
        .devices
        .get(index)
        .filter(|device| device.id == device_id)
        .is_some_and(|device| device.bluetooth.is_none());
    if !gone || written.devices.len() != config.devices.len() {
        return Err(anyhow!(
            "解除不了 {device_id} 的蓝牙登记：删掉那一块再读回来，不是只少了这一台的地址"
        ));
    }
    Ok(Some(kept))
}

/// 第 `index` 个 `[[device]]` 的 `[device.bluetooth]` 那一块占着原文的哪几行（从 0 数）：块名那一行，块里每个键从键到
/// 值的那几行；块名正上方是一行空行时连它一起——那是 [`register_ble`] 登记时补在块前面的，登记之后再解除，文件逐字节
/// 回到登记之前。用户手写的块，前面那一行空行也跟着走：分不出那一行是谁写的，而删的是空行、不是注释（ADR-0003 守的
/// 是注释一句不丢）。
///
/// 只认写成一张表、下面不再挂子表的那种（草稿与 [`register_ble`] 写的都是）。写成行内表或者点号键，就说认不出、一个
/// 字节都不删：照一个没想到的形状删行，删坏了用户的配置比不删糟得多。
fn block_lines(parsed: &toml_edit::Document<&str>, text: &str, index: usize) -> Result<Vec<usize>> {
    let unrecognised = || {
        anyhow!(
            "第 {} 个 [[device]] 的蓝牙那一块不是写成 [device.bluetooth] 一张表的，程序不去拆它：请在配置文件里手动删掉那一块",
            index + 1
        )
    };
    let table = parsed
        .get("device")
        .and_then(toml_edit::Item::as_array_of_tables)
        .and_then(|devices| devices.get(index))
        .and_then(|device| device.get(EndpointKind::Ble.config_key()))
        .and_then(toml_edit::Item::as_table)
        .filter(|table| !table.is_dotted() && table.iter().all(|(_, item)| item.is_value()))
        .ok_or_else(unrecognised)?;
    let line_of = |offset: usize| text[..offset].matches('\n').count();

    // 块名那一行认的是 `]` 所在的那一行：span 的尾巴一定落在块名上。
    let header = table.span().ok_or_else(unrecognised)?;
    let header_line = line_of(header.end.saturating_sub(1));
    let mut lines = vec![header_line];
    for (name, _) in table.iter() {
        let (key, item) = table.get_key_value(name).ok_or_else(unrecognised)?;
        let (Some(from), Some(to)) = (key.span(), item.span()) else {
            return Err(unrecognised());
        };
        lines.extend(line_of(from.start)..=line_of(to.end.saturating_sub(1)));
    }
    let blank_above = header_line.checked_sub(1).filter(|above| {
        text.split_inclusive('\n')
            .nth(*above)
            .is_some_and(|content| content.trim().is_empty())
    });
    lines.extend(blank_above);
    Ok(lines)
}
