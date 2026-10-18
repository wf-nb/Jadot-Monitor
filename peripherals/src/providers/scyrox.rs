//! Scyrox V6S/V8S (塞洛斯) battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct ScyroxS;

const SCYROX_VID: u16 = 0x395A;

impl PeripheralBattery for ScyroxS {
    fn name(&self) -> &'static str { "scyrox_s" }
    fn matches(&self, vid: u16, pid: u16, _up: u16, _u: u16) -> bool {
        vid == SCYROX_VID && pid != 0x0006
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x09;
        req[1] = 0x2E;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut reply = vec![0u8; 64];
        reply[0] = 0x09;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 10 { return Err(BatteryError::InvalidReply); }
        let level = reply[5];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        let charging = (reply[6] & 0x40) != 0;
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging,
        })
    }
}
