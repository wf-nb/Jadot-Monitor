//! Peripheral config commands: read/write peripherals.json, list detected devices.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeripheralEntry {
    #[serde(with = "hex_vidpid")]
    pub vid: u16,
    #[serde(with = "hex_vidpid")]
    pub pid: u16,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub label: String,
}

fn default_true() -> bool { true }

mod hex_vidpid {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u16, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{:04x}", v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        let s = String::deserialize(d)?;
        u16::from_str_radix(&s, 16).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeripheralConfig {
    #[serde(default)]
    pub devices: Vec<PeripheralEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectedDevice {
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub product_string: String,
    pub supported: bool,
    pub provider: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(|p| p.join("peripherals.json"))
}

#[tauri::command]
pub fn peripherals_get() -> PeripheralConfig {
    let Some(path) = config_path() else { return PeripheralConfig::default() };
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => PeripheralConfig::default(),
    }
}

#[tauri::command]
pub fn peripherals_set(cfg: PeripheralConfig) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("无法定位配置文件".into());
    };
    let text = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn peripherals_devices() -> Vec<DetectedDevice> {
    // Ensure DLL is loaded
    match crate::peripherals_dll::start() {
        crate::peripherals_dll::LoadResult::NotFound => return Vec::new(),
        crate::peripherals_dll::LoadResult::AbiMismatch => return Vec::new(),
        crate::peripherals_dll::LoadResult::Failed => return Vec::new(),
        crate::peripherals_dll::LoadResult::Ok => {}
    }
    let Some(json) = crate::peripherals_dll::devices_json() else {
        return Vec::new();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) else {
        return Vec::new();
    };
    let arr = match val.as_array() { Some(a) => a, None => return Vec::new() };
    arr.iter().filter_map(|d| {
        Some(DetectedDevice {
            vid: d.get("vid")?.as_u64()? as u16,
            pid: d.get("pid")?.as_u64()? as u16,
            usage_page: d.get("usage_page")?.as_u64()? as u16,
            usage: d.get("usage")?.as_u64()? as u16,
            product_string: d.get("product_string")?.as_str().unwrap_or("").to_string(),
            supported: d.get("supported")?.as_bool().unwrap_or(false),
            provider: d.get("provider")?.as_str().map(|s| s.to_string()),
        })
    }).collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DllStatus {
    /// Whether DLL file exists next to exe.
    pub exists: bool,
    /// Whether DLL is loaded and ready.
    pub loaded: bool,
    /// Load state: "ok" | "not_found" | "abi_mismatch" | "failed".
    pub state: String,
}

#[tauri::command]
pub fn peripherals_status() -> DllStatus {
    let exe = std::env::current_exe().ok();
    let exists = exe.as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join("peripherals.dll").exists())
        .unwrap_or(false);
    let loaded = crate::peripherals_dll::is_loaded();
    let state = if loaded {
        "ok".to_string()
    } else if !exists {
        "not_found".to_string()
    } else {
        "stopped".to_string()
    };
    DllStatus { exists, loaded, state }
}
