/* 手势层(修 W7 拖拽跟手):拖拽/trim 的**临时投影**驱动。
 *
 * ADR-0013 纪律:拖拽中的候选值只写 ephemeral.dragGhost / ephemeral.snapMs
 * (不进 IR / 不落盘 / 不参与撤销);ghost 逐帧跟手,mouseup 才提交内核命令,
 * 提交后以重投影收敛。Esc 取消 → 投影态原样恢复。
 */
import { ephemeralStore, timelineStore, projectStore, uiStore, mediaStore } from "../core/store.js";
import { PX_PER_MS, msAdd, snapMs, frameMsOf, clipKindOf, clipLabelOf } from "../core/model.js";
import { selectClip, moveClip, updateClip, insertMedia } from "../core/commands.js";
import { renderTimelineView, redrawOverlayOnly } from "./timeline-view.js";

const DRAG_MEDIA_MIME = "text/cutforge-media"; // 素材拖放 MIME(旧壳契约,兼容红线)

function snapAt(ms) {
  return snapMs(ms, uiStore.get().magnet, frameMsOf(projectStore.get().project));
}

/* ---------------- clip 拖拽(移动/trim) ---------------- */

/**
 * clip mousedown 入口(时间线视图在创建节点时挂接)。
 * 主体 = 移动;edge-l/edge-r = 左/右 trim。
 */
export function onClipMouseDown(e) {
  const el = /** @type {HTMLElement} */ (e.currentTarget);
  const clipId = el.dataset.id;
  const row = timelineStore.get().clips.find((c) => c.id === clipId);
  if (!row) return;
  e.stopPropagation();
  selectClip(clipId);
  const edge = e.target.classList.contains("edge-l") ? "l"
    : e.target.classList.contains("edge-r") ? "r" : null;
  const x0 = e.clientX;
  const orig = { startMs: row.startMs, durationMs: row.endMs - row.startMs, srcIn: row.sourceInMs || 0, track: row.track };
  let active = true;
  let changed = false;
  const label = clipLabelOf(row);

  const onEsc = (ev) => {
    if (ev.key !== "Escape") return;
    active = false; // 取消:清理后由投影态重画,不提交任何命令
    cleanup();
    renderTimelineView();
  };

  const onMove = (ev) => {
    if (!active) return;
    const dMs = (ev.clientX - x0) / PX_PER_MS; // px→ms:显示映射
    if (edge === null) {
      // 移动:跨轨跟随(同 kind 才可落),ghost 实时跟手
      const laneEl = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.(".track");
      const targetTrack = laneEl ? laneEl.dataset.trackId : orig.track;
      const trackKind = laneEl ? laneEl.dataset.kind : clipKindOf(row);
      const ns = Math.max(0, snapAt(msAdd(orig.startMs, dMs)));
      const ghostTrack = trackKind === clipKindOf(row) ? targetTrack : orig.track;
      changed = changed || ns !== orig.startMs;
      ephemeralStore.set({
        dragGhost: { clipId, trackId: ghostTrack, startMs: ns, durationMs: orig.durationMs, label },
        snapMs: ns,
      });
      redrawOverlayOnly();
    } else if (edge === "l") {
      // 左 trim:入点右移 → start 增、时长减、sourceIn 同步(全部来自投影值 + 指针位移)
      const ns = Math.max(0, snapAt(msAdd(orig.startMs, dMs)));
      const dur = msAdd(orig.durationMs, orig.startMs - ns);
      if (dur > 0 && ns !== orig.startMs) {
        changed = true;
        el.classList.add("trim-preview");
        el.style.left = `${Math.round(ns * PX_PER_MS)}px`;
        el.style.width = `${Math.max(6, Math.round(dur * PX_PER_MS))}px`;
        ephemeralStore.set({ snapMs: ns, dragGhost: null });
      }
      redrawOverlayOnly();
    } else {
      // 右 trim:仅出点
      const nd = Math.max(1, snapAt(msAdd(orig.durationMs, dMs)));
      if (nd !== orig.durationMs) {
        changed = true;
        el.classList.add("trim-preview");
        el.style.width = `${Math.max(6, Math.round(nd * PX_PER_MS))}px`;
        ephemeralStore.set({ snapMs: null, dragGhost: null });
      }
    }
  };

  const onUp = (ev) => {
    if (!active) return;
    active = false;
    cleanup();
    const dMs = (ev.clientX - x0) / PX_PER_MS;
    if (!changed) return; // 纯点击:只有选中,零命令
    if (edge === null) {
      const ns = Math.max(0, snapAt(msAdd(orig.startMs, dMs)));
      const laneEl = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.(".track");
      const toTrack = laneEl && laneEl.dataset.trackId !== orig.track
        && laneEl.dataset.kind === clipKindOf(row) ? laneEl.dataset.trackId : undefined;
      if (ns !== orig.startMs || toTrack) moveClip(clipId, ns, toTrack);
    } else if (edge === "l") {
      const ns = Math.max(0, snapAt(msAdd(orig.startMs, dMs)));
      if (ns !== orig.startMs && ns < msAdd(orig.startMs, orig.durationMs)) {
        updateClip(clipId, {
          startMs: ns,
          durationMs: msAdd(orig.durationMs, orig.startMs - ns),
          sourceInMs: msAdd(orig.srcIn, ns - orig.startMs),
        });
      }
    } else {
      const nd = Math.max(1, snapAt(msAdd(orig.durationMs, dMs)));
      if (nd !== orig.durationMs) updateClip(clipId, { durationMs: nd });
    }
    renderTimelineView(); // 投影回来前先按 store 态复位 ghost/trim 视图
  };

  function cleanup() {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
    document.removeEventListener("keydown", onEsc, true);
    el.classList.remove("trim-preview");
    ephemeralStore.set({ dragGhost: null, snapMs: null });
    redrawOverlayOnly();
  }

  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
  document.addEventListener("keydown", onEsc, true);
}

