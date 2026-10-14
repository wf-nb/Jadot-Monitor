//! CPU 占用率**自算** —— 不再依赖 LHS 的 `CPU Total`。
//!
//! ## 为什么必须自算
//!
//! LHM 的算法是「每个逻辑处理器 `1 − ΔIdleTime / ΔTotalTime`，再对全部逻辑处理器取平均」。
//! **公式没错、取平均也没错，错在「从哪读 idle」**：它用的
//! `SystemProcessorPerformanceInformation.IdleTime` 在 **Windows 11（22H2 起）返回无效值**，
//! 导致占用率**严重偏低**（上游实测：战地 1 场景 LHM 显示 ~2%，任务管理器 ~30%）。
//! 上游早已给出绕行方案（PR #933：改读 `SystemProcessorIdleInformation`），但
//! **LHS 打包的 LHM 版本未包含**，且微软至今未修 → 我们只能自己算。
//!
//! ## 三条取数路径（详细对比见仓库外的《CPU口径与LHS可用性-分析建议.md》）
//!
//! | | 路径 | 口径 | 与任务管理器 |
//! |---|---|---|---|
//! | **A** | PDH `\Processor Information(_Total)\% Processor Utility` | 效用（按标称性能折算频率） | ✅ 一致（Win8+） |
//! | **B** | PDH `\Processor Information(_Total)\% Processor Time` | 时间占比 | ❌（= 任务管理器"详细信息"页） |
//! | **C** | `GetSystemTimes` 差值 | 时间占比（全系统） | ❌ 同上 |
//!
//! **主口径 = A**（与用户在任务管理器里看到的数天然对齐，最不容易产生"你显示的和我看的不一样"的争议）；
//! **B 顺手取来交叉校验**：两者应当**同量级、同向**（实测本机 idle：A 16.6/19.4、B 19.5/14.7、
//! 频率 108/114）。**注意：`A ≈ B × 频率` 这种简单相乘并不成立**（Windows 内部是更复杂的
//! "效用"模型），所以不要把三者当成可以互相验算的恒等式，只能当作"量级是否离谱"的旁证；
//! **C 作为兜底**（PDH 不可用时仍有值）与独立校验 —— 实测 C 与 PDH 的时间口径高度一致
//! （18.9/14.8 vs 19.5/14.7）。
//!
//! ## 为什么放在后台线程按固定节奏采
//!
//! `metrics::basic()` 一秒会被调用**两次**（界面侧 `basic_metrics` + 面板侧 `GET /metrics`，
//! 两者相位差约 0.28s）。若在调用点各自算差值：窗口又短又不规则，两个调用者还会拿到**不同的数**，
//! 面板与界面显示不一致。所以统一由后台线程按 1s 采样、结果放全局，取数只读缓存。
//!
//! ## PDH 的坑（已规避）
//!
//! - **必须用 `PdhAddEnglishCounterW`**：中文 Windows 上计数器名是本地化的，
//!   `PdhAddCounterW` 配英文字符串会因找不到对象而失败。
//! - **首次 `PdhCollectQueryData` 只建基线**，此时取值无意义 → 第一轮丢弃。
//! - 查询句柄**常驻**（只开一次），不要每次取指标重建。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhGetFormattedCounterValue, PdhOpenQueryW,
};
use windows_sys::Win32::System::SystemInformation::{
    CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
};
use windows_sys::Win32::System::Threading::GetSystemTimes;

/// 采样节奏（与界面 / 面板的 1Hz 刷新对齐）。
const SAMPLE_MS: u64 = 1000;
/// 采样结果超过这么久没被更新，就认为**不可用**（别把卡住的旧值当好值）。
const STALE_MS: u128 = 5000;

/// `% Processor Utility` 的 BDH 计数路径（_Total 实例）。
const PATH_UTILITY: &str = r"\Processor Information(_Total)\% Processor Utility";
/// `% Processor Time`（时间口径，用于与效用口径交叉校验）。
const PATH_TIME: &str = r"\Processor Information(_Total)\% Processor Time";
/// `% Processor Performance`（实际频率 / 标称频率 ×100）。
const PATH_PERF: &str = r"\Processor Information(_Total)\% Processor Performance";
/// **逐逻辑处理器**的效用（通配实例）。仅在「大小核兼容模式」开启时才采。
const PATH_UTILITY_ALL: &str = r"\Processor Information(*)\% Processor Utility";

