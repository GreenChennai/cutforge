/* 历史面板(T4.3):oplog_tail 驱动的时间轴式历史 + 回跳(批量 undo/redo)+ 快照标记。
 *
 * - 人话行:oplog 每笔自带 summary(内核写入器生成,如 "clip_trim roll V1-001 in +40ms
 *   (边界联动)"),面板补 kind 徽标 / actor / 时刻 / rev;不二次发明事实;
 * - 撤销栈推演:顺序扫 ops(non-auto 进栈;undo 出栈进重做栈;redo 回栈)得每笔
 *   「当前生效/已撤销」——当前指针 = 栈顶;与内核 LIFO 语义同构(ADR-0001);
 * - 回跳:点生效行 = 撤销比它新的 N 笔(确认弹窗「将撤销 N 笔」);点已撤销行 =
 *   重做它及其后 M 笔;点当前指针 = 无操作。截断(超 limit)时更早的历史如实标不可回跳;
 * - 快照标记:ephemeral.historyMarks(会话态不落盘);导出前/批量前自动打标 +
 *   空白菜单手动打标;列表中显示分隔线。
 */
import { h, clear } from "../ui/dom.js";
import { ephemeralStore } from "../core/store.js";
import { call } from "../core/api.js";
import { redo, undoBatch } from "../core/commands.js";
import { markHistory } from "../core/edit-commands.js";
import { openDialog } from "../ui/dialog.js";
import { confirmBatch } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";

const FETCH_LIMIT = 500;
const KIND_GLYPH = {
  set: "改", insert: "增", delete: "删", move: "移", split: "切",
  merge: "合", "resolve-conflict": "裁", undo: "↩", redo: "↪",
};

/** 字段兼容:oplog_tail 序列化为 snake_case(op_kind/op_id);驼峰留作演进容错。 */
const kindOf = (op) => op.op_kind ?? op.opKind;
const idOf = (op) => op.op_id ?? op.opId;

let rowsEl = null;
let headEl = null;
/** @type {{ops: Array, count: number, rev: number}|null} */
let lastFetch = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["历史(撤销栈推演;点击回跳)"]));
  headEl = h("div", { class: "hist-head", testid: "history-head" });
  container.appendChild(headEl);
  rowsEl = h("div", { class: "hist-rows", testid: "history-rows", role: "list", "aria-label": "操作历史" });
  container.appendChild(rowsEl);
  container.appendChild(h("div", { class: "hint" }, [
    "点生效行 = 撤销到该点(批量 undo);点已撤销行 = 重做恢复;导出/批量前自动打快照分隔(会话态)",
  ]));
}

export async function refresh() {
  const env = await call("oplog_tail", { limit: FETCH_LIMIT });
  if (!env.ok) {
    headEl.textContent = `历史不可用:${env.message || env.code}`;
    return;
  }
  lastFetch = { ops: env.data.ops || [], count: env.data.count || 0, rev: env.data.rev || 0 };
  render();
}

/** 撤销栈推演:返回 {liveIds, undoStack, redoStack}。op_kind ∈ set/insert/delete/move/
 * split/merge/resolve-conflict 进栈;undo 出栈进重做栈;redo 回栈(与内核 LIFO 同构)。 */
function simulate(ops) {
  /** @type {Array<{id: string, rev: number}>} */
  const undoStack = [];
  /** @type {Array<{id: string, rev: number}>} */
  const redoStack = [];
  for (const op of ops) {
    if (op.auto) continue; // 自动簿记不入撤销栈(ADR-0001)
    const kind = kindOf(op);
    if (kind === "undo") {
      const top = undoStack.pop();
      if (top) redoStack.push(top);
    } else if (kind === "redo") {
      const top = redoStack.pop();
      if (top) undoStack.push(top);
    } else {
      undoStack.push({ id: idOf(op), rev: op.rev || 0 });
    }
  }
  const liveIds = new Set(undoStack.map((x) => x.id));
  return { liveIds, undoStack, redoStack };
}

function render() {
  const { ops, count, rev } = lastFetch;
  const { liveIds, undoStack } = simulate(ops);
  const topRev = undoStack.length ? undoStack[undoStack.length - 1].rev : 0;
  const marks = (ephemeralStore.get().historyMarks || []).slice().reverse();
  clear(headEl);
  headEl.appendChild(h("span", { class: "dim" }, [
    `当前 rev ${rev} · 撤销栈深 ${undoStack.length} · 载入 ${ops.length}/${count} 笔`,
  ]));
  if (count > ops.length) {
    headEl.appendChild(h("div", { class: "hist-truncated", testid: "history-truncated" }, [
      `仅载入最近 ${ops.length} 笔(count=${count});更早历史不参与回跳推演`,
    ]));
  }
  clear(rowsEl);
  const marksByRev = new Map();
  for (const m of marks) {
    if (!marksByRev.has(m.rev)) marksByRev.set(m.rev, m);
  }
  const ordered = [...ops].sort((a, b) => (b.rev || 0) - (a.rev || 0));
  let emitted = false;
  for (const op of ordered) {
    // 快照分隔线:锚在 rev ≤ 标记 rev 的第一行之前(上方 = 比快照新)
    for (const [mrev, m] of marksByRev) {
      if ((op.rev || 0) <= mrev && (emitted || mrev >= (ordered[ordered.length - 1].rev || 0))) {
        rowsEl.appendChild(markRow(m));
        marksByRev.delete(mrev);
      }
    }
    rowsEl.appendChild(opRow(op, liveIds.has(idOf(op)), (op.rev || 0) === topRev));
    emitted = true;
  }
  // 尾部残留标记(比最老记录还早)
  for (const m of marksByRev.values()) rowsEl.appendChild(markRow(m));
}

