//! 设备管理：首期单设备，接口保留多设备扩展。

use screen_adapter_api::DeviceInfo;
use screen_core::{can_transition, DeviceState};

use crate::error::DeviceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedDevice {
    pub info: DeviceInfo,
    pub state: DeviceState,
}

#[derive(Debug, Clone)]
pub struct DeviceManager {
    max_devices: u8,
    devices: Vec<ManagedDevice>,
}

impl DeviceManager {
    pub fn new(max_devices: u8) -> Result<Self, DeviceError> {
        if max_devices == 0 {
            return Err(DeviceError::InvalidConfig("max_devices"));
        }
        Ok(Self {
            max_devices,
            devices: Vec::new(),
        })
    }

    /// 首期固定单设备（A-9）。
    pub fn single_device() -> Self {
        Self::new(1).expect("max_devices=1 恒合法")
    }

    pub fn max_devices(&self) -> u8 {
        self.max_devices
    }

    pub fn devices(&self) -> &[ManagedDevice] {
        &self.devices
    }

    pub fn active_count(&self) -> usize {
        self.devices.len()
    }

    /// 接入一台已识别的设备。重复 key 或超上限都会被拒绝。
    pub fn attach(&mut self, info: DeviceInfo) -> Result<usize, DeviceError> {
        if self.devices.iter().any(|d| d.info.key == info.key) {
            return Err(DeviceError::AlreadyAttached(info.key));
        }
        if self.devices.len() >= self.max_devices as usize {
            return Err(DeviceError::MaxDevices(self.max_devices));
        }
        self.devices.push(ManagedDevice {
            info,
            state: DeviceState::Identified,
        });
        Ok(self.devices.len() - 1)
    }

    pub fn state(&self, index: usize) -> Result<DeviceState, DeviceError> {
        self.devices
            .get(index)
            .map(|d| d.state)
            .ok_or(DeviceError::NoDevice(index))
    }

    pub fn set_state(&mut self, index: usize, next: DeviceState) -> Result<(), DeviceError> {
        let current = self
            .devices
            .get(index)
            .map(|d| d.state)
            .ok_or(DeviceError::NoDevice(index))?;
        if !can_transition(current, next) {
            return Err(DeviceError::InvalidTransition {
                from: current,
                to: next,
            });
        }
        if let Some(device) = self.devices.get_mut(index) {
            device.state = next;
        }
        Ok(())
    }

    /// 移除设备（拔出/关闭），允许后续接管。
    pub fn detach(&mut self, index: usize) -> Result<ManagedDevice, DeviceError> {
        if index >= self.devices.len() {
            return Err(DeviceError::NoDevice(index));
        }
        Ok(self.devices.remove(index))
    }

    pub fn clear(&mut self) {
        self.devices.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screen_core::TransportKind;

    fn info(key: &str) -> DeviceInfo {
        DeviceInfo {
            key: key.to_string(),
            model_id: "turzx-3.5-cdc".to_string(),
            stable_id: Some("USB35INCHIPSV2".to_string()),
            transport: TransportKind::Cdc,
            open_path: None,
        }
    }

    #[test]
    fn single_device_rejects_second_attach() {
        let mut mgr = DeviceManager::single_device();
        mgr.attach(info("a")).expect("first");
        assert_eq!(mgr.attach(info("b")), Err(DeviceError::MaxDevices(1)));
        assert_eq!(mgr.active_count(), 1);
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let mut mgr = DeviceManager::new(2).expect("mgr");
        mgr.attach(info("a")).expect("first");
        assert_eq!(
            mgr.attach(info("a")),
            Err(DeviceError::AlreadyAttached("a".to_string()))
        );
    }

    #[test]
    fn multi_device_capacity_is_reserved() {
        let mut mgr = DeviceManager::new(2).expect("mgr");
        mgr.attach(info("a")).expect("a");
        mgr.attach(info("b")).expect("b");
        assert_eq!(mgr.active_count(), 2);
    }

    #[test]
    fn invalid_transition_is_rejected() {
        let mut mgr = DeviceManager::single_device();
        let idx = mgr.attach(info("a")).expect("attach");
        assert!(mgr.set_state(idx, DeviceState::Ready).is_err());
        mgr.set_state(idx, DeviceState::Opening).expect("opening");
        mgr.set_state(idx, DeviceState::Initializing).expect("init");
        mgr.set_state(idx, DeviceState::Ready).expect("ready");
        assert_eq!(mgr.state(idx), Ok(DeviceState::Ready));
    }

    #[test]
    fn detach_frees_capacity() {
        let mut mgr = DeviceManager::single_device();
        mgr.attach(info("a")).expect("attach");
        mgr.detach(0).expect("detach");
        mgr.attach(info("b")).expect("reattach");
        assert_eq!(mgr.active_count(), 1);
    }

    #[test]
    fn zero_capacity_is_invalid() {
        assert!(DeviceManager::new(0).is_err());
    }
}
