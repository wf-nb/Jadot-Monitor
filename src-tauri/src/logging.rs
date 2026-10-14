//! 简易文件日志：`log/jiaidian.log`，单文件上限 100 KB，超过就轮转成 `jiaidian.log.1`。
//!
//! 目的：面板切换卡顿 / 需要点多次这类问题，光看 UI 无从下手，落到文件里才能追溯。
//! 同时打到 stderr（CLI 直接可见）。
//!
//! 目录解析与 `assets/` 一致：先看 cwd，再从 exe 所在目录逐级向上找 `log/`。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// 单个日志文件上限。
const MAX_BYTES: u64 = 100 * 1024;

static FILE: Mutex<Option<File>> = Mutex::new(None);
static PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 日志输出总开关。**默认开**（保持原有行为）；关闭后 [`line`] 直接返回——
/// 不写文件、不打 stderr、不做 flush，也就不再产生任何日志相关的 I/O 与内存开销。
static ENABLED: AtomicBool = AtomicBool::new(true);

/// 当前是否允许输出日志。
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// 运行时开关日志输出（**立即生效**）。
///
/// 持久化不在这里：真值在 `settings.ini` 的 `[app] logging`（由 `settings::apply_patch` 写），
/// 启动时 [`init`] 从那儿读。
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    if !on {
        // 关闭后释放文件句柄（下次开启时会按需重开）。
        *FILE.lock().unwrap() = None;
    }
}

/// `log/` 目录（cwd，或从 exe 目录逐级向上）。
pub fn dir() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = vec![PathBuf::from("log")];
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent().map(|x| x.to_path_buf());
        for _ in 0..4 {
            let Some(cur) = d else { break };
            cands.push(cur.join("log"));
            d = cur.parent().map(|x| x.to_path_buf());
        }
    }
    cands.into_iter().find(|p| p.is_dir())
}

/// 打开日志文件（幂等）。找不到 `log/` 就自己建一个。
pub fn init() {
    // 开关来自 `settings.ini`（`[app] logging`，默认开）。
    if !crate::settings::current().logging {
        ENABLED.store(false, Ordering::Relaxed);
    }
    if !enabled() {
        return; // 关闭状态：不建目录、不建文件
    }
    if FILE.lock().unwrap().is_some() {
        return;
    }
    let dir = dir().or_else(|| {
        let d = PathBuf::from("log");
        fs::create_dir_all(&d).ok().map(|_| d)
    });
    let Some(dir) = dir else { return };
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("jiaidian.log");
    let f = OpenOptions::new().create(true).append(true).open(&path).ok();
    *PATH.lock().unwrap() = Some(path);
    *FILE.lock().unwrap() = f;
}

fn timestamp() -> String {
    let mut st = SYSTEMTIME {
        wYear: 0,
        wMonth: 0,
        wDayOfWeek: 0,
        wDay: 0,
        wHour: 0,
        wMinute: 0,
        wSecond: 0,
        wMilliseconds: 0,
    };
    unsafe { GetLocalTime(&mut st) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

/// 写一行日志。开关关闭时直接返回（不格式化、不写文件、不打 stderr）。
pub fn line(level: &str, msg: &str) {
    if !enabled() {
        return;
    }
    let text = format!("{} [{level}] {msg}\n", timestamp());
    eprint!("{text}");
    {
        let mut g = FILE.lock().unwrap();
        if g.is_none() {
            drop(g);
            init();
            g = FILE.lock().unwrap();
        }
        if let Some(f) = g.as_mut() {
            let _ = f.write_all(text.as_bytes());
            let _ = f.flush();
        }
    }
    rotate_if_needed();
}

fn rotate_if_needed() {
    let Some(path) = PATH.lock().unwrap().clone() else { return };
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size <= MAX_BYTES {
        return;
    }
    *FILE.lock().unwrap() = None;
    let bak = PathBuf::from(format!("{}.1", path.to_string_lossy()));
    let _ = fs::rename(&path, &bak);
    let f = OpenOptions::new().create(true).append(true).open(&path).ok();
    *FILE.lock().unwrap() = f;
}

/// `INFO` 快捷方式。
pub fn info(msg: &str) {
    line("INFO", msg);
}

/// `WARN` 快捷方式。
pub fn warn(msg: &str) {
    line("WARN", msg);
}

/// `ERROR` 快捷方式。
pub fn error(msg: &str) {
    line("ERROR", msg);
}
