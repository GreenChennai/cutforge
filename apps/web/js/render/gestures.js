/* 手势层(修 W7,册三 T3.3):具体手势——clip 移动/trim、框选、素材拖放、标尺 scrub、缩放。
 * 通用管线/气泡/卷入/吸附在 gesture-kit.js。纪律:
 * - 拖拽零动画(直接跟手):几何每帧直写,不挂 transition(C-R1);
 * - 拖拽过程不产 Op:候选值只写 ephemeral.dragGhost / ephemeral.snapMs(ADR-0013),
 *   松手一次手势一个命令,提交后以重投影收敛;Esc/失焦取消不提交。
 */
import { $, h } from "../ui/dom.js";
import { ephemeralStore, timelineStore, mediaStore, selectionStore, uiStore } from "../core/store.js";
import { PX_PER_MS, msAdd, clipKindOf, clipLabelOf, setPxPerMs } from "../core/model.js";
import { selectClip, moveClip, updateClip, insertMedia, splitAt } from "../core/commands.js";
import { toast } from "../ui/toast.js";
import { renderTimelineView, redrawOverlayOnly } from "./timeline-view.js";
import {
  runGesture, snapAt, fmtSec, showBubble, hideBubble,
  highlightLane, clearLaneHighlight, edgeScroll, snapCandidate, makePulser,
} from "./gesture-kit.js";

const DRAG_MEDIA_MIME = "text/cutforge-media"; // 素材拖放 MIME(旧壳契约,兼容红线)

/** Esc/失焦取消后把 trim 期间的内联几何复位回投影值(renderClips 按 meta 比对不会重写)。 */
function resetClipGeometry(el, clipId) {
  const row = timelineStore.get().clips.find((c) => c.id === clipId);
  el.classList.remove("trim-preview");
  if (!row) return;
  el.style.left = `${Math.round(row.startMs * PX_PER_MS)}px`;
  el.style.width = `${Math.max(6, Math.round((row.endMs - row.startMs) * PX_PER_MS))}px`;
}

/* ---------------- clip 拖拽(移动/trim) ---------------- */

/**
 * clip pointerdown 入口(时间线视图在创建节点时挂接)。
 * 主体 = 移动;edge-l/edge-r = 左/右 trim(相邻片段碰撞约束)。
 */
