/* CutForge Web 编辑器(壳不持有真相:查询走投影,变更走 MCP 工具 → Op)。
 * 壳纯度纪律(check-shell-purity):禁 node/fs/子进程;时间语义一律来自内核投影。 */
"use strict";

const $ = (id) => document.getElementById(id);
const state = {
  root: "", token: "", rev: 0, project: null, timeline: null,
  selected: null, playheadMs: 0, pxPerMs: 0.06, clipboard: null, eventSeq: 0,
  // E2 预览:playing=播放中;baseMs/t0=播放起点(挂钟推算播放头)
  playing: false, baseMs: 0, t0: 0,
  pvRows: [],          // [{row, el}] —— 每个带 src 的时间线行对应一个隐藏媒体元素
  pvDrift: 0,          // 上次漂移校正时刻(节流)
};

function status(msg, ok = true) { $("status").textContent = msg; $("status").style.color = ok ? "" : "#c25b5b"; }

async function api(name, args = {}) {
  const resp = await fetch("/rpc", {
    method: "POST",
    headers: { "Content-Type": "application/json", "Authorization": `Bearer ${state.token}` },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call",
      params: { name, arguments: { root: state.root, ...args } } }),
  });
  const rpc = await resp.json();
  const env = JSON.parse(rpc.result.content[0].text);
  if (!env.ok) { status(`${name}: ${env.code} ${env.message}`, false); }
  return env;
}

/* ---------------- 渲染 ---------------- */

function frameMs() { return 1000 / ((state.project && state.project.fps) || 30); }
function snap(ms) {
  const snapped = $("magnet").checked ? Math.round(ms / frameMs()) * frameMs() : ms;
  return Math.round(snapped);
}

function renderTimeline() {
  const p = state.project;
  if (!p || !state.timeline) return;
  const tracksEl = $("tracks");
  const ruler = $("ruler");
  tracksEl.innerHTML = ""; ruler.innerHTML = "";
  const end = Math.max(10000, ...state.timeline.map((c) => c.endMs));
  const width = Math.ceil(end * state.pxPerMs) + 40;
  ruler.style.width = width + "px";
  const secMs = 1000;
  for (let t = 0; t <= end; t += secMs) {
    const tick = document.createElement("span");
    tick.className = "tick"; tick.style.left = (t * state.pxPerMs) + "px";
    tick.textContent = (t / 1000) + "s";
    ruler.appendChild(tick);
  }
  // 时间线投影来自内核(timeline_get):壳零时间线语义运算(check-shell-purity)
  for (const t of p.tracks) {
    const lane = document.createElement("div");
    lane.className = "track"; lane.style.width = width + "px";
    lane.dataset.trackId = t.id;
    const label = document.createElement("span");
    label.className = "lane-label"; label.textContent = t.id;
    lane.appendChild(label);
    const kindOf = (tid) => ({ V: "video", A: "audio", T: "text" })[tid[0]] || "video";
    for (const c of state.timeline.filter((x) => x.track === t.id)) {
      const el = document.createElement("div");
      const kind = kindOf(t.id);
      el.className = "clip" + (kind !== "video" ? " " + kind : "") + (state.selected === c.id ? " selected" : "");
      el.dataset.id = c.id; el.dataset.track = t.id;
      el.style.left = (c.startMs * state.pxPerMs) + "px";
      el.style.width = Math.max(6, (c.endMs - c.startMs) * state.pxPerMs) + "px";
      el.textContent = `${c.id} ${c.endMs - c.startMs}ms`;
      el.appendChild(Object.assign(document.createElement("span"), { className: "edge edge-l" }));
      el.appendChild(Object.assign(document.createElement("span"), { className: "edge edge-r" }));
      el.addEventListener("mousedown", (e) => onClipMouseDown(e, c, kind, t.id));
      lane.appendChild(el);
    }
    lane.addEventListener("mousedown", (e) => { if (e.target === lane) select(null); });
    tracksEl.appendChild(lane);
  }
  const ph = $("playhead");
  ph.style.left = (state.playheadMs * state.pxPerMs) + "px";
  $("playhead-ms").textContent = Math.round(state.playheadMs);
  $("rev").textContent = state.rev;
}

