/* 时间线视图(T2.4,修 W2/W6):DOM 交互层的 keyed reconciliation。
 *
 * - 以稳定 id(轨道 id / 片段 id)做「新增/删除/移动/更新」四类 patch,只动变化节点;
 *   彻底删除旧壳「innerHTML 全量清空重建」(AC-2.3:一次 clip 移动只碰相关节点);
 * - 视口外片段不渲染(virtualizer,1k clips 只渲染视口内);
 * - 播放头推进绝不进入本模块(播放循环只碰 transform/文本/canvas,ADR-0012);
 * - ghost(ephemeral.dragGhost)是临时投影层(ADR-0013):投影到达即清除重画。
 */
import { $, h } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore, ephemeralStore, uiStore } from "../core/store.js";
import { PX_PER_MS, clipKindOf, clipLabelOf, timelineEndMsOf } from "../core/model.js";
import { createVirtualizer } from "./virtualizer.js";
import { setRulerContent, drawRuler } from "./ruler.js";
import { setPlayheadPx, drawOverlay } from "./playhead.js";
import { drawWaveIfAudio } from "./waveform.js";
import { onClipPointerDown } from "./clip-gestures.js";
import { onLanePointerDown, onLaneDragOver, onLaneDragLeave, onLaneDrop } from "./gestures.js";
import { buildTrackHead, syncTrackHead } from "./track-head.js";
import { openClipContextMenu, openTrackContextMenu, openTimelineContextMenu } from "../ui/menu.js";

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
  mountEmptyState();
  mountToolBadges();
  // 轨道属性(track_update)只改 projectStore:挂订阅让轨头/锁定态跟着重绘
  projectStore.subscribe((patch) => {
    if (patch.project !== undefined) renderTimelineView();
  });
  // 工具模式(T4.2):光标与命中区随模式(wrap class;e2e 断言面)
  const applyTool = (st) => {
    const wrap = $("timeline-wrap");
    wrap.classList.toggle("blade-mode", st.tool === "blade");
    wrap.classList.toggle("trim-mode", st.tool === "trim");
    wrap.classList.toggle("tool-select", (st.tool || "select") === "select");
  };
  uiStore.subscribe((patch, st) => {
    if (patch.tool !== undefined) applyTool(st);
  });
  applyTool(uiStore.get());
  return virt;
}

/** 空工程下一步提示(T3.7:空面板给明确下一步,不出现空列表框)。
 * 片段数 0 时显示;插第一个片段即隐(display 切换,常驻节点零增量变更)。 */
function mountEmptyState() {
  const wrap = $("timeline-wrap");
  const el = h("div", {
    class: "timeline-empty", testid: "timeline-empty", "aria-live": "polite",
  }, [
    h("b", null, ["时间线还是空的"]),
    h("span", null, ["下一步:双击左侧素材卡插入第一个片段(或把素材拖到轨道上)"]),
  ]);
  el.id = "timeline-empty";
  wrap.appendChild(el);  timelineStore.subscribe((patch) => {
    if (patch.clips !== undefined || patch.__reset__) {
      el.classList.toggle("show", !(timelineStore.get().clips || []).length);
    }
  });
  el.classList.add("show"); // 装配期投影未到:先按空工程口径显示,clips 到达即收敛
}

/** 工具模式徽标(T4.2:A/B/T 三模式;blade 徽标保持 T3.4 e2e 口径,B/A 文案微调)。
 * 切割=点击即分割;裁剪=点片段边缘修剪。文本态,非颜色单线索。 */
function mountToolBadges() {
  const badge = h("span", {
    class: "blade-badge", testid: "blade-mode", hidden: true,
    "aria-live": "polite",
  }, ["✂ 切割模式:点击片段即分割(B 或 A 退出)"]);
  const trimBadge = h("span", {
    class: "blade-badge", testid: "trim-mode", hidden: true,
    "aria-live": "polite",
  }, ["⇱ 裁剪模式:点片段边缘拖动修剪(V 或 A 退出)"]);
  const toolbar = $("toolbar");
  if (toolbar) { toolbar.appendChild(badge); toolbar.appendChild(trimBadge); }
  uiStore.subscribe((patch, st) => {
    if (patch.tool !== undefined) {
      badge.hidden = st.tool !== "blade";
      trimBadge.hidden = st.tool !== "trim";
    }
  });
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

/** 选中态变化只碰 class(keyed 更新的最廉价路径;clipId 主选中 + clipIds 框选集)。
 * T3.6:键盘 ±1 clip 导航时把主选中卷入视口(nearest:已可见零滚动,点击路径无感)。 */
export function updateSelectionView() {
  const sel = selectionStore.get().clipId;
  const ids = new Set(selectionStore.get().clipIds || []);
  let selEl = null;
  for (const [id, meta] of clipEls) {
    const want = id === sel || ids.has(id);
    if (meta.selected !== want) {
      meta.selected = want;
      meta.el.classList.toggle("selected", want);
    }
    if (id === sel) selEl = meta.el;
  }
  if (selEl) selEl.scrollIntoView({ block: "nearest", inline: "nearest" });
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
    syncTrackHead(lane, t); // 轨头七字段状态(锁定/静音/独奏/眼睛/名称/高度/标识色)
  }
}

