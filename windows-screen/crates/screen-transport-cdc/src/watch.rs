//! 设备接口到达/移除通知（CM_Register_Notification）。
//!
//! 回调只投递事件，不做打开/关闭；句柄由设备线程独占管理。

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use screen_adapter_api::AdapterError;
use windows::core::GUID;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL,
    CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_0,
    CM_NOTIFY_FILTER_0_2, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CR_SUCCESS, HCMNOTIFICATION,
};

const GUID_DEVINTERFACE_COMPORT: GUID = GUID::from_u128(0x86e0d1e0_8089_11d0_9ce4_08003e301f73);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEvent {
    Arrival,
    Removal,
    Other,
}

struct WatchContext {
    tx: Sender<DeviceEvent>,
}

pub struct DeviceWatcher {
    notify: HCMNOTIFICATION,
    context: Box<WatchContext>,
    rx: Receiver<DeviceEvent>,
}

// 上下文只包含 mpsc::Sender；HCMNOTIFICATION 由本结构独占并在 Drop 注销。
unsafe impl Send for DeviceWatcher {}

impl DeviceWatcher {
    pub fn register() -> Result<Self, AdapterError> {
        let filter = CM_NOTIFY_FILTER {
            cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
            Flags: 0,
            FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE,
            Reserved: 0,
            u: CM_NOTIFY_FILTER_0 {
                DeviceInterface: CM_NOTIFY_FILTER_0_2 {
                    ClassGuid: GUID_DEVINTERFACE_COMPORT,
                },
            },
        };
        let (tx, rx) = mpsc::channel();
        let context = Box::new(WatchContext { tx });
        let mut notify = HCMNOTIFICATION::default();
        let result = unsafe {
            CM_Register_Notification(
                &filter,
                Some(context.as_ref() as *const WatchContext as *const core::ffi::c_void),
                Some(callback),
                &mut notify,
            )
        };
        if result != CR_SUCCESS {
            return Err(AdapterError::Io(format!(
                "CM_Register_Notification 失败：{}",
                result.0
            )));
        }
        Ok(Self {
            notify,
            context,
            rx,
        })
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<DeviceEvent> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    pub fn try_recv(&self) -> Option<DeviceEvent> {
        match self.rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for DeviceWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = CM_Unregister_Notification(self.notify);
        }
        // 注销后再释放 context；这里显式读取一次，保证字段存活到注销返回。
        let _ = &self.context;
    }
}

unsafe extern "system" fn callback(
    _hnotify: HCMNOTIFICATION,
    context: *const core::ffi::c_void,
    action: windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_ACTION,
    _eventdata: *const windows::Win32::Devices::DeviceAndDriverInstallation::CM_NOTIFY_EVENT_DATA,
    _eventdatasize: u32,
) -> u32 {
    if let Some(ctx) = (context as *const WatchContext).as_ref() {
        let event = if action == CM_NOTIFY_ACTION_DEVICEINTERFACEARRIVAL {
            DeviceEvent::Arrival
        } else if action == CM_NOTIFY_ACTION_DEVICEINTERFACEREMOVAL {
            DeviceEvent::Removal
        } else {
            DeviceEvent::Other
        };
        let _ = ctx.tx.send(event);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_registers_and_drops_without_panic() {
        let watcher = DeviceWatcher::register().expect("register watcher");
        let _ = watcher.try_recv();
        drop(watcher);
    }
}
