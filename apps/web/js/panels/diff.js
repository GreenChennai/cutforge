/* 差异面板(T2.5):OpLog 逐字段审计(actor 过滤/limit/勾选批量撤销 undo ×N)。 */
import { h, clear } from "../ui/dom.js";
import { selectField, textField } from "../ui/controls.js";
import { undoBatch } from "../core/commands.js";
import { call } from "../core/api.js";
import { reproject } from "../core/projector.js";
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
  await undoBatch(n);
  await refresh();
  await reproject();
}
