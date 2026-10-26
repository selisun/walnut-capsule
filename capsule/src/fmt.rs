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

/// 字节 → 标准单位写法（1024 进制）："180 B" / "845 KB" / "1.4 MB" / "3.5 GB" / "1.2 TB"。
/// 三位整数以上不带小数，避免 "999.9 GB" 这类读起来费劲的写法。
pub fn bytes_unit(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0usize;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// 标准单位速率："180 B/s" / "845 KB/s" / "1.4 MB/s"。
pub fn rate_unit(bps: u64) -> String {
    format!("{}/s", bytes_unit(bps))
}

/// 网卡协商速率（Mbps）→ "100 Mbps" / "1 Gbps" / "2.5 Gbps" / "10 Gbps"。
/// 0（未知）返回空串，调用方据此不显示。
pub fn link_speed(mbps: u32) -> String {
    if mbps == 0 {
        return String::new();
    }
    if mbps < 1000 {
        return format!("{mbps} Mbps");
    }
    let g = mbps as f64 / 1000.0;
    if (g - g.round()).abs() < 0.05 {
        format!("{} Gbps", g.round() as u32)
    } else {
        format!("{g:.1} Gbps")
    }
}

/// 紧凑速率：整数位达 3 位时舍弃小数（"1023.9M" → "1023M"），
/// 供空间紧张的三环内使用，避免长串顶出环外；其余与 `rate` 一致。
pub fn rate_compact(bps: u64) -> String {
    let s = rate(bps);
    match s.find('.') {
        Some(dot) if dot >= 3 => format!("{}{}", &s[..dot], &s[dot + 2..]),
        _ => s,
    }
}