function findClip(id) {
  const row = state.timeline && state.timeline.find((c) => c.id === id);
  if (!row) return null;
  const track = state.project.tracks.find((t) => t.id === row.track);
  const clip = track && track.clips.find((c) => c.id === id);
  return clip ? { track, clip } : null;
}

function select(id) {
  state.selected = id;
  document.querySelectorAll(".clip.selected").forEach((e) => e.classList.remove("selected"));
  if (!id) { $("sel-info").textContent = "未选中片段"; $("insp-fields").textContent = "选择一个片段"; renderTimeline(); return; }
  const hit = findClip(id);
  if (!hit) return;
  const el = document.querySelector(`.clip[data-id="${id}"]`);
  if (el) el.classList.add("selected");
  $("sel-info").textContent = `选中 ${id}(${hit.track.kind})`;
  $("insp-fields").textContent = `id=${id} track=${hit.track.id} src=${hit.clip.src ?? "-"}`;
  $("insp-start").value = hit.clip.startMs;
  $("insp-dur").value = hit.clip.durationMs;
  $("insp-vol").value = hit.clip.volume ?? 0;
  renderTimeline();
}

/* ---------------- E2 预览(画质代理;一切时间字段来自 timeline_get 投影) ---------------- */

function mediaUrl(src) {
  return `/media?path=${encodeURIComponent(src)}&token=${encodeURIComponent(state.token)}`;
}

/* 壳纯度约定:可见区间/源时间都只读投影字段(startMs/endMs/sourceInMs/speed),
 * 不推导时间线几何;播放头→媒体 currentTime 是播放映射,不是时间线运算。 */
function rowSourceMs(row, t) {
  const spd = row.speed || 1;
  return (row.sourceInMs || 0) + (t - row.startMs) * spd;
}

function rowVisible(row, t) {
  return row.src && t >= row.startMs && t < row.endMs;
}

function rebuildPreviewMedia() {
  const host = $("pv-media");
  host.innerHTML = "";
  state.pvRows = [];
  if (!state.timeline) return;
  const kindOfTrack = (tid) => ({ V: "video", A: "audio", T: "text" })[tid[0]] || "video";
  for (const row of state.timeline) {
    if (!row.src || kindOfTrack(row.track) === "text") continue;
    const isAudioTrack = kindOfTrack(row.track) === "audio";
    const el = document.createElement(isAudioTrack ? "audio" : "video");
    el.preload = "auto";
    // 视频轨元素静音:同源画面/声音常被切成 V+A 双轨, audible 交给音频轨元素,避免双声
    el.muted = !isAudioTrack;
    el.src = mediaUrl(row.src);
    host.appendChild(el);
    state.pvRows.push({ row, el });
  }
}

function syncPreview(forceSeek = false) {
  const t = state.playheadMs;
  for (const { row, el } of state.pvRows) {
    if (!rowVisible(row, t)) {
      if (!el.paused) el.pause();
      continue;
    }
    const want = rowSourceMs(row, t) / 1000;
    const drift = Math.abs(el.currentTime - want);
    if (forceSeek || drift > (state.playing ? 0.12 : 1 / 1000)) {
      try { el.currentTime = want; } catch { /* 元素未就绪,下一帧再试 */ }
    }
    if (state.playing && el.paused) el.play().catch(() => { /* 自动播放被策略拦截:画面仍走 seek 代理 */ });
    if (!state.playing && !el.paused) el.pause();
  }
}

function drawPreview() {
  const cv = $("pv-canvas"), ctx = cv.getContext("2d");
  const p = state.project;
  if (p && (cv.width !== p.canvas.width || cv.height !== p.canvas.height)) {
    cv.width = p.canvas.width; cv.height = p.canvas.height;
  }
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, cv.width, cv.height);
  if (!state.project || !state.timeline) return;
  const t = state.playheadMs;
  const trackOrder = (tid) => state.project.tracks.findIndex((x) => x.id === tid);
  const vis = state.pvRows
    .filter(({ row }) => rowVisible(row, t))
    .filter(({ row }) => (row.track[0] !== "A"))
    .sort((a, b) => trackOrder(a.row.track) - trackOrder(b.row.track));
  for (const { row, el } of vis) {
    ctx.save();
    const ov = row.overlay;
    const pos = row.position;
    const scale = row.scale || 1;
    if (ov) {                      // overlay 矩形(服务端投影字段)
      ctx.globalAlpha = ov.opacity ?? 1;
      ctx.drawImage(el, ov.x, ov.y, ov.w, ov.h);
    } else if (pos || scale !== 1) {
      const w = cv.width * scale, h = cv.height * scale;
      const dx = pos ? (pos.x / 100) * (cv.width - w) : (cv.width - w) / 2;
      const dy = pos ? (pos.y / 100) * (cv.height - h) : (cv.height - h) / 2;
      ctx.drawImage(el, dx, dy, w, h);
    } else {
      ctx.drawImage(el, 0, 0, cv.width, cv.height);
    }
    ctx.restore();
  }
}

