/* CutForge Web 编辑器(壳不持有真相:查询走投影,变更走 MCP 工具 → Op)。
 * 壳纯度纪律(check-shell-purity):禁 node/fs/子进程;时间语义一律来自内核投影。
 * 阶段二:素材面板(E3-4)、分组检查器(E4,字段集来自 GET /ui-fields 单一真相源)、
 * 新建向导(B11-2)、toast/横幅/空态置灰(E8)、token 变更提示(E6-2)。 */
"use strict";

const $ = (id) => document.getElementById(id);
const state = {
  root: "", token: "", rev: 0, project: null, timeline: null,
  selected: null, playheadMs: 0, pxPerMs: 0.06, clipboard: null, eventSeq: 0,
  // E2 预览:playing=播放中;baseMs/t0=播放起点(挂钟推算播放头)
  playing: false, baseMs: 0, t0: 0,
  pvRows: [],          // [{row, el}] —— 每个带 src 的时间线行对应一个隐藏媒体元素
  pvDrift: 0,          // 上次漂移校正时刻(节流)
  uiFields: null,      // E4-2:GET /ui-fields 下发的可编辑字段分组(单一真相源)
  media: [],           // E3-4:素材面板当前列表
  conflicts: 0,        // E8:未裁决冲突数(>0 顶部停写横幅)
};

/* ---------------- 状态反馈(E8:toast 顶替单行 footer;#status 保留作诊断锚点) */

function status(msg, ok = true) {
  $("status").textContent = msg;
  const box = $("toasts");
  const t = document.createElement("div");
  t.className = "toast" + (ok ? "" : " err");
  t.textContent = msg;
  box.appendChild(t);
  while (box.children.length > 4) box.removeChild(box.firstChild);
  setTimeout(() => t.remove(), 4000);
}

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
    lane.dataset.kind = t.kind;
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
    // E3-4:素材可拖放到任意轨道落点(dragover/drop;落点换算只做像素→ms 显示映射)
    lane.addEventListener("dragover", (e) => { e.preventDefault(); lane.classList.add("drop-hint"); });
    lane.addEventListener("dragleave", () => lane.classList.remove("drop-hint"));
    lane.addEventListener("drop", (e) => {
      e.preventDefault();
      lane.classList.remove("drop-hint");
      const src = e.dataTransfer.getData("text/cutforge-media");
      if (!src) return;
      const rect = lane.getBoundingClientRect();
      const at = snap(Math.max(0, (e.clientX - rect.left) / state.pxPerMs));
      insertMedia(src, t.id, at);
    });
    tracksEl.appendChild(lane);
  }
  const ph = $("playhead");
  ph.style.left = (state.playheadMs * state.pxPerMs) + "px";
  $("playhead-ms").textContent = Math.round(state.playheadMs);
  $("rev").textContent = state.rev;
  $("tb-time").textContent = Math.round(state.playheadMs) + "ms";
}

function findClip(id) {
  const row = state.timeline && state.timeline.find((c) => c.id === id);
  if (!row) return null;
  const track = state.project.tracks.find((t) => t.id === row.track);
  const clip = track && track.clips.find((c) => c.id === id);
  return clip ? { track, clip } : null;
}

/* ---------------- E4 分组检查器(字段集来自 /ui-fields 单一真相源) */

// 展示元数据:仅输入控件形态(min/step 等),字段全集与分组以 ui-fields 为准
const FIELD_META = {
  startMs:     { type: "number", step: 1, min: 0, legacy: "insp-start" },
  durationMs:  { type: "number", step: 1, min: 1, legacy: "insp-dur" },
  sourceInMs:  { type: "number", step: 1, min: 0 },
  speed:       { type: "number", step: 0.05, min: 0.25, max: 4 },
  volume:      { type: "number", step: 0.1, min: 0, max: 2, legacy: "insp-vol" },
  opacity:     { type: "number", step: 0.05, min: 0, max: 1 },
  scale:       { type: "number", step: 0.05, min: 0 },
  text:        { type: "text" },
  freezeMs:    { type: "number", step: 1, min: 0 },
};

