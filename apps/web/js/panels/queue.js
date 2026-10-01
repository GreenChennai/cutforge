/* 渲染队列面板(T5.6 壳侧):render_queue list 驱动 + 五 action(pause/resume/cancel/
 * retry)+ render.progress 事件联动。状态机(后端裁决,按钮按态给/不给并 why 说明):
 * queued|running --pause→ paused;paused --resume→ queued;queued|running|paused
 * --cancel→ canceled;fail|canceled|ok --retry→ queued。
 * 诚实口径:运行中暂停 = 终止子进程转 paused,resume 重新入队(渲染缓存吸收重跑成本;
 * 进程级冻结跨平台不可达——后端如实声明,本面板提示原文)。 */
import { h, clear } from "../ui/dom.js";
import { uiStore } from "../core/store.js";
import { queueList, queueAction } from "../core/pro-commands.js";
import { subscribe } from "../core/event-bus.js";
import { toast } from "../ui/toast.js";

/** state → 允许的 action(与后端状态机同表;禁用按钮走 why 文案)。 */
const ACTIONS = [
  ["pause", ["queued", "running"]],
  ["resume", ["paused"]],
  ["cancel", ["queued", "running", "paused"]],
  ["retry", ["fail", "canceled", "ok"]],
];
const ACTION_LABEL = { pause: "暂停", resume: "继续", cancel: "取消", retry: "重试" };
const STATE_LABEL = {
  queued: "排队", running: "渲染中", paused: "已暂停",
  ok: "完成", fail: "失败", canceled: "已取消",
};

let rowsHost = null;
let headEl = null;
let pollTimer = 0;
let refreshing = false;

export function mount(container) {
  container.appendChild(h("h3", null, ["渲染队列(render_queue;T5.6)"]));
  container.appendChild(h("div", { class: "hint" }, [
    "render_run 入队即回 runId;并发上限 env CUTFORGE_RENDER_CONCURRENCY(缺省 1)。"
    + "运行中暂停 = 终止子进程转 paused,继续 = 重新入队(渲染缓存吸收重跑成本)。",
  ]));
  headEl = h("div", { class: "q-head", testid: "queue-head" }, [
    h("button", { class: "mini", testid: "queue-refresh", title: "render_queue list:刷新队列", onclick: () => refresh() }, ["刷新"]),
    h("span", { class: "dim", testid: "queue-count" }, ["(未加载)"]),
  ]);
  container.appendChild(headEl);
  rowsHost = h("div", { class: "q-rows", testid: "queue-rows" });
  container.appendChild(rowsHost);

  // render.progress 状态事件(queued/paused/canceled 新状态如实发布)→ 即时刷新(去抖)
  let deb = 0;
  subscribe("render.progress", () => {
    clearTimeout(deb);
    deb = setTimeout(() => refresh(), 150);
  });
  refresh();
}

/** 拉清单并渲染;running/queued 存在且**本页签可见**时 1s 轻轮询兜底(事件丢失也能
 * 收敛)。页签不可见时零请求(导出进度预算不背后台轮询;T3.5 perf-budget 实测口径)。 */
export async function refresh() {
  if (refreshing || uiStore.get().tab !== "queue") return;
  refreshing = true;
  try {
    const env = await queueList();
    if (env.ok) {
      const items = env.data.jobs || [];
      renderRows(items, env.data.concurrency);
      schedulePoll(items);
    } else if (rowsHost && !rowsHost.childElementCount) {
      clear(rowsHost);
      rowsHost.appendChild(h("div", { class: "dim", testid: "queue-empty" }, ["(队列不可用或为空)"]));
    }
  } finally {
    refreshing = false;
  }
}

function schedulePoll(items) {
  clearTimeout(pollTimer);
  if (items.some((it) => it.state === "running" || it.state === "queued")) {
    pollTimer = setTimeout(() => refresh(), 1000);
  }
}

function renderRows(items, concurrency) {
  clear(rowsHost);
  const count = /** @type {HTMLElement} */ (document.querySelector('[data-testid="queue-count"]'));
  if (count) {
    count.textContent = items.length
      ? `${items.length} 任务 · ${items.filter((i) => i.state === "running").length} 渲染中 · 并发 ${concurrency ?? 1}`
      : `(队列为空;并发上限 ${concurrency ?? 1})`;
  }
  if (!items.length) {
    rowsHost.appendChild(h("div", { class: "dim", testid: "queue-empty" }, [
      "(队列为空:导出面板「导出成片」即入队;render_run 入队即回 runId)",
    ]));
    return;
  }
  for (const it of items) rowsHost.appendChild(rowEl(it));
}

function rowEl(it) {
  const row = h("div", { class: "q-row", testid: "queue-row", dataset: { runId: it.runId, state: it.state } });
  row.appendChild(h("span", {
    class: `q-state q-${it.state}`, testid: "queue-state",
    title: "状态徽标(queued/running/paused/ok/fail/canceled)",
  }, [STATE_LABEL[it.state] || it.state]));
  row.appendChild(h("span", { class: "mono q-id", testid: "queue-runid", title: it.runId }, [shortId(it.runId)]));
  const detail = it.error || it.output || "";
  if (detail) {
    row.appendChild(h("span", { class: "q-detail", title: detail }, [String(detail).split(/[\\/]/).pop()]));
  }
  const btns = h("span", { class: "q-actions" });
  for (const [action, allowed] of ACTIONS) {
    const ok = allowed.includes(it.state);
    btns.appendChild(h("button", {
      class: "mini", testid: `queue-${action}`,
      disabled: ok ? null : true,
      "aria-disabled": ok ? "false" : "true",
      title: ok ? `${ACTION_LABEL[action]}(${action};单任务)` : `state=${it.state} 不可 ${action}(状态机见面板头注)`,
      onclick: () => act(action, it.runId),
    }, [ACTION_LABEL[action]]));
  }
  row.appendChild(btns);
  return row;
}

async function act(action, runId) {
  const env = await queueAction(action, runId);
  if (env.ok) {
    const st = env.data && env.data.state;
    toast(`已 ${ACTION_LABEL[action]}:${shortId(runId)} → ${STATE_LABEL[st] || st}`);
  }
  await refresh();
}

function shortId(id) {
  const s = String(id || "");
  return s.length > 12 ? `${s.slice(0, 6)}…${s.slice(-4)}` : s;
}
