/* 冲突面板(T2.5,E8):未裁决冲突清单;存在冲突 → 顶部停写横幅(横幅由 banner.js
 * 依 uiStore.conflicts 渲染,数据源 projector.refreshConflicts)。 */
import { h, clear } from "../ui/dom.js";
import { call } from "../core/api.js";
import { refreshConflicts } from "../core/projector.js";

let rowsEl = null;

export function mount(container) {
  const panel = h("div", { class: "panel tab-page" });
  panel.appendChild(h("h3", null, ["冲突(存在冲突时停写;裁决后删除 .cutforge/conflicts/*.json)"]));
  rowsEl = h("div", { id: "conflict-rows", testid: "conflict-rows" });
  panel.appendChild(rowsEl);
  container.appendChild(panel);
}

export async function refresh() {
  const env = await call("conflict_list");
  if (env.ok) await refreshConflicts();
  clear(rowsEl);
  const rows = (env.ok && env.data.conflicts) || [];
  if (!rows.length) {
    rowsEl.appendChild(document.createTextNode("(无冲突)"));
    return;
  }
  for (const c of rows) {
    rowsEl.appendChild(h("div", { class: "row", testid: "conflict-row" }, [
      h("span", { class: "badge" }, [c.code]),
      h("span", { class: "bd" }, [`${c.conflictId} @ ${c.pointer}`]),
    ]));
  }
}
