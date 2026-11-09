//! 硬件品牌探测：供官方皮肤按品牌选底图（两处 logo）。
//!
//! - CPU：`CPUID` 叶 0 的厂商字符串（`AuthenticAMD` / `GenuineIntel`），不需要任何系统 API。
//! - GPU：DXGI 枚举适配器，读 `DXGI_ADAPTER_DESC1::VendorId`（0x10DE / 0x1002 / 0x8086）。
//!
//! 探测失败一律返回 `Unknown`，由主题的 `background.variants` 决定回退到哪张图。

/// CPU 厂商。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuBrand {
    Amd,
    Intel,
    Unknown,
}

impl CpuBrand {
    pub fn key(self) -> &'static str {
        match self {
            CpuBrand::Amd => "amd",
            CpuBrand::Intel => "intel",
            CpuBrand::Unknown => "unknown",
        }
    }
}

/// GPU 厂商。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBrand {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

impl GpuBrand {
    pub fn key(self) -> &'static str {
        match self {
            GpuBrand::Nvidia => "nvidia",
            GpuBrand::Amd => "amd",
            GpuBrand::Intel => "intel",
            GpuBrand::Unknown => "unknown",
        }
    }
}

/// 主题 `background.variants` 的键：`{cpu}-{gpu}`。
pub fn variant_key(cpu: CpuBrand, gpu: GpuBrand) -> String {
    format!("{}-{}", cpu.key(), gpu.key())
}

/// 由 NAS 上报的厂商标识生成变体键（官方皮肤按**被监控的 NAS** 选底图）。
///
/// - CPU：`amd` / `intel`，其它或缺失记 `unknown`；
/// - GPU：`nvidia` / `amd` / `intel`；**缺失时回退到 CPU 品牌**——NAS 没有独显时显卡
///   基本只有核显，Intel 平台的核显是 Intel、AMD 平台的是 AMD，正好对上
///   `intel-intel` / `amd-amd` 两套底图。
///
/// 两个厂商都缺失时返回 `None`，调用方应保留原有底图（如本机探测结果）。
pub fn variant_key_from_vendors(cpu: Option<&str>, gpu: Option<&str>) -> Option<String> {
    let cpu = vendor_brand(cpu)?;
    let gpu = match gpu.and_then(gpu_vendor_brand) {
        Some(gpu) => gpu,
        None => gpu_brand_like_cpu(cpu),
    };
    Some(variant_key(cpu, gpu))
}

/// 显卡厂商缺失时按 CPU 平台推断核显品牌。
fn gpu_brand_like_cpu(cpu: CpuBrand) -> GpuBrand {
    match cpu {
        CpuBrand::Amd => GpuBrand::Amd,
        CpuBrand::Intel => GpuBrand::Intel,
        CpuBrand::Unknown => GpuBrand::Unknown,
    }
}

/// `amd` / `intel` → 品牌；大小写与空白不敏感。
fn vendor_brand(v: Option<&str>) -> Option<CpuBrand> {
    match v?.trim().to_ascii_lowercase().as_str() {
        "amd" => Some(CpuBrand::Amd),
        "intel" => Some(CpuBrand::Intel),
        _ => None,
    }
}

/// `nvidia` / `amd` / `intel` → 品牌。
fn gpu_vendor_brand(v: &str) -> Option<GpuBrand> {
    match v.trim().to_ascii_lowercase().as_str() {
        "nvidia" => Some(GpuBrand::Nvidia),
        "amd" => Some(GpuBrand::Amd),
        "intel" => Some(GpuBrand::Intel),
        _ => None,
    }
}

/// 厂商字符串 → CPU 品牌（大小写与前后空白不敏感）。
pub fn cpu_brand_from_vendor(vendor: &str) -> CpuBrand {
    let v = vendor.trim().to_ascii_lowercase();
    if v.contains("authenticamd") {
        CpuBrand::Amd
    } else if v.contains("genuineintel") {
        CpuBrand::Intel
    } else {
        CpuBrand::Unknown
    }
}

/// PCI 厂商 ID → GPU 品牌。`0x1414`（Microsoft 基本显示）等一律视为未知。
pub fn gpu_brand_from_vendor_id(id: u32) -> GpuBrand {
    match id {
        0x10DE => GpuBrand::Nvidia,
        0x1002 | 0x1022 => GpuBrand::Amd,
        0x8086 => GpuBrand::Intel,
        _ => GpuBrand::Unknown,
    }
}

/// 读本机 CPU 厂商标识。
pub fn cpu_brand() -> CpuBrand {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        let bytes = cpuid_vendor();
        match std::str::from_utf8(&bytes) {
            Ok(vendor) => cpu_brand_from_vendor(vendor),
            Err(_) => CpuBrand::Unknown,
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        CpuBrand::Unknown
    }
}

