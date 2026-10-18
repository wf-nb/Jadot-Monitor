//! Rapoo (雷柏) wireless mouse battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct Rapoo;

const RAPOO_VID: u16 = 0x24AE;
const RAPOO_PIDS: &[u16] = &[0x1453, 0x1412, 0x14A2, 0x14A7, 0x14A5];

impl PeripheralBattery for Rapoo {
    fn name(&self) -> &'static str { "rapoo" }
    fn matches(&self, vid: u16, pid: u16, usage_page: u16, _usage: u16) -> bool {
        vid == RAPOO_VID && RAPOO_PIDS.contains(&pid) && usage_page == 0xFF00
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let mut req = vec![0u8; 7];
        req[0] = 0x07;
        req[1] = 0x20;
        let _ = dev.write(&req)?;
        std::thread::sleep(std::time::Duration::from_millis(80));
        let mut buf = vec![0u8; 64];
        let n = dev.read_timeout(&mut buf, 200)?;
        if n < 7 { return Err(BatteryError::InvalidReply); }
        let level = buf[6];
        if level > 100 { return Err(BatteryError::InvalidReply); }
        let charging = (buf[5] & 0x01) != 0;
        Ok(BatteryReading {
            provider: self.name(),
            device_label: device_label.to_string(),
            device_key: String::new(),
            level_pct: level,
            charging,
        })
    }
}