/// 一次采样的全部取值（百分数；`utility` 在睿频时可能 >100）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// A：PDH `% Processor Utility` —— **主口径**
    pub utility: Option<f64>,
    /// B：PDH `% Processor Time` —— 交叉校验
    pub time: Option<f64>,
    /// PDH `% Processor Performance` —— 解释 A 与 B 的差异
    pub perf: Option<f64>,
    /// C：`GetSystemTimes` 差值 —— 兜底
    pub sys_busy: Option<f64>,
    /// 大小核兼容模式：**P 核组**的逻辑处理器效用平均
    pub load_p: Option<f64>,
    /// 大小核兼容模式：**E 核组**的逻辑处理器效用平均
    pub load_e: Option<f64>,
}

impl Sample {
    /// 给指标用的占用率：**效用 → 时间 → 系统差值** 依次回退。
    pub fn load(&self) -> Option<f64> {
        self.utility.or(self.time).or(self.sys_busy)
    }
    /// 当前用的是哪条路径（写进指标的 `source`，便于排查）。
    pub fn source(&self) -> &'static str {
        if self.utility.is_some() {
            "自算 · PDH % Processor Utility（= 任务管理器口径）"
        } else if self.time.is_some() {
            "自算 · PDH % Processor Time（时间口径）"
        } else if self.sys_busy.is_some() {
            "自算 · GetSystemTimes（PDH 不可用）"
        } else {
            "自算 · 采样未就绪"
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 常驻的 PDH 查询（计数器一次开好，之后每秒只 `Collect`）。
struct Pdh {
    query: PDH_HQUERY,
    utility: Option<PDH_HCOUNTER>,
    time: Option<PDH_HCOUNTER>,
    perf: Option<PDH_HCOUNTER>,
    /// 逐逻辑处理器的效用（通配实例）；「大小核兼容模式」才需要
    per_proc: Option<PDH_HCOUNTER>,
}

impl Pdh {
    fn open() -> Option<Pdh> {
        unsafe {
            let mut query: PDH_HQUERY = std::ptr::null_mut();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let add = |path: &str| -> Option<PDH_HCOUNTER> {
                let w = wide(path);
                let mut h: PDH_HCOUNTER = std::ptr::null_mut();
                if PdhAddEnglishCounterW(query, w.as_ptr(), 0, &mut h) != 0 {
                    None
                } else {
                    Some(h)
                }
            };
            let utility = add(PATH_UTILITY);
            let time = add(PATH_TIME);
            let perf = add(PATH_PERF);
            let per_proc = add(PATH_UTILITY_ALL);
            if utility.is_none() && time.is_none() {
                // 主口径与备选口径都拿不到 → 整个 PDH 路走不通，交给 GetSystemTimes 兜底
                PdhCloseQuery(query);
                return None;
            }
            Some(Pdh { query, utility, time, perf, per_proc })
        }
    }

    fn collect(&self) -> bool {
        unsafe { PdhCollectQueryData(self.query) == 0 }
    }

    fn value(h: Option<PDH_HCOUNTER>) -> Option<f64> {
        let h = h?;
        unsafe {
            let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
            let mut ty: u32 = 0;
            if PdhGetFormattedCounterValue(h, PDH_FMT_DOUBLE, &mut ty, &mut v) != 0 {
                return None;
            }
            // CStatus：0 = VALID_DATA、1 = NEW_DATA；其余是无效/无数据
            if v.CStatus > 1 {
                return None;
            }
            Some(v.Anonymous.doubleValue)
        }
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

/// `GetSystemTimes` 的一次快照（100ns 计时单位）。
#[derive(Clone, Copy)]
struct SysTimes {
    idle: u64,
    total: u64,
}

/// 取系统时间快照。**注意 `kernel` 已包含 `idle`**，所以 `total = kernel + user`。
fn sys_times() -> Option<SysTimes> {
    unsafe {
        let mut idle = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let mut kern = idle;
        let mut user = idle;
        if GetSystemTimes(&mut idle, &mut kern, &mut user) == 0 {
            return None;
        }
        let f = |t: &FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        Some(SysTimes { idle: f(&idle), total: f(&kern) + f(&user) })
    }
}

/// 读 Windows 宽字符串（PDH 返回的实例名）。
unsafe fn pwstr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0usize;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, n) })
}