function timelineEndMs() {
  if (!state.timeline || !state.timeline.length) return 10000;
  return Math.max(10000, ...state.timeline.map((c) => c.endMs));
}

function pvTick() {
  if (state.playing) {
    state.playheadMs = state.baseMs + (performance.now() - state.t0);
    if (state.playheadMs >= timelineEndMs()) {
      state.playheadMs = timelineEndMs();
      setPlaying(false);
    }
    renderTimeline();
  }
  drawPreview();
  const now = performance.now();
  if (state.playing && now - state.pvDrift > 250) { state.pvDrift = now; syncPreview(false); }
  const sec = state.playheadMs / 1000;
  $("pv-time").textContent = `${sec.toFixed(3)}s${state.playing ? " ▶" : ""}`;
  requestAnimationFrame(pvTick);
}

function setPlaying(on) {
  if (on && !state.playing) { state.baseMs = state.playheadMs; state.t0 = performance.now(); }
  if (!on && state.playing) state.playheadMs = state.baseMs + (performance.now() - state.t0);
  state.playing = on;
  $("pv-play").textContent = on ? "⏸ 暂停" : "▶ 播放";
  syncPreview(true);
  if (!on) renderTimeline();
}

function seekTo(ms) {
  const clamped = Math.max(0, Math.min(ms, timelineEndMs()));
  if (state.playing) { state.baseMs = clamped; state.t0 = performance.now(); }
  state.playheadMs = clamped;
  syncPreview(true);
  renderTimeline();
}

async function refreshExportFiles() {
  const env = await api("render_probe");
  if (!env.ok) return;
  const files = env.data.files || [];
  $("exp-files").innerHTML = files.length
    ? files.map((f) => `<div class="row"><span class="oid">${escapeHtml(String(f.file))}</span><span class="bd">${f.bytes} B</span></div>`).join("")
    : "(暂无产物)";
}

/* ---------------- E5 导出(cutforge 后端走 render_run/render_progress 异步轮询) ---------------- */

let expPoll = null;

async function runExport() {
  const backend = $("exp-backend").value;
  const prog = $("exp-progress");
  prog.textContent = "提交中…";
  if (backend === "cutforge") {
    const ass = state.project && state.project.subtitle && state.project.subtitle.ass;
    const r = await api("render_run", { backend: "cutforge", ...(ass ? { ass } : {}) });
    if (!r.ok) { prog.textContent = `提交失败:${r.code}`; return; }
    const runId = r.data.runId;
    prog.textContent = "渲染中…";
    if (expPoll) clearInterval(expPoll);
    expPoll = setInterval(async () => {
      const s = await api("render_progress", { runId });
      if (!s.ok) return;
      const tail = (s.data.lines || []).slice(-3).join("\n");
      prog.textContent = `[${s.data.state}] ${tail}`;
      if (s.data.state !== "running") {
        clearInterval(expPoll); expPoll = null;
        if (s.data.state === "ok") { prog.textContent = `完成:${s.data.output}`; await refreshExportFiles(); }
        else prog.textContent = `失败:${s.data.error || "见服务端日志"}`;
      }
    }, 800);
  } else {
    const ratio = $("exp-ratio").value;
    const r = await api("render", { backend: "ffmpeg", ratio,
      scriptArgs: ["05_ir/project.json", "--ratio", ratio, "--profile", "final"] });
    prog.textContent = r.ok ? "完成(CutFlow rs_render)" : `失败:${r.code} ${r.message}`;
    if (r.ok) await refreshExportFiles();
  }
}

