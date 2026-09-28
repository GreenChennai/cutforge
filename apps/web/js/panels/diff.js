/* 差异面板(T2.5):OpLog 逐字段审计(actor 过滤/limit/勾选批量撤销 undo ×N)。
 * T3.7:批量撤销前确认弹窗(设置可关;prefs.confirmBatch)。 */
import { h, clear } from "../ui/dom.js";
import { selectField, textField } from "../ui/controls.js";
import { undoBatch } from "../core/commands.js";
import { call } from "../core/api.js";
import { reproject } from "../core/projector.js";
import { openDialog } from "../ui/dialog.js";
import { confirmBatch } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";

let actorSel = null;
let limitField = null;
let rowsEl = null;

export function mount(container) {
  const panel = h("div", { class: "panel tab-page" });
  panel.appendChild(h("h3", null, ["OpLog(AI 改了什么,逐字段可审计)"]));
  actorSel = selectField({
    id: "diff-actor", testid: "diff-actor",
    options: [["", "全部"], ["agent", "agent(AI)"], ["user", "user(人)"], ["script", "script(脚本)"]],
  });
  panel.appendChild(h("label", { class: "filter-row" }, ["actor ", actorSel.root]));
  limitField = textField({ id: "diff-limit", testid: "diff-limit", onEnter: () => refresh() });
  limitField.set("50");
  panel.appendChild(h("label", { class: "filter-row" }, ["limit ", limitField.root]));
  panel.appendChild(h("div", { class: "filter-row" }, [
    h("button", { id: "diff-refresh", testid: "diff-refresh", onclick: () => refresh() }, ["刷新"]),
    h("button", { id: "diff-undo-batch", testid: "diff-undo-batch", onclick: () => batchUndo() }, ["一键撤销该批(undo ×N)"]),
  ]));
  rowsEl = h("div", { id: "diff-rows", testid: "diff-rows" });
  panel.appendChild(rowsEl);
  container.appendChild(panel);
}

export async function refresh() {
  const actor = actorSel.get() || undefined;
  const limit = Number(limitField.get()) || 50;
  const env = await call("oplog_tail", { limit, ...(actor ? { actor } : {}) });
  if (!env.ok) return;
  clear(rowsEl);
  for (const op of env.data.ops || []) {
    const pick = h("input", { type: "checkbox", class: "pick", testid: "diff-pick" });
    rowsEl.appendChild(h("div", { class: "row", testid: "diff-row" }, [
      pick,
      h("span", { class: "oid" }, [op.opId]),
      h("span", { class: `badge ${op.actor.kind}` }, [op.actor.kind]),
      h("span", { class: "bd" }, [
        String(op.summary ?? ""),
        h("div", { class: "delta" }, [
          `${JSON.stringify(op.before).slice(0, 80)} → ${JSON.stringify(op.after).slice(0, 80)}`,
        ]),
      ]),
    ]));
  }
}

async function batchUndo() {
  const n = rowsEl.querySelectorAll(".pick:checked").length;
  if (!n) {
    toast("先勾选要撤销的 Op 行", false);
    return;
  }
  if (confirmBatch()) {
    const yes = await confirmBatchDialog(n);
    if (!yes) return;
  }
  await undoBatch(n);
  await refresh();
  await reproject();
}

/** 批量确认弹窗(T3.7;testid=confirm-dialog;批量影响 N 笔 Op,先问一句)。 */
function confirmBatchDialog(n) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v) => { if (!settled) { settled = true; resolve(v); } };
    const dlg = openDialog({
      id: "confirm-dialog",
      title: `确认撤销 ${n} 笔操作?`,
      onClose: () => done(false), // Esc/遮罩关闭 = 取消
      build: (body) => {
        body.appendChild(h("p", null, [`将按 OpLog 顺序批量撤销 ${n} 笔(可重做)。`]));
        const ok = h("button", { testid: "confirm-ok" }, ["撤销这批"]);
        const cancel = h("button", { testid: "confirm-cancel" }, ["取消"]);
        ok.addEventListener("click", () => { done(true); dlg.close(); });
        cancel.addEventListener("click", () => { done(false); dlg.close(); });
        body.appendChild(h("div", { class: "wizard-actions" }, [ok, cancel]));
        ok.focus();
      },
    });
  });
}
