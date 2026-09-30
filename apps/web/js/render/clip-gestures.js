/* 片段手势(T4.2 四件套):移动 / trim / roll / slip / slide + 工具模式(A/B/T)。
 * 从 gestures.js 分册(单文件 ≤400 纪律),通用管线在 gesture-kit.js。
 *
 * 手势映射(一次手势恰好一个 Op;拖拽全程零动画、零 Op,松手单命令,Esc/失焦取消):
 * | 输入               | 模式   | 命令                            |
 * |-------------------|--------|--------------------------------|
 * | 主体拖(无修饰)     | 移动   | clip_move                      |
 * | 主体拖 + Alt       | slip  | clip_trim slip   deltaMs       |
 * | 主体拖 + Ctrl/⌘    | slide | clip_trim slide  deltaMs       |
 * | 边缘拖(无修饰)     | trim  | clip_trim trim   edge + deltaMs |
 * | 边缘拖 + Shift     | roll  | clip_trim roll   edge + deltaMs |
 * | 切割模式(B)点主体   | 分割   | clip_split                     |
 * | V 工具主体按下      | 就近边 trim(前 40% 入点/后 40% 出点)|
 * 锁定轨拒绝一切编辑手势(toast + lane 抖动;视觉 .lane-locked)。
 * (T4.7 起 T 键 = 文本工具,裁剪模式迁 V;注册表/keymap 帮助表同步。)
 */
import { $ } from "../ui/dom.js";
import { ephemeralStore, timelineStore, uiStore, projectStore } from "../core/store.js";
import { PX_PER_MS, msAdd, clipKindOf, clipLabelOf } from "../core/model.js";
import { selectClip, moveClip, splitAt } from "../core/commands.js";
import { trimClip } from "../core/edit-commands.js";
import { toast } from "../ui/toast.js";
import { renderTimelineView, redrawOverlayOnly } from "./timeline-view.js";
import {
  runGesture, snapCandidateEx, fmtSec, showBubble, hideBubble,
  highlightLane, clearLaneHighlight, edgeScroll, makePulser,
} from "./gesture-kit.js";

/** 轨道是否锁定(投影 track.locked;track_update 落盘)。 */
export function trackLockedOf(trackId) {
  const t = (projectStore.get().project?.tracks || []).find((x) => x.id === trackId);
  return Boolean(t && t.locked);
}

/** 锁定提示:lane 抖动一次(css cf-shake 一次性动画)。 */
function flashLocked(laneEl) {
  if (!laneEl) return;
  laneEl.classList.remove("lane-locked-flash");
  void laneEl.offsetWidth;
  laneEl.classList.add("lane-locked-flash");
  toast("该轨道已锁定:先在轨头解锁再编辑", false);
}

/** 同轨邻接查找:side="in" 返回与该片段入点贴合的左邻,"out" 返回出点贴合的右邻。 */
function adjacentNeighbor(row, side) {
  const same = timelineStore.get().clips.filter((c) => c.track === row.track && c.id !== row.id);
  const end = msAdd(row.startMs, row.endMs - row.startMs);
  if (side === "in") {
    return same.find((c) => msAdd(c.startMs, c.endMs - c.startMs) === row.startMs
      && c.startMs < row.startMs) || null;
  }
  return same.find((c) => c.startMs === end) || null;
}

/** Esc/失焦取消后把 trim 期间的内联几何复位回投影值(renderClips 按 meta 比对不会重写)。 */
function resetClipGeometry(el, clipId) {
  const row = timelineStore.get().clips.find((c) => c.id === clipId);
  el.classList.remove("trim-preview", "slip-preview");
  const wave = el.querySelector(".clip-wave");
  if (wave) wave.style.transform = "";
  if (!row) return;
  el.style.left = `${Math.round(row.startMs * PX_PER_MS)}px`;
  el.style.width = `${Math.max(6, Math.round((row.endMs - row.startMs) * PX_PER_MS))}px`;
}

