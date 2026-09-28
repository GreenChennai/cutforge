/* dev 性能面板(T3.5):Shift+D 唤起,默认关。
 * 上半 = 运行时采集(帧率/rAF 分布/DOM 节点/未完成请求/投影耗时/媒体池),
 * 下半 = 预算表(交互 ≤16ms / 播放 60fps / 1k 滚动 60fps / 首屏 <1s / tab <100ms /
 * 导出 ≥2Hz),每行可视「当前值 + 是否达标」(✓/✗/— 文本线索,非颜色单线索)。
 * 预算口径与 docs/design/perf-budget.md 一一对应。 */
import { h } from "./dom.js";
import { getMetrics, refreshWindowStats, setSampling, budgetRows, statRows } from "./perf.js";

let panel = null;
let refreshTimer = 0;

/** 切换显示(键位 view.perf 调用)。 */
export function togglePerfPanel() {
  if (panel) hide();
  else show();
}

function show() {
  if (panel) return;
  panel = h("div", {
    class: "perf-panel", testid: "perf-panel", role: "region", "aria-label": "性能面板(dev)",
  }, [
    h("div", { class: "perf-head" }, [
      h("b", null, ["性能(DEV)"]),
      h("button", { testid: "perf-close", title: "关闭(Shift+D)", onclick: () => hide() }, ["×"]),
    ]),
    buildRows("perf-stats"),
    h("div", { class: "perf-sub" }, ["预算(docs/design/perf-budget.md)"]),
    buildRows("perf-budgets"),
  ]);
  document.body.appendChild(panel);
  setSampling(true);
  refreshTimer = setInterval(render, 500);
  render();
}

function buildRows(hostTestid) {
  return h("div", { class: "perf-rows", testid: hostTestid });
}

function render() {
  if (!panel) return;
  refreshWindowStats();
  fill($in(panel, "perf-stats"), statRows(), "perf-stat");
  fill($in(panel, "perf-budgets"), budgetRows(), "perf-budget");
}

function $in(root, testid) {
  return /** @type {HTMLElement} */ (root.querySelector(`[data-testid="${testid}"]`));
}

function fill(host, rows, rowTestid) {
  if (!host) return;
  host.textContent = "";
  for (const r of rows) {
    const mark = r.pass === true ? "✓" : r.pass === false ? "✗" : "—";
    const row = h("div", {
      class: "perf-row", testid: rowTestid, "data-pass": r.pass === true ? "1" : r.pass === false ? "0" : "na",
      title: `${r.name}:当前 ${r.value} / 预算 ${r.target}`,
    }, [
      h("span", { class: "perf-mark", "aria-hidden": "true" }, [mark]),
      h("span", { class: "perf-name" }, [r.name]),
      h("span", { class: "perf-value" }, [r.value]),
      h("span", { class: "perf-target" }, [r.target]),
    ]);
    if (r.id) row.dataset.budgetId = r.id;
    host.appendChild(row);
  }
}

function hide() {
  if (refreshTimer) clearInterval(refreshTimer);
  refreshTimer = 0;
  setSampling(false);
  if (panel) panel.remove();
  panel = null;
}

/** 面板是否打开(自测/e2e 断言)。 */
export function perfPanelOpen() {
  return Boolean(panel);
}

/** 供 e2e/控制台读取原始度量(不依赖 DOM)。 */
export function perfMetrics() {
  return getMetrics();
}
