//! CDC 传输：打开接口路径、设置 115200/8N1、RTS 硬件握手、DTR 使能、同步读写。

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use screen_adapter_api::{AdapterError, PortCandidate, Transport};
use screen_core::TransportKind;
use windows::core::PCWSTR;
use windows::Win32::Devices::Communication::{
    ClearCommError, GetCommState, PurgeComm, SetCommState, SetCommTimeouts, SetupComm,
    COMMTIMEOUTS, COMSTAT, DCB, NOPARITY, ONESTOPBIT, PURGE_RXCLEAR, PURGE_TXCLEAR,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE, OPEN_EXISTING,
};
use windows::Win32::System::IO::OVERLAPPED;

use crate::enumerate::enumerate_comports;

pub const BAUD_RATE: u32 = 115_200;
pub const WRITE_TIMEOUT_MS: u32 = 3_000;

const F_BINARY: u32 = 1 << 0;
const F_PARITY: u32 = 1 << 1;
const F_OUTX_CTS_FLOW: u32 = 1 << 2;
const F_ABORT_ON_ERROR: u32 = 1 << 14;
const DTR_ENABLE: u32 = 1 << 4; // fDtrControl = 1（2 bit）
const RTS_HANDSHAKE: u32 = 2 << 12; // fRtsControl = 2（2 bit）

/// 配置后的可读参数，用于测试报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommSettings {
    pub dcb_length: u32,
    pub baud_rate: u32,
    pub byte_size: u8,
    pub parity: u8,
    pub stop_bits: u8,
    pub dtr_control: u8,
    pub rts_control: u8,
    pub outx_cts_flow: bool,
}

/// 一次 FlushFileBuffers 的诊断结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlushReport {
    pub started_epoch_ms: u128,
    pub elapsed_us: u128,
    pub error: Option<AdapterError>,
}

impl FlushReport {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

pub struct CdcTransport {
    handle: Option<HANDLE>,
}

// HANDLE 是内核句柄；CdcTransport 独占持有并在同一线程使用，移动所有权是安全的。
unsafe impl Send for CdcTransport {}

impl Default for CdcTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl CdcTransport {
    pub fn new() -> Self {
        Self { handle: None }
    }

    pub fn is_open(&self) -> bool {
        self.handle.is_some()
    }

    /// 读取当前实际 DCB 参数。
    pub fn current_settings(&self) -> Result<CommSettings, AdapterError> {
        let handle = self.handle.ok_or(AdapterError::Disconnected)?;
        unsafe {
            let mut dcb = DCB {
                DCBlength: std::mem::size_of::<DCB>() as u32,
                ..Default::default()
            };
            GetCommState(handle, &mut dcb).map_err(|e| map_win32_error(&e))?;
            Ok(settings_from_dcb(&dcb))
        }
    }

    /// 丢弃收发缓冲（恢复/关闭前使用）。
    pub fn purge(&mut self) -> Result<(), AdapterError> {
        let handle = self.handle.ok_or(AdapterError::Disconnected)?;
        unsafe { PurgeComm(handle, PURGE_TXCLEAR | PURGE_RXCLEAR).map_err(|e| map_win32_error(&e)) }
    }

    /// 查询通信错误（用于健康检查）。
    pub fn clear_errors(&mut self) -> Result<(), AdapterError> {
        let handle = self.handle.ok_or(AdapterError::Disconnected)?;
        unsafe { ClearCommError(handle, None, None).map_err(|e| map_win32_error(&e)) }
    }

    /// 驱动输出队列字节数（可靠时用于 Flush 前后对照）。
    pub fn output_queue_bytes(&self) -> Result<u32, AdapterError> {
        let handle = self.handle.ok_or(AdapterError::Disconnected)?;
        let mut stat = COMSTAT::default();
        unsafe {
            ClearCommError(handle, None, Some(&mut stat)).map_err(|e| map_win32_error(&e))?;
        }
        Ok(stat.cbOutQue)
    }