/// **逐逻辑处理器**取 `% Processor Utility`：实例名形如 `0,3`（处理器组, 组内逻辑处理器号）。
fn per_processor_utility(h: PDH_HCOUNTER) -> Vec<(String, f64)> {
    unsafe {
        let mut size: u32 = 0;
        let mut count: u32 = 0;
        // 第一次只问"要多大缓冲"（返回 PDH_MORE_DATA，属预期）
        PdhGetFormattedCounterArrayW(h, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
        if size == 0 || count == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        let st = PdhGetFormattedCounterArrayW(
            h,
            PDH_FMT_DOUBLE,
            &mut size,
            &mut count,
            buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W,
        );
        if st != 0 {
            return Vec::new();
        }
        let items = std::slice::from_raw_parts(
            buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W,
            count as usize,
        );
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            if it.FmtValue.CStatus > 1 {
                continue;
            }
            out.push((pwstr(it.szName), it.FmtValue.Anonymous.doubleValue));
        }
        out
    }
}

/// 实例名是否代表**真实逻辑处理器**：形如 `0,3`。
///
/// 排除两类聚合实例：`_Total`（系统合计）与 `0,_Total`（组内合计）—— 它们也以数字开头，
/// 若混进分组/均值会重复计数。
fn is_logical_processor(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_digit()) && !name.contains("_Total")
}

/// 逻辑处理器 → 是否"快核"（P 核）。键 = `组,组内逻辑处理器号`（与 PDH 实例名一致）。
///
/// 依据 `GetSystemCpuSetInformation` 的 **`EfficiencyClass`**：MSDN 写明「**数值越大 = 越快、
/// 但越不省电**」，实机口径是 Intel 12/13 代 **P 核 = 1、E 核 = 0**。同构核（AMD / 老 Intel）
/// 全部相同 → 返回 `None`（此时"大小核"没有意义，开关开着也不出分组值）。
fn fast_core_map() -> Option<HashMap<String, bool>> {
    unsafe {
        let mut len: u32 = 0;
        GetSystemCpuSetInformation(std::ptr::null_mut(), 0, &mut len, std::ptr::null_mut(), 0);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if GetSystemCpuSetInformation(
            buf.as_mut_ptr() as *mut SYSTEM_CPU_SET_INFORMATION,
            len,
            &mut len,
            std::ptr::null_mut(),
            0,
        ) == 0
        {
            return None;
        }
        // 变长结构：靠每条自己的 `Size` 步进
        let mut classes: Vec<(String, u8)> = Vec::new();
        let mut off = 0usize;
        while off + 8 <= len as usize {
            let p = buf.as_ptr().add(off) as *const SYSTEM_CPU_SET_INFORMATION;
            let size = (*p).Size as usize;
            if size < 8 || off + size > len as usize {
                break;
            }
            if (*p).Type == CpuSetInformation {
                let cs = &(*p).Anonymous.CpuSet;
                classes.push((
                    format!("{},{}", cs.Group, cs.LogicalProcessorIndex),
                    cs.EfficiencyClass,
                ));
            }
            off += size;
        }
        if classes.len() < 2 {
            return None;
        }
        let max = classes.iter().map(|(_, c)| *c).max()?;
        let min = classes.iter().map(|(_, c)| *c).min()?;
        if max == min {
            return None; // 同构核
        }
        Some(classes.into_iter().map(|(k, c)| (k, c == max)).collect())
    }
}

/// 核型拓扑（进程存续期间不变，只算一次）。
fn cores() -> &'static Option<HashMap<String, bool>> {
    static C: OnceLock<Option<HashMap<String, bool>>> = OnceLock::new();
    C.get_or_init(fast_core_map)
}

/// 读一次"逐逻辑处理器"读数并存进 [`PER_PROC`]（供 `--dump-cpu` 诊断）。
fn refresh_per_proc(pdh: Option<&Pdh>) -> Option<Vec<(String, f64)>> {
    let h = pdh.and_then(|p| p.per_proc)?;
    let list = per_processor_utility(h);
    *PER_PROC.lock().unwrap() = Some(list.clone());
    Some(list)
}

