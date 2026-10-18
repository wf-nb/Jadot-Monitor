//! WebBrands Feature83 (AttackShark / IPI / LunaFury / Lamzu / KO-ONE) battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct WebBrands;

impl PeripheralBattery for WebBrands {
    fn name(&self) -> &'static str { "webbrands" }
    fn matches(&self, vid: u16, _pid: u16, _up: u16, _u: u16) -> bool {
        const VIDS: &[u16] = &[0x1014, 0x36A7, 0x372E, 0x373E, 0x38EE];
        VIDS.contains(&vid)
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x83;
        req[1] = 0xA1;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut reply = vec![0u8; 64];
        reply[0] = 0x83;
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
