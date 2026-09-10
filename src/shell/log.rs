//! 日志文件：写在配置文件旁边（`juicebar.log`），限制大小、滚动覆盖（spec「开机自启、单实例、日志」）。
//!
//! 托盘没有控制台，取数失败的完整原因（图标上只有一个"!"、悬停提示也放不下整句）只能落在这里。写什么
//! 由内核定（[`crate::tray::Action::Log`]），这里只管往哪写、写多大。
//!
//! 滚动：当前那一份再写一行就超过 [`MAX_BYTES`] 时，把它改名成 `juicebar.log.1`（盖掉上一份），另起一份。
//! 于是磁盘上最多两份、合起来不超过两倍的上限，而最近的那些行总在。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use windows::Win32::System::SystemInformation::GetLocalTime;

/// 一份日志最大多少字节。一行取数失败的原因一两百字节，这够记好几千行。
const MAX_BYTES: u64 = 1024 * 1024;

pub(super) struct LogFile {
    path: PathBuf,
}

impl LogFile {
    /// 配置文件旁边那一份。
    pub(super) fn beside(config_path: &Path) -> Self {
        Self {
            path: config_path.with_file_name("juicebar.log"),
        }
    }

    /// 写一行，前面带上本机时间。
    ///
    /// **写不进去就算了**：日志是最后一个能说话的地方，它自己出了事，没有别处可以再说。托盘照常跑。
    pub(super) fn write(&mut self, message: &str) {
        let line = format!("{}  {message}\n", local_time_text());
        let size = std::fs::metadata(&self.path).map_or(0, |meta| meta.len());
        if size > 0 && size + line.len() as u64 > MAX_BYTES {
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
        }
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// 本机此刻的时间，`YYYY-MM-DD HH:MM:SS`。日志是给人对着自己的钟看的，所以按本机时区。
fn local_time_text() -> String {
    // SAFETY: 读一次本机时间。
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}
