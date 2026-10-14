<!--
  发版说明（Release 正文）
  每次发版前改这里；正文会原样放进 GitHub Release，末尾自动附上「本次包含的提交」列表。
  版本号改这三处：package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml（必须一致）。
-->

## v0.3.0 · CPU 占用率自算 + 大小核兼容模式

这一版修掉一个**长期存在的取数错误**，并为 **Intel 大小核（P 核 + E 核）**机型补上专用口径。

### 修复：CPU 占用率严重偏低

面板上的 CPU 占用率此前取自 LibreHardwareService（内含 LibreHardwareMonitor）的 `CPU Total`。
LHM 用每个逻辑处理器的 `1 − ΔIdleTime / ΔTotalTime` 再取平均 —— **公式没错、取平均也没错，
错在"从哪读 idle"**：`SystemProcessorPerformanceInformation.IdleTime` 在 **Windows 11（22H2 起）
返回无效值**，导致占用率严重偏低（上游实测：同一时刻任务管理器约 30%，LHM 仅约 2%）。
上游早已给出绕行方案（LibreHardwareMonitor PR #933），但**服务端打包的 LHM 版本未包含**，
微软至今未修复。

现在**改为本地自算**，不再依赖该服务：

| | 取数 | 说明 |
|---|---|---|
| 主口径 | PDH `\Processor Information(_Total)\% Processor Utility` | **与任务管理器（Win8+）同源同口径**，降频时会同步下降，睿频时可超 100%（显示按任务管理器的做法截到 100） |
| 交叉校验 | PDH `% Processor Time`、`% Processor Performance` | 用于判断量级是否离谱 |
| 兜底 | `GetSystemTimes` 差值 | PDH 不可用时仍有值；纯 Win32，无额外依赖 |

**附带好处**：CPU 占用率不再随数据源瞬断而缺项 —— 面板上不会再出现"数字冻住不动"。

### 新增：大小核兼容模式

设置页「性能」卡片新增开关（也可直接改 `settings.ini` 的 `[app] hybrid_compat`）：

- **关（默认）**：全部逻辑处理器**等权平均** —— 与任务管理器一致。**同构核机型请保持关闭。**
- **开**：额外按 Windows 的 `EfficiencyClass` 把逻辑处理器分成 **P 核 / E 核两组、组内平均**，
  额外出 `cpu_load_p` / `cpu_load_e` 两个槽；**总数 `cpu_load` 不变**。
  同构核机型即使打开也不会输出分组值，并会记一条提示。

> 是「兼容模式」而不是「优化」：这里没有自创加权系数 —— 各主流工具（任务管理器、Process Lasso、
> OSHI、HWiNFO）都不按核型加权，自创系数只会让数字与任务管理器不可比。

### 改进：数据源瞬断不再刷屏

- 告警**判据与文案对齐**：过去只要共享内存打不开就报「服务未运行」，而**服务其实在跑**
  （服务刚启动、硬件还没枚举完、尚未发布共享内存时也会这样）。现在会读错误码区分
  「不存在」与「权限不足」，并在文案里写明这种情况。
- **同一原因的故障只记一条**，恢复时补一条汇总（含期间抑制了多少次采样）。
  原先每次故障会以 2 条/秒刷屏。

### 新增：诊断命令

```
jiaidian-monitor.exe --dump-cpu     # CPU 四条口径原始值 + 大小核分组情况
```

`--dump-cpu` 会同时给出逐逻辑处理器读数的体检信息，便于远程核对：
`perProcessorMean` 应当 ≈ `utility`（后者取系统的 `_Total` 实例）—— 两者相近即说明取数链路正常。

### 升级提示

- 便携版直接替换 `jiaidian-monitor.exe` 即可；`settings.ini` 会保留原有设置，
  新版本首次写入时会补上 `hybrid_compat` 一行。
- **同构核机型（AMD 全大核等）无需任何改动**，行为与上一版一致。

### 许可证

本项目采用 **PolyForm Noncommercial License 1.0.0** —— 源码可见，**禁止商业使用**（非 OSI 开源许可）。
第三方组件声明见 `THIRD-PARTY-NOTICES.md`。