function markRow(m) {
  return h("div", {
    class: "hist-mark", testid: "history-mark", role: "separator",
    title: `快照标记 rev ${m.rev}(会话态,不落盘)`,
  }, [
    h("span", { class: "hist-mark-line", "aria-hidden": "true" }),
    h("span", { class: "hist-mark-label" }, [`快照 ${m.label} · rev ${m.rev} · ${timeOf(m.t)}`]),
  ]);
}

function opRow(op, live, isTop) {
  const kind = kindOf(op);
  const isRecord = kind === "undo" || kind === "redo"; // 撤销/重做记录行:回跳以原始操作行为准
  const undone = !live && !isRecord;
  const state = live ? "live" : isRecord ? "record" : "undone";
  const row = h("div", {
    class: `hist-row${undone ? " undone" : ""}${isTop ? " current" : ""}${isRecord ? " record" : ""}`,
    testid: "history-row", role: "listitem",
    dataset: { opId: idOf(op), rev: String(op.rev || 0), state },
    tabindex: "0", "aria-label": `${undone ? "已撤销" : isRecord ? "记录" : "生效"} rev ${op.rev} ${op.summary}`,
    title: isRecord ? "撤销/重做记录(回跳请点对应的原始操作行)"
      : undone ? "点击重做恢复到此处" : "点击撤销到此处",
  }, [
    h("span", { class: `badge kind kind-${kind}`, title: `opKind=${kind}` }, [KIND_GLYPH[kind] || kind]),
    h("span", { class: "rev" }, [`r${op.rev ?? "-"}`]),
    h("span", { class: "bd" }, [
      h("span", { class: "sum" }, [String(op.summary || "")]),
      h("span", { class: "delta dim" }, [
        `${op.target ? `${op.target.file}${op.target.path}` : ""}`,
      ]),
    ]),
    h("span", { class: `badge actor ${op.actor?.kind}` }, [op.actor?.kind || "?"]),
    h("span", { class: "dim ts" }, [timeOf(op.ts)]),
    undone ? h("span", { class: "badge undone-badge" }, ["已撤销"]) : null,
    isRecord ? h("span", { class: "badge record-badge", title: "撤销/重做记录" }, ["记录"]) : null,
    isTop ? h("span", { class: "badge current-badge", title: "当前指针" }, ["当前"]) : null,
  ]);
  const act = () => (isRecord
    ? toast("这是撤销/重做记录行,不可回跳;请点对应的原始操作行")
    : jump(op, undone));
  row.addEventListener("click", act);
  row.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      act();
    }
  });
  return row;
}

function timeOf(ts) {
  const d = ts instanceof Date ? ts : new Date(ts);
  if (Number.isNaN(d.getTime())) return String(ts).slice(11, 19) || "-";
  const p2 = (x) => String(x).padStart(2, "0");
  return `${p2(d.getHours())}:${p2(d.getMinutes())}:${p2(d.getSeconds())}`;
}

/** 回跳:生效行 → undo N(比它新的生效笔数);已撤销行 → redo M。 */
async function jump(op, undone) {
  if (!lastFetch) return;
  const { liveIds, undoStack, redoStack } = simulate(lastFetch.ops);
  const r = op.rev || 0;
  if (!undone) {
    const n = undoStack.filter((x) => x.rev > r).length;
    if (!n) {
      toast("已是当前状态,无需撤销");
      return;
    }
    if (confirmBatch()) {
      const yes = await confirmDialog(`将撤销 ${n} 笔操作(回到 rev ${r})`, "撤销到此处");
      if (!yes) return;
    }
    markHistory(`回跳撤销前(rev ${r})`);
    await undoBatch(n);
  } else {
    // 重做栈是 LIFO(数组尾 = 下一个重做):恢复到该笔 = 重做它及其「之后被撤销」的全部,
    // 即数组中从该笔到栈尾的连续段(按弹出顺序补齐,恰好回到该笔后的状态)。
    const idx = redoStack.findIndex((x) => x.id === idOf(op));
    if (idx < 0) {
      toast("该笔不可直接重做(撤销栈顺序已变化)", false);
      return;
    }
    const m = redoStack.length - idx;
    if (confirmBatch()) {
      const yes = await confirmDialog(`将重做 ${m} 笔操作(恢复到 rev ${r})`, "重做到此处");
      if (!yes) return;
    }
    await redo(m);
  }
  await refresh();
}

/** 确认弹窗(同 diff 面批量撤销 confirm-dialog 语义;设置可关)。 */
function confirmDialog(text, okLabel) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v) => { if (!settled) { settled = true; resolve(v); } };
    const dlg = openDialog({
      id: "confirm-dialog",
      title: text,
      onClose: () => done(false),
      build: (body) => {
        body.appendChild(h("p", null, ["历史回跳按 OpLog 顺序批量执行(可再撤销/重做)。"]));
        const ok = h("button", { testid: "confirm-ok" }, [okLabel]);
        const cancel = h("button", { testid: "confirm-cancel" }, ["取消"]);
        ok.addEventListener("click", () => { done(true); dlg.close(); });
        cancel.addEventListener("click", () => { done(false); dlg.close(); });
        body.appendChild(h("div", { class: "wizard-actions" }, [ok, cancel]));
        ok.focus();
      },
    });
  });
}
