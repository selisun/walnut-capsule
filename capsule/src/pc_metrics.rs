//! 电脑本机指标（CPU / 内存），供小屏「双显」主题右列使用。
//!
//! 只用两个系统 API，不引入 PDH/WMI：
//! - `GetSystemTimes`：两次采样差值算 CPU 占用；
//! - `GlobalMemoryStatusEx`：`dwMemoryLoad` 直接就是内存占用百分比。

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

/// CPU 采样器：首次调用只建立基线，返回 `None`。
#[derive(Default)]
pub struct PcSampler {
    last: Option<(u64, u64, u64)>,
}

impl PcSampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 返回 (cpu_pct, mem_pct)，任一取不到为 `None`。
    pub fn sample(&mut self) -> (Option<f32>, Option<f32>) {
        (self.cpu_pct(), mem_pct())
    }

    fn cpu_pct(&mut self) -> Option<f32> {
        let mut idle = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
            return None;
        }
        let now = (to_u64(idle), to_u64(kernel), to_u64(user));
        let (prev_idle, prev_kernel, prev_user) = self.last.replace(now)?;
        let d_idle = now.0.saturating_sub(prev_idle);
        // kernel 已包含 idle
        let d_total = now.1.saturating_sub(prev_kernel) + now.2.saturating_sub(prev_user);
        if d_total == 0 {
            return None;
        }
        let busy = d_total.saturating_sub(d_idle);
        Some((busy as f64 / d_total as f64 * 100.0) as f32)
    }
}

fn to_u64(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

fn mem_pct() -> Option<f32> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut status) }.is_err() {
        return None;
    }
    Some(status.dwMemoryLoad as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_has_no_cpu_baseline() {
        let mut s = PcSampler::new();
        let (cpu, _mem) = s.sample();
        assert!(cpu.is_none(), "首次采样只能建立基线");
    }

    #[test]
    fn second_sample_reports_cpu_in_range() {
        let mut s = PcSampler::new();
        let _ = s.sample();
        std::thread::sleep(std::time::Duration::from_millis(80));
        let (cpu, mem) = s.sample();
        if let Some(v) = cpu {
            assert!((0.0..=100.0).contains(&v), "cpu={v}");
        }
        if let Some(v) = mem {
            assert!((0.0..=100.0).contains(&v), "mem={v}");
        }
    }
}
