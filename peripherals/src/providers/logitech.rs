//! Logitech Unifying receiver battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct Logitech;

const LOGI_VID: u16 = 0x046D;
const LOGI_PIDS: &[u16] = &[0xC52B, 0xC531, 0xC534, 0xC539, 0xC53F, 0xC548, 0xC54B, 0xC54D];

impl PeripheralBattery for Logitech {
    fn name(&self) -> &'static str { "logitech" }
    fn matches(&self, vid: u16, pid: u16, _up: u16, _u: u16) -> bool {
        vid == LOGI_VID && LOGI_PIDS.contains(&pid)
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 7];
        req[0] = 0x10;
        req[1] = 0x07;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut reply = vec![0u8; 7];
        reply[0] = 0x10;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 5 { return Err(BatteryError::InvalidReply); }
        let level = reply[4];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        let charging = (reply[5] & 0x01) != 0;
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging,
        })
    }
}