function buildInspectorGroups() {
  const host = $("insp-groups");
  host.innerHTML = "";
  if (!state.uiFields || !state.uiFields.editable) return;
  for (const [group, fields] of Object.entries(state.uiFields.editable)) {
    const box = document.createElement("fieldset");
    box.className = "insp-group";
    const legend = document.createElement("legend");
    legend.textContent = group;
    box.appendChild(legend);
    for (const f of fields) {
      const meta = FIELD_META[f] || { type: "text" };
      const label = document.createElement("label");
      label.textContent = f;
      const input = document.createElement("input");
      input.type = meta.type;
      if (meta.step !== undefined) input.step = meta.step;
      if (meta.min !== undefined) input.min = meta.min;
      if (meta.max !== undefined) input.max = meta.max;
      // 兼容既有 e2e/手势锚点:基础三字段沿用原 id
      input.id = meta.legacy || ("insp-f-" + f);
      input.dataset.field = f;
      label.appendChild(input);
      box.appendChild(label);
    }
    host.appendChild(box);
  }
}

// 只读展示(E4-3):投影里有、ClipPatch 未承接的字段(转场/位置/淡入淡出等)
function renderReadonly(hit) {
  const box = $("insp-readonly");
  const ro = (state.uiFields && state.uiFields.readonly) || [];
  const vals = [];
  for (const f of ro) {
    const v = hit.clip[f];
    if (v === undefined || v === null) continue;
    vals.push(`${f}=${JSON.stringify(v)}`);
  }
  if (!vals.length) { box.hidden = true; box.textContent = ""; return; }
  box.hidden = false;
  box.textContent = "只读(内核 ClipPatch 暂未承接,E4-3):" + vals.join("  ");
}

function setInspectEnabled(on) {
  $("insp-apply").disabled = !on;
  $("insp-dup").disabled = !on;
}

function select(id) {
  state.selected = id;
  document.querySelectorAll(".clip.selected").forEach((e) => e.classList.remove("selected"));
  if (!id) {
    $("sel-info").textContent = "未选中片段:点击时间线片段,或从左侧素材面板双击插入(S 分割 / Del 删除)";
    $("insp-fields").textContent = "未选中片段:点击时间线片段,或从左侧素材面板双击插入";
    document.querySelectorAll("#insp-groups input").forEach((i) => { i.value = ""; });
    $("insp-readonly").hidden = true;
    setInspectEnabled(false);
    renderTimeline();
    return;
  }
  const hit = findClip(id);
  if (!hit) return;
  const el = document.querySelector(`.clip[data-id="${id}"]`);
  if (el) el.classList.add("selected");
  $("sel-info").textContent = `选中 ${id}(${hit.track.kind})`;
  $("insp-fields").textContent = `id=${id} track=${hit.track.id} src=${hit.clip.src ?? "-"}`;
  for (const input of document.querySelectorAll("#insp-groups input")) {
    const f = input.dataset.field;
    const v = hit.clip[f];
    input.value = (v === undefined || v === null) ? "" : v;
  }
  renderReadonly(hit);
  setInspectEnabled(true);
  renderTimeline();
}

async function applyInspector() {
  if (!state.selected) return;
  const hit = findClip(state.selected);
  if (!hit) return;
  const patch = {};
  for (const input of document.querySelectorAll("#insp-groups input")) {
    const f = input.dataset.field;
    if (input.value === "") continue;
    const meta = FIELD_META[f] || {};
    const v = meta.type === "number" ? Number(input.value) : input.value;
    if (Number.isNaN(v)) continue;
    const cur = hit.clip[f];
    const curN = (cur === undefined || cur === null) ? null : cur;
    if (meta.type === "number" ? Number(v) !== Number(curN ?? NaN) : String(v) !== String(curN ?? "")) {
      patch[f] = v;
    }
  }
  if (!Object.keys(patch).length) { status("无改动"); return; }
  const r = await api("clip_update", { clipId: state.selected, patch });
  if (r.ok) status(`已应用 ${Object.keys(patch).join("/")}(可撤销)`);
  await refresh();
}

/* ---------------- E3-4 素材面板 ---------------- */

function kindOfMedia(k) { return k === "audio" ? "audio" : (k === "image" ? "video" : k); }

