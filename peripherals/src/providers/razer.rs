//! Razer wireless mouse battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct Razer;

const RAZER_VID: u16 = 0x1532;

impl PeripheralBattery for Razer {
    fn name(&self) -> &'static str { "razer" }
    fn matches(&self, vid: u16, _pid: u16, _up: u16, _u: u16) -> bool { vid == RAZER_VID }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 90];
        req[0] = 0x06;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut reply = vec![0u8; 90];
        reply[0] = 0x06;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 10 { return Err(BatteryError::InvalidReply); }
        let level = reply[8];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        let charging = (reply[9] & 0x01) != 0;
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging,
        })
    }
}
