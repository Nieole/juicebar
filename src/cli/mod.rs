//! 命令行这一层：一个子命令一个模块，外加它们共用的那一点东西。

pub mod caps;
pub mod config_refresh;
pub mod probe;
pub mod scan;
pub mod status;

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

/// 配置文件的默认位置：`%APPDATA%\juicebar\config.toml`。
///
/// 住在这里而不是某个子命令里，是因为不止一个子命令要用它，而"配置在哪"只能有一个答案
/// ——两处各自算一遍，迟早会有一处算的是另一个路径，而那种错表现出来是"程序说没有配置，
/// 可我明明写了"。
pub fn default_config_path() -> Result<PathBuf> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| anyhow!("读不到环境变量 APPDATA，请用 --config 指定配置路径"))?;
    Ok(Path::new(&appdata).join("juicebar").join("config.toml"))
}

/// 状态文件的位置：**解析出来的配置文件旁边**，`state.toml`。
///
/// 缺省因此是 `%APPDATA%\juicebar\state.toml`，也就是 spec 说的「`%APPDATA%\juicebar\` 下的
/// 状态文件」。它与 [`default_config_path`] 是两回事，所以另起一个名字而不是复用那一个
/// ——`docs/adr/0003` 专门划了这条界：用户可见的配置选择回写 `config.toml`，这一份只放
/// 运行时缓存。两者共用一个名字迟早会让人以为它们是同一份文件的两种叫法。
///
/// **跟着 `--config` 走而不是钉死在 `%APPDATA%`**：拿一份临时配置跑一次 `status`，缓存该跟着
/// 那份配置，而不是往用户真正在用的那一份上盖——那一份里存的是他真设备的历史值。代价是同一台
/// 设备用两份配置跑会各记一份历史，而那正是想要的：两份配置里的同一个 `id` 未必指同一台设备。
pub fn state_path_beside_config(config_path: &Path) -> PathBuf {
    config_path.with_file_name("state.toml")
}

/// 子命令的 `--config` 落到哪个路径上：给了就用给的，没给就用默认位置。
///
/// 和 [`default_config_path`] 同理住在一处：每个吃 `--config` 的子命令都要做这一步，
/// 各写一遍就是给"两处算出不同路径"留门。
pub fn resolve_config_path(config_path: Option<PathBuf>) -> Result<PathBuf> {
    match config_path {
        Some(path) => Ok(path),
        None => default_config_path(),
    }
}