/// 「大小核兼容模式」开启时才做分组；返回 `(P 组平均, E 组平均)`。
///
/// 顺带把**逐逻辑处理器的原始读数**存进 [`PER_PROC`]（`--dump-cpu` 诊断用）——
/// 这样即使机器是同构核（分不出 P/E），也能确认"通配读取 + 实例名映射"这条链是通的。
fn grouped(pdh: Option<&Pdh>) -> (Option<f64>, Option<f64>) {
    if !crate::settings::current().hybrid_compat {
        return (None, None);
    }
    let Some(list) = refresh_per_proc(pdh) else {
        return (None, None);
    };
    let Some(cores) = cores().as_ref() else {
        note_uniform_once();
        return (None, None);
    };
    let (mut pf, mut pn, mut ef, mut en) = (0.0f64, 0u32, 0.0f64, 0u32);
    for (name, v) in list {
        match cores.get(&name) {
            Some(true) => {
                pf += v;
                pn += 1;
            }
            Some(false) => {
                ef += v;
                en += 1;
            }
            None => {} // `_Total` 等非逻辑处理器实例
        }
    }
    ((pn > 0).then(|| pf / pn as f64), (en > 0).then(|| ef / en as f64))
}

/// 最近一次「逐逻辑处理器」原始读数（仅开关开启时刷新）——诊断用。
static PER_PROC: Mutex<Option<Vec<(String, f64)>>> = Mutex::new(None);

/// 开关开着但机器是同构核 —— 只在日志里说明一次，别每分钟刷。
fn note_uniform_once() {
    static ONCE: OnceLock<()> = OnceLock::new();
    if ONCE.set(()).is_ok() {
        crate::logging::warn(
            "大小核兼容模式已开启，但本机是同构核（EfficiencyClass 只有一类）→ 不输出 P/E 分组值；\
             同构核机型请把该开关关掉",
        );
    }
}

/// 由两次系统时间快照算忙碌占比。
fn busy(a: Option<SysTimes>, b: Option<SysTimes>) -> Option<f64> {
    let (a, b) = (a?, b?);
    let dt = b.total.saturating_sub(a.total);
    let di = b.idle.saturating_sub(a.idle);
    if dt == 0 {
        return None;
    }
    Some((dt - di) as f64 / dt as f64 * 100.0)
}

/// 最新采样（时间戳 + 取值）。
fn cell() -> &'static Mutex<Option<(Instant, Sample)>> {
    static C: OnceLock<Mutex<Option<(Instant, Sample)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// **同步**采一次并写进全局（CLI / 诊断专用）。
///
/// 为什么不复用后台线程：`--dump-cpu` / `--dump-metrics` 都是**一次性进程**，
/// 而线程是「先 sleep 一个采样周期再产出」。首次加载性能计数器库（perflib）本身可能就要几百毫秒，
/// 很容易把线程的第一次写入推到进程读值之后 → 拿到"采样未就绪"。
/// 这里改成显式「取基线 → 等 `gap` → 再取」，用**真实间隔**算差值，与线程逻辑等价但确定性更强。
pub fn sample_once(gap: Duration) {
    let pdh = Pdh::open();
    let a = sys_times();
    if let Some(p) = &pdh {
        p.collect(); // 建基线
    }
    std::thread::sleep(gap);
    let b = sys_times();
    let (utility, time, perf) = match &pdh {
        Some(p) if p.collect() => (Pdh::value(p.utility), Pdh::value(p.time), Pdh::value(p.perf)),
        _ => (None, None, None),
    };
    let (load_p, load_e) = grouped(pdh.as_ref());
    // 诊断：CLI 一次性采样时**无条件**读一次逐核读数，不受开关影响 ——
    // 这样验证人不用先打开开关，就能确认"通配读取 + 实例名映射"这条链是通的。
    let _ = refresh_per_proc(pdh.as_ref());
    *cell().lock().unwrap() = Some((
        Instant::now(),
        Sample { utility, time, perf, sys_busy: busy(a, b), load_p, load_e },
    ));
}

