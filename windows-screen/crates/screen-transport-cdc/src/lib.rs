//! Windows CDC/串口传输实现。
//!
//! 阶段 A 批准 feature：Foundation / Storage_FileSystem / Devices_Communication /
//! Devices_DeviceAndDriverInstallation / Devices_Properties / Security / System_IO。
//! `Security` 与 `System_IO` 仅用于 `CreateFileW`/`ReadFile`/`WriteFile` 的编译门控，
//! 不实现 ACL、身份验证或异步 IO。

mod enumerate;
pub mod port;
pub mod watch;

pub use port::{map_win32_code, CdcTransport, FlushReport};
pub use watch::{DeviceEvent, DeviceWatcher};

#[cfg(test)]
mod api_probe {
    //! 最小编译验证：确认阶段 A 七项 feature 下这些 API/类型可导入。
    #[test]
    fn approved_stage_a_apis_import() {
        use windows::core::PCWSTR;
        use windows::Win32::Devices::Communication::{
            GetCommState, SetCommState, SetCommTimeouts, COMMTIMEOUTS, DCB,
        };
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{CreateFileW, ReadFile, WriteFile};

        let _ = std::mem::size_of::<DCB>();
        let _ = std::mem::size_of::<COMMTIMEOUTS>();
        let _: unsafe fn(
            PCWSTR,
            u32,
            windows::Win32::Storage::FileSystem::FILE_SHARE_MODE,
            Option<*const windows::Win32::Security::SECURITY_ATTRIBUTES>,
            windows::Win32::Storage::FileSystem::FILE_CREATION_DISPOSITION,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES,
            HANDLE,
        ) -> windows::core::Result<HANDLE> = CreateFileW::<PCWSTR, HANDLE>;
        let _: unsafe fn(
            HANDLE,
            Option<&mut [u8]>,
            Option<*mut u32>,
            Option<*mut windows::Win32::System::IO::OVERLAPPED>,
        ) -> windows::core::Result<()> = ReadFile::<HANDLE>;
        let _: unsafe fn(
            HANDLE,
            Option<&[u8]>,
            Option<*mut u32>,
            Option<*mut windows::Win32::System::IO::OVERLAPPED>,
        ) -> windows::core::Result<()> = WriteFile::<HANDLE>;
        let _: unsafe fn(HANDLE, *mut DCB) -> windows::core::Result<()> = GetCommState::<HANDLE>;
        let _: unsafe fn(HANDLE, *const DCB) -> windows::core::Result<()> = SetCommState::<HANDLE>;
        let _: unsafe fn(HANDLE, *const COMMTIMEOUTS) -> windows::core::Result<()> =
            SetCommTimeouts::<HANDLE>;
    }
}
