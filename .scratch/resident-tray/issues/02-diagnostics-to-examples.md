# 02: 诊断工具挪成示例程序

**What to build:** `scan`、`probe`、`caps`、`probe --listen` 从子命令变成开发者用的示例程序
（`cargo run --example …`），发布构建里不再有它们。**行为一个字不变。** 这是命令行退场的第一步，
也是纯粹的预重构：它们本来就只调用库的公开面，键盘那条 2.4G 以后要逆也离不开它们。

**Blocked by:** None (can start immediately) —— 与票 01、03 一个文件都不相交，三张可并行。

**Status:** ready-for-agent

- [ ] 四个诊断能力都能以示例程序跑起来，参数与输出和今天一样（`probe` 的 `--listen`、
      `--no-write`、`--feature`、`--vgn-crc`、`--reads`、`--timeout` 全保留）
- [ ] 子命令里不再有 `scan` / `probe` / `caps`；`status` 与 `config-refresh` 暂时留着（票 14 收）
- [ ] 示例程序只用库的公开面，**不为它们新开"仅供示例"的后门**
- [ ] `probe` 那句错误信息里的 13 个空格：若 `screen-as-a-function` 票 02 已落地，随代码搬走即可；
      若尚未落地，本票不修——那是票 02 的活，它会跟着代码找到新位置
- [ ] README 里提到这几条子命令的地方，改成示例程序的跑法
- [ ] gate 三条全绿