    /// 完整帧写入后调用一次：等待驱动/设备发送队列排空。
    /// 同步调用，若底层长时间不返回，本调用也会一直等待（不做强制超时/关闭）。
    pub fn flush_with_report(&mut self) -> FlushReport {
        let started_epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let start = Instant::now();
        let error = match self.handle {
            None => Some(AdapterError::Disconnected),
            Some(handle) => unsafe { FlushFileBuffers(handle).err().map(|e| map_win32_error(&e)) },
        };
        FlushReport {
            started_epoch_ms,
            elapsed_us: start.elapsed().as_micros(),
            error,
        }
    }
}

impl Transport for CdcTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Cdc
    }

    fn enumerate(&mut self) -> Result<Vec<PortCandidate>, AdapterError> {
        enumerate_comports()
    }

    fn open(&mut self, candidate: &PortCandidate) -> Result<(), AdapterError> {
        self.close();
        if candidate.transport != TransportKind::Cdc {
            return Err(AdapterError::Unsupported("非 CDC 候选"));
        }
        let path = candidate
            .open_path
            .as_deref()
            .ok_or(AdapterError::Unsupported("候选缺少接口路径"))?;
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let handle = CreateFileW(
                PCWSTR::from_raw(wide.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None::<*const SECURITY_ATTRIBUTES>,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE(std::ptr::null_mut()),
            )
            .map_err(|e| map_win32_error(&e))?;

            if let Err(err) = configure(handle) {
                let _ = CloseHandle(handle);
                return Err(err);
            }
            self.handle = Some(handle);
        }
        Ok(())
    }

    fn write(&mut self, bytes: &[u8], _timeout_ms: u32) -> Result<usize, AdapterError> {
        let handle = self.handle.ok_or(AdapterError::Disconnected)?;
        let mut written = 0u32;
        unsafe {
            WriteFile(
                handle,
                Some(bytes),
                Some(&mut written),
                None::<*mut OVERLAPPED>,
            )
            .map_err(|e| map_win32_error(&e))?;
        }
        let written = written as usize;
        if written != bytes.len() {
            return Err(AdapterError::Io(format!(
                "写入不完整：{written}/{}",
                bytes.len()
            )));
        }
        Ok(written)
    }

    fn read(&mut self, _buf: &mut [u8], _timeout_ms: u32) -> Result<usize, AdapterError> {
        // 官方读回协议尚未验证；阶段 A 只写不读。
        Err(AdapterError::Unsupported("读回未验证"))
    }

    fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

impl Drop for CdcTransport {
    fn drop(&mut self) {
        self.close();
    }
}

fn configure(handle: HANDLE) -> Result<(), AdapterError> {
    unsafe {
        let mut dcb = DCB {
            DCBlength: std::mem::size_of::<DCB>() as u32,
            ..Default::default()
        };
        GetCommState(handle, &mut dcb).map_err(|e| map_win32_error(&e))?;
        apply_legacy_settings(&mut dcb);
        SetCommState(handle, &dcb).map_err(|e| map_win32_error(&e))?;

        let timeouts = COMMTIMEOUTS {
            ReadIntervalTimeout: 0,
            ReadTotalTimeoutMultiplier: 0,
            ReadTotalTimeoutConstant: 500,
            WriteTotalTimeoutMultiplier: 0,
            WriteTotalTimeoutConstant: WRITE_TIMEOUT_MS,
        };
        SetCommTimeouts(handle, &timeouts).map_err(|e| map_win32_error(&e))?;

        SetupComm(handle, 4096, 8192).map_err(|e| map_win32_error(&e))?;
    }
    Ok(())
}