/* ---------------- 手势(全部落为 MCP 工具调用) ---------------- */

function onClipMouseDown(e, clip, kind, trackId) {
  e.stopPropagation();
  select(clip.id);
  const el = e.currentTarget;
  const edge = e.target.classList.contains("edge-l") ? "l" : e.target.classList.contains("edge-r") ? "r" : null;
  const x0 = e.clientX;
  const orig = { start: clip.startMs, dur: clip.durationMs, srcIn: clip.sourceInMs ?? 0 };
  let lastApplied = false;
  const onMove = (ev) => {
    const dMs = (ev.clientX - x0) / state.pxPerMs;
    let want = null;
    if (edge === "l") {
      const ns = Math.max(0, snap(orig.start + dMs));
      if (ns < orig.start + orig.dur && ns !== orig.start) want = { startMs: ns, durationMs: orig.dur - (ns - orig.start), sourceInMs: orig.srcIn + (ns - orig.start) };
    } else if (edge === "r") {
      const nd = snap(orig.dur + dMs);
      if (nd > 0 && nd !== orig.dur) want = { durationMs: nd };
    } else {
      const ns = Math.max(0, snap(orig.start + dMs));
      if (ns !== orig.start) want = { startMs: ns };
    }
    if (want) { want.__changed = JSON.stringify(want) !== lastApplied; lastApplied = JSON.stringify(want); }
  };
  const onUp = async (ev) => {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
    const dMs = (ev.clientX - x0) / state.pxPerMs;
    if (edge === "l") {
      const ns = Math.max(0, snap(orig.start + dMs));
      if (ns !== orig.start && ns < orig.start + orig.dur) {
        await api("clip_update", { clipId: clip.id, patch: { startMs: ns, durationMs: orig.dur - (ns - orig.start), sourceInMs: orig.srcIn + (ns - orig.start) } });
        await refresh();
      }
    } else if (edge === "r") {
      const nd = snap(orig.dur + dMs);
      if (nd > 0 && nd !== orig.dur) { await api("clip_update", { clipId: clip.id, patch: { durationMs: nd } }); await refresh(); }
    } else {
      const ns = Math.max(0, snap(orig.start + dMs));
      if (ns !== orig.start) {
        const r = await api("clip_move", { clipId: clip.id, startMs: ns });
        if (r.ok) await refresh();
      }
    }
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
}

async function rippleDelete(clipId) {
  // 波纹删:删除后,同轨后继片段整体左移被删时长(每步都是 clip_move Op)
  const hit = findClip(clipId);
  if (!hit) return;
  const row = state.timeline.find((c) => c.id === clipId);
  if (!row) return;
  const end = row.endMs;
  const del = end - hit.clip.startMs;
  const followers = hit.track.clips.filter((c) => c.startMs >= end).map((c) => c.id);
  const r = await api("clip_delete", { clipId });
  if (!r.ok) return;
  for (const fid of followers) {
    const f = findClip(fid);
    if (f) await api("clip_move", { clipId: fid, startMs: f.clip.startMs - del });
  }
  await refresh();
}

async function doSplit() {
  if (!state.selected) return status("先选中片段再分割", false);
  const hit = findClip(state.selected);
  if (!hit) return;
  const { clip } = hit;
  const row = state.timeline.find((c) => c.id === clip.id);
  if (!row) return;
  if (state.playheadMs <= clip.startMs || state.playheadMs >= row.endMs)
    return status("播放头不在选中片段内部", false);
  await api("clip_split", { clipId: clip.id, tMs: snap(state.playheadMs) });
  await refresh();
}

/* ---------------- 数据刷新与事件 ---------------- */

async function refresh() {
  const env = await api("project_get");
  if (env.ok) {
    state.project = env.data.project;
    state.rev = env.data.rev;
  }
  const tl = await api("timeline_get");
  if (tl.ok) state.timeline = tl.data.clips;
  renderTimeline();
  rebuildPreviewMedia();
  syncPreview(true);
  if ($("tab-notes").classList.contains("active")) await refreshNotes();
  if ($("tab-diff").classList.contains("active")) await refreshDiff();
  if ($("tab-conflicts").classList.contains("active")) await refreshConflicts();
}

async function pollEvents() {
  try {
    const r = await fetch(`/events?since=${state.eventSeq}`, { headers: { "Authorization": `Bearer ${state.token}` } });
    const v = await r.json();
    if (v.seq) state.eventSeq = v.seq;
    if (v.event === "workspace.changed") await refresh();
  } catch { /* 服务暂不可达,继续轮询 */ }
  setTimeout(pollEvents, 300);
}

async function refreshNotes() {
  const env = await api("notes_list");
  if (!env.ok) return;
  const open = (env.data.notes || []).filter((n) => n.state === "open");
  const orphans = env.data.orphans || 0;
  $("notes-open").innerHTML = "";
  for (const n of open) {
    const row = document.createElement("div");
    row.className = "row";
    row.innerHTML = `<span class="badge open">${n.id}</span><span class="bd">${escapeHtml(n.body)}
      <input placeholder="回执 reply" style="width:220px"><input placeholder="opIds(op-1,op-2)" style="width:180px">
      <button>结案</button></span>`;
    row.querySelector("button").addEventListener("click", async () => {
      const [reply, opIds] = [row.querySelectorAll("input")[0].value, row.querySelectorAll("input")[1].value];
      await api("notes_resolve", { noteId: n.id, reply, opIds: opIds.split(",").map((s) => s.trim()).filter(Boolean) });
      await refreshNotes();
    });
    $("notes-open").appendChild(row);
  }
  if (!open.length) $("notes-open").textContent = "(无)";
  $("notes-orphans").textContent = orphans ? `${orphans} 条孤儿(见 notes_list)` : "(无)";
}

async function refreshDiff() {
  const actor = $("diff-actor").value || undefined;
  const limit = Number($("diff-limit").value) || 50;
  const env = await api("oplog_tail", { limit, ...(actor ? { actor } : {}) });
  if (!env.ok) return;
  $("diff-rows").innerHTML = "";
  for (const op of env.data.ops || []) {
    const row = document.createElement("div");
    row.className = "row";
    row.innerHTML = `<input type="checkbox" class="pick">
      <span class="oid">${op.opId}</span><span class="badge ${op.actor.kind}">${op.actor.kind}</span>
      <span class="bd">${escapeHtml(op.summary)}<div class="delta">${escapeHtml(JSON.stringify(op.before).slice(0, 80))} → ${escapeHtml(JSON.stringify(op.after).slice(0, 80))}</div></span>`;
    $("diff-rows").appendChild(row);
  }
}

async function refreshConflicts() {
  const env = await api("conflict_list");
  if (!env.ok) return;
  const rows = env.data.conflicts || [];
  $("conflict-rows").innerHTML = rows.length
    ? rows.map((c) => `<div class="row"><span class="badge">${c.code}</span><span class="bd">${c.conflictId} @ ${c.pointer}</span></div>`).join("")
    : "(无冲突)";
}

function escapeHtml(s) { return String(s).replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[ch])); }

