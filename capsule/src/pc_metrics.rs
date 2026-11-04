//! 电脑本机指标（CPU / 内存 / 显卡），供小屏「双显」主题右列使用。
//!
//! - `GetSystemTimes`：两次采样差值算 CPU 占用；
//! - `GlobalMemoryStatusEx`：`dwMemoryLoad` 直接就是内存占用百分比；
//! - `nvidia-smi`：GPU 使用率与温度（与旧版 `screen.ps1` 完全一致的做法，
//!   没有 N 卡或命令不存在时留空，不引入 PDH/WMI）。

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

/// 一次本机采样结果；取不到的项为 `None`。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PcSample {
    pub cpu_pct: Option<f32>,
    pub mem_pct: Option<f32>,
    pub gpu_pct: Option<f32>,
    pub gpu_temp: Option<f32>,
}

impl PcSample {
    pub fn is_empty(&self) -> bool {
        self.cpu_pct.is_none()
            && self.mem_pct.is_none()
            && self.gpu_pct.is_none()
            && self.gpu_temp.is_none()
    }
}

/// 采样器：CPU 首次调用只建立基线；GPU 每 5 次采样查询一次（旧版同节奏）。
#[derive(Default)]
pub struct PcSampler {
    last: Option<(u64, u64, u64)>,
    tick: u32,
    gpu: (Option<f32>, Option<f32>),
}

impl PcSampler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sample(&mut self) -> PcSample {
        let mem_pct = mem_pct();
        let cpu_pct = self.cpu_pct();
        self.tick = (self.tick + 1) % 5;
        if self.tick == 0 {
            self.gpu = query_gpu();
        }
        PcSample {
            cpu_pct,
            mem_pct,
            gpu_pct: self.gpu.0,
            gpu_temp: self.gpu.1,
        }
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

/// 查询 NVIDIA 显卡：`nvidia-smi --query-gpu=utilization.gpu,temperature.gpu`。
/// 没有 N 卡 / 未安装驱动时返回 `(None, None)`。
fn query_gpu() -> (Option<f32>, Option<f32>) {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW：不弹出控制台窗口
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=utilization.gpu,temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return (None, None);
    };
    if !output.status.success() {
        return (None, None);
    }
    parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout)).unwrap_or((None, None))
}

/// 解析 `nvidia-smi` 首行：`12, 45`（单位已去掉）。
fn parse_nvidia_smi(text: &str) -> Option<(Option<f32>, Option<f32>)> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let mut parts = line.split(',');
    let usage = parts
        .next()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| (0.0..=100.0).contains(v));
    let temp = parts
        .next()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| (0.0..=150.0).contains(v));
    if usage.is_none() && temp.is_none() {
        return None;
    }
    Some((usage, temp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_has_no_cpu_baseline() {
        let mut s = PcSampler::new();
        assert!(s.sample().cpu_pct.is_none(), "首次采样只能建立基线");
    }

    #[test]
    fn second_sample_reports_cpu_in_range() {
        let mut s = PcSampler::new();
        let _ = s.sample();
        std::thread::sleep(std::time::Duration::from_millis(80));
        let sample = s.sample();
        if let Some(v) = sample.cpu_pct {
            assert!((0.0..=100.0).contains(&v), "cpu={v}");
        }
        if let Some(v) = sample.mem_pct {
            assert!((0.0..=100.0).contains(&v), "mem={v}");
        }
    }

    #[test]
    fn nvidia_smi_output_is_parsed() {
        assert_eq!(parse_nvidia_smi("37, 56\n"), Some((Some(37.0), Some(56.0))));
        assert_eq!(parse_nvidia_smi("0, 40"), Some((Some(0.0), Some(40.0))));
    }

    #[test]
    fn nvidia_smi_garbage_is_ignored() {
        assert_eq!(parse_nvidia_smi(""), None);
        assert_eq!(parse_nvidia_smi("N/A, N/A"), None);
        assert_eq!(parse_nvidia_smi("[N/A], 45"), Some((None, Some(45.0))));
    }
}