/// 启动采样线程（**幂等**：重复调用只生效一次）——**GUI 用**（长驻进程）。
///
/// CLI / 一次性诊断请用 [`sample_once`]（本函数的第一次产出要等一个采样周期，
/// 而进程可能在此之前就退出了）。
pub fn spawn() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::spawn(|| {
        let pdh = Pdh::open();
        crate::logging::info(match &pdh {
            Some(_) => "CPU 自算已启动：PDH % Processor Utility（= 任务管理器口径）+ GetSystemTimes 兜底",
            None => "CPU 自算已启动：PDH 不可用，改用 GetSystemTimes 兜底（时间口径）",
        });
        // GetSystemTimes 的基线
        let mut prev = sys_times();
        // PDH 首次 collect 只建基线，值无效 → 第一轮丢弃
        let mut pdh_baseline = true;
        loop {
            std::thread::sleep(Duration::from_millis(SAMPLE_MS));

            // C：系统整体时间占比
            let sys_busy = {
                let now = sys_times();
                let v = busy(prev, now);
                prev = now;
                v
            };

            // A / B / 频率
            let collected = pdh.as_ref().is_some_and(|p| p.collect());
            let (utility, time, perf) = if collected && !pdh_baseline {
                match &pdh {
                    Some(p) => (
                        Pdh::value(p.utility),
                        Pdh::value(p.time),
                        Pdh::value(p.perf),
                    ),
                    None => (None, None, None),
                }
            } else {
                (None, None, None)
            };
            if collected {
                pdh_baseline = false;
            }

            // 大小核分组（仅在「大小核兼容模式」开启且确实是混合核时才有值）
            let (load_p, load_e) = if collected && !pdh_baseline {
                grouped(pdh.as_ref())
            } else {
                (None, None)
            };

            *cell().lock().unwrap() = Some((
                Instant::now(),
                Sample { utility, time, perf, sys_busy, load_p, load_e },
            ));
        }
    });
}

/// 读最新采样；超过 [`STALE_MS`] 没更新则返回 `None`（不拿旧值充数）。
pub fn read() -> Option<Sample> {
    let g = cell().lock().unwrap();
    match g.as_ref() {
        Some((t, s)) if t.elapsed().as_millis() <= STALE_MS => Some(*s),
        _ => None,
    }
}

/// 诊断用：把四条口径 + 大小核分组情况原样吐出来（`--dump-cpu`）。
pub fn dump_json() -> String {
    // 核型拓扑摘要：让验证人一眼看出"分组识别对不对"（同构核只有一类）
    let (classes, counts) = match cores().as_ref() {
        Some(m) => {
            let p = m.values().filter(|v| **v).count();
            (
                "P/E 两类（EfficiencyClass 最大者为 P 核）".to_string(),
                format!("P={p} 逻辑处理器, E={} 逻辑处理器", m.len() - p),
            )
        }
        None => ("同构核（EfficiencyClass 只有一类）".to_string(), "不分组".to_string()),
    };
    // 逐逻辑处理器的"体检"：条数应 = 逻辑处理器数（另加 `_Total` 等非数字名实例）；
    // `perProcessorMean` 应 ≈ `utility`（后者取的是系统 `_Total` 实例）——两者接近即证明这条链是通的。
    let per = PER_PROC.lock().unwrap().clone().unwrap_or_default();
    let (mut sum, mut n) = (0.0f64, 0u32);
    for (name, v) in &per {
        if is_logical_processor(name) {
            sum += v;
            n += 1;
        }
    }
    let per_mean = (n > 0).then(|| sum / n as f64);
    let per_sample: Vec<String> = per.iter().take(8).map(|(k, v)| format!("{k}={v:.1}")).collect();
    // 非"组,序号"形式的实例（一般是 `_Total`）——列出来便于确认没被误算进分组
    let per_other: Vec<String> = per
        .iter()
        .filter(|(k, _)| !is_logical_processor(k))
        .map(|(k, v)| format!("{k}={v:.1}"))
        .collect();

    match read() {
        Some(s) => serde_json::json!({
            "utility": s.utility,
            "time": s.time,
            "perf": s.perf,
            "sysBusy": s.sys_busy,
            "used": s.load(),
            "source": s.source(),
            "hybridCompat": crate::settings::current().hybrid_compat,
            "coreTopology": classes,
            "coreCounts": counts,
            "loadP": s.load_p,
            "loadE": s.load_e,
            "perProcessorCount": per.len(),
            "perProcessorMean": per_mean,
            "perProcessorSample": per_sample,
            "perProcessorOther": per_other,
        })
        .to_string(),
        None => r#"{"error":"尚无采样（CLI 请先调用 sample_once；GUI 请确认采样线程已启动且未超过 5s 未更新）"}"#.to_string(),
    }
}
