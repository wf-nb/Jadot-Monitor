//! Peripherals DLL: wireless mouse battery monitoring component.


use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use std::ffi::{CStr, CString};

use hidapi::HidApi;
use serde::Serialize;
use thiserror::Error;

pub mod config;
pub mod providers;

use config::PeripheralConfig;

/// Result of one battery read.
#[derive(Debug, Clone, Serialize)]
pub struct BatteryReading {
    /// Provider name that produced this reading (e.g. `"comps"`).
    pub provider: &'static str,
    /// Human-readable device label (e.g. `"Akko 8K Receiver"`).
    pub device_label: String,
    /// Stable device key (VID:PID:usagePage:usage:index).
    pub device_key: String,
    /// Battery percentage 0–100.
    pub level_pct: u8,
    /// True when the peripheral is on the charger.
    pub charging: bool,
}

#[derive(Debug, Error)]
pub enum BatteryError {
    #[error("HID error: {0}")]
    Hid(#[from] hidapi::HidError),
    #[error("no matching provider for device")]
    NoProvider,
    #[error("invalid reply from device")]
    InvalidReply,
    #[error("device did not reply in time")]
    Timeout,
}

/// A vendor-specific battery protocol.
pub trait PeripheralBattery: Send + Sync {
    fn name(&self) -> &'static str;
    fn matches(&self, vid: u16, pid: u16, usage_page: u16, usage: u16) -> bool;
    fn read(&self, api: &HidApi, path: &CStr, device_label: &str) -> Result<BatteryReading, BatteryError>;
}

/// Build the default set of providers.
pub fn builtin_providers() -> Vec<Box<dyn PeripheralBattery>> {
    vec![
        Box::new(providers::Compx),
        Box::new(providers::Ninjutso),
        Box::new(providers::Rapoo),
        Box::new(providers::WlMouseCube),
        Box::new(providers::Zynoo),
        Box::new(providers::VgnLd),
        Box::new(providers::ScyroxS),
        Box::new(providers::SoraV3),
        Box::new(providers::RogStandard),
        Box::new(providers::SteelSeries),
        Box::new(providers::GWolves),
        Box::new(providers::Pulsar),
        Box::new(providers::WebBrands),
        Box::new(providers::Ajazz),
        Box::new(providers::Logitech),
        Box::new(providers::Razer),
        Box::new(providers::Corsair),
        Box::new(providers::HyperX),
    ]
}

/// One entry in the multi-device cache.
struct CacheEntry {
    reading: BatteryReading,
    last_ok: Instant,
}

/// Enumerated HID device (for /devices endpoint and UI selection).
#[derive(Debug, Clone, Serialize)]
pub struct HidDeviceInfo {
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub product_string: String,
    pub path: String,
    /// Whether any provider claims this device.
    pub supported: bool,
    /// Which provider claims it.
    pub provider: Option<&'static str>,
}

/// Background poller that keeps the latest battery readings for ALL matching
/// peripherals fresh.
pub struct Monitor {
    providers: Vec<Box<dyn PeripheralBattery>>,
    cache: RwLock<HashMap<String, CacheEntry>>,
    config: PeripheralConfig,
    /// Cached set of device paths; only re-enumerate when this changes.
    known_paths: RwLock<std::collections::HashSet<String>>,
}

impl Monitor {
    pub fn new() -> Arc<Self> {
        Self::with_providers(builtin_providers())
    }

    pub fn with_providers(providers: Vec<Box<dyn PeripheralBattery>>) -> Arc<Self> {
        let config = PeripheralConfig::load();
        Arc::new(Self {
            providers,
            cache: RwLock::new(HashMap::new()),
            config,
            known_paths: RwLock::new(std::collections::HashSet::new()),
        })
    }