async function refreshMedia() {
  const dir = $("media-dir").value.trim();
  const env = await api("media_browse", dir ? { dir } : {});
  const list = $("media-list");
  if (!env.ok) { list.innerHTML = `<div class="empty-hint">${escapeHtml(env.message)}</div>`; return; }
  state.media = env.data.files || [];
  list.innerHTML = "";
  if (!state.media.length) {
    list.innerHTML = `<div class="empty-hint">(${escapeHtml(env.data.dir || "工程根")}) 无可导入媒体;把 mp4/mp3/png 等放进该目录后点 ⟳</div>`;
    return;
  }
  for (const f of state.media) {
    const row = document.createElement("div");
    row.className = "media-item";
    row.draggable = true;
    row.title = `${f.path}${f.durationMs ? ` · ${f.durationMs}ms` : ""}`;
    row.innerHTML = `<span class="badge">${f.kind}</span><span class="bd">${escapeHtml(f.name)}${
      f.durationMs ? ` <span class="dim">${(f.durationMs / 1000).toFixed(1)}s</span>` : ""}</span>`;
    row.addEventListener("dblclick", () => insertMedia(f.path, targetTrackFor(f.kind), snap(state.playheadMs)));
    row.addEventListener("dragstart", (e) => e.dataTransfer.setData("text/cutforge-media", f.path));
    list.appendChild(row);
  }
}

function targetTrackFor(kind) {
  const want = kindOfMedia(kind);
  const tracks = (state.project && state.project.tracks) || [];
  return tracks.find((t) => t.kind === want)?.id
      || tracks.find((t) => t.kind === "video")?.id
      || tracks[0]?.id || null;
}

async function insertMedia(src, trackId, startMs) {
  if (!trackId) { status("工程没有轨道:先点「+视频轨」再插入素材", false); return; }
  // durationMs 优先用浏览元信息(避免无谓的二次探测);缺省由服务端 ffprobe 自动填
  const item = state.media.find((m) => m.path === src);
  const args = { trackId, src, startMs, requestId: `ui-${Date.now()}-${Math.random().toString(36).slice(2, 8)}` };
  if (item && item.durationMs) args.durationMs = item.durationMs;
  const r = await api("clip_add", args);
  if (!r.ok) return;
  status(`已插入 ${src.split("/").pop()} → ${trackId}@${startMs}ms`);
  await refresh();
  const clips = state.timeline.filter((c) => c.track === trackId);
  const mine = clips[clips.length - 1];
  if (mine) select(mine.id);
}

/* ---------------- E2 预览(画质代理;一切时间字段来自 timeline_get 投影) */

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

/* ---------------- E5 导出(cutforge 后端走 render_run/render_progress 异步轮询) */

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
    // 目录契约 0.5:工程相对路径由 /session 下发(projectRel),壳不得硬编码目录名
    const projectRel = state.projectRel || "05_时间线工程/project.json";
    const r = await api("render", { backend: "ffmpeg", ratio,
      scriptArgs: [projectRel, "--ratio", ratio, "--profile", "final"] });
    prog.textContent = r.ok ? "完成(CutFlow rs_render)" : `失败:${r.code} ${r.message}`;
    if (r.ok) await refreshExportFiles();
  }
}

/* ---------------- 手势(全部落为 MCP 工具调用) */

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
  if (!state.selected) { status("先选中片段再分割(点时间线上的片段)", false); return; }
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
  await refreshConflictBanner();
  if ($("tab-notes").classList.contains("active")) await refreshNotes();
  if ($("tab-diff").classList.contains("active")) await refreshDiff();
  if ($("tab-conflicts").classList.contains("active")) await refreshConflicts();
}

