//! 给 `juicebar.exe` 嵌一份程序清单（`juicebar.exe.manifest`）：要求管理员权限，按显示器感知缩放。
//!
//! **要求管理员权限**：走 2.4G 必须是管理员，而权限不够时 Windows 把写静默吞掉，症状和硬件故障分不开
//! （`docs/gaps.md`）。清单一套上，双击就弹 UAC，那条路就不再挡人。要求提权那一段由链接器生成
//! （`/MANIFESTUAC`），清单文件里只写其余的。
//!
//! **只落在 bin 目标上**（`rustc-link-arg-bins`，不是 `rustc-link-arg`）：落到测试与示例上，每一个测试
//! 二进制都会要求提权，非管理员的 `cargo test` 就会以 os error 740 全军覆没。bin 自己的单元测试二进制
//! 在 Cargo 眼里也是 bin 目标，所以 `Cargo.toml` 的 `[[bin]]` 关了 `test`。
//!
//! 只认 MSVC 的链接器：别的工具链上不嵌，并且说一声——一份没有清单的 `juicebar.exe` 看着一样能跑，
//! 只是走 2.4G 永远读超时。

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=juicebar.exe.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        println!(
            "cargo:warning=不是 MSVC 工具链，没有给 juicebar.exe 嵌程序清单：它不会要求管理员权限，走 2.4G 会读超时"
        );
        return;
    }
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").expect("Cargo 总会给 CARGO_MANIFEST_DIR");
    let manifest = Path::new(&manifest_dir).join("juicebar.exe.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:level='requireAdministrator'");
}
