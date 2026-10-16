// Offline regression tests: no application launch, sockets, USB or service access.
// Run with the managed Node runtime; Vue/TypeScript are loaded from existing app dependencies.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const root = path.resolve(__dirname, "..");
const ts = require(path.join(root, "node_modules/typescript"));
const vue = require(path.join(root, "node_modules/vue"));
const defaults = {
  silentStart: false, logging: true, highFps: false, hybridCompat: false,
  theme: "dark", closeToTray: true, devtools: false, unitMode: "auto",
  unitCapacity: "", unitRate: "", unitTemp: "", unitDuration: "",
};
const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
function deferred() {
  let resolve, reject;
  const promise = new Promise((a, b) => { resolve = a; reject = b; });
  return { promise, resolve, reject };
}
function settings(invoke) {
  const input = fs.readFileSync(path.join(root, "src/composables/useSettings.ts"), "utf8");
  const code = ts.transpileModule(input, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, {
    exports,
    require: (id) => id === "vue" ? vue : id === "@tauri-apps/api/core" ? { invoke } : require(id),
  }, { filename: "useSettings.offline.cjs" });
  return exports.useSettings();
}
async function main() {
  const results = [];
  async function test(name, fn) { await fn(); results.push(name); console.log("PASS " + name); }
  await test("initialization does not persist or sample", async () => {
    const saves = [];
    const s = settings(async (name, arg) => {
      if (name === "settings_get") return { ...defaults, hybridCompat: true, theme: "light" };
      if (name === "settings_set") saves.push(arg);
    });
    await s.loadSettings(); await settle();
    assert.equal(s.hybridCompat.value, true);
    assert.equal(s.settingsReady.value, true);
    assert.equal(saves.length, 0);
  });
  await test("failed save rolls mode back and exposes error", async () => {
    const s = settings(async (name) => {
      if (name === "settings_get") return { ...defaults };
      if (name === "settings_set") throw new Error("read-only fixture");
    });
    await s.loadSettings();
    s.hybridCompat.value = true;
    await settle();
    assert.equal(s.hybridCompat.value, false);
    assert.match(s.settingsError.value, /read-only fixture/);
    assert.equal(s.settingsSaving.value, false);
  });
  await test("failed value is removed from later queued full snapshots", async () => {
    const first = deferred(); const saves = [];
    const s = settings((name, arg) => {
      if (name === "settings_get") return Promise.resolve({ ...defaults });
      if (name === "settings_set") {
        saves.push({ ...arg.patch });
        return saves.length === 1 ? first.promise : Promise.resolve(arg.patch);
      }
      return Promise.resolve();
    });
    await s.loadSettings();
    s.hybridCompat.value = true;
    s.theme.value = "light";
    assert.equal(saves.length, 1);
    first.reject(new Error("first save failed"));
    await settle();
    assert.equal(saves.length, 2);
    assert.equal(saves[1].hybridCompat, false);
    assert.equal(saves[1].theme, "light");
    assert.equal(s.hybridCompat.value, false);
    assert.equal(s.theme.value, "light");
  });
  await test("rapid same-field changes preserve latest user revision", async () => {
    const first = deferred(); const saves = [];
    const s = settings((name, arg) => {
      if (name === "settings_get") return Promise.resolve({ ...defaults });
      if (name === "settings_set") {
        saves.push({ ...arg.patch });
        return saves.length === 1 ? first.promise : Promise.resolve(arg.patch);
      }
      return Promise.resolve();
    });
    await s.loadSettings();
    s.hybridCompat.value = true;
    s.hybridCompat.value = false;
    s.hybridCompat.value = true;
    first.reject(new Error("first failed"));
    await settle();
    assert.equal(saves.length, 3);
    assert.equal(saves[1].hybridCompat, false);
    assert.equal(saves[2].hybridCompat, true);
    assert.equal(s.hybridCompat.value, true);
  });
  await test("load failure blocks writes until successful retry", async () => {
    let reads = 0; let writes = 0;
    const s = settings(async (name, arg) => {
      if (name === "settings_get") {
        if (++reads === 1) throw new Error("IPC fixture");
        return { ...defaults, hybridCompat: true };
      }
      if (name === "settings_set") { writes++; return arg.patch; }
    });
    await s.loadSettings(); s.theme.value = "light"; await settle();
    assert.equal(s.settingsReady.value, false); assert.equal(writes, 0);
    await s.loadSettings(); await settle();
    assert.equal(s.settingsReady.value, true); assert.equal(s.hybridCompat.value, true);
    assert.equal(writes, 0);
  });
  const source = fs.readFileSync(path.join(root, "src-tauri/src/pipeline.rs"), "utf8");
  const body = source.match(/const JS: &str = r#"([\s\S]*?)"#;/)[1].replaceAll("__PORT__", "8766");
  function panel(fetch) {
    const calls = []; const timers = []; let start;
    vm.runInNewContext(body, {
      window: { __setSlot: (key, value) => calls.push([key, value]) },
      document: { querySelectorAll: () => [] },
      fetch, AbortController,
      setInterval: (fn) => { start = fn; return 1; }, clearInterval: () => {},
      setTimeout: (fn, ms) => { timers.push({ fn, ms }); return timers.length; }, clearTimeout: () => {},
    }, { filename: "panel-pull.offline.js" });
    start(); return { calls, timers };
  }
  const response = (json) => Promise.resolve({ ok: true, json: async () => json });
  await test("panel clears null/missing CPU slots and restores valid values", async () => {
    let stage = 0;
    const p = panel((url) => response(url.endsWith("/units") ? { mode: "auto", units: {} } : (
      stage === 0 ? [{ key: "cpu_load", value: 33, unit: "%", kind: "number" }] :
      stage === 1 ? [{ key: "cpu_load", value: null, unit: "%", kind: "number" }] :
      [{ key: "cpu_load", value: 44, unit: "%", kind: "number" }]
    )));
    await settle();
    assert.deepEqual(p.calls.filter(([key]) => key === "cpu_load").at(-1), ["cpu_load", "33"]);
    assert(p.calls.some(([key, value]) => key === "cpu_load_p" && value === "—"));
    stage = 1; p.timers.filter((t) => t.ms === 1000).at(-1).fn(); await settle();
    assert.deepEqual(p.calls.filter(([key]) => key === "cpu_load").at(-1), ["cpu_load", "—"]);
    stage = 2; p.timers.filter((t) => t.ms === 1000).at(-1).fn(); await settle();
    assert.deepEqual(p.calls.filter(([key]) => key === "cpu_load").at(-1), ["cpu_load", "44"]);
  });
  await test("panel does not schedule overlapping poll while response pending", async () => {
    const d = deferred(); let count = 0;
    const p = panel((url) => { count++; return url.endsWith("/metrics") ? d.promise : response({}); });
    await settle();
    assert.equal(count, 2);
    assert.equal(p.timers.filter((t) => t.ms === 1000).length, 0);
    d.resolve({ ok: true, json: async () => [] }); await settle();
    assert.equal(p.timers.filter((t) => t.ms === 1000).length, 1);
  });
  await test("panel request timeout clears stale CPU rather than publishing zero", async () => {
    const p = panel((_url, options) => new Promise((_resolve, reject) => {
      options.signal.addEventListener("abort", () => reject(new Error("aborted fixture")));
    }));
    for (const timer of p.timers.filter((t) => t.ms === 4000)) timer.fn();
    await settle();
    assert(p.calls.some(([key, value]) => key === "cpu_load" && value === "—"));
    assert(!p.calls.some(([, value]) => value === "0"));
    assert.equal(p.timers.filter((t) => t.ms === 1000).length, 1);
  });
  console.log(`Offline regression: ${results.length} passed, 0 failed.`);
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
