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
