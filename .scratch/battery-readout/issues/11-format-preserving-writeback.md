# 11: 格式保留回写与 primary 落盘

**What to build:** 程序改用户的配置文件时，不洗掉那些解释性注释（这份配置的价值有一半在注释里），
也绝不覆盖用户手改过的值——用户为绕开某个坑做的修改不该被默默改回去。同时，菜单里切换的
Primary Device 要落盘，重启后还在。

**Blocked by:** 09, 10

**Status:** resolved

- [x] 引入格式保留的 TOML 编辑能力，并在依赖清单里写明收它的理由（照仓库惯例）—— **票 10 做的**
- [x] 回写后配置里的注释完好无损 —— `config-refresh` 那一半是**票 10** 的；`primary` 回写这一半是本票
- [x] 只补空缺和程序自己管的字段（`primary`、`config-refresh` 填充的 Endpoint 块）——
      Endpoint 块那一半是**票 10** 的，`primary` 这一半是本票
- [x] 用户手写过的值一律不修改；扫描结果与其不一致时只在命令行提醒 —— **票 10 做的**
- [x] Primary Device 的运行时选择回写 `config.toml`，重启后仍生效 ——
      **只挣到了机制那一半，触发点不存在**（见下）
- [x] 测试：一份带注释、含用户手改值的配置，回写后注释与手改值都原样保留 ——
      `config-refresh` 那条是**票 10** 的，`primary` 回写这条是本票

## 哪几条是票 10 做的

**第 1 条整条、第 4 条整条，以及第 2、3、6 条各一半，都是票 10（配置自举与 config-refresh）
做的。**它的 parking lot **Q46** 原话：「`toml_edit` 在本票就收进来（票 11 的第一条框变成
'确认已引入'）」。它为什么不能等：票面设想的「追加式补块」是**错的**而不只是差——TOML 里
`[device.wired]` 属于它前面最近的那个 `[[device]]`，追加到文件末尾会把键盘的有线身份挂到鼠标
名下，而文件照样解析得动。

本票逐条核实过现状：

- 第 1 条：`Cargo.toml` 里 `toml_edit = "0.25.13"` 连同五行理由都在（票 08 后来修订过措辞，
  区分了「写用户手写的那份配置」与「状态文件」）。本票只把那段注释里的「票 11 的 `primary`
  回写」换成指向 `config::pin_primary`——一条票号引用变成一条代码引用。
- 第 2 条：`config::refresh` 已经做到，`tests/config.rs::config_refresh_touches_nothing_the_user_wrote`
  守着它。本票为 `primary` 回写这条新路径补了同形状的用例，见下。
- 第 3 条：Endpoint 块那一半是 `config::refresh`；`primary` 那一半是本票的 `config::pin_primary`。
- 第 4 条：`config_refresh_touches_nothing_the_user_wrote` 与
  `config_refresh_only_warns_when_the_scan_disagrees_with_what_the_user_wrote` 两条已经守住。
  **`primary` 是这条边界唯一的例外，而那是 ADR-0003 点名给的**：「只补空缺和程序自己管的
  字段（`primary`、`config-refresh` 填充的 Endpoint 块）」——用户在菜单里点一下就是要改这一格。

## 本票做了什么

`src/config.rs` 里一个纯函数，与 `draft` / `refresh` 住在一处（同一份文件的同一套 schema）：

```rust
pub fn pin_primary(text: &str, device_id: &str) -> Result<Pinned>
pub struct Pinned { pub text: String, pub changed: bool }
```

格式保留靠 `toml_edit`，只换 `primary` 那个值**里面的内容**并把它的 decor 原样搬过来——行尾那句
注释住在值的 decor 里，键上方那句住在键的 decor 里，换掉整个值只会丢前者，而那种半条命的错最难
在肉眼下发现。`[general]` 整节缺席时新建一张**真表**（不是 `general = { … }` 行内表）并把位置钉
在 0。写完还有一道**自检**：这份文本拿启动时那条读法（`Config::parse`）读回来必须真的是这一台。

`tests/config.rs` 新增 9 条（33 → 42），全套 156 → 165：

- `primary_writeback_pins_the_device_the_user_chose` —— 读回来的方式与真正启动时同一条
- `primary_writeback_touches_nothing_else_the_user_wrote` —— 票面第 6 条，**逐行**比一遍，
  只允许 `primary` 那一行不同，行数不许变
- `primary_writeback_keeps_the_comment_the_user_left_on_that_very_line` —— 值的 decor 那一句
- `primary_writeback_writes_nothing_when_it_is_already_that_device` —— 逐字节不变、`changed` 为假
- `primary_writeback_adds_the_setting_when_the_general_section_has_none`
- `primary_writeback_creates_the_general_section_when_the_config_has_none`
- `primary_writeback_refuses_an_id_that_is_not_a_registered_device`
- `primary_writeback_refuses_a_device_whose_id_is_the_rule_word`
- `primary_writeback_survives_a_general_section_written_as_an_inline_table` —— 那条路上
  `toml_edit` 的 `IndexMut` 对认不出的形状是 panic，而往用户配置里写字的程序 panic 是最糟的结局

**没有碰**票 09 划的那条界：自动选出来的 Primary **不回写** `config.toml`（`primary` 只有一个
格子，`"lowest"` 这条规则和一个具体 id 共用它）。它的家还是票 08 的
`state.toml::last_primary`；`src/state.rs` 的行为一行都没动，只把那里一处指向「票 11」的
注释改成指向 `config::pin_primary`——回写这一头现在有代码可指了。

## 第 5 条落到了哪一步

**只挣到了机制那一半。** 回写能力做好并测到了，但**触发点不存在**：ADR-0003 说的是「托盘菜单
允许切换 Primary Device」，而**托盘菜单在 spec 的 Out of Scope 里**（「图标设计按共识必须先做成
本地可交互网页给用户审…审过才允许写渲染代码」）。没有菜单，就没有「在菜单里切换」这个动作——
今天的成品二进制里没有任何一条路会调到 `pin_primary`。

**缺的不止是那个按钮：落盘那一步（`fs::write`）也整条不存在**，`config::pin_primary` 在成品
二进制里没有任何调用方（全仓唯一调用点在 `tests/config.rs`）。「重启后仍生效」的机制是「启动时
读 `config.toml`」，而用例正是拿那条读法读回来断言的，所以**机制这一半是验过的**——但 spec 的
用户故事 17（「我想让这个手动选择在重启后还在」）在本 spec 收口时仍然是空的，而且**本趟队列里
没有任何一张票会造出那个触发点**（与票 09 的 `HeldOver` 不同：那一格 Q41 点名派给了票 08，
现在真的接上了）。落盘那一步照 `cli::config_refresh` 那几行的样子写即可：
读全文、调 `pin_primary`、`Pinned::changed` 为真才 `fs::write`——`changed` 这个字段就是为了让
那一步不必自己再判一遍「改没改」。

选这条路而不是顺手加一个 `juicebar primary <id>` 子命令的理由记在 parking lot **Q51**。
另外两条记录：**Q52**（回写只收一个 Device id，且拒绝不在册的 id 与 id 恰好叫 `lowest` 的设备）、
**Q53**（`[general]` 缺席时新建一张真表、位置钉 0，且不写程序自己的注释）。

**Q51 里还记了一处对用户仍然不成立的话**：`config.example.toml` 已经写着「在托盘菜单里切换
Primary Device 时，新选择会写回这里」。那句话现在是半真的——机制在了，按不到那个按钮。没有改它，
与票 09 留下 `HeldOver` 那半句同一个处置。
