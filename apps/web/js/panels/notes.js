/* 标注面板(T2.5,M10-2 行为对拍):创建(notes_add)→ open 列表(回执结案
 * notes_resolve)→ 孤儿标注如实展示。行结构是 e2e 兼容红线:
 * #notes-open .row 内依次 [回执 input][opIds input][结案 button],行文本含标注 id。 */
import { h, clear } from "../ui/dom.js";
import { selectionStore } from "../core/store.js";
import { addNote, resolveNote } from "../core/commands.js";
import { reproject } from "../core/projector.js";
import { playback } from "../render/preview-loop.js";
import { toast } from "../ui/toast.js";
import { call } from "../core/api.js";

let bodyInput = null;
let openBox = null;
let orphanBox = null;

export function mount(container) {
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["新建标注(锚定当前选中片段 / 播放头)"]),
    bodyInput = h("input", { type: "text", id: "note-body", testid: "note-body", placeholder: "这里要怎么改?" }),
    h("button", { id: "note-create", testid: "note-create", onclick: () => create() }, ["创建(notes_add)"]),
  ]));
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["open 态标注"]),
    openBox = h("div", { id: "notes-open", testid: "notes-open" }),
  ]));
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["孤儿标注(显式保留,不静默丢弃)"]),
    orphanBox = h("div", { id: "notes-orphans", testid: "notes-orphans" }),
  ]));
}

async function create() {
  const body = bodyInput.value.trim();
  if (!body) {
    toast("标注正文必填", false);
    return;
  }
  const sel = selectionStore.get().clipId;
  const anchor = sel
    ? { kind: "clip", ref: sel, tMs: Math.round(playback.clockMs()) }
    : { kind: "time", tMs: Math.round(playback.clockMs()) };
  const env = await addNote(body, anchor);
  if (env && env.ok) {
    bodyInput.value = "";
    await refresh();
  }
}

/** 刷新 open 列表 + 孤儿数(面板级精准刷新;M10-2 的 ≤5s 全链依赖此处轻快)。 */
export async function refresh() {
  const env = await call("notes_list");
  if (!env.ok) return;
  const open = (env.data.notes || []).filter((n) => n.state === "open");
  const orphans = env.data.orphans || 0;
  clear(openBox);
  for (const n of open) {
    const replyInput = h("input", { placeholder: "回执 reply", style: "width:220px", testid: "note-reply" });
    const opIdsInput = h("input", { placeholder: "opIds(op-1,op-2)", style: "width:180px", testid: "note-opids" });
    const row = h("div", { class: "row", testid: "note-row" }, [
      h("span", { class: "badge open" }, [n.id]),
      h("span", { class: "bd" }, [
        n.body, " ",
        replyInput, " ",
        opIdsInput, " ",
        h("button", {
          testid: "note-resolve",
          onclick: () => resolveRow(n.id, row),
        }, ["结案"]),
      ]),
    ]);
    openBox.appendChild(row);
  }
  if (!open.length) openBox.appendChild(document.createTextNode("(无)"));
  orphanBox.textContent = orphans ? `${orphans} 条孤儿(见 notes_list)` : "(无)";
}

async function resolveRow(noteId, row) {
  const inputs = row.querySelectorAll("input");
  const reply = /** @type {HTMLInputElement} */ (inputs[0]).value;
  const opIds = /** @type {HTMLInputElement} */ (inputs[1]).value
    .split(",").map((s) => s.trim()).filter(Boolean);
  const env = await resolveNote(noteId, reply, opIds);
  if (env.ok) {
    await refresh();
    reproject(); // 结案不必然改工程;保守重投影对齐 caused_by 审计面
  } else {
    toast(`结案失败:${env.code}`, false);
  }
}