    /// Snapshot all cached readings, ordered by config (never blocks on HID IO).
    pub fn snapshot_all(&self) -> Vec<BatteryReading> {
        let map = match self.cache.read() {
            Ok(m) => m,
            Err(_) => return Vec::new(),
        };
        // Build ordered list from config, fall back to cache order.
        let ordered = self.config.ordered_enabled();
        let mut out = Vec::new();
        for (vid, pid) in &ordered {
            let key = format!("{:04x}:{:04x}", vid, pid);
            if let Some(entry) = map.get(&key) {
                out.push(entry.reading.clone());
            }
        }
        // If config is empty (first run), return all cached.
        if out.is_empty() {
            out = map.values().map(|e| e.reading.clone()).collect();
        }
        out
    }

    /// Enumerate all HID devices on the bus (for /devices endpoint).
    pub fn enumerate_devices(&self) -> Vec<HidDeviceInfo> {
        let api = match HidApi::new() {
            Ok(a) => a,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for info in api.device_list() {
            let vid = info.vendor_id();
            let pid = info.product_id();
            let up = info.usage_page();
            let u = info.usage();
            // Deduplicate by VID:PID (one entry per device, not per interface)
            let key = (vid, pid);
            if !seen.insert(key) {
                continue;
            }
            let label = info.product_string().unwrap_or("Unknown").to_string();
            let path = info.path().to_string_lossy().to_string();
            let mut supported = false;
            let mut provider_name: Option<&'static str> = None;
            for p in &self.providers {
                if p.matches(vid, pid, up, u) {
                    supported = true;
                    provider_name = Some(p.name());
                    break;
                }
            }
            out.push(HidDeviceInfo {
                vid, pid, usage_page: up, usage: u,
                product_string: label, path,
                supported, provider: provider_name,
            });
        }
        out
    }

    fn match_provider(&self, vid: u16, pid: u16, usage_page: u16, usage: u16) -> Option<&dyn PeripheralBattery> {
        self.providers.iter()
            .find(|p| p.matches(vid, pid, usage_page, usage))
            .map(|p| p.as_ref())
    }

    /// One polling cycle: enumerate matching devices, filter by config, read each.
    /// Only re-enumerates when the set of device paths changes (HaloBattery strategy).
    pub fn poll_once(&self) {
        let api = match HidApi::new() {
            Ok(a) => a,
            Err(_) => return,
        };

        // Collect matching devices.
        let mut matches: Vec<(u16, u16, CString, String, &dyn PeripheralBattery)> = Vec::new();
        for info in api.device_list() {
            let vid = info.vendor_id();
            let pid = info.product_id();
            let up = info.usage_page();
            let u = info.usage();
            let label = info.product_string().unwrap_or("Unknown HID device").to_string();
            let path = info.path().to_owned();
            if let Some(p) = self.match_provider(vid, pid, up, u) {
                if !self.config.is_enabled(vid, pid) {
                    continue;
                }
                matches.push((vid, pid, path, label, p));
            }
        }

        // Auto-generate config on first run.
        if self.config.devices.is_empty() {
            let detected: Vec<(u16, u16, &str)> = matches.iter()
                .map(|(v, p, _, l, _)| (*v, *p, l.as_str()))
                .collect();
            if !detected.is_empty() {
                let _ = PeripheralConfig::auto_generate(&detected);
            }
        }

        // Read each matching device independently.
        let mut new_cache: HashMap<String, CacheEntry> = HashMap::new();
        for (vid, pid, path, label, provider) in &matches {
            let key = format!("{:04x}:{:04x}", vid, pid);
            let display_label = self.config.label_for(*vid, *pid).unwrap_or_else(|| label.clone());
            match provider.read(&api, path, &display_label) {
                Ok(mut reading) => {
                    reading.device_key = key.clone();
                    new_cache.insert(key, CacheEntry {
                        reading,
                        last_ok: Instant::now(),
                    });
                }
                Err(_) => {
                    if !new_cache.contains_key(&key) {
                        if let Ok(old) = self.cache.read() {
                            if let Some(old_entry) = old.get(&key) {
                                new_cache.insert(key, old_entry.clone());
                            }
                        }
                    }
                }
            }
        }

        if let Ok(mut c) = self.cache.write() {
            *c = new_cache;
        }
    }

    pub fn spawn(self: Arc<Self>, interval: Duration) -> MonitorHandle {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let handle = MonitorHandle { stop: stop.clone() };
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                self.poll_once();
                let step = Duration::from_millis(100);
                let mut waited = Duration::ZERO;
                while waited < interval && !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(step);
                    waited += step;
                }
            }
        });
        handle
    }
}