/* E8:冲突未裁决 → 顶部固定停写横幅(写操作会在服务端被停写,这里只提示) */
async function refreshConflictBanner() {
  const env = await api("conflict_list");
  const n = env.ok ? (env.data.conflicts || []).length : 0;
  state.conflicts = n;
  const b = $("conflict-banner");
  b.hidden = n === 0;
  if (n) b.textContent = `⛔ 存在 ${n} 项未裁决冲突,已停写(到「冲突」页查看;裁决后删除 .cutforge/conflicts/*.json)`;
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

/* ---------------- B11-2 新建工程向导 ---------------- */

function openWizard() {
  $("wizard").hidden = false;
  $("wiz-name").value = "";
  $("wiz-name").focus();
}

async function createProject() {
  const name = $("wiz-name").value.trim();
  if (!name || /[\\/:*?"<>|]/.test(name)) { status("工程名必填,且不能含路径非法字符", false); return; }
  const [w, h] = $("wiz-ratio").value.split("x").map(Number);
  const tracks = [];
  if ($("wiz-tr-v").checked) tracks.push("video");
  if ($("wiz-tr-a").checked) tracks.push("audio");
  if ($("wiz-tr-t").checked) tracks.push("text");
  if (!tracks.length) { status("至少勾选一条初始轨道", false); return; }
  // 新工程目录 = 当前工程根的同名父目录下(壳只做字符串拼接,创建由服务端完成)
  const sep = state.root.includes("\\") ? "\\" : "/";
  const parts = state.root.replace(/[\\/]+$/, "").split(/[\\/]/);
  parts.pop();
  const target = parts.join(sep) + sep + name;
  const r = await api("project_new", {
    root: target, slug: name, fps: Number($("wiz-fps").value), canvasW: w, canvasH: h, tracks,
  });
  if (!r.ok) return;
  $("wizard").hidden = true;
  const hint = `已创建 ${r.data.project}。新工程需单独启动服务:cutforge-cli serve "${target}" --open`;
  status(hint);
  $("token-banner").hidden = false;
  $("token-banner").textContent = "✅ " + hint;
}

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
  $("pv-to-start").addEventListener("click", () => seekTo(0));
  $("pv-to-end").addEventListener("click", () => seekTo(timelineEndMs()));
  $("insp-apply").addEventListener("click", applyInspector);
  $("insp-dup").addEventListener("click", async () => {
    if (!state.selected) return;
    await api("clip_duplicate", { clipId: state.selected, startMs: snap(state.playheadMs) });
    await refresh();
  });
  $("track-add-video").addEventListener("click", async () => { await api("track_add", { kind: "video" }); await refresh(); });
  $("track-add-audio").addEventListener("click", async () => { await api("track_add", { kind: "audio" }); await refresh(); });
  $("track-add-text").addEventListener("click", async () => { await api("track_add", { kind: "text" }); await refresh(); });
  // ---- E3-4 素材面板 ----
  $("media-refresh").addEventListener("click", refreshMedia);
  $("media-dir").addEventListener("keydown", (e) => { if (e.key === "Enter") refreshMedia(); });
  // ---- B11-2 新建向导 ----
  $("btn-project-new").addEventListener("click", openWizard);
  $("wiz-cancel").addEventListener("click", () => { $("wizard").hidden = true; });
  $("wiz-create").addEventListener("click", createProject);
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
    else if (e.key === "Home") { e.preventDefault(); seekTo(0); }
    else if (e.key === "End") { e.preventDefault(); seekTo(timelineEndMs()); }
    else if (e.key === "s" || e.key === "S") { await doSplit(); }
    else if (e.key === "Delete") {
      if (!state.selected) return;
      if (e.shiftKey || $("ripple").checked) await rippleDelete(state.selected);
      else { await api("clip_delete", { clipId: state.selected }); state.selected = null; await refresh(); }
    }
    else if ((e.ctrlKey || e.metaKey) && e.key === "z") { await api("undo", {}); await refresh(); }
    else if ((e.ctrlKey || e.metaKey) && (e.key === "y" || (e.shiftKey && e.key === "Z"))) { await api("redo", {}); await refresh(); }
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
  let sess;
  try {
    sess = await fetch("/session", { headers: { "Authorization": `Bearer ${state.token}` } }).then((r) => {
      if (!r.ok) throw new Error(String(r.status));
      return r.json();
    });
  } catch {
    // E6-2:token 已随服务重启更换(或缺失)——旧链接无法过数据面鉴权
    $("token-banner").hidden = false;
    $("token-banner").textContent = "⚠ token 已变更或缺失(服务重启会换新 token):请回到服务窗口复制最新链接重新打开本页。";
    status("token 鉴权失败", false);
    return;
  }
  state.root = sess.root;
  // 目录契约 0.5:project.json 相对路径(新布局中文目录;0.4.x 旧工程回退 05_ir/)
  state.projectRel = sess.projectRel || "05_时间线工程/project.json";
  if (sess.token && sess.token !== state.token) {
    // 正常经 URL 进入时二者相等;不等说明链接与令牌不一致,提示刷新
    $("token-banner").hidden = false;
    $("token-banner").textContent = "⚠ 本页 token 与服务当前 token 不一致,请改用服务窗口打印的最新链接。";
  }
  $("session-info").textContent = `root=${sess.root}`;
  bindUI();
  // E4-2:检查器字段分组由服务端单一真相源下发(壳不读文件)
  try {
    const uf = await fetch("/ui-fields", { headers: { "Authorization": `Bearer ${state.token}` } });
    if (uf.ok) {
      state.uiFields = await uf.json();
      buildInspectorGroups();
    } else {
      status("ui-fields 下发失败:检查器退化为空分组", false);
    }
  } catch { status("ui-fields 下发异常", false); }
  await refresh();
  await refreshExportFiles();
  await refreshMedia();
  requestAnimationFrame(pvTick);
  pollEvents();
}

boot();
