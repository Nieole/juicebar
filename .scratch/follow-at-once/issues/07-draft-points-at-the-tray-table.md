# 07: 草稿说一句 `[tray]` 从哪来

**What to build:** 首次运行生成的配置草稿，在 `[general]` 之后、第一个 `[[device]]` 之前多两行注释，大意
"`[tray]` 图标样式与菜单显示：在托盘右键菜单里改，程序会写回到这里；各键的含义见 `config.example.toml`"——打开
文件的人知道那张表会从哪来。**不写那八个键**：它们在菜单里都看得见、点得动。（spec「小尾巴」；parking lot Q303。）

- 草稿开头那段文档里"一个用户看不见的开关等于不存在"那句补一句：`[tray]` 的开关在菜单里看得见，所以只留一个指路的注释。
- 草稿仍然照 TOML 的规矩把这两行放在顶层键之后、任何表之前不引入新的表头（注释不是表头，位置只为了让人在 `[general]`
  旁边读到它）。

**Blocked by:** None —— can start immediately

**Status:** ready-for-agent

- [ ] 配置用例：草稿里有这两行注释，位置在 `[general]` 那一节之后、第一个 `[[device]]` 之前；草稿里没有 `[tray]` 表头、没有那八个键
- [ ] 现有"草稿与样例的 `[general]` 逐项一致"等草稿用例照旧全绿
- [ ] gate 三条全绿（`cargo test` 只能在 Windows 上跑）
