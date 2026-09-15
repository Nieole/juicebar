# 14: 命令行退场

**What to build:** 成品只剩一个程序，就是托盘。`status` 与 `config-refresh` 两条子命令删掉——它们的活
已经由托盘接手：设备行（票 06）、首次运行（票 09）、自动补空块（票 10）；诊断工具早就挪成了示例程序
（票 02）。命令行那一层的排版用例迁到内核那一层：断言对象从命令行那几行字，换成菜单行与悬停提示，
**一条行为都不丢**。README 的"用法"整节重写。

**Blocked by:** 02、06、09、10、11（抄 MAC 那几句提示由 11 改成指向"登记设备"，本票的 grep 才归得了零）

**Status:** ready-for-agent

- [ ] 程序不再有子命令；带不带参数启动都是托盘
- [ ] 原来的排版用例逐条迁到内核层（菜单行、悬停提示），每条原先守着的行为都还有人守——在 PR 里
      附一张迁移前后的对照
- [ ] 取代 `screen-as-a-function` 票 01、03 的那部分到此完成：命令行那一层整个不在了
- [ ] README："用法"重写（双击就是托盘；手动启动弹 UAC；开机自启在菜单里；诊断工具的示例程序跑法）；
      "支持的设备"与"状态"跟着改
- [ ] `docs/gaps.md` 里凡是说"`status` 那一行"的，改成说菜单那一行
- [ ] 仓库里用户看得见的文字与文档不再叫人跑命令行：`git grep -nE 'juicebar (scan|caps|probe|status|config-refresh)'`
      排掉 `docs/protocol.md` 那几条实测记录与 `.scratch/` 之后归零（parking lot Q131、Q275，队列提出、用户未
      反对的默认安排）
- [ ] 删掉配置项 `show_unknown_ble`（用户 2026-09-15 定，parking lot Q286）：它只剩命令行 `status` 在用，随命令行
      一起走；`config.example.toml`、草稿与相关用例跟着改
- [ ] **写一条 ADR**：成品不带命令行，诊断工具只留在示例程序里；备选"两个 exe"被否的理由照 spec
- [ ] gate 三条全绿

## Comments

### 迁移对照：`tests/status.rs` 的 29 条 → 内核层

一行一条：旧用例名 → 新用例名（或"行为随命令行一起消失，理由"）。新落点在四个文件：`tests/tray_hover.rs`（14 条新写，从真的
一次取数排出来，断言确切字符串）、`tests/tray_device_row.rs`（2 条新写）、`tests/tray_register.rs`（1 条新写）、`tests/tray_round.rs`
（现成）。

