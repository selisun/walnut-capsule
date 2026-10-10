//! 数值格式化：字节速率、容量、百分比。

/// 字节/秒 → "12.3M" "845K" "1.2G"
pub fn rate(bps: u64) -> String {
    if bps >= 1 << 30 {
        format!("{:.1}G", bps as f64 / (1u64 << 30) as f64)
    } else if bps >= 1 << 20 {
        format!("{:.1}M", bps as f64 / (1u64 << 20) as f64)
    } else if bps >= 1 << 10 {
        format!("{}K", bps >> 10)
    } else {
        format!("{bps}B")
    }
}

/// 字节 → "9.2G"
pub fn bytes(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1}G", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.0}M", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{}K", b >> 10)
    }
}
