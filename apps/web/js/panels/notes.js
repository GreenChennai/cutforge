/* 标注面板(T2.5,M10-2 行为对拍):创建(notes_add)→ open 列表(回执结案
 * notes_resolve)→ 孤儿标注如实展示。行结构是 e2e 兼容红线:
 * #notes-open .row 内依次 [回执 input][opIds input][结案 button],行文本含标注 id。
 * 册七 T7.5:标注线程化 UI——thread[](多轮对话,author 徽标)+ 回复输入(note_reply,
 * 不改 state 不碰结案回执);会话报告(session_report 人话 Markdown 渲染 + 下载)。 */
import { h, clear } from "../ui/dom.js";
import { selectionStore } from "../core/store.js";
import { addNote, resolveNote } from "../core/commands.js";
import { reproject } from "../core/projector.js";
import { playback } from "../render/preview-loop.js";
import { toast } from "../ui/toast.js";
import { call } from "../core/api.js";

let bodyInput = null;
let openBox = null;
let doneBox = null;
let orphanBox = null;
let reportHost = null;

export function mount(container) {
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["新建标注(锚定当前选中片段 / 播放头)"]),
    bodyInput = h("input", { type: "text", id: "note-body", testid: "note-body", placeholder: "这里要怎么改?" }),
    h("button", { id: "note-create", testid: "note-create", onclick: () => create() }, ["创建(notes_add)"]),
  ]));
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["open 态标注(线程可多轮讨论)"]),
    openBox = h("div", { id: "notes-open", testid: "notes-open" }),
  ]));
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["已结案 / 已否决(线程留档)"]),
    doneBox = h("div", { id: "notes-done", testid: "notes-done" }),
  ]));
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["孤儿标注(显式保留,不静默丢弃)"]),
    orphanBox = h("div", { id: "notes-orphans", testid: "notes-orphans" }),
  ]));
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["会话报告(session_report;人话 Markdown)"]),
    h("div", { class: "filter-row" }, [
      h("button", { testid: "report-run", onclick: () => runReport(), title: "汇总一次会话(sinceRev 起)的改动段/操作分布/回执率" }, ["生成报告"]),
      h("button", { testid: "report-download", onclick: () => downloadReport(), disabled: true, title: "下载报告 .md 文件" }, ["下载 .md"]),
    ]),
    reportHost = h("div", { class: "plan-report", testid: "report-body" }, [
      h("span", { class: "hint" }, ["(尚未生成:点「生成报告」拉取 session_report)"]),
    ]),
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

/** 刷新 open 列表 + 已结案留档 + 孤儿数(M10-2 ≤5s 全链依赖此处轻快)。 */
export async function refresh() {
  const env = await call("notes_list");
  if (!env.ok) return;
  const notes = env.data.notes || [];
  const open = notes.filter((n) => n.state === "open");
  const done = notes.filter((n) => n.state === "resolved" || n.state === "rejected");
  const orphans = env.data.orphans || 0;
  clear(openBox);
  for (const n of open) openBox.appendChild(openRow(n));
  if (!open.length) openBox.appendChild(document.createTextNode("(无)"));
  clear(doneBox);
  for (const n of done) doneBox.appendChild(doneRow(n));
  if (!done.length) doneBox.appendChild(document.createTextNode("(无)"));
  orphanBox.textContent = orphans ? `${orphans} 条孤儿(见 notes_list)` : "(无)";
}

/** open 行(e2e 红线:[note-reply][note-opids][note-resolve];行文本含标注 id)。 */
function openRow(n) {
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
    threadBlock(n),
  ]);
  return row;
}

/** 已结案/否决行(只读留档:线程 + 结案回执;不提供回复入口——讨论已收敛)。 */
function doneRow(n) {
  const bd = [n.body];
  if (n.resolvedBy) bd.push(` 回执:${n.resolvedBy.reply}${(n.resolvedBy.opIds || []).length ? `(op:${n.resolvedBy.opIds.join(",")})` : ""}`);
  if (n.rejectedReason) bd.push(` 否决原因:${n.rejectedReason}`);
  return h("div", { class: "row note-done-row", testid: "note-done-row" }, [
    h("span", { class: `badge ${n.state}` }, [`${n.id} ${n.state}`]),
    h("span", { class: "bd" }, bd),
    threadBlock(n, true),
  ]);
}

/** 线程块(T7.5):thread[] 逐条(author 徽标 + 时刻 + 正文)+ open 态回复输入。
 * note_reply 只追加讨论,不改 state、不碰 resolved_by(结案回执独立留痕)。 */