1. `the_status_line_says_which_endpoint_it_came_from` → `tray_hover::a_reading_says_which_endpoint_it_came_from`
2. `the_status_line_tells_the_ble_cache_apart_from_the_two_hid_endpoints` → `tray_hover::a_ble_reading_is_told_apart_from_the_two_hid_endpoints`
3. `the_status_line_marks_a_stale_reading_apart_from_a_fresh_one` → `tray_hover::a_stale_ble_cache_is_marked_apart_from_a_fresh_one`
4. `a_last_known_reading_is_marked_stale_even_when_it_was_taken_seconds_ago` → `tray_hover::a_last_known_value_is_marked_stale_even_when_it_was_taken_seconds_ago`
5. `does_not_claim_a_device_is_charging_when_the_number_is_a_last_known_value` → `tray_hover::does_not_claim_a_device_is_charging_when_the_number_is_a_last_known_value`；"电压照印"那一句随命令行消失：悬停提示与菜单行不写电压（`src/tray/hover.rs` 模块文档，parking lot Q343）
6. `still_says_charging_on_a_line_it_read_this_round` → `tray_hover::a_reading_says_which_endpoint_it_came_from`（真取数，插着线读到充电中）+ `tray_hover::a_device_charging_right_now_says_so_after_its_level` + `tray_device_row::a_charging_reading_says_so_after_where_it_came_from`；电压那一句同 5
7. `a_ble_reading_without_a_timestamp_does_not_look_freshly_taken` → `tray_hover::a_ble_reading_without_a_timestamp_does_not_look_freshly_taken`
8. `every_status_line_says_how_long_ago_the_reading_was_taken` → `tray_device_row::a_fresh_reading_writes_where_and_when_it_came_from_and_its_level`（"0 秒前"，不说缓存、不说陈旧）+ `tray_hover::a_reading_says_which_endpoint_it_came_from`
9. `shows_only_the_date_when_a_reading_is_older_than_very_stale_after` → `tray_hover::a_reading_too_stale_for_its_percentage_gives_only_its_date` + `tray_device_row::a_reading_too_stale_to_show_its_percentage_writes_the_icon_state_instead`
10. `a_reading_exactly_at_the_threshold_is_still_current` → `tray_hover::a_reading_exactly_at_the_stale_threshold_is_not_marked_stale`
11. `the_ble_age_is_the_number_windows_reported_not_one_recomputed_from_the_clock` → 行为随命令行一起消失：托盘常驻，Windows 当时报的缓存年龄过一分钟就不对了，悬停提示与菜单行有意从取得时刻算到此刻（`tray_device_row::the_age_is_counted_to_the_moment_the_menu_pops_up` 守着反过来的那一条）；"陈旧与否照晚一点的当下判"那一半由 `tray_device_row::a_stale_reading_is_grayed` 守
12. `does_not_list_unregistered_ble_devices_by_default` → `tray_register::the_top_level_does_not_list_unregistered_ble_devices`（开关随 `show_unknown_ble` 删掉，一级永远不列，去处是"登记设备"）
13. `lists_unregistered_ble_devices_when_show_unknown_ble_is_on` → `tray_register::an_unregistered_ble_device_can_be_registered_to_each_device_without_an_address`（列出来）+ `tray_register::a_ble_device_whose_address_is_already_registered_is_not_listed`（写法不同的 MAC 也认得出，登记过的不重复）；"MAC 与电量写在那一行上好抄"随命令行消失：点"登记到"就写进配置，不必抄（票 11）
14. `lists_every_ble_device_when_no_device_is_registered_at_all` → `tray_register::with_no_device_registered_every_scanned_ble_device_is_listed`
15. `the_status_line_says_which_of_the_two_percentages_it_is` → `tray_hover::a_reading_says_which_of_the_two_percentages_it_is`
16. `the_status_line_says_unknown_instead_of_calling_a_zero_full` → `tray_hover::a_keyboard_that_reports_zero_is_unknown_not_full`
17. `keeps_the_last_known_value_while_paused_and_says_which_it_is` → `tray_hover::keeps_the_last_known_value_while_paused_and_says_which_it_is`
18. `does_not_claim_a_paused_device_is_charging_either` → `tray_hover::does_not_claim_a_paused_device_is_charging_either`；电压那一句同 5
19. `says_it_paused_even_when_the_ble_cache_answered` → `tray_hover::says_it_paused_even_when_the_ble_cache_answered`
20. `does_not_mark_a_ble_only_device_as_paused` → `tray_hover::does_not_mark_a_ble_only_device_as_paused`（耳机不再编 `driver`）
21. `marks_the_primary_device_on_its_own_line` → `tray_device_row::the_row_of_the_primary_device_is_checked`
22. `marks_a_pinned_device_that_could_not_be_read` → `tray_device_row::the_row_of_a_pinned_device_that_could_not_be_read_is_checked`
23. `adds_a_closing_note_when_no_device_got_the_marker` → `tray_hover::with_no_primary_device_it_says_why` + `tray_hover::a_pinned_id_that_is_not_registered_says_so`（一行都没标时，悬停提示说为什么）
24. `adds_no_closing_note_when_the_marker_speaks_for_itself` → `tray_hover::a_fresh_reading_names_the_device_its_level_and_where_and_when_it_came_from`（选出来时只写那一台，确切字符串里没有多一句）
25. `holds_over_the_previous_choice_when_nothing_is_trustworthy_this_round` → `tray_device_row::the_row_of_a_primary_device_held_over_is_checked` + `tray_round::once_selected_the_primary_device_is_held_over_when_nothing_can_be_trusted`；"交出那个 id 好记回状态文件"那一半不单独守：保持的是同一个 id，记回去等于不写（托盘只在换了人时存），而它算不算这一轮选出的，由打勾那一条守着
26. `reports_which_device_it_chose_so_the_caller_can_remember_it` → `tray_round::a_round_that_selects_a_primary_device_remembers_it_in_the_state_file`
27. `chooses_nothing_when_there_is_no_candidate_and_no_previous` → `tray_round::a_round_that_selects_no_primary_device_leaves_that_cell_of_the_state_file_alone` + `tray_round::a_failed_fetch_that_selects_no_primary_device_writes_nothing`
28. `stays_silent_when_no_device_is_configured` → `tray_hover::with_no_device_registered_it_says_so_and_where_to_add_one`（一台都没有时只说这一句，不说"选不出 Primary Device"）
29. `composes_a_lost_row_from_the_real_failure_sentence` → `tray_hover::composes_a_lost_device_from_the_real_failure_sentence`

### 随命令行一起走的其它用例

- `primary::marks_the_primary_device_by_name`、`primary::leaves_the_other_device_names_alone` → 随 `Selection::label` 删：名字后面接"（Primary Device）"是命令行那一行的排版；托盘上是设备行打勾（`tray_device_row::the_row_of_the_primary_device_is_checked`）
- `primary::explains_a_pinned_id_that_names_no_device`、`primary::explains_why_no_device_got_the_marker`、`primary::says_nothing_extra_when_the_marker_speaks_for_itself` → 随 `Selection::note` 删：托盘里由悬停提示说（`tray_hover::a_pinned_id_that_is_not_registered_says_so`、`tray_hover::with_no_primary_device_it_says_why`、`tray_hover::a_fresh_reading_names_the_device_its_level_and_where_and_when_it_came_from`）
- `primary::explains_a_marker_that_comes_from_the_last_round` → `tray_hover::a_primary_device_held_over_says_so_at_the_end` + `tray_round::a_held_over_primary_device_keeps_saying_so_as_the_clock_ticks`：那句交代搬进悬停提示末尾（review 跟进补上，parking lot Q342）
- `config::show_unknown_ble_is_off_unless_the_config_asks_for_it` → 开关删掉；`config::a_config_that_still_has_the_removed_show_unknown_ble_key_reads_as_before` 守"写着它的旧配置照读"

### 改写的用例

- `tray_round::at_startup_the_icon_shows_the_last_known_value_of_the_primary_device_on_record`、`tray_round::once_selected_the_primary_device_is_held_over_when_nothing_can_be_trusted`：换回真实形状（几十秒前的上次已知值，另一台的更低），正面断言保持上次的选择（Q154 修掉之后才走得到，Q166）；新增 `round::a_last_known_value_taken_seconds_ago_does_not_take_the_lowest_spot`
- `tray_hover::with_no_device_registered_it_says_so` → `with_no_device_registered_it_says_so_and_where_to_add_one`（Q333 / Q344）
