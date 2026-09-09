//! juicebar —— 把无线键鼠的电量常驻在 Windows 任务栏。
//!
//! 设计取舍见 README，词汇见 CONTEXT.md，协议细节见 docs/protocol.md。
//!
//! 这个 crate 同时编出一个库和一个可执行文件（`src/main.rs` 只负责解析命令行）。
//! 分成两半是为了测试：`tests/` 下的集成测试只看得见 `pub` 接口，测不到私有函数
//! ——spec 要的"只测外部行为"因此由编译器守住，而不是靠自觉。

pub mod bluetooth;
pub mod cli;
pub mod config;
pub mod hid;
pub mod sources;
