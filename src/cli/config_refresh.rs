//! `juicebar config-refresh` —— 没有配置就生成一份草稿，有配置就把空着的 Endpoint 块补上。
//!
//! 一条命令管两种状态是有意的：用户记不住两个命令名，而"配置还不存在"和"配置在、但缺一块"
//! 对他是同一件事——"帮我把配置弄对"。程序自己分得清，那就不该让人替它分。
//!
//! 这里只负责认路、落盘、把做过和没做的事印出来；判断该补什么的那些逻辑都是
//! `crate::config` 里的纯函数，好让它们不碰硬件也测得到。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cli::resolve_config_path;
use crate::{config, hid};

pub fn run(config_path: Option<PathBuf>) -> Result<()> {
    let path = resolve_config_path(config_path)?;
    if !path.exists() {
        return bootstrap(&path);
    }

    let before =
        std::fs::read_to_string(&path).with_context(|| format!("读不到配置 {}", path.display()))?;
    let refreshed = config::refresh(&before, &hid::enumerate()?)
        .with_context(|| format!("配置 {} 有问题", path.display()))?;

    // 没有要补的东西就一个字节都不写。原地编辑本身是保格式的，但"没改动却重写一遍文件"
    // 会白白改掉文件的修改时间，也让人以为程序动过它。
    if refreshed.filled.is_empty() {
        println!("配置 {} 没有需要补的空缺。", path.display());
    } else {
        std::fs::write(&path, &refreshed.text)
            .with_context(|| format!("写不进配置 {}", path.display()))?;
        println!("已经更新 {}：", path.display());
        for line in &refreshed.filled {
            println!("  · {line}");
        }
    }
    for note in &refreshed.notes {
        println!("  提醒：{note}");
    }
    Ok(())
}

/// 首次运行：扫一遍本机，写一份带注释的草稿。
///
/// **只从无到有，绝不覆盖。**用 `create_new` 而不是先 `exists()` 再写：调用方已经查过
/// 文件不在了，但那之后到这里之间它可能被建出来（另一个 juicebar 实例、用户自己），
/// 而配置是用户的东西，宁可报一句错也不能把它盖掉。
pub fn bootstrap(path: &Path) -> Result<()> {
    let draft = config::draft(&hid::enumerate()?);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("建不了目录 {}", dir.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("写不进配置 {}", path.display()))?;
    std::io::Write::write_all(&mut file, draft.as_bytes())
        .with_context(|| format!("写不进配置 {}", path.display()))?;

    println!("已经在 {} 生成一份配置草稿。", path.display());
    println!("里面**注释掉的块**是程序猜不动、或者此刻扫不到的东西——打开看一眼，");
    println!("别在不知情的情况下缺一整条 Endpoint。");
    println!(
        "该插的插好之后（键盘还要把机身模式开关拨到有线档）再跑一次 `juicebar config-refresh`，"
    );
    println!("当时在场而配置里空着的块会被自动补上，你写过的东西一概不动。");
    Ok(())
}
