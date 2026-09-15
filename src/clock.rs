//! Clock 接缝：陈旧判定要的"当下"。
//!
//! spec 的三个接缝里的第二条，理由只有一句：**避免测试依赖真实时间流逝**。一份读数
//! 陈不陈旧是"取得时刻距今多久"的函数，而"距今"这一半是环境。把它收进一个 trait，
//! 陈旧判定的全部行为就能在不 sleep 一秒的情况下断言——把时钟拨到任意一天，比等着
//! 那一天到来便宜得多。
//!
//! 接缝之下的一切都只收一个 [`Timestamp`]，不收 `&dyn Clock`：问环境只在链路的**顶上**
//! 发生一次（托盘的取数线程里，一次取数一次，`crate::shell`），那一个"当下"再被取数与陈旧判定
//! 共用。往下全是纯函数，所以用例根本碰不到系统时钟——那比给它们一个假时钟更硬。
//!
//! 问两次不是浪费而是**错的**：几条 Endpoint 依次试下来会花掉真实时间（一次鼠标超时就是
//! 三秒），用第二个"当下"去判第一个"当下"盖的时间戳，一份当场读到的读数会凭空老几秒。
//! code review 在本票上抓到的正是这个（parking lot Q27）。
//!
//! 那也是为什么这个 trait 只有一个方法——它要答的问题只有一个。

use std::fmt;

/// 一个绝对时刻，精度到秒。
///
/// **存的是 Unix 纪元秒**，不是 Windows 的 FILETIME。两个理由：一是它是读数要落到
/// 磁盘上的形式（票 08 的持久化要把取得时刻写出去再读回来，一个十位数的十进制整数
/// 比 100 纳秒计数好认得多），二是用例里写得出来——`from_unix_secs(0)` 就是
/// 1970-01-01，而同一个时刻的 FILETIME 是 116444736000000000。
///
/// 秒是够用的精度：判的是"几分钟前还是几个月前"，而 Windows 那份蓝牙缓存的年龄本身
/// 就是按秒交出来的（`bluetooth::BleBattery::age_secs`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp {
    unix_secs: u64,
}

impl Timestamp {
    /// Unix 纪元（1970-01-01 00:00:00 UTC）之后的第几秒。
    ///
    /// `const` 是为了用例里写得出 `const NOW: Timestamp = …`。
    pub const fn from_unix_secs(unix_secs: u64) -> Self {
        Self { unix_secs }
    }

    /// 这个时刻是 Unix 纪元之后的第几秒。持久化要的就是这个数。
    pub const fn as_unix_secs(self) -> u64 {
        self.unix_secs
    }

    /// 往回退若干秒。
    ///
    /// `Ble` 的取得时刻就是这么来的：Windows 只说那份缓存"多少秒之前更新过"，
    /// 绝对时刻得由当下减出来。
    ///
    /// 纪元之前不表示：退过头就停在纪元。那种输入只可能来自一个坏掉的系统时钟，
    /// 而下溢成一个天文数字会让一份读数看着像来自公元 5 亿年后。
    pub const fn minus_secs(self, secs: u64) -> Self {
        Self {
            unix_secs: self.unix_secs.saturating_sub(secs),
        }
    }

    /// 从 `earlier` 到这个时刻过了多少秒。
    ///
    /// `earlier` 比自己还晚时交 0，不交负数：那意味着系统时钟被回拨过（`bluetooth.rs`
    /// 算缓存年龄时踩的是同一个坑，处置也相同）。把它说成"0 秒前"是这两条路里唯一
    /// 不会把陈旧读数说成新鲜的那条——它落在最保守的一侧。
    pub const fn secs_since(self, earlier: Self) -> u64 {
        self.unix_secs.saturating_sub(earlier.unix_secs)
    }

    /// 这个时刻落在日历上的哪一天，`YYYY-MM-DD`。
    ///
    /// **按 UTC 算，不按本机时区。**只在一份读数超过 `very_stale_after`（缺省一天）
    /// 时才印得出来，那时"哪一天"要传达的是"这个数不是现在的"，而时区最多让它差一天
    /// ——相对于那个信号是噪声。换来的是它是个纯函数：不问系统时区、不收日期库，
    /// 用例断言得到确切的字符串（parking lot Q28）。
    pub fn utc_date_text(self) -> String {
        let (year, month, day) = self.utc_ymd();
        format!("{year:04}-{month:02}-{day:02}")
    }

    /// 纪元秒 → 公历年月日。
    ///
    /// Howard Hinnant 的 `civil_from_days`：把纪元挪到 3 月 1 日开头的"内部年"，
    /// 闰日于是落在内部年的末尾，月长因此有闭式解，全程整数运算、没有循环、没有查表。
    fn utc_ymd(self) -> (i64, u32, u32) {
        let days_since_unix_epoch = (self.unix_secs / 86_400) as i64;
        // 719_468 = 1970-01-01 距 0000-03-01 的天数，把纪元对齐到内部年的开头。
        let days_since_internal_epoch = days_since_unix_epoch + 719_468;
        let era = days_since_internal_epoch.div_euclid(146_097); // 146_097 = 400 年的天数
        let day_of_era = days_since_internal_epoch.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
        // 内部年从 3 月起算：月份 0..=9 是 3 月到 12 月，10 与 11 是次年的 1、2 月。
        let month = if month_prime < 10 {
            month_prime + 3
        } else {
            month_prime - 9
        } as u32;
        let year = if month <= 2 { year + 1 } else { year };
        (year, month, day)
    }
}

impl fmt::Display for Timestamp {
    /// 印成日期。一个时刻出现在用户面前时要答的问题只有"哪一天"。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.utc_date_text())
    }
}

/// Clock 接缝：唯一被允许去问"现在几点"的地方。
pub trait Clock {
    /// 当下时刻。
    fn now(&self) -> Timestamp;
}

/// 落到真机上的时钟：一次 `GetSystemTimeAsFileTime`。
///
/// 与 `bluetooth.rs` 里那个私有的 `now_filetime` 问的是同一个系统调用。没有把那一个
/// 借过来用，是因为那边要的是 FILETIME 本身（它要和一个同样是 FILETIME 的设备属性
/// 相减），这边要的是纪元秒；为了共用三行 Win32 而把那边的内部表示暴露出来，代价
/// 比这三行大。
pub struct SystemClock;

/// FILETIME 的纪元（1601-01-01）到 Unix 纪元（1970-01-01）之间的秒数。
const FILETIME_EPOCH_TO_UNIX_SECS: u64 = 11_644_473_600;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

        let ft = unsafe { GetSystemTimeAsFileTime() };
        let filetime = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
        // FILETIME 是 100ns 单位。系统时钟被设到 1970 以前时相减会下溢，saturating
        // 处理成纪元——理由与 [`Timestamp::minus_secs`] 那一句相同。
        Timestamp::from_unix_secs(
            (filetime / 10_000_000).saturating_sub(FILETIME_EPOCH_TO_UNIX_SECS),
        )
    }
}