/**
 * clip pointerdown 入口(timeline-view 在创建节点时挂接)。
 * @param {PointerEvent} e
 */
export function onClipPointerDown(e) {
  const el = /** @type {HTMLElement} */ (e.currentTarget);
  const clipId = el.dataset.id;
  const row = timelineStore.get().clips.find((c) => c.id === clipId);
  if (!row || e.button !== 0) return;
  e.stopPropagation();
  selectClip(clipId);
  if (trackLockedOf(row.track)) {
    flashLocked(document.querySelector(`[data-testid="track-lane-${row.track}"]`));
    return;
  }
  const tool = uiStore.get().tool || "select";
  // 切割模式:点击即分割,分割点 = 点击位置(统一吸附),零拖拽
  if (tool === "blade") {
    const wrap = $("timeline-wrap");
    const ms = (e.clientX - wrap.getBoundingClientRect().left + wrap.scrollLeft) / PX_PER_MS;
    const cand = snapCandidateEx(Math.max(0, ms), [clipId]);
    if (cand.ms > row.startMs && cand.ms < row.endMs) splitAt(clipId, Math.round(cand.ms));
    else toast("点击位置不在片段内部,未分割", false);
    return;
  }
  const edge = e.target.classList.contains("edge-l") ? "l"
    : e.target.classList.contains("edge-r") ? "r" : null;
  const mod = e.altKey ? "alt" : (e.ctrlKey || e.metaKey) ? "ctrl" : e.shiftKey ? "shift" : "";
  const plan = edge
    ? { kind: mod === "shift" ? "roll" : "trim", side: edge === "l" ? "in" : "out" }
    : mod === "alt" ? { kind: "slip", side: "in" }
      : mod === "ctrl" ? { kind: "slide", side: "in" }
        : tool === "trim" ? planTrimFromBody(e)
          : { kind: "move", side: "in" };
  if (!plan) { toast("V 裁剪工具:点片段边缘裁剪;普通移动切回 A 选择(A 键)", false); return; }
  if (plan.kind === "move") { runMove(el, e, row); return; }
  runTrimFamily(el, e, row, plan);
}

/** T 裁剪工具:主体按下 → 就近边缘(前 40% 入点 / 后 40% 出点),中段不拖。 */
function planTrimFromBody(e) {
  const rect = /** @type {HTMLElement} */ (e.currentTarget).getBoundingClientRect();
  const x = e.clientX - rect.left;
  if (x < rect.width * 0.4) return { kind: "trim", side: "in" };
  if (x > rect.width * 0.6) return { kind: "trim", side: "out" };
  return null;
}

/* ---------------- 移动(跨轨 + ghost;统一吸附候选) ---------------- */

function runMove(el, e, row) {
  const clipId = row.id;
  const x0 = e.clientX;
  const orig = { startMs: row.startMs, durationMs: row.endMs - row.startMs, track: row.track };
  const kind = clipKindOf(row);
  const label = clipLabelOf(row);
  let changed = false;
  let scroller = null;
  const pulse = makePulser();
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
      const dMs = (ev.clientX - x0) / PX_PER_MS;
      const laneEl = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.(".track");
      const targetTrack = laneEl ? laneEl.dataset.trackId : orig.track;
      const trackKind = laneEl ? laneEl.dataset.kind : kind;
      const invalid = Boolean(laneEl) && trackKind !== kind;
      const cand = snapCandidateEx(msAdd(orig.startMs, dMs), [clipId]);
      changed = changed || cand.ms !== orig.startMs || (!invalid && targetTrack !== orig.track);
      ephemeralStore.set({
        dragGhost: {
          clipId, trackId: invalid ? orig.track : targetTrack,
          startMs: cand.ms, durationMs: orig.durationMs, label, invalid,
        },
        snapMs: invalid ? null : cand.ms,
      });
      pulse(cand);
      highlightLane(laneEl, !invalid);
      showBubble(ev.clientX, ev.clientY, invalid ? "无法落此轨(轨型不符)" : fmtSec(cand.ms));
      redrawOverlayOnly();
    },
    end: (ev) => {
      clearTransient();
      if (!changed) return; // 纯点击:只有选中,零命令
      const dMs = (ev.clientX - x0) / PX_PER_MS;
      const cand = snapCandidateEx(msAdd(orig.startMs, dMs), [clipId]);
      const laneEl = document.elementFromPoint(ev.clientX, ev.clientY)?.closest?.(".track");
      const toTrack = laneEl && laneEl.dataset.trackId !== orig.track
        && laneEl.dataset.kind === kind ? laneEl.dataset.trackId : undefined;
      if (cand.ms !== orig.startMs || toTrack) moveClip(clipId, cand.ms, toTrack);
      renderTimelineView();
    },
    cancel: () => { clearTransient(); renderTimelineView(); },
  });
}

