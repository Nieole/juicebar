//! 诊断工具（`examples/` 下的示例程序）共用的那一点东西。

/// 十六进制入参统一用这个解析，写 `391d` 和 `0x391d` 都认。
pub fn parse_hex_u16(s: &str) -> Result<u16, String> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).map_err(|e| format!("{s} 不是十六进制数: {e}"))
}