/* ---------------- 启动 ---------------- */

function bindUI() {
  for (const b of document.querySelectorAll("#tabs button")) {
    b.addEventListener("click", () => {
      document.querySelectorAll("#tabs button").forEach((x) => x.classList.remove("active"));
      document.querySelectorAll(".tab").forEach((x) => x.classList.remove("active"));
      b.classList.add("active");
      $("tab-" + b.dataset.tab).classList.add("active");
      if (b.dataset.tab === "notes") refreshNotes();
      if (b.dataset.tab === "diff") refreshDiff();
      if (b.dataset.tab === "conflicts") refreshConflicts();
    });
  }
  $("ruler").addEventListener("mousedown", (e) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const seekFromEvent = (ev) => {
      const ms = Math.max(0, snap((ev.clientX - rect.left) / state.pxPerMs));
      state.playheadMs = ms;          // 时间线显示走原路径
      seekTo(ms);                     // E2-5:标尺点击/拖拽 → 预览与播放头双向联动
    };
    seekFromEvent(e);
    const onMove = (ev) => { if (e.buttons & 1) seekFromEvent(ev); };
    const onUp = () => { document.removeEventListener("mousemove", onMove); document.removeEventListener("mouseup", onUp); };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  });
  $("btn-undo").addEventListener("click", async () => { await api("undo", {}); await refresh(); });
  $("btn-redo").addEventListener("click", async () => { await api("redo", {}); await refresh(); });
  $("insp-apply").addEventListener("click", async () => {
    if (!state.selected) return;
    const patch = {};
    const s = Number($("insp-start").value), d = Number($("insp-dur").value), v = Number($("insp-vol").value);
    const hit = findClip(state.selected);
    if (hit && s !== hit.clip.startMs) patch.startMs = s;
    if (hit && d !== hit.clip.durationMs) patch.durationMs = d;
    if (hit && v !== (hit.clip.volume ?? 0)) patch.volume = v;
    if (Object.keys(patch).length) { await api("clip_update", { clipId: state.selected, patch }); await refresh(); }
  });
  $("insp-dup").addEventListener("click", async () => {
    if (!state.selected) return;
    await api("clip_duplicate", { clipId: state.selected, startMs: snap(state.playheadMs) });
    await refresh();
  });
  $("track-add-video").addEventListener("click", async () => { await api("track_add", { kind: "video" }); await refresh(); });
  $("track-add-audio").addEventListener("click", async () => { await api("track_add", { kind: "audio" }); await refresh(); });
  $("note-create").addEventListener("click", async () => {
    const body = $("note-body").value.trim();
    if (!body) return status("标注正文必填", false);
    const anchor = state.selected
      ? { kind: "clip", ref: state.selected, tMs: state.playheadMs }
      : { kind: "time", tMs: state.playheadMs };
    await api("notes_add", { anchor, body, author: "user" });
    $("note-body").value = "";
    await refreshNotes();
  });
  $("diff-refresh").addEventListener("click", refreshDiff);
  // ---- E2 预览传输控制 / E5 导出面板 ----
  $("pv-play").addEventListener("click", () => setPlaying(!state.playing));
  $("pv-frame-prev").addEventListener("click", () => seekTo(state.playheadMs - frameMs()));
  $("pv-frame-next").addEventListener("click", () => seekTo(state.playheadMs + frameMs()));
  $("exp-run").addEventListener("click", runExport);
  document.addEventListener("keydown", async (e) => {
    if (["INPUT", "TEXTAREA", "SELECT"].includes(e.target.tagName)) return;
    if (e.key === " ") {
      e.preventDefault();
      setPlaying(!state.playing);
    } else if (e.key === "ArrowLeft") { e.preventDefault(); seekTo(state.playheadMs - frameMs()); }
    else if (e.key === "ArrowRight") { e.preventDefault(); seekTo(state.playheadMs + frameMs()); }
    else if (e.key === "s" || e.key === "S") { await doSplit(); }
    else if (e.key === "Delete") {
      if (!state.selected) return;
      if (e.shiftKey || $("ripple").checked) await rippleDelete(state.selected);
      else { await api("clip_delete", { clipId: state.selected }); state.selected = null; await refresh(); }
    }
    else if ((e.ctrlKey || e.metaKey) && e.key === "z") { await api("undo", {}); await refresh(); }
    else if ((e.ctrlKey || e.metaKey) && (e.key === "y" || e.shiftKey)) { await api("redo", {}); await refresh(); }
    else if ((e.ctrlKey || e.metaKey) && e.key === "c") { state.clipboard = state.selected; }
    else if ((e.ctrlKey || e.metaKey) && e.key === "v") {
      if (state.clipboard) { await api("clip_duplicate", { clipId: state.clipboard, startMs: snap(state.playheadMs) }); await refresh(); }
    }
  });
  $("diff-undo-batch").addEventListener("click", async () => {
    const n = document.querySelectorAll("#diff-rows .pick:checked").length;
    if (!n) return status("先勾选要撤销的 Op 行", false);
    await api("undo", { batch: n });
    await refresh();
  });
}

async function boot() {
  const params = new URLSearchParams(location.search);
  state.token = params.get("token") || "";
  const sess = await fetch("/session", { headers: { "Authorization": `Bearer ${state.token}` } }).then((r) => r.json());
  state.root = sess.root;
  $("session-info").textContent = `root=${sess.root}`;
  bindUI();
  await refresh();
  await refreshExportFiles();
  requestAnimationFrame(pvTick);
  pollEvents();
}

boot();