function createLane(t) {
  const kind = t.kind || clipKindOf({ track: t.id });
  const { label, grip } = buildTrackHead(t);
  const lane = h("div", {
    class: "track", dataset: { trackId: t.id, kind },
    testid: `track-lane-${t.id}`,
  }, [label, grip]);
  lane.addEventListener("pointerdown", onLanePointerDown);
  lane.addEventListener("dragover", onLaneDragOver);
  lane.addEventListener("dragleave", onLaneDragLeave);
  lane.addEventListener("drop", onLaneDrop);
  // 右键上下文(T3.6 四菜单之二/四;T4.2 轨道全功能菜单):轨头 = 轨道菜单;空白 = 时间线菜单
  lane.addEventListener("contextmenu", (e) => {
    if (e.target.closest(".clip")) return; // 片段菜单由 clip 自己处理
    e.preventDefault();
    if (e.target.closest(".lane-label") || e.target === grip) openTrackContextMenu(t.id, e.clientX, e.clientY);
    else openTimelineContextMenu(e.clientX, e.clientY);
  });
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
  const selIds = new Set(selectionStore.get().clipIds || []);
  for (const row of wantRows) {
    const left = Math.round(row.startMs * PX_PER_MS);
    const width = Math.max(6, Math.round((row.endMs - row.startMs) * PX_PER_MS));
    const kind = clipKindOf(row);
    const selected = row.id === sel || selIds.has(row.id);
    const text = clipLabelOf(row);
    // T4.2:overlay(画中画)片段视觉区分(虚线描边 + 角标);音频轨静音/独奏态灰化由 lane class 下传
    const cls = `clip${kind !== "video" ? ` ${kind}` : ""}${selected ? " selected" : ""}`
      + `${row.overlay ? " overlay" : ""}`;
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
    row.overlay ? h("span", { class: "clip-pip", "data-tip": "画中画(overlay 层)" }, ["画中画"]) : null,
    h("span", { class: "edge edge-l", testid: "clip-edge-l", "data-tip": "拖动裁剪入点(Shift+拖 = 双边联动 roll)" }),
    h("span", { class: "edge edge-r", testid: "clip-edge-r", "data-tip": "拖动裁剪出点(Shift+拖 = 双边联动 roll)" }),
  ]);
  el.addEventListener("pointerdown", onClipPointerDown);
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation(); // 轨道/空白菜单不得顶替片段菜单
    // 右键菜单先选中再开(菜单项依赖选中态)
    selectionStore.set({ clipId: row.id });
    openClipContextMenu(e.clientX, e.clientY);
  });
  return el;
}

/* ---------------- ghost(ephemeral.dragGhost,ADR-0013)----------------
 * 拖拽零动画:节点跨手势复用(只改几何与 invalid 态),不挂 transition。 */

function renderGhost() {
  const g = ephemeralStore.get().dragGhost;
  if (!g) {
    if (ghostEl) {
      ghostEl.remove();
      ghostEl = null;
      ghostKey = "";
    }
    return;
  }
  const key = `${g.clipId}@${g.trackId}`;
  const lane = laneEls.get(g.trackId);
  if (!lane) return;
  if (!ghostEl || ghostKey !== key) {
    if (ghostEl) ghostEl.remove();
    ghostEl = h("div", {
      class: "clip ghost", testid: "drag-ghost",
      "aria-hidden": "true",
    }, [h("span", { class: "clip-name" }, [g.label || ""])]);
    lane.appendChild(ghostEl);
    ghostKey = key;
  }
  ghostEl.classList.toggle("invalid", Boolean(g.invalid));
  ghostEl.style.left = `${Math.round(g.startMs * PX_PER_MS)}px`;
  ghostEl.style.width = `${Math.max(6, Math.round(g.durationMs * PX_PER_MS))}px`;
}

/** 手势层通知:ephemeral.snapMs 变化 → 只重画 overlay(不整棵重渲染)。 */
export function redrawOverlayOnly() {
  drawOverlay(ephemeralStore.get().snapMs);
}
