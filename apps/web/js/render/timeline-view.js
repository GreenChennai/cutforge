/* 时间线视图(T2.4,修 W2/W6):DOM 交互层的 keyed reconciliation。
 *
 * - 以稳定 id(轨道 id / 片段 id)做「新增/删除/移动/更新」四类 patch,只动变化节点;
 *   彻底删除旧壳「innerHTML 全量清空重建」(AC-2.3:一次 clip 移动只碰相关节点);
 * - 视口外片段不渲染(virtualizer,1k clips 只渲染视口内);
 * - 播放头推进绝不进入本模块(播放循环只碰 transform/文本/canvas,ADR-0012);
 * - ghost(ephemeral.dragGhost)是临时投影层(ADR-0013):投影到达即清除重画。
 */
import { $, h } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore, ephemeralStore } from "../core/store.js";
import { PX_PER_MS, clipKindOf, clipLabelOf, timelineEndMsOf } from "../core/model.js";
import { createVirtualizer } from "./virtualizer.js";
import { setRulerContent, drawRuler } from "./ruler.js";
import { setPlayheadPx, drawOverlay } from "./playhead.js";
import { drawWaveIfAudio } from "./waveform.js";
import { svgUse } from "../../assets/icons.js";
import { onClipMouseDown, onLaneMouseDown, onLaneDragOver, onLaneDragLeave, onLaneDrop } from "./gestures.js";
import { openClipContextMenu } from "../ui/menu.js";

/** @type {Map<string, HTMLElement>} */
const laneEls = new Map();
/** @type {Map<string, {el: HTMLElement, left: number, width: number, text: string, cls: string, track: string, selected: boolean}>} */
const clipEls = new Map();
let ghostEl = null;
let ghostKey = "";
/** @type {ReturnType<typeof createVirtualizer>|null} */
let virt = null;

export function mountTimeline() {
  virt = createVirtualizer($("timeline-wrap"));
  virt.onChange(() => renderTimelineView());
  return virt;
}

export function virtualizerOf() {
  return virt;
}

/** 滚动/投影/ephemeral 变化的统一重算入口(增量;只碰变化节点)。 */
export function renderTimelineView() {
  const project = projectStore.get().project;
  const clips = timelineStore.get().clips;
  if (!project || !clips) return;
  const end = timelineEndMsOf(clips);
  const contentW = Math.ceil(end * PX_PER_MS) + 40;
  setRulerContent(contentW);

  const wrap = $("timeline-wrap");
  const win = virt ? virt.window() : { t0: 0, t1: (wrap.clientWidth || 1200) / PX_PER_MS, scrollLeft: wrap.scrollLeft };
  renderLanes(project.tracks, contentW);
  renderClips(clips, win);
  renderGhost();
  const phMs = playheadMsCache();
  drawRuler(phMs, win.scrollLeft);
  setPlayheadPx(phMs * PX_PER_MS);
  drawOverlay(ephemeralStore.get().snapMs);
}

/** 选中态变化只碰两枚 class(keyed 更新的最廉价路径)。 */
export function updateSelectionView() {
  const sel = selectionStore.get().clipId;
  for (const [id, meta] of clipEls) {
    const want = id === sel;
    if (meta.selected !== want) {
      meta.selected = want;
      meta.el.classList.toggle("selected", want);
    }
  }
}

function playheadMsCache() {
  const sel = selectionStore.get();
  return sel.playheadMs || 0;
}

/* ---------------- 轨道行(keyed by track.id)---------------- */

function renderLanes(tracks, contentW) {
  const host = $("tracks");
  const want = new Set(tracks.map((t) => t.id));
  for (const [id, el] of [...laneEls]) {
    if (!want.has(id)) {
      el.remove();
      laneEls.delete(id);
    }
  }
  let prev = null;
  for (const t of tracks) {
    let lane = laneEls.get(t.id);
    if (!lane) {
      lane = createLane(t);
      laneEls.set(t.id, lane);
    }
    // 顺序维护:追加到正确位置(轨道数少,直接序插)
    if (prev) {
      if (prev.nextElementSibling !== lane) prev.after(lane);
    } else if (host.firstElementChild !== lane) {
      host.insertBefore(lane, host.firstElementChild);
    }
    prev = lane;
    lane.style.width = `${contentW}px`;
    const eye = lane.querySelector(".lane-eye");
    if (eye) {
      const hidden = ephemeralStore.get().hiddenTracks.includes(t.id);
      eye.setAttribute("aria-pressed", hidden ? "false" : "true");
      lane.classList.toggle("hidden-by-user", hidden);
    }
  }
}

function createLane(t) {
  const kind = t.kind || clipKindOf({ track: t.id });
  const label = h("span", { class: "lane-label", testid: `lane-label-${t.id}` }, [
    h("span", { class: "lane-kind" }, [t.id]),
    h("button", {
      class: "lane-eye", testid: `track-visibility-${t.id}`,
      "aria-pressed": "true", "aria-label": `切换 ${t.id} 轨显示(仅视图,不落盘)`,
      "data-tip": "眼睛开关:仅隐藏视图(ephemeral,不落盘/不参与撤销)",
      onclick: (e) => {
        e.stopPropagation();
        toggleTrackVisible(t.id);
      },
    }, [svgUse("icon-eye")]),
    h("span", { class: "badge", "data-tip": "轨型" }, [kind]),
  ]);
  const lane = h("div", {
    class: "track", dataset: { trackId: t.id, kind },
    testid: `track-lane-${t.id}`,
  }, [label]);
  lane.addEventListener("mousedown", onLaneMouseDown);
  lane.addEventListener("dragover", onLaneDragOver);
  lane.addEventListener("dragleave", onLaneDragLeave);
  lane.addEventListener("drop", onLaneDrop);
  return lane;
}