#[cfg(target_arch = "x86_64")]
fn cpuid_vendor() -> [u8; 12] {
    // 叶 0 返回 12 字节厂商串：EBX、EDX、ECX 依次拼接。
    let regs = std::arch::x86_64::__cpuid(0);
    let mut out = [0u8; 12];
    for (i, chunk) in [regs.ebx, regs.edx, regs.ecx].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&chunk.to_le_bytes());
    }
    out
}

#[cfg(target_arch = "x86")]
fn cpuid_vendor() -> [u8; 12] {
    let regs = std::arch::x86::__cpuid(0);
    let mut out = [0u8; 12];
    for (i, chunk) in [regs.ebx, regs.edx, regs.ecx].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&chunk.to_le_bytes());
    }
    out
}

/// 枚举本机 GPU，返回显存最大的那张卡所属品牌。
///
/// 笔记本/带核显的台式机通常先枚举到核显，直接取第一个会误判成 Intel；
/// 官方皮肤的 logo 应跟随独立显卡，因此按 `DedicatedVideoMemory` 取最大。
#[cfg(windows)]
pub fn gpu_brand() -> GpuBrand {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    let mut best: Option<(GpuBrand, usize)> = None;
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
            return GpuBrand::Unknown;
        };
        let mut index = 0u32;
        loop {
            let Ok(adapter) = factory.EnumAdapters1(index) else {
                break;
            };
            index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            let brand = gpu_brand_from_vendor_id(desc.VendorId);
            if brand == GpuBrand::Unknown {
                continue;
            }
            let mem = desc.DedicatedVideoMemory;
            if best.map_or(true, |(_, best_mem)| mem > best_mem) {
                best = Some((brand, mem));
            }
        }
    }
    best.map(|(brand, _)| brand).unwrap_or(GpuBrand::Unknown)
}

#[cfg(not(windows))]
pub fn gpu_brand() -> GpuBrand {
    GpuBrand::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_vendor_strings_map_to_brands() {
        assert_eq!(cpu_brand_from_vendor("AuthenticAMD"), CpuBrand::Amd);
        assert_eq!(cpu_brand_from_vendor("GenuineIntel"), CpuBrand::Intel);
        assert_eq!(cpu_brand_from_vendor(" authenticamd "), CpuBrand::Amd);
        assert_eq!(cpu_brand_from_vendor(""), CpuBrand::Unknown);
        assert_eq!(cpu_brand_from_vendor("ARM"), CpuBrand::Unknown);
    }

    #[test]
    fn pci_vendor_ids_map_to_gpu_brands() {
        assert_eq!(gpu_brand_from_vendor_id(0x10DE), GpuBrand::Nvidia);
        assert_eq!(gpu_brand_from_vendor_id(0x1002), GpuBrand::Amd);
        assert_eq!(gpu_brand_from_vendor_id(0x8086), GpuBrand::Intel);
        // Microsoft 基本显示适配器不是真实显卡
        assert_eq!(gpu_brand_from_vendor_id(0x1414), GpuBrand::Unknown);
        assert_eq!(gpu_brand_from_vendor_id(0), GpuBrand::Unknown);
    }

    #[test]
    fn variant_key_is_cpu_then_gpu() {
        assert_eq!(variant_key(CpuBrand::Amd, GpuBrand::Nvidia), "amd-nvidia");
        assert_eq!(
            variant_key(CpuBrand::Intel, GpuBrand::Unknown),
            "intel-unknown"
        );
    }

    #[test]
    fn nas_vendors_pick_variant_key() {
        // NAS 同时上报 CPU 与显卡
        assert_eq!(
            variant_key_from_vendors(Some("intel"), Some("nvidia")).as_deref(),
            Some("intel-nvidia")
        );
        assert_eq!(
            variant_key_from_vendors(Some("AMD"), Some("Intel")).as_deref(),
            Some("amd-intel")
        );
        // 显卡缺失 → 回退到同品牌核显（Intel 平台 / AMD 平台各一套底图）
        assert_eq!(
            variant_key_from_vendors(Some("intel"), None).as_deref(),
            Some("intel-intel")
        );
        assert_eq!(
            variant_key_from_vendors(Some("amd"), Some("unknown")).as_deref(),
            Some("amd-amd")
        );
        // CPU 未知 → 不猜，交回调用方保留原底图
        assert_eq!(variant_key_from_vendors(None, Some("nvidia")), None);
        assert_eq!(variant_key_from_vendors(Some("  "), Some("nvidia")), None);
    }

    #[test]
    fn local_detection_never_panics() {
        // 具体结果取决于本机硬件，只断言不 panic 且能生成合法键
        let key = variant_key(cpu_brand(), gpu_brand());
        assert!(key.contains('-'), "键应形如 cpu-gpu：{key}");
    }
}
