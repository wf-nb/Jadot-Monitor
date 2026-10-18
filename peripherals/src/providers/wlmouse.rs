//! WL Mouse Cube (万灵花兽魔方版) battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct WlMouseCube;

const WLM_VID: u16 = 0x3554;
const WLM_CUBE_PIDS: &[u16] = &[0xA887, 0xA888];

impl PeripheralBattery for WlMouseCube {
    fn name(&self) -> &'static str { "wlmouse_cube" }
    fn matches(&self, vid: u16, pid: u16, _up: u16, _u: u16) -> bool {
        vid == WLM_VID && WLM_CUBE_PIDS.contains(&pid)
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 64];
        req[0] = 0x06;
        req[1] = 0x1A;
        dev.send_feature_report(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut reply = vec![0u8; 64];
        reply[0] = 0x06;
        let n = dev.get_feature_report(&mut reply)?;
        if n < 8 { return Err(BatteryError::InvalidReply); }
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
