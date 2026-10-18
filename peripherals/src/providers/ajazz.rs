//! Ajazz (黑爵 AJ159 series) battery.

use std::ffi::CStr;
use hidapi::HidApi;
use crate::{BatteryError, BatteryReading, PeripheralBattery};
use super::drain;

pub struct Ajazz;

impl PeripheralBattery for Ajazz {
    fn name(&self) -> &'static str { "ajazz" }
    fn matches(&self, vid: u16, pid: u16, usage_page: u16, _u: u16) -> bool {
        if usage_page != 0x01 { return false; }
        matches!((vid, pid),
            (0x3151, 0x4027) | (0x248A, 0x5C2F) | (0x249A, 0x5C2F) | (0x248A, 0x5B2F))
    }
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError> {
        let dev = api.open_path(path)?;
        drain(&dev);
        let (vid, pid) = device_vid_pid(path);
        match (vid, pid) {
            (0x3151, 0x4027) => {
                let mut req = vec![0u8; 64];
                req[0] = 0xF7;
                dev.send_feature_report(&req)?;
                std::thread::sleep(std::time::Duration::from_millis(50));
                let mut reply = vec![0u8; 64];
                reply[0] = 0xF7;
                let n = dev.get_feature_report(&mut reply)?;
                if n < 6 { return Err(BatteryError::InvalidReply); }
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
            (0x248A, 0x5C2F) | (0x249A, 0x5C2F) => {
                let req = [0x00u8, 0x10u8];
                dev.write(&req)?;
                std::thread::sleep(std::time::Duration::from_millis(50));
                let mut reply = [0u8; 16];
                let n = dev.read_timeout(&mut reply, 200)?;
                if n < 14 { return Err(BatteryError::InvalidReply); }
                let level = reply[13];
                if level > 100 { return Err(BatteryError::InvalidReply); }
                Ok(BatteryReading {
                    provider: self.name(),
                    device_label: device_label.to_string(),
                    device_key: String::new(),
                    level_pct: level,
                    charging: false,
                })
            }
            (0x248A, 0x5B2F) => {
                let mut req = vec![0u8; 33];
                req[0] = 12;
                req[1] = 0x04;
                req[9] = 0x12;
                req[13..17].copy_from_slice(b"M620");
                dev.send_feature_report(&req)?;
                std::thread::sleep(std::time::Duration::from_millis(50));
                let mut reply = vec![0u8; 33];
                reply[0] = 12;
                let n = dev.get_feature_report(&mut reply)?;
                if n < 20 { return Err(BatteryError::InvalidReply); }
                if &reply[13..17] != b"M620" { return Err(BatteryError::InvalidReply); }
                let level = reply[18];
                if level > 100 { return Err(BatteryError::InvalidReply); }
                let charging = (reply[19] & 0x01) != 0;
                Ok(BatteryReading {
                    provider: self.name(),
                    device_label: device_label.to_string(),
                    device_key: String::new(),
                    level_pct: level,
                    charging,
                })
            }
            _ => Err(BatteryError::NoProvider),
        }
    }
}

fn device_vid_pid(path: &CStr) -> (u16, u16) {
    if let Ok(api) = hidapi::HidApi::new() {
        if let Some(info) = api.device_list().find(|d| d.path() == path) {
            return (info.vendor_id(), info.product_id());
        }
    }
    (0, 0)
}
