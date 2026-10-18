//! peripherals.dll dynamic loading and polling.
//!
//! Loads `peripherals.dll` from the same directory as the exe, calls init,
//! and runs a background thread that polls battery readings every 2 seconds.

use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type FnAbiVersion = unsafe extern "C" fn() -> c_int;
type FnInit = unsafe extern "C" fn() -> c_int;
type FnPoll = unsafe extern "C" fn() -> *mut c_char;
type FnDevices = unsafe extern "C" fn() -> *mut c_char;
type FnFree = unsafe extern "C" fn(*mut c_char);
type FnShutdown = unsafe extern "C" fn();

const EXPECTED_ABI: c_int = 1;

struct DllApi {
    init: FnInit,
    poll: FnPoll,
    devices: FnDevices,
    free: FnFree,
    shutdown: FnShutdown,
}

static LOADED: AtomicBool = AtomicBool::new(false);
static API: OnceLock<DllApi> = OnceLock::new();
static CACHE: Mutex<Option<String>> = Mutex::new(None);
static THREAD_HANDLE: OnceLock<std::thread::JoinHandle<()>> = OnceLock::new();

#[repr(C)]
struct HMODULE(isize);

/// Result of loading the DLL.
#[derive(Debug, Clone)]
pub enum LoadResult {
    Ok,
    NotFound,
    AbiMismatch,
    Failed,
}

/// Load peripherals.dll and start polling thread.
pub fn start() -> LoadResult {
    if LOADED.load(Ordering::Relaxed) {
        return LoadResult::Ok;
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return LoadResult::Failed,
    };
    let dir = match exe.parent() {
        Some(d) => d,
        None => return LoadResult::Failed,
    };
    let dll_path = dir.join("peripherals.dll");
    if !dll_path.exists() {
        return LoadResult::NotFound;
    }

    let path_w: Vec<u16> = dll_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe extern "system" {
        fn LoadLibraryExW(lp_path: *const u16, h_file: isize, flags: u32) -> isize;
        fn GetProcAddress(h: isize, name: *const u8) -> isize;
        fn FreeLibrary(h: isize) -> i32;
    }

    // LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_APPLICATION_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32
    const FLAGS: u32 = 0x00000100 | 0x00000200 | 0x00000800;
    let h = unsafe { LoadLibraryExW(path_w.as_ptr(), 0, FLAGS) };
    if h == 0 {
        return LoadResult::Failed;
    }

    macro_rules! get_fn {
        ($name:expr) => {{
            let name_c = CString::new($name).unwrap();
            let addr = unsafe { GetProcAddress(h, name_c.as_ptr() as *const u8) };
            if addr == 0 {
                unsafe { FreeLibrary(h) };
                return LoadResult::Failed;
            }
            addr
        }};
    }

    // ABI version handshake
    let abi_fn: FnAbiVersion = unsafe { std::mem::transmute::<isize, FnAbiVersion>(get_fn!("peripherals_abi_version")) };
    let abi = unsafe { abi_fn() };
    if abi != EXPECTED_ABI {
        unsafe { FreeLibrary(h) };
        return LoadResult::AbiMismatch;
    }

    let api = DllApi {
        init: unsafe { std::mem::transmute::<isize, FnInit>(get_fn!("peripherals_init")) },
        poll: unsafe { std::mem::transmute::<isize, FnPoll>(get_fn!("peripherals_poll")) },
        devices: unsafe { std::mem::transmute::<isize, FnDevices>(get_fn!("peripherals_devices")) },
        free: unsafe { std::mem::transmute::<isize, FnFree>(get_fn!("peripherals_free_string")) },
        shutdown: unsafe { std::mem::transmute::<isize, FnShutdown>(get_fn!("peripherals_shutdown")) },
    };

    if unsafe { (api.init)() } != 0 {
        unsafe { FreeLibrary(h) };
        return LoadResult::Failed;
    }

    // Spawn polling thread
    let poll_fn = api.poll;
    let free_fn = api.free;
    let handle = std::thread::spawn(move || {
        loop {
            let ptr = unsafe { (poll_fn)() };
            if !ptr.is_null() {
                let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string();
                *CACHE.lock().unwrap() = Some(s);
                unsafe { (free_fn)(ptr) };
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    let _ = THREAD_HANDLE.set(handle);

    let _ = API.set(api);
    LOADED.store(true, Ordering::Relaxed);
    LoadResult::Ok
}

/// Get the latest cached battery readings as JSON string.
pub fn latest_json() -> Option<String> {
    CACHE.lock().unwrap().clone()
}

/// Get enumerated devices as JSON string.
pub fn devices_json() -> Option<String> {
    let api = API.get()?;
    let ptr = unsafe { (api.devices)() };
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy().to_string();
    unsafe { (api.free)(ptr) };
    Some(s)
}

/// Check if the DLL is actually loaded and ready.
pub fn is_loaded() -> bool {
    LOADED.load(Ordering::Relaxed) && API.get().is_some()
}

/// Shutdown and unload the DLL.
pub fn shutdown() {
    if !LOADED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(api) = API.get() {
        unsafe { (api.shutdown)() };
    }
    LOADED.store(false, Ordering::Relaxed);
}