/// 复现旧系统：115200/8/None/One、DTR 使能、RTS/CTS 硬件握手（不是 RtsEnable=true）。
fn apply_legacy_settings(dcb: &mut DCB) {
    dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
    dcb.BaudRate = BAUD_RATE;
    dcb.ByteSize = 8;
    dcb.Parity = NOPARITY;
    dcb.StopBits = ONESTOPBIT;

    let mut bits = dcb._bitfield;
    bits |= F_BINARY | F_OUTX_CTS_FLOW;
    bits &= !F_PARITY;
    bits &= !F_ABORT_ON_ERROR;
    bits = (bits & !(0b11 << 4)) | DTR_ENABLE;
    bits = (bits & !(0b11 << 12)) | RTS_HANDSHAKE;
    dcb._bitfield = bits;
}

fn settings_from_dcb(dcb: &DCB) -> CommSettings {
    CommSettings {
        dcb_length: dcb.DCBlength,
        baud_rate: dcb.BaudRate,
        byte_size: dcb.ByteSize,
        parity: dcb.Parity.0,
        stop_bits: dcb.StopBits.0,
        dtr_control: ((dcb._bitfield >> 4) & 0b11) as u8,
        rts_control: ((dcb._bitfield >> 12) & 0b11) as u8,
        outx_cts_flow: (dcb._bitfield & F_OUTX_CTS_FLOW) != 0,
    }
}

pub fn map_win32_error(err: &windows::core::Error) -> AdapterError {
    map_win32_code((err.code().0 as u32) & 0xFFFF)
}

pub fn map_win32_code(code: u32) -> AdapterError {
    match code {
        5 | 32 => AdapterError::Busy, // ACCESS_DENIED / SHARING_VIOLATION
        2 | 3 | 1167 | 995 => AdapterError::Disconnected, // 文件不存在 / 设备未连接 / 操作中止
        121 => AdapterError::Timeout, // ERROR_SEM_TIMEOUT
        other => AdapterError::Io(format!("Win32 error {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_set_rts_handshake_and_dtr() {
        let mut dcb = DCB::default();
        apply_legacy_settings(&mut dcb);
        let s = settings_from_dcb(&dcb);
        assert_eq!(s.dcb_length, 28); // Windows DCB 实际大小
        assert_eq!(s.baud_rate, 115_200);
        assert_eq!(s.byte_size, 8);
        assert_eq!(s.parity, 0); // NOPARITY
        assert_eq!(s.stop_bits, 0); // ONESTOPBIT
        assert_eq!(s.dtr_control, 1); // DTR_CONTROL_ENABLE
        assert_eq!(s.rts_control, 2); // RTS_CONTROL_HANDSHAKE
        assert!(s.outx_cts_flow);
        assert_eq!(dcb._bitfield & F_PARITY, 0);
        assert_eq!(dcb._bitfield & F_ABORT_ON_ERROR, 0);
    }

    #[test]
    fn error_codes_map_to_states() {
        assert_eq!(map_win32_code(5), AdapterError::Busy);
        assert_eq!(map_win32_code(32), AdapterError::Busy);
        assert_eq!(map_win32_code(1167), AdapterError::Disconnected);
        assert_eq!(map_win32_code(995), AdapterError::Disconnected);
        assert_eq!(map_win32_code(121), AdapterError::Timeout);
        assert!(matches!(map_win32_code(87), AdapterError::Io(_)));
    }

    #[test]
    fn unopened_transport_reports_disconnected() {
        let mut t = CdcTransport::new();
        assert!(!t.is_open());
        assert_eq!(t.current_settings(), Err(AdapterError::Disconnected));
        assert_eq!(t.write(&[1, 2, 3], 100), Err(AdapterError::Disconnected));
        assert_eq!(t.output_queue_bytes(), Err(AdapterError::Disconnected));
        let flush = t.flush_with_report();
        assert!(flush.error == Some(AdapterError::Disconnected));
        assert!(flush.elapsed_us <= 1_000_000);
        t.close();
    }

    #[test]
    fn read_is_explicitly_unsupported() {
        let mut t = CdcTransport::new();
        let mut buf = [0u8; 4];
        assert_eq!(
            t.read(&mut buf, 100),
            Err(AdapterError::Unsupported("读回未验证"))
        );
    }
}
