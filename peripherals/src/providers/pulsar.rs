//! Pulsar Bibimbap wireless mouse battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct Pulsar;

const PULSAR_VID: u16 = 0x3554;

impl PeripheralBattery for Pulsar {
    fn name(&self) -> &'static str { "pulsar" }
    fn matches(&self, vid: u16, pid: u16, _up: u16, _u: u16) -> bool {
        vid == PULSAR_VID && (pid == 0x5500 || pid == 0x5501 || pid == 0x6300)
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x06;
        req[1] = 0x55;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut reply = vec![0u8; 64];
        reply[0] = 0x06;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 6 { return Err(BatteryError::InvalidReply); }
        let level = reply[3];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging: false,
        })
    }
}
