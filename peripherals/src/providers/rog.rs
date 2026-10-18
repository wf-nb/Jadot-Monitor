//! ROG Standard / Angry Miao (Gear Link) battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct RogStandard;

const ROG_VIDS: &[u16] = &[0x0B05];

impl PeripheralBattery for RogStandard {
    fn name(&self) -> &'static str { "rog_standard" }
    fn matches(&self, vid: u16, _pid: u16, _up: u16, _u: u16) -> bool { ROG_VIDS.contains(&vid) }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x12;
        req[1] = 0x07;
        let _ = dev.write(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut buf = vec![0u8; 64];
        let n = dev.read_timeout(&mut buf, 200)?;
        if n < 8 { return Err(BatteryError::InvalidReply); }
        let level = buf[5];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        let charging = (buf[6] & 0x01) != 0;
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging,
        })
    }
}