/* ---------------- 轨道行手势 ---------------- */

/** 点击轨道空白处 = 取消选中(旧壳口径)。 */
export function onLaneMouseDown(e) {
  if (e.target === e.currentTarget) selectClip(null);
}

export function onLaneDragOver(e) {
  e.preventDefault();
  /** @type {HTMLElement} */ (e.currentTarget).classList.add("drop-hint");
}

export function onLaneDragLeave(e) {
  /** @type {HTMLElement} */ (e.currentTarget).classList.remove("drop-hint");
}

/** 素材拖放到轨道落点(px→ms 显示映射 + 磁吸;插入由 clip_add 命令提交)。 */
export function onLaneDrop(e) {
  e.preventDefault();
  const lane = /** @type {HTMLElement} */ (e.currentTarget);
  lane.classList.remove("drop-hint");
  const src = e.dataTransfer.getData(DRAG_MEDIA_MIME);
  if (!src) return;
  const rect = lane.getBoundingClientRect();
  const at = snapAt(Math.max(0, (e.clientX - rect.left) / PX_PER_MS));
  const trackId = lane.dataset.trackId;
  // durationMs 优先用素材面板浏览元信息(避免无谓的二次探测)
  const item = mediaStore.get().files.find((f) => f.path === src);
  insertMedia(src, trackId, at, item && item.durationMs);
}

/* ---------------- 标尺 seek 手势(e2e 锚点:#ruler 点击坐标 ÷ 0.06 = ms) ---------------- */

export function mountRulerGestures(seekFn) {
  const ruler = document.getElementById("ruler");
  ruler.addEventListener("mousedown", (e) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const seekFromEvent = (ev) => {
      seekFn(snapAt(Math.max(0, (ev.clientX - rect.left) / PX_PER_MS)));
    };
    seekFromEvent(e);
    const onMove = (ev) => {
      if (ev.buttons & 1) seekFromEvent(ev);
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  });
}
