//! juicebar —— 把无线键鼠的电量常驻在 Windows 任务栏。
//!
//! 设计取舍见 README，词汇见 CONTEXT.md，协议细节见 docs/protocol.md。
//!
//! 这个 crate 同时编出一个库和一个可执行文件（`src/main.rs` 只负责分流：不带参数就是托盘，
//! 过渡期的两条子命令走命令行）。分成两半是为了测试：`tests/` 下的集成测试只看得见 `pub` 接口，
//! 测不到私有函数——spec 要的"只测外部行为"因此由编译器守住，而不是靠自觉。
//!
//! 托盘分成两半：`tray` 是内核（纯粹的事件进、动作出，用例全在它上面），`shell` 是 Win32 外壳
//! （把事件喂进去、把动作执行出来，不做自动测试）。

pub mod bluetooth;
pub mod cli;
pub mod clock;
pub mod config;
pub mod endpoints;
pub mod hid;
pub mod icon;
pub mod primary;
pub mod readout;
pub mod round;
pub mod shell;
pub mod sources;
pub mod staleness;
pub mod state;
pub mod tray;
pub mod vendor_hub;
