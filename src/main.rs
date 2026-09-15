//! juicebar 的入口：托盘。实现都在库那一半（见 `src/lib.rs`），这里只把它跑起来（`juicebar::shell`）。
//!
//! **成品只有这一个样子**：没有子命令，带不带参数启动都是托盘（`docs/adr/0008`）。做协议逆向用的诊断工具
//! （`scan`、`caps`、`probe`）不在这里，在 `examples/` 下，不进发布构建。
//!
//! **它是窗口程序，不是控制台程序**（`windows_subsystem = "windows"`）：双击托盘不该闪出一个黑框。没有控制台
//! 可印，所以起不来时那一句说在消息框里（`juicebar::shell::report_fatal`）。

#![cfg_attr(not(test), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = juicebar::shell::run() {
        juicebar::shell::report_fatal(&e);
    }
}
