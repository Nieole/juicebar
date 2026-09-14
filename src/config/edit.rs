//! 格式保留的原地编辑：`config-refresh` 的补全，与 `primary` 的回写。
//!
//! 两样都走 `toml_edit` 而不是 serde 的序列化——后者会把整份文件重写一遍、洗掉全部
//! 注释，而这份配置的价值有一半在注释里（见 `docs/adr/0003`）。两样也都是纯函数：
//! 文本进、文本出，落盘那一步在调用方。
//!
//! **改回写机制只改这一个文件**，`toml_edit` 那几个坑（从别的文档克隆过来的表带着
//! 别人的位置、`[general]` 缺席时建表要钉位置）因此集中在一处。

use anyhow::{Context, Result, anyhow};

use crate::config::draft::endpoint_block;
use crate::config::known_devices::{is_present, recognise, same_scanned_identity};
use crate::config::{Config, HidEndpoint};
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
            insert_endpoint_block(&mut doc, index, kind, identity)?;
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

/// 把一条 Endpoint 的块插进第 `index` 个 `[[device]]` 里。
///
/// 块是**先渲染成文本再解析回来**的：`toml_edit` 保留解析时看到的原始写法，于是
/// `vid = 0x391D` 落到文件里仍然是十六进制，而不是被规范化成 14621。手工构造
/// `Formatted<i64>` 再改 repr 能得到同样的结果，但那要跟 `toml_edit` 的内部表示打交道，
/// 而这里已经有一个"块该长什么样"的唯一写法（[`endpoint_block`]），复用它同时也保证了
/// 补出来的块和草稿里写的块逐字一致。
fn insert_endpoint_block(
    doc: &mut toml_edit::DocumentMut,
    index: usize,
    kind: EndpointKind,
    endpoint: &HidEndpoint,
) -> Result<()> {
    let rendered = endpoint_block(kind, endpoint);
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
    /// 回写之后的全文。`primary` 本来就是这个 id 时与入参**逐字节相同**。
    pub text: String,
    /// 这一次真的改了 `primary` 吗。`false` = 本来就钉在这一台上，一个字节都不必写。
    pub changed: bool,
}

/// 把 `primary` 钉到这个 Device 上，文件里别的一个字节都不动。
///
/// 这是 ADR-0003 那句"运行时选择回写 `config.toml`"的**能力**那一半。与 [`draft`] 和
/// [`refresh`] 一样是纯函数：文本进、文本出，落盘另放。**落盘那一步眼下没有主，因为触发点
/// 不存在**——触发它的是托盘菜单里的手动切换，而托盘菜单在 spec 的 Out of Scope 里
/// （parking lot Q51）。到那一步照 `cli::config_refresh` 那几行的样子写就行：读全文、
/// 调这个函数、[`Pinned::changed`] 为真才落盘。
///
/// **只回写用户主动选的那一个。**按 `"lowest"` 自动选出来的 id 绝不写到这里来：`primary`
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
pub fn pin_primary(text: &str, device_id: &str) -> Result<Pinned> {
    // 读用 serde、写用 toml_edit，同一段文本解析两遍，与 [`refresh`] 同一条理由：
    // "这个 id 在册吗、现在钉的是谁"是类型化的问题，"保住注释"是文档树的问题。
    let config = Config::parse(text)?;
    // "钉在这一台上"长什么样只说一遍：下面提前返回那一处和末尾那道自检问的是同一件事。
    let wanted = PrimaryRule::Pinned(device_id.to_string());

    // 不在册的 id **一个字节都不写**。写下去的话，下一次启动 `primary::select` 交回的是
    // `Selection::PinnedNotFound`：一行都没标，命令行上多一句"配置里 primary 钉的 id 不在
    // 登记的 Device 里"。让程序自己写出那种配置，等于替用户造一个他没犯的错——而菜单只
    // 可能把在册的设备列出来，所以走到这里的一个不在册的 id 是**调用方的 bug**。
    if !config.devices.iter().any(|device| device.id == device_id) {
        return Err(anyhow!(
            "primary 钉不到 \"{device_id}\" 上：配置里没有这个 id 的 Device"
        ));
    }

    // 本来就钉在这一台上就什么都不做，理由见 [`Pinned`]。
    if config.general.primary == wanted {
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
    let mut chosen = toml_edit::Value::from(device_id);
    if let Some(previous) = slot.as_value() {
        *chosen.decor_mut() = previous.decor().clone();
    }
    *slot = toml_edit::Item::Value(chosen);
    let text = doc.to_string();

    // **自检**：这份写完的文本，拿启动时那条读法读回来必须真的是这一台。它守的是这次回写
    // 唯一的承诺——"重启之后生效的就是用户刚点的那一个"——而它用的正是重启时会走的那段
    // 代码，不是把那条规则在这里抄第二遍。
    //
    // 眼下唯一会落进这里的是一台 id 恰好叫 `lowest` 的 Device：那个词先被当成"电量最低的
    // 那个"这条规则（`primary::PrimaryRule` 的 `Deserialize`），于是写下去的东西读回来是
    // 一条规则、不是这一台。照样写就是最难发现的那一种错——那条规则多数时候恰好也选中它。
    // `config.example.toml` 早就写着这件事："id 恰好叫 lowest 的 Device 钉不住，换个 id"。
    let written =
        Config::parse(&text).context("回写之后的配置自己解析不动了，这是程序的错，不是配置的错")?;
    if written.general.primary != wanted {
        return Err(anyhow!(
            "primary 钉不到 \"{device_id}\" 上：写进去再读回来不是这一台（lowest 是规则的写法）"
        ));
    }

    Ok(Pinned {
        text,
        changed: true,
    })
}