impl Clone for CacheEntry {
    fn clone(&self) -> Self {
        Self {
            reading: self.reading.clone(),
            last_ok: self.last_ok,
        }
    }
}

pub struct MonitorHandle {
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl MonitorHandle {
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// C ABI exports for peripherals.dll
// ---------------------------------------------------------------------------

use std::os::raw::c_char;
use std::sync::OnceLock;

static MONITOR: OnceLock<(Arc<Monitor>, MonitorHandle)> = OnceLock::new();

/// ABI version. Host checks this before loading.
pub const ABI_VERSION: i32 = 1;

/// Return ABI version.
#[no_mangle]
pub extern "C" fn peripherals_abi_version() -> i32 {
    ABI_VERSION
}

/// Initialize the peripherals DLL. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn peripherals_init() -> i32 {
    match std::panic::catch_unwind(|| {
        if MONITOR.get().is_some() {
            return 0;
        }
        let monitor = Monitor::new();
        let handle = monitor.clone().spawn(Duration::from_secs(2));
        if MONITOR.set((monitor, handle)).is_err() {
            return -1;
        }
        0
    }) {
        Ok(v) => v,
        Err(_) => -1,
    }
}

/// Poll battery readings and return JSON string.
/// Caller must free the returned pointer with peripherals_free_string().
#[no_mangle]
pub extern "C" fn peripherals_poll() -> *mut c_char {
    match std::panic::catch_unwind(|| {
        let Some((m, _)) = MONITOR.get() else {
            return std::ptr::null_mut();
        };
        let readings = m.snapshot_all();
        let json = serde_json::to_string(&readings).unwrap_or_else(|_| "[]".into());
        match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }) {
        Ok(p) => p,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Enumerate all HID devices and return JSON string.
#[no_mangle]
pub extern "C" fn peripherals_devices() -> *mut c_char {
    match std::panic::catch_unwind(|| {
        let Some((m, _)) = MONITOR.get() else {
            return std::ptr::null_mut();
        };
        let devices = m.enumerate_devices();
        let json = serde_json::to_string(&devices).unwrap_or_else(|_| "[]".into());
        match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }) {
        Ok(p) => p,
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a string returned by peripherals_poll() or peripherals_devices().
#[no_mangle]
pub extern "C" fn peripherals_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)); }
    }
}

/// Shutdown the DLL.
#[no_mangle]
pub extern "C" fn peripherals_shutdown() {
    let _ = std::panic::catch_unwind(|| {
        if let Some((_, handle)) = MONITOR.get() {
            handle.stop();
        }
    });
}

/// Return diagnostics: enumerated devices, provider matches, last readings.
#[no_mangle]
pub extern "C" fn peripherals_diagnostics() -> *mut c_char {
    match std::panic::catch_unwind(|| {
        let Some((m, _)) = MONITOR.get() else {
            return std::ptr::null_mut();
        };
        let devices = m.enumerate_devices();
        let readings = m.snapshot_all();
        let diag = serde_json::json!({
            "devices": devices,
            "readings": readings,
        });
        let json = serde_json::to_string(&diag).unwrap_or_else(|_| "{}".into());
        match CString::new(json) {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    }) {
        Ok(p) => p,
        Err(_) => std::ptr::null_mut(),
    }
}
