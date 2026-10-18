//! SteelSeries Aerox battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct SteelSeries;

const STEEL_VID: u16 = 0x1038;

impl PeripheralBattery for SteelSeries {
    fn name(&self) -> &'static str { "steelseries" }
    fn matches(&self, vid: u16, _pid: u16, _up: u16, _u: u16) -> bool { vid == STEEL_VID }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x00;
        req[1] = 0xD2;
        req[2] = 0x01;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut reply = vec![0u8; 64];
        reply[0] = 0x00;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 6 { return Err(BatteryError::InvalidReply); }
        let level = reply[3];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        let charging = (reply[4] & 0x01) != 0;
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging,
        })
    }
}
