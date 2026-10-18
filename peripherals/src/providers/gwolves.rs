//! G-Wolves wireless mouse battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct GWolves;

const GWOLVES_VID: u16 = 0x3554;

impl PeripheralBattery for GWolves {
    fn name(&self) -> &'static str { "gwolves" }
    fn matches(&self, vid: u16, pid: u16, _up: u16, _u: u16) -> bool {
        vid == GWOLVES_VID && (pid == 0x8A1A || pid == 0x8A1B || pid == 0x8A1C)
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x83;
        req[1] = 0xA1;
        req[2] = 0x02;
        req[3] = 0x8F;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut reply = vec![0u8; 64];
        reply[0] = 0x83;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 6 { return Err(BatteryError::InvalidReply); }
        let level = reply[4];
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
