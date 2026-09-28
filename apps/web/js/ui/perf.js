/* 性能度量(T3.5):采集面(帧率/rAF 分布/DOM 节点/未完成请求/投影耗时)+
 * 预算表(与 docs/design/perf-budget.md 一一对应;面板可视达标态)。
 * 采样纪律:只有性能面板打开时才起采样 rAF(默认关,零常态开销);
 * 预算中「1k 滚动」「导出进度」属 e2e/导出期实测项,面板如实标注数据来源。 */
import { netInflight } from "../core/api.js";
import { lastProjectionTime } from "../core/projector.js";
import { poolStats } from "../render/media-pool.js";

const metrics = {
  fpsNow: 0,            // 最近 500ms 窗口帧率
  gaps: { le17: 0, le34: 0, le50: 0, gt50: 0 }, // 最近窗口 rAF 间隔分布
  domNodes: 0,
  bootMs: 0,            // 首屏可交互(boot→投影+素材首览完成;main 记录)
  tabMs: 0,             // 最近一次页签切换耗时
  exportTicks: [],      // 导出进度回调时间戳(Hz 估算窗)
  exportHz: 0,
  lastExportState: "",
};

let sampling = false;
let rafId = 0;
let lastFrame = 0;
let windowFrames = [];

/** 面板开/关采样(默认关;打开才付出度量成本)。 */
export function setSampling(on) {
  if (on && !sampling) {
    sampling = true;
    lastFrame = performance.now();
    rafId = requestAnimationFrame(sampleTick);
  } else if (!on && sampling) {
    sampling = false;
    if (rafId) cancelAnimationFrame(rafId);
    rafId = 0;
  }
}

function sampleTick(now) {
  if (!sampling) return;
  const gap = now - lastFrame;
  lastFrame = now;
  windowFrames.push({ now, gap });
  // 滚出 500ms 窗外的旧帧
  while (windowFrames.length && now - windowFrames[0].now > 500) windowFrames.shift();
  rafId = requestAnimationFrame(sampleTick);
}

/** 聚合当前窗口(面板 500ms 刷新时调用)。 */
export function refreshWindowStats() {
  const now = performance.now();
  windowFrames = windowFrames.filter((f) => now - f.now <= 500);
  const gaps = windowFrames.map((f) => f.gap).filter((g) => g > 0);
  metrics.gaps = { le17: 0, le34: 0, le50: 0, gt50: 0 };
  for (const g of gaps) {
    if (g <= 17) metrics.gaps.le17 += 1;
    else if (g <= 34) metrics.gaps.le34 += 1;
    else if (g <= 50) metrics.gaps.le50 += 1;
    else metrics.gaps.gt50 += 1;
  }
  const wall = windowFrames.length > 1 ? now - windowFrames[0].now : 0;
  metrics.fpsNow = wall > 0 ? Math.round(((windowFrames.length - 1) / wall) * 1000) : 0;
  metrics.domNodes = document.getElementsByTagName("*").length;
  // 导出进度 Hz(最近 6 个 tick 的时间跨度)
  const t = metrics.exportTicks;
  if (t.length >= 2) {
    const recent = t.slice(-6);
    const span = (recent[recent.length - 1] - recent[0]) / 1000;
    metrics.exportHz = span > 0 ? Number(((recent.length - 1) / span).toFixed(1)) : 0;
  }
}

export const getMetrics = () => metrics;

/* ---- 事件记录口(main / panels 调用)---- */

export function recordBoot(ms) { metrics.bootMs = Math.round(ms); }
export function recordTabSwitch(ms) { metrics.tabMs = Math.round(ms); }
export function recordExportTick(state) {
  metrics.lastExportState = state || "";
  const now = performance.now();
  const t = metrics.exportTicks;
  t.push(now);
  while (t.length && now - t[0] > 10000) t.shift();
}

/** 预算表(docs/design/perf-budget.md 的面板可视面)。pass: true/false/null(null=e2e 侧测)。 */
export function budgetRows() {
  const m = metrics;
  return [
    { id: "interact", name: "交互响应(最近投影)", target: "≤16ms", value: `${Math.round(lastProjectionTime())}ms`, pass: lastProjectionTime() > 0 && lastProjectionTime() <= 16 ? true : lastProjectionTime() > 16 ? false : null },
    { id: "playfps", name: "播放帧率(实时)", target: "60fps(门禁 55)", value: m.fpsNow ? `${m.fpsNow}fps` : "—(播放中测)", pass: m.fpsNow ? m.fpsNow >= 55 : null },
    { id: "scrollfps", name: "1k 片段滚动", target: "60fps", value: "e2e_perf_timeline 实测", pass: null },
    { id: "boot", name: "首屏可交互", target: "<1s", value: m.bootMs ? `${m.bootMs}ms` : "装配中…", pass: m.bootMs ? m.bootMs < 1000 : null },
    { id: "tab", name: "页签切换", target: "<100ms", value: m.tabMs ? `${m.tabMs}ms` : "—(切换后测)", pass: m.tabMs ? m.tabMs < 100 : null },
    { id: "export", name: "导出进度回调", target: "≥2Hz", value: m.exportHz ? `${m.exportHz}Hz` : "—(导出时测)", pass: m.exportHz ? m.exportHz >= 2 : null },
  ];
}

/** 采集行(面板上半:非预算的运行时指标)。 */
export function statRows() {
  const m = metrics;
  const pool = poolStats();
  return [
    { id: "fps", name: "帧率(500ms 窗)", value: `${m.fpsNow}fps` },
    { id: "rafgaps", name: "rAF 间隔分布", value: `≤17ms:${m.gaps.le17} ≤34:${m.gaps.le34} ≤50:${m.gaps.le50} >50:${m.gaps.gt50}` },
    { id: "dom", name: "DOM 节点数", value: String(m.domNodes) },
    { id: "inflight", name: "未完成请求", value: String(netInflight()) },
    { id: "proj", name: "最近一次投影耗时", value: `${Math.round(lastProjectionTime())}ms` },
    { id: "pool", name: "媒体元素池", value: `${pool.size}/${pool.cap}(累计淘汰 ${pool.evicted})` },
  ];
}
