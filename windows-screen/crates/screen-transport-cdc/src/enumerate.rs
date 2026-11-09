//! SetupAPI 枚举 COMPORT 接口，返回「实例 ID + 接口路径」三元组中的两个。

use screen_adapter_api::{AdapterError, PortCandidate};
use screen_core::TransportKind;
use windows::core::{GUID, PCWSTR};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
};
use windows::Win32::Foundation::HWND;

use crate::port::map_win32_error;

/// `GUID_DEVINTERFACE_COMPORT`（`Win32::System::Ioctl` 常量，未开该 feature，按 GUID 值使用）。
const GUID_DEVINTERFACE_COMPORT: GUID = GUID::from_u128(0x86e0d1e0_8089_11d0_9ce4_08003e301f73);

pub fn enumerate_comports() -> Result<Vec<PortCandidate>, AdapterError> {
    let mut out = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_COMPORT),
            PCWSTR::null(),
            HWND::default(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .map_err(|e| map_win32_error(&e))?;

        for index in 0..64u32 {
            let mut ifdata = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(
                set,
                None,
                &GUID_DEVINTERFACE_COMPORT,
                index,
                &mut ifdata,
            )
            .is_err()
            {
                break;
            }

            let mut required = 0u32;
            let _ =
                SetupDiGetDeviceInterfaceDetailW(set, &ifdata, None, 0, Some(&mut required), None);
            let detail_size = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            if (required as usize) < detail_size {
                continue;
            }

            // Vec<u64> 保证 8 字节对齐（Vec<u8> 只保证 1 字节，强转结构体指针属未对齐写）；
            // 长度向上取整到 8 的倍数，cbSize 仍写结构体自身大小。
            let mut buf = vec![0u64; (required as usize).div_ceil(8)];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = detail_size as u32;

            let mut devinfo = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiGetDeviceInterfaceDetailW(
                set,
                &ifdata,
                Some(detail),
                required,
                None,
                Some(&mut devinfo),
            )
            .is_err()
            {
                continue;
            }

            let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, required as usize);
            let path = read_device_path(bytes, required as usize);
            let instance_id = match device_instance_id(set, &devinfo) {
                Ok(id) => id,
                Err(_) => continue,
            };
            if path.is_empty() || instance_id.is_empty() {
                continue;
            }

            let stable_id = instance_id.rsplit('\\').next().map(ToOwned::to_owned);
            out.push(PortCandidate {
                key: instance_id.clone(),
                description: instance_id,
                transport: TransportKind::Cdc,
                stable_id,
                open_path: Some(path),
            });
        }

        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    Ok(out)
}

unsafe fn read_device_path(buf: &[u8], required: usize) -> String {
    // 结构体前 4 字节是 cbSize，DevicePath 从 offset 4 开始（x64 对齐下仍为 4）。
    if required <= 4 {
        return String::new();
    }
    let count = (required - 4) / 2;
    let ptr = buf.as_ptr().add(4) as *const u16;
    let wide = std::slice::from_raw_parts(ptr, count);
    let end = wide.iter().position(|&c| c == 0).unwrap_or(count);
    String::from_utf16_lossy(&wide[..end])
}

unsafe fn device_instance_id(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    devinfo: &SP_DEVINFO_DATA,
) -> Result<String, AdapterError> {
    let mut required = 0u32;
    let _ = SetupDiGetDeviceInstanceIdW(set, devinfo, None, Some(&mut required));
    if required == 0 {
        return Err(AdapterError::Io("PnP 实例 ID 为空".to_string()));
    }
    let mut buf = vec![0u16; required as usize];
    SetupDiGetDeviceInstanceIdW(set, devinfo, Some(&mut buf[..]), None)
        .map_err(|e| map_win32_error(&e))?;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_value_matches_comport_interface() {
        assert_eq!(
            GUID_DEVINTERFACE_COMPORT,
            GUID::from_u128(0x86e0d1e0_8089_11d0_9ce4_08003e301f73)
        );
    }

    #[test]
    fn read_device_path_stops_at_nul() {
        // 前 4 字节是 cbSize，DevicePath 从 offset 4 起、UTF-16 以 0 结尾
        let mut bytes = vec![0u8; 4 + 10 * 2];
        let wide: Vec<u16> = "COM3".encode_utf16().chain(std::iter::once(0)).collect();
        for (i, w) in wide.iter().enumerate() {
            bytes[4 + i * 2..4 + i * 2 + 2].copy_from_slice(&w.to_le_bytes());
        }
        let path = unsafe { read_device_path(&bytes, bytes.len()) };
        assert_eq!(path, "COM3");
    }

    #[test]
    fn read_device_path_handles_short_buffer() {
        assert_eq!(unsafe { read_device_path(&[0u8; 4], 4) }, "");
    }
}