/** 轨头眼睛开关(ephemeral.hiddenTracks;不进 IR/不落盘/不参与撤销,ADR-0013)。 */
export function toggleTrackVisible(trackId) {
  const cur = ephemeralStore.get().hiddenTracks;
  const next = cur.includes(trackId) ? cur.filter((x) => x !== trackId) : [...cur, trackId];
  ephemeralStore.set({ hiddenTracks: next });
  renderTimelineView();
}

/* ---------------- 片段块(keyed by clip.id,四类 patch)---------------- */

function renderClips(clips, win) {
  const wantRows = clips.filter((c) => c.endMs > win.t0 && c.startMs < win.t1);
  const wantIds = new Set(wantRows.map((c) => c.id));

  // 删除(含被虚拟化裁除的)
  for (const [id, meta] of [...clipEls]) {
    if (!wantIds.has(id)) {
      meta.el.remove();
      clipEls.delete(id);
    }
  }

  const sel = selectionStore.get().clipId;
  for (const row of wantRows) {
    const left = Math.round(row.startMs * PX_PER_MS);
    const width = Math.max(6, Math.round((row.endMs - row.startMs) * PX_PER_MS));
    const kind = clipKindOf(row);
    const selected = row.id === sel;
    const text = clipLabelOf(row);
    const cls = `clip${kind !== "video" ? ` ${kind}` : ""}${selected ? " selected" : ""}`;
    const meta = clipEls.get(row.id);
    if (!meta) {
      // 新增(创建即落几何:absolute 无 left/width 会退化为 shrink-to-fit)
      const el = createClipEl(row, cls, text);
      el.style.left = `${left}px`;
      el.style.width = `${width}px`;
      const lane = laneEls.get(row.track);
      if (lane) lane.appendChild(el);
      clipEls.set(row.id, { el, left, width, text, cls, track: row.track, selected });
      drawWaveIfAudio(el, row, width);
      continue;
    }
    // 更新/移动:仅触碰变化的属性
    const el = meta.el;
    if (meta.track !== row.track) {
      const lane = laneEls.get(row.track);
      if (lane) lane.appendChild(el); // 移动(跨轨):DOM 节点复用
      meta.track = row.track;
      el.dataset.track = row.track;
    }
    if (meta.left !== left) {
      meta.left = left;
      el.style.left = `${left}px`;
    }
    if (meta.width !== width) {
      meta.width = width;
      el.style.width = `${width}px`;
      drawWaveIfAudio(el, row, width);
    }
    if (meta.text !== text) {
      meta.text = text;
      el.querySelector(".clip-name").textContent = text;
    }
    if (meta.cls !== cls) {
      meta.cls = cls;
      el.className = cls;
    }
  }
}

function createClipEl(row, cls, text) {
  const el = h("div", {
    class: cls, dataset: { id: row.id, track: row.track }, testid: "clip",
    "aria-label": `片段 ${text}`,
  }, [
    h("span", { class: "clip-name" }, [text]),
    h("span", { class: "edge edge-l", testid: "clip-edge-l", "data-tip": "拖动裁剪入点" }),
    h("span", { class: "edge edge-r", testid: "clip-edge-r", "data-tip": "拖动裁剪出点" }),
  ]);
  el.addEventListener("mousedown", onClipMouseDown);
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    // 右键菜单先选中再开(菜单项依赖选中态)
    selectionStore.set({ clipId: row.id });
    openClipContextMenu(e.clientX, e.clientY);
  });
  return el;
}

/* ---------------- ghost(ephemeral.dragGhost,ADR-0013)---------------- */

function renderGhost() {
  const g = ephemeralStore.get().dragGhost;
  const key = g ? `${g.clipId}@${g.trackId}:${Math.round(g.startMs)}x${Math.round(g.durationMs)}` : "";
  if (key === ghostKey) return;
  if (ghostEl && (!g || key !== ghostKey)) {
    ghostEl.remove();
    ghostEl = null;
    ghostKey = "";
  }
  if (!g) return;
  const lane = laneEls.get(g.trackId);
  if (!lane) return;
  ghostEl = h("div", {
    class: "clip ghost", testid: "drag-ghost",
    "aria-hidden": "true",
  }, [h("span", { class: "clip-name" }, [g.label || ""])]);
  ghostEl.style.left = `${Math.round(g.startMs * PX_PER_MS)}px`;
  ghostEl.style.width = `${Math.max(6, Math.round(g.durationMs * PX_PER_MS))}px`;
  lane.appendChild(ghostEl);
  ghostKey = key;
}

/** 手势层通知:ephemeral.snapMs 变化 → 只重画 overlay(不整棵重渲染)。 */
export function redrawOverlayOnly() {
  drawOverlay(ephemeralStore.get().snapMs);
}
