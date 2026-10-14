import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";

// 单例：所有调用者共享同一份设置状态（模块级 ref）。
//
// **真值在 exe 同级的 `settings.ini`**（后端唯一持有）：程序启动时读进内存，
// 前端起来后 `settings_get` 拿一份、改任何一项都整体 `settings_set` 写回（原子替换文件）。
// 前端这些 ref 只是投影 —— 之所以不放 localStorage：静默启动 / 日志开关必须在前端
// 跑起来之前就知道，只有文件能在那一刻读到。
// 唯一的例外是「开机自启动」：它的真值在注册表，走 `autostart_get` / `autostart_set`。
const theme = ref<"dark" | "light">("dark");
const devtools = ref(false);
const autostart = ref(false);
/// 静默启动：启动时不显示窗口，直接收进托盘。**下次启动生效**。
const silentStart = ref(false);
const closeToTray = ref(true);
/// 运行日志输出：关掉后不再写 `log/jiaidian.log`，省掉这部分磁盘 I/O。
const logging = ref(true);
/// 高刷模式：开 = 面板 / 预览 ~30fps；关 = 普通 ~10fps（省 CPU）。
const highFps = ref(false);

// ---- 单位设置 ----
/// 显示单位模式：`auto` 全自动（按素材 data-unit）/ `manual` 手动指定（按下面四族）
const unitMode = ref<"auto" | "manual">("auto");
/// 各族目标单位；空串 = 该族跟随素材。**功耗不在这里**（固定 W，没有可切换的余地）。
const unitCapacity = ref(""); // 容量：B KB MB GB TB
const unitRate = ref(""); // 速率：B/s KB/s MB/s GB/s
const unitTemp = ref(""); // 温度：℃ ℉
const unitDuration = ref(""); // 时长：秒 分 小时 天

/// `settings.ini` 里那份设置（后端 `settings_get` 的返回结构）。
interface Settings {
  silentStart: boolean;
  logging: boolean;
  highFps: boolean;
  theme: "dark" | "light";
  closeToTray: boolean;
  devtools: boolean;
  unitMode: "auto" | "manual";
  unitCapacity: string;
  unitRate: string;
  unitTemp: string;
  unitDuration: string;
}

/// 首次读盘完成前不回写，否则会把默认值覆盖掉真实设置。
let loaded = false;

/// 读全部设置（来自 `settings.ini`）。**要 await**：后面的 `boot()` 依赖这里的值。
async function loadSettings() {
  try {
    const s = await invoke<Settings>("settings_get");
    if (s.theme === "light" || s.theme === "dark") theme.value = s.theme;
    if (typeof s.devtools === "boolean") devtools.value = s.devtools;
    if (typeof s.closeToTray === "boolean") closeToTray.value = s.closeToTray;
    if (typeof s.silentStart === "boolean") silentStart.value = s.silentStart;
    if (typeof s.logging === "boolean") logging.value = s.logging;
    if (typeof s.highFps === "boolean") highFps.value = s.highFps;
    if (s.unitMode === "manual" || s.unitMode === "auto") unitMode.value = s.unitMode;
    if (typeof s.unitCapacity === "string") unitCapacity.value = s.unitCapacity;
    if (typeof s.unitRate === "string") unitRate.value = s.unitRate;
    if (typeof s.unitTemp === "string") unitTemp.value = s.unitTemp;
    if (typeof s.unitDuration === "string") unitDuration.value = s.unitDuration;
  } catch {
    /* 读不到就用默认值 */
  }
  loaded = true;
}

/// 整体写回 `settings.ini`。后端同时把副作用**立即应用**（日志开关 / 帧率档位 / 单位换算）。
function persist() {
  if (!loaded) return;
  invoke("settings_set", {
    patch: {
      theme: theme.value,
      devtools: devtools.value,
      closeToTray: closeToTray.value,
      silentStart: silentStart.value,
      logging: logging.value,
      highFps: highFps.value,
      unitMode: unitMode.value,
      unitCapacity: unitCapacity.value,
      unitRate: unitRate.value,
      unitTemp: unitTemp.value,
      unitDuration: unitDuration.value,
    },
  }).catch(() => {});
}

// 主题 / DevTools / 收托盘：改动即写回；主题还要同步原生窗口明暗（原生标题栏跟随深浅）
watch([theme, devtools, closeToTray], () => {
  persist();
  invoke("set_window_theme", { dark: theme.value === "dark" }).catch(() => {});
});

// 其余设置：改动即写回（单位改动面板注入脚本每秒拉 `GET /units` → 立刻生效）
watch(
  [silentStart, logging, highFps, unitMode, unitCapacity, unitRate, unitTemp, unitDuration],
  () => persist(),
);

async function loadAutostart() {
  try {
    autostart.value = await invoke<boolean>("autostart_get");
  } catch {
    /* 忽略 */
  }
}

async function toggleAutostart() {
  try {
    await invoke<string>("autostart_set", { value: autostart.value });
  } catch {
    autostart.value = !autostart.value;
  }
}

async function openDevtools() {
  if (!devtools.value) return;
  try {
    await invoke("open_devtools");
  } catch {
    /* 忽略 */
  }
}

export function useSettings() {
  return {
    theme,
    devtools,
    autostart,
    silentStart,
    closeToTray,
    logging,
    highFps,
    unitMode,
    unitCapacity,
    unitRate,
    unitTemp,
    unitDuration,
    loadSettings,
    loadAutostart,
    toggleAutostart,
    openDevtools,
  };
}