/* ---------------- trim 四件套(单命令单 Op)---------------- */

function runTrimFamily(el, e, row, plan) {
  const clipId = row.id;
  const x0 = e.clientX;
  const orig = {
    startMs: row.startMs, durationMs: row.endMs - row.startMs, srcIn: row.sourceInMs || 0,
    leftN: adjacentNeighbor(row, "in"), rightN: adjacentNeighbor(row, "out"),
  };
  let delta = 0;
  let changed = false;
  let scroller = null;
  const pulse = makePulser();
  const clearTransient = () => {
    if (scroller) { scroller.stop(); scroller = null; }
    el.classList.remove("trim-preview", "slip-preview");
    const wave = el.querySelector(".clip-wave");
    if (wave) wave.style.transform = "";
    hideBubble();
    ephemeralStore.set({ snapMs: null, trimLineMs: null });
    redrawOverlayOnly();
  };

  runGesture(el, e, {
    start: () => {
      scroller = edgeScroll($("timeline-wrap"));
      if (plan.kind === "slip") el.classList.add("slip-preview");
    },
    move: (ev) => {
      if (scroller) scroller.move(ev.clientX);
      const dMs = (ev.clientX - x0) / PX_PER_MS;
      const view = renderTrimFrame(el, row, orig, plan, dMs, pulse, ev);
      changed = changed || view.changed;
      delta = view.delta;
      redrawOverlayOnly();
    },
    end: () => {
      clearTransient();
      if (changed && delta !== 0) {
        trimClip(clipId, plan.kind, plan.side, Math.round(delta)); // 恰好一笔 Op
      }
      renderTimelineView();
    },
    cancel: () => {
      clearTransient();
      resetClipGeometry(el, clipId);
      renderTimelineView();
    },
  });
}

/** 手势帧渲染:按模式写内联几何/气泡/吸附线;返回 { changed, delta }。 */
function renderTrimFrame(el, row, orig, plan, dMs, pulse, ev) {
  if (plan.kind === "trim") return frameTrim(el, row, orig, plan.side, dMs, pulse, ev);
  if (plan.kind === "roll") return frameRoll(el, row, orig, plan.side, dMs, pulse, ev);
  if (plan.kind === "slip") return frameSlip(el, row, orig, dMs, ev);
  return frameSlide(el, row, orig, dMs, pulse, ev);
}