export function onClipPointerDown(e) {
  const el = /** @type {HTMLElement} */ (e.currentTarget);
  const clipId = el.dataset.id;
  const row = timelineStore.get().clips.find((c) => c.id === clipId);
  if (!row || e.button !== 0) return;
  e.stopPropagation();
  selectClip(clipId);
  // 切割模式(T3.4 B 键):点击即分割,分割点 = 点击位置(帧磁吸),零拖拽
  if (uiStore.get().blade) {
    const wrap = $("timeline-wrap");
    const ms = Math.max(0, (e.clientX - wrap.getBoundingClientRect().left + wrap.scrollLeft) / PX_PER_MS);
    if (ms > row.startMs && ms < row.endMs) splitAt(clipId, Math.round(ms));
    else toast("点击位置不在片段内部,未分割", false);
    return;
  }
  const edge = e.target.classList.contains("edge-l") ? "l"
    : e.target.classList.contains("edge-r") ? "r" : null;
  const x0 = e.clientX;
  const orig = { startMs: row.startMs, durationMs: row.endMs - row.startMs, srcIn: row.sourceInMs || 0, track: row.track };
  const kind = clipKindOf(row);
  const label = clipLabelOf(row);
  let changed = false;
  let scroller = null;
  const pulse = makePulser();

  // 相邻片段碰撞边界(trim 约束;移动允许重叠,沿旧口径)
  const neighbors = timelineStore.get().clips.filter((c) => c.track === orig.track && c.id !== clipId);
  const prevEnd = Math.max(0, ...neighbors.filter((c) => c.endMs <= orig.startMs).map((c) => c.endMs));
  const nextStarts = neighbors.filter((c) => c.startMs >= msAdd(orig.startMs, orig.durationMs)).map((c) => c.startMs);
  const nextStart = nextStarts.length ? Math.min(...nextStarts) : Infinity;

  const clearTransient = () => {
    if (scroller) { scroller.stop(); scroller = null; }
    el.classList.remove("drag-source");
    clearLaneHighlight();
    hideBubble();
    ephemeralStore.set({ dragGhost: null, snapMs: null });
    redrawOverlayOnly();
  };

  runGesture(el, e, {
    start: () => {
      el.classList.add("drag-source");
      scroller = edgeScroll($("timeline-wrap"));
    },
    move: (ev) => {
      if (scroller) scroller.move(ev.clientX);
      const dMs = (ev.clientX - x0) / PX_PER_MS; // px→ms:显示映射
      if (edge === null) {
        // 移动:跨轨跟随(同 kind 才可落),ghost 逐帧跟手
        const laneEl = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.(".track");
        const targetTrack = laneEl ? laneEl.dataset.trackId : orig.track;
        const trackKind = laneEl ? laneEl.dataset.kind : kind;
        const invalid = Boolean(laneEl) && trackKind !== kind;
        const ghostTrack = invalid ? orig.track : targetTrack;
        const cand = snapCandidate(msAdd(orig.startMs, dMs));
        changed = changed || cand.ms !== orig.startMs || (!invalid && targetTrack !== orig.track);
        ephemeralStore.set({
          dragGhost: { clipId, trackId: ghostTrack, startMs: cand.ms, durationMs: orig.durationMs, label, invalid },
          snapMs: invalid ? null : cand.ms,
        });
        pulse(cand);
        highlightLane(laneEl, !invalid);
        showBubble(ev.clientX, ev.clientY, invalid ? "无法落此轨(轨型不符)" : fmtSec(cand.ms));
      } else if (edge === "l") {
        // 左 trim:入点位移 → 时长增减、sourceIn 同步;不越过左邻
        let ns = snapCandidate(msAdd(orig.startMs, dMs)).ms;
        ns = Math.min(Math.max(ns, prevEnd), msAdd(orig.startMs, orig.durationMs) - 1);
        if (ns !== orig.startMs) {
          changed = true;
          el.classList.add("trim-preview");
          el.style.left = `${Math.round(ns * PX_PER_MS)}px`;
          const dur = msAdd(orig.durationMs, orig.startMs - ns);
          el.style.width = `${Math.max(6, Math.round(dur * PX_PER_MS))}px`;
          ephemeralStore.set({ snapMs: ns, dragGhost: null });
          pulse({ ms: ns });
          showBubble(ev.clientX, ev.clientY, fmtSec(dur));
        }
      } else {
        // 右 trim:仅出点;不越过右邻
        let nd = snapCandidate(msAdd(orig.durationMs, dMs)).ms;
        nd = Math.min(nd, Math.round(nextStart - orig.startMs));
        if (nd !== orig.durationMs && nd >= 1) {
          changed = true;
          el.classList.add("trim-preview");
          el.style.width = `${Math.max(6, Math.round(nd * PX_PER_MS))}px`;
          ephemeralStore.set({ snapMs: null, dragGhost: null });
          showBubble(ev.clientX, ev.clientY, fmtSec(nd));
        }
      }
      redrawOverlayOnly();
    },
    end: (ev) => {
      clearTransient();
      if (!changed) return; // 纯点击:只有选中,零命令
      const dMs = (ev.clientX - x0) / PX_PER_MS;
      if (edge === null) {
        const cand = snapCandidate(msAdd(orig.startMs, dMs));
        const laneEl = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.(".track");
        const toTrack = laneEl && laneEl.dataset.trackId !== orig.track
          && laneEl.dataset.kind === kind ? laneEl.dataset.trackId : undefined;
        if (cand.ms !== orig.startMs || toTrack) moveClip(clipId, cand.ms, toTrack);
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
    },
    cancel: () => {
      clearTransient();
      if (edge !== null) resetClipGeometry(el, clipId);
      renderTimelineView(); // 取消:不提交任何命令,投影态原样恢复
    },
  });
}

/* ---------------- 轨道行手势(点空白取消选中 / 空白拉框多选) ---------------- */

/** 轨道空白 pointerdown:未过阈值 = 取消选中(旧壳口径);过阈值 = 框选多选。 */
export function onLanePointerDown(e) {
  if (e.target !== e.currentTarget || e.button !== 0) return;
  const laneEl = /** @type {HTMLElement} */ (e.currentTarget);
  const wrap = $("timeline-wrap");
  const additive = e.shiftKey || e.ctrlKey || e.metaKey;
  const baseIds = additive ? new Set(selectionStore.get().clipIds || []) : new Set();
  let box = null;
  let scroller = null;
  let bx0 = 0;
  let by0 = 0;

  /** 框选矩形(内容坐标直写,跟手零动画)+ 可见片段相交命中。 */
  const applyMarquee = (x1, y1) => {
    const left = Math.min(bx0, x1);
    const top = Math.min(by0, y1);
    box.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
    box.style.width = `${Math.max(1, Math.round(Math.abs(x1 - bx0)))}px`;
    box.style.height = `${Math.max(1, Math.round(Math.abs(y1 - by0)))}px`;
    const r = wrap.getBoundingClientRect();
    const cx0 = r.left + left - wrap.scrollLeft;
    const cy0 = r.top + top - wrap.scrollTop;
    const ids = new Set(baseIds);
    let anchor = selectionStore.get().clipId;
    for (const el of wrap.querySelectorAll(".clip:not(.ghost)")) {
      const cr = el.getBoundingClientRect();
      if (cr.right > cx0 && cr.left < cx0 + Math.abs(x1 - bx0)
        && cr.bottom > cy0 && cr.top < cy0 + Math.abs(y1 - by0)) {
        ids.add(el.dataset.id);
        if (!anchor) anchor = el.dataset.id;
      }
    }
    selectionStore.set({ clipIds: [...ids], clipId: anchor });
  };
  const teardown = () => {
    if (scroller) { scroller.stop(); scroller = null; }
    if (box) { box.remove(); box = null; }
  };

  runGesture(laneEl, e, {
    start: (ev) => {
      const r = wrap.getBoundingClientRect();
      bx0 = ev.clientX - r.left + wrap.scrollLeft;
      by0 = ev.clientY - r.top + wrap.scrollTop;
      box = h("div", { id: "marquee-box", "aria-hidden": "true" });
      wrap.appendChild(box);
      box.style.display = "block";
      if (!additive) selectClip(null);
      scroller = edgeScroll(wrap);
    },
    move: (ev) => {
      if (!box) return;
      if (scroller) scroller.move(ev.clientX);
      const r = wrap.getBoundingClientRect();
      applyMarquee(ev.clientX - r.left + wrap.scrollLeft, ev.clientY - r.top + wrap.scrollTop);
    },
    end: teardown,
    cancel: teardown,
    click: () => { if (!additive) selectClip(null); },
  });
}

/* ---------------- 素材拖放(HTML5 DnD,旧壳契约) ---------------- */

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

/* ---------------- 标尺手势(e2e 锚点:#ruler 点击坐标 ÷ 0.06 = ms) ---------------- */

/** 标尺 seek:按下即跳 + 连续 scrub(rAF 节流 = 预览抽帧节流)+ 悬停时间气泡。 */
export function mountRulerGestures(seekFn) {
  const ruler = document.getElementById("ruler");
  const msOf = (ev) => Math.max(0, (ev.clientX - ruler.getBoundingClientRect().left) / PX_PER_MS);
  ruler.addEventListener("pointermove", (e) => {
    if (e.buttons) return; // 拖拽中的气泡由 scrub 路径接管
    showBubble(e.clientX, e.clientY + 14, fmtSec(msOf(e)));
  });
  ruler.addEventListener("pointerleave", hideBubble);
  ruler.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    let raf = 0;
    let pendingMs = null;
    const scrub = (ms) => { // 连续 seek 合帧:预览抽帧至多 60Hz
      pendingMs = ms;
      if (!raf) {
        raf = requestAnimationFrame(() => {
          raf = 0;
          seekFn(snapAt(pendingMs));
        });
      }
    };
    seekFn(snapAt(msOf(e))); // 按下即跳(不经阈值)
    runGesture(ruler, e, {
      move: (ev) => {
        showBubble(ev.clientX, ev.clientY + 14, fmtSec(msOf(ev)));
        scrub(msOf(ev));
      },
      end: () => { if (raf) cancelAnimationFrame(raf); raf = 0; hideBubble(); },
      cancel: () => { if (raf) cancelAnimationFrame(raf); raf = 0; hideBubble(); },
    });
  });
}

/* ---------------- 时间轴缩放(Ctrl/⌘/Alt+滚轮;触控板捏合=Ctrl+滚轮) ---------------- */

/** 视口中心锚定缩放:中心时刻在新映射下保持屏中;0.06 缺省永不被装配路径触碰(e2e 红线)。 */
export function mountTimelineZoom() {
  const wrap = $("timeline-wrap");
  wrap.addEventListener("wheel", (e) => {
    if (!(e.ctrlKey || e.metaKey || e.altKey)) return; // 纯滚轮/双指 = 原生滚动
    e.preventDefault();
    const oldPx = PX_PER_MS;
    const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    if (!setPxPerMs(oldPx * factor)) return;
    const centerMs = (wrap.scrollLeft + wrap.clientWidth / 2) / oldPx;
    renderTimelineView();
    wrap.scrollLeft = Math.max(0, centerMs * PX_PER_MS - wrap.clientWidth / 2);
  }, { passive: false });
}
