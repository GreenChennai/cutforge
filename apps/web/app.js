// CutForge Web 壳(最小可用;计划书 7.6 / M5)。
// 纯度:壳不解析真相、不算时间线语义——一切投影经 forge_*(wasm 内核)取得。
import init, {
  forge_open, forge_timeline, forge_notes, forge_oplog, forge_conflicts,
  forge_validate, forge_script_run,
} from "../../crates/cutforge-wasm/pkg/cutforge_wasm.js";

const $ = (id) => document.getElementById(id);
let projectText = null;

// Tab 切换
for (const btn of $("tabs").querySelectorAll("button")) {
  btn.onclick = () => {
    for (const b of $("tabs").querySelectorAll("button")) b.classList.remove("active");
    for (const t of document.querySelectorAll(".tab")) t.classList.remove("active");
    btn.classList.add("active");
    $(`tab-${btn.dataset.tab}`).classList.add("active");
  };
}

function status(msg, ok = true) {
  $("status").textContent = msg;
  $("status").style.color = ok ? "#9aa4ae" : "var(--warn)";
}

async function readFile(input) {
  const f = input.files?.[0];
  return f ? f.text() : null;
}

async function refreshProject() {
  projectText = await readFile($("f-project"));
  if (!projectText) return;
  const verdict = JSON.parse(forge_validate(projectText));
  status(verdict.ok ? "工程已载入(v2 契约通过)" : `SCHEMA_INVALID: ${verdict.message.slice(0, 120)}`, verdict.ok);
  renderTimeline();
}

function renderTimeline() {
  if (!projectText) return;
  const tl = JSON.parse(forge_timeline(projectText));
  const end = Math.max(1, ...tl.clips.map((c) => c.endMs));
  const byTrack = {};
  for (const c of tl.clips) (byTrack[c.track] ??= []).push(c);
  $("timeline").innerHTML = Object.entries(byTrack).map(([track, clips]) => `
    <div class="track"><h3>${track}</h3><div class="lane">
      ${clips.map((c) => `<div class="clip" title="${c.id} ${c.startMs}–${c.endMs}ms"
        style="left:${(c.startMs / end) * 100}%;width:${((c.endMs - c.startMs) / end) * 100}%">${c.id}</div>`).join("")}
    </div></div>`).join("");
}

async function refreshNotes() {
  const text = await readFile($("f-notes"));
  if (!text) return;
  const d = JSON.parse(forge_notes(text));
  $("notes").innerHTML =
    `<p>共 ${d.counts.total} 条:open ${d.counts.open} / resolved ${d.counts.resolved} / rejected ${d.counts.rejected} / <b>orphan ${d.counts.orphan}</b></p>` +
    d.notes.map((n) => `
      <div class="note ${n.state === "orphan" ? "orphan" : ""}">
        <div class="id">${n.id} · ${n.author} · ${n.state}${n.relocated ? " · 已重定位" : ""}</div>
        <div>${n.body}</div>
        ${n.resolvedBy ? `<div class="id">回执:${n.resolvedBy.reply}(opIds:${n.resolvedBy.opIds.join(", ")})</div>` : ""}
        ${n.orphanReason ? `<div class="id">孤儿原因:${n.orphanReason}</div>` : ""}
      </div>`).join("");
}

async function refreshOplog() {
  const text = await readFile($("f-oplog"));
  if (!text) return;
  const d = JSON.parse(forge_oplog(text));
  $("diff").innerHTML =
    `<p>共 ${d.total} 条操作:${Object.entries(d.byActor).map(([k, v]) => `${k} ${v}`).join(" / ")}</p>` +
    [...d.ops].reverse().map((o) => `
      <div class="op">
        <div class="meta">rev${o.rev ?? "?"} · ${o.actor?.kind ?? "?"}:${o.actor?.id ?? ""} · ${o.opKind ?? ""} @ ${o.target?.path ?? ""}</div>
        <div>${o.summary ?? ""}</div>
        ${o.causedBy ? `<div class="meta">因标注 ${o.causedBy.join(", ")}</div>` : ""}
      </div>`).join("");
}

async function refreshConflicts() {
  const text = await readFile($("f-conflicts"));
  if (!text) return;
  const d = JSON.parse(forge_conflicts(text));
  const rows = Array.isArray(d.conflicts) ? d.conflicts : [];
  $("conflicts").innerHTML = rows.length
    ? rows.map((c) => `<div class="op"><div class="meta">${c.code} @ ${c.pointer}</div></div>`).join("")
    : "无冲突快照";
}

$("f-project").onchange = refreshProject;
$("f-notes").onchange = refreshNotes;
$("f-oplog").onchange = refreshOplog;
$("f-conflicts").onchange = refreshConflicts;

$("script-run").onclick = () => {
  if (!projectText) {
    $("script-out").textContent = "先加载 project.json";
    return;
  }
  try {
    $("script-out").textContent = forge_script_run(projectText, $("script-in").value);
  } catch (e) {
    $("script-out").textContent = `GUARD_FAILED: ${e}`;
  }
};

// wasm 装载(相对路径:由任意静态服务器以仓库根或 apps/web 为根提供)
init().then(() => status("wasm 内核就绪;请加载工程文件")).catch((e) => status(`wasm 装载失败: ${e}`, false));