function frameTrim(el, row, orig, side, dMs, pulse, ev) {
  let delta = 0;
  if (side === "in") {
    let ns = snapCandidateEx(msAdd(orig.startMs, dMs), [row.id]).ms;
    ns = Math.min(Math.max(ns, orig.leftN ? orig.leftN.endMs : 0),
      msAdd(orig.startMs, orig.durationMs) - 1);
    delta = msAdd(ns, -orig.startMs);
    if (delta !== 0) {
      el.classList.add("trim-preview");
      el.style.left = `${Math.round(ns * PX_PER_MS)}px`;
      const dur = msAdd(orig.durationMs, -delta);
      el.style.width = `${Math.max(6, Math.round(dur * PX_PER_MS))}px`;
      ephemeralStore.set({ snapMs: ns, trimLineMs: null });
      pulse({ ms: ns });
      showBubble(ev.clientX, ev.clientY, fmtSec(dur));
    }
  } else {
    let nd = snapCandidateEx(msAdd(orig.durationMs, dMs), [row.id]).ms;
    const maxDur = orig.rightN ? msAdd(orig.rightN.startMs, -orig.startMs) : Number.MAX_SAFE_INTEGER;
    nd = Math.min(nd, maxDur);
    delta = msAdd(nd, -orig.durationMs);
    if (delta !== 0 && nd >= 1) {
      el.classList.add("trim-preview");
      el.style.width = `${Math.max(6, Math.round(nd * PX_PER_MS))}px`;
      ephemeralStore.set({ snapMs: null, trimLineMs: null });
      showBubble(ev.clientX, ev.clientY, fmtSec(nd));
    }
  }
  return { changed: delta !== 0, delta };
}

/** roll:共享边界双边联动;内联画指示线(ephemeral.trimLineMs),松手单 Op。 */
function frameRoll(el, row, orig, side, dMs, pulse, ev) {
  const base = side === "in" ? orig.startMs : msAdd(orig.startMs, orig.durationMs);
  let nb = snapCandidateEx(msAdd(base, dMs), [row.id]).ms;
  const lo = side === "in"
    ? msAdd(orig.leftN ? orig.leftN.startMs : 0, 1)
    : msAdd(orig.startMs, 1);
  const hi = side === "in"
    ? msAdd(orig.startMs, orig.durationMs) - 1
    : (orig.rightN ? msAdd(orig.rightN.startMs, orig.rightN.endMs - orig.rightN.startMs) - 1
      : Number.MAX_SAFE_INTEGER);
  nb = Math.min(Math.max(nb, lo), hi);
  const delta = msAdd(nb, -base);
  if (delta !== 0) {
    el.classList.add("trim-preview");
    ephemeralStore.set({ trimLineMs: nb, snapMs: null });
    pulse({ ms: nb });
    showBubble(ev.clientX, ev.clientY, `边界 ${fmtSec(nb)}(双边联动)`);
  }
  return { changed: delta !== 0, delta };
}

/** slip:内容平移(占位不变);波形纹理反向平移 + 气泡报源时间码。 */
function frameSlip(el, row, orig, dMs, ev) {
  let delta = Math.round(dMs);
  delta = Math.max(delta, -orig.srcIn); // sourceIn 不得为负;素材末尾越界由服务端裁决
  if (delta !== 0) {
    el.classList.add("slip-preview");
    const wave = el.querySelector(".clip-wave");
    if (wave) wave.style.transform = `translateX(${Math.round(-delta * PX_PER_MS)}px)`;
    showBubble(ev.clientX, ev.clientY, `内容 ${fmtSec(msAdd(orig.srcIn, delta))}(占位不变)`);
  }
  return { changed: delta !== 0, delta };
}

/** slide:位置平移(邻居让位);ghost 跟手,松手单 Op。 */
function frameSlide(el, row, orig, dMs, pulse, ev) {
  const cand = snapCandidateEx(msAdd(orig.startMs, dMs), [row.id]);
  const delta = msAdd(cand.ms, -orig.startMs);
  if (delta !== 0) {
    ephemeralStore.set({
      dragGhost: {
        clipId: row.id, trackId: row.track, startMs: cand.ms,
        durationMs: orig.durationMs, label: clipLabelOf(row), invalid: false, slide: true,
      },
      snapMs: cand.source !== "frame" ? cand.ms : null,
    });
    pulse(cand);
    showBubble(ev.clientX, ev.clientY, `位置 ${fmtSec(cand.ms)}(邻居让位)`);
  }
  return { changed: delta !== 0, delta };
}