function threadBlock(n, readonly = false) {
  const box = h("div", { class: "note-thread", testid: "note-thread" });
  const thread = n.thread || [];
  for (const r of thread) {
    box.appendChild(h("div", { class: "note-thread-item", testid: "note-thread-item" }, [
      h("span", { class: `badge ${r.author === "agent" ? "agent" : "user"}` }, [r.author === "agent" ? "AI" : "人"]),
      h("span", { class: "note-thread-at" }, [String(r.at || "").replace("T", " ").slice(0, 19)]),
      h("span", { class: "note-thread-body" }, [r.body]),
    ]));
  }
  if (!thread.length && readonly) {
    box.appendChild(h("span", { class: "hint" }, ["(无追加讨论)"]));
  }
  if (!readonly) {
    const input = h("input", {
      type: "text", placeholder: "追加回复(线程讨论,不改结案状态)…",
      testid: "note-thread-body", "aria-label": `回复标注 ${n.id}`,
    });
    const send = h("button", {
      testid: "note-thread-send",
      onclick: async () => {
        const body = input.value.trim();
        if (!body) { toast("回复内容必填", false); return; }
        const env = await call("note_reply", { noteId: n.id, body, author: "user" });
        if (env.ok) { toast(`已追加回复(线程 ${n.id},现 ${env.data.replies} 条)`); await refresh(); }
        else toast(`回复失败:${env.code}`, false);
      },
    }, ["回复"]);
    box.appendChild(h("div", { class: "filter-row" }, [input, send]));
  }
  return box;
}

async function resolveRow(noteId, row) {
  const reply = /** @type {HTMLInputElement} */ (row.querySelector('[data-testid="note-reply"]')).value;
  const opIds = /** @type {HTMLInputElement} */ (row.querySelector('[data-testid="note-opids"]')).value
    .split(",").map((s) => s.trim()).filter(Boolean);
  const env = await resolveNote(noteId, reply, opIds);
  if (env.ok) {
    await refresh();
    reproject(); // 结案不必然改工程;保守重投影对齐 caused_by 审计面
  } else {
    toast(`结案失败:${env.code}`, false);
  }
}

/* ---------------- 会话报告(T7.5)---------------- */

let lastReportMd = "";

async function runReport() {
  const env = await call("session_report");
  if (!env.ok) {
    toast(`会话报告失败:${env.code}`, false);
    return;
  }
  lastReportMd = env.data.markdown || "";
  clear(reportHost);
  reportHost.appendChild(renderMarkdown(lastReportMd));
  const dl = /** @type {HTMLButtonElement} */ (document.querySelector('[data-testid="report-download"]'));
  if (dl) dl.disabled = false;
  toast(`会话报告已生成(rev → ${env.data.revTo},${env.data.opCount} Op)`);
}

/** 下载 .md(会话态 Blob,不写盘直写——浏览器沙箱内唯一诚实通道)。 */
function downloadReport() {
  if (!lastReportMd) return;
  const blob = new Blob([lastReportMd], { type: "text/markdown;charset=utf-8" });
  const a = h("a", { href: URL.createObjectURL(blob), download: `cutforge-session-report-${Date.now()}.md` });
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
}

/** 极简 Markdown → DOM(白名单:标题/列表/代码段/行内 **bold** 与 `code`;
 * 全程 createTextNode/h() 构建零 innerHTML,报告内容不构成注入面)。 */
function renderMarkdown(md) {
  const root = h("div", { class: "md-render", testid: "report-markdown" });
  let list = null;
  let code = null;
  const flushList = () => { if (list) { root.appendChild(list); list = null; } };
  const flushCode = () => { if (code) { root.appendChild(code); code = null; } };
  for (const line of md.split("\n")) {
    if (line.startsWith("```")) { // 代码栅栏开合
      if (code) { flushCode(); } else { flushList(); code = h("pre", { class: "md-pre" }); }
      continue;
    }
    if (code) { code.appendChild(document.createTextNode(line + "\n")); continue; }
    if (line.startsWith("## ")) { flushList(); root.appendChild(h("h4", { class: "md-h" }, inline(line.slice(3)))); continue; }
    if (line.startsWith("### ")) { flushList(); root.appendChild(h("h5", { class: "md-h" }, inline(line.slice(4)))); continue; }
    if (line.startsWith("- ")) {
      flushCode();
      if (!list) list = h("ul", { class: "md-ul" });
      list.appendChild(h("li", null, inline(line.slice(2))));
      continue;
    }
    if (line.trim()) { flushList(); root.appendChild(h("p", { class: "md-p" }, inline(line))); }
  }
  flushList();
  flushCode();
  return root;
}

/** 行内 `code` 与 **bold**(不识别其余语法,原文保留——诚实降级)。 */
function inline(text) {
  const out = [];
  const re = /`([^`]+)`|\*\*([^*]+)\*\*/g;
  let last = 0;
  let m;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    if (m[1] !== undefined) out.push(h("code", null, [m[1]]));
    else out.push(h("b", null, [m[2]]));
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}
