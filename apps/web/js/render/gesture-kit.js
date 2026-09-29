/* 手势基座(T3.3):通用 pointer 管线 + 拖拽气泡 + 边缘卷入 + 吸附候选。
 * 与具体手势解耦:clip 移动/trim/框选/标尺 scrub 在 gestures.js,素材拖放走 HTML5 DnD。 */

import { h } from "../ui/dom.js";
import { PX_PER_MS, snapMs, frameMsOf, msAdd } from "../core/model.js";
import { projectStore, timelineStore, uiStore, selectionStore, ephemeralStore } from "../core/store.js";
import { pulseSnap } from "./playhead.js";

export const THRESHOLD_PX = 3;     // 点击/拖拽判定阈值
export const EDGE_ZONE_PX = 60;    // 自动卷入视口边缘带宽
export const PLAYHEAD_SNAP_PX = 8; // 播放头吸附半径
export const ZOOM_STEP = 1.1;      // 滚轮缩放步长
export const PULSE_MIN_GAP_MS = 120; // 对齐脉冲最小心跳(防连续频闪)

export function snapAt(ms) {
  return snapMs(ms, uiStore.get().magnet, frameMsOf(projectStore.get().project));
}
export function fmtSec(ms) {
  return `${(ms / 1000).toFixed(2)}s`;
}

/**
 * 单指针手势管线:pointer capture + document 监听(capture 后事件冒泡可达,
 * 源元素被虚拟化移除时手势不死)。h:{ start?, move, end?, cancel?, click? }:
 * 位移过阈值 → start + 连续 move;未过阈值松手 → click;Esc/失焦/pointercancel → cancel。
 * move 经 rAF 合帧(一帧至多一次,60fps 跟手);end 收到最后一次指针事件(含落点)。
 */
export function runGesture(el, e, h) {
  const pid = e.pointerId;
  const x0 = e.clientX;
  const y0 = e.clientY;
  let started = false;
  let done = false;
  let pending = null;
  let rafId = 0;
  try { el.setPointerCapture(pid); } catch { /* 元素已移除:手势自然失效 */ }

  const flush = () => {
    rafId = 0;
    const ev = pending;
    pending = null;
    if (ev && started && !done) h.move(ev);
  };
  const onMove = (ev) => {
    if (ev.pointerId !== pid || done) return;
    if (!started) {
      if (Math.hypot(ev.clientX - x0, ev.clientY - y0) < THRESHOLD_PX) return;
      started = true;
      if (h.start) h.start(ev);
    }
    pending = ev;
    if (!rafId) rafId = requestAnimationFrame(flush);
  };
  const finish = (committed, ev) => {
    if (done) return;
    done = true;
    cleanup();
    if (!started) {
      if (h.click) h.click(e);
      return;
    }
    if (committed && h.end) h.end(ev);
    else if (h.cancel) h.cancel();
  };
  const onUp = (ev) => { if (ev.pointerId === pid) finish(true, ev); };
  const onCancel = (ev) => { if (ev.pointerId === pid) finish(false, ev); };
  const onKey = (ev) => {
    if (ev.key === "Escape" && started) { ev.stopPropagation(); finish(false, ev); }
  };
  const onBlur = () => finish(false, null);
  function cleanup() {
    if (rafId) cancelAnimationFrame(rafId);
    rafId = 0;
    document.removeEventListener("pointermove", onMove);
    document.removeEventListener("pointerup", onUp);
    document.removeEventListener("pointercancel", onCancel);
    document.removeEventListener("keydown", onKey, true);
    window.removeEventListener("blur", onBlur);
    try { el.releasePointerCapture(pid); } catch { /* 已释放 */ }
  }
  document.addEventListener("pointermove", onMove);
  document.addEventListener("pointerup", onUp);
  document.addEventListener("pointercancel", onCancel);
  document.addEventListener("keydown", onKey, true);
  window.addEventListener("blur", onBlur);
}

/* ---------------- 拖拽气泡(单例;trim 时长/移动时间码/标尺悬停) ---------------- */

let bubbleEl = null;
export function showBubble(clientX, clientY, text) {
  const wrap = document.getElementById("timeline-wrap");
  if (!bubbleEl || !bubbleEl.isConnected) {
    bubbleEl = h("div", { class: "cf-bubble", "aria-hidden": "true" });
    wrap.appendChild(bubbleEl);
  }
  const rect = wrap.getBoundingClientRect();
  bubbleEl.style.transform =
    `translate(${Math.round(clientX - rect.left + wrap.scrollLeft + 12)}px,` +
    ` ${Math.round(clientY - rect.top + wrap.scrollTop - 26)}px)`;
  if (bubbleEl.textContent !== text) bubbleEl.textContent = text;
  bubbleEl.classList.add("show");
}
export function hideBubble() {
  if (bubbleEl) bubbleEl.classList.remove("show");
}

/* ---------------- 轨道高亮(兼容落点 / 非法落点红态) ---------------- */

export function highlightLane(laneEl, valid) {
  for (const t of document.querySelectorAll(".track.drop-target, .track.drop-invalid")) {
    t.classList.remove("drop-target", "drop-invalid");
  }
  if (laneEl) laneEl.classList.add(valid ? "drop-target" : "drop-invalid");
}
export function clearLaneHighlight() {
  highlightLane(null, true);
}

/* ---------------- 边缘卷入(拖到视口边缘 60px 内;速度随深入线性) ---------------- */

export function edgeScroll(wrap) {
  let raf = 0;
  let px = 0;
  const tick = () => {
    const rect = wrap.getBoundingClientRect();
    const x = px - rect.left;
    let dx = 0;
    if (x < EDGE_ZONE_PX) dx = -Math.ceil((EDGE_ZONE_PX - x) / 6);
    else if (x > rect.width - EDGE_ZONE_PX) dx = Math.ceil((x - (rect.width - EDGE_ZONE_PX)) / 6);
    if (dx) wrap.scrollLeft += dx;
    raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
  return {
    move(clientX) { px = clientX; },
    stop() { if (raf) cancelAnimationFrame(raf); raf = 0; },
  };
}

/* ---------------- 吸附体系(T4.2 统一候选源)----------------
 * 候选源:帧网格(既有)/ 片段边缘 / 播放头 / 会话标记;主开关 = 磁吸 checkbox;
 * 强度档(prefs snapStrength):loose=仅帧网格,standard=+播放头/片段边缘(8px),
 * strong=+标记(12px)。优先级:播放头 > 标记 > 片段边缘 > 帧网格。 */
import { pref } from "../ui/prefs.js";

export const SNAP_RADIUS_PX = 8;      // standard 档吸附半径
export const SNAP_RADIUS_STRONG_PX = 12;

/** 当前吸附半径(px;强度档)。 */
export function snapRadiusPx() {
  return pref("snapStrength", "standard") === "strong" ? SNAP_RADIUS_STRONG_PX : SNAP_RADIUS_PX;
}

/**
 * 统一吸附候选(T4.2):@param excludeIds 排除自身片段(拖拽中的 clip 边缘不作候选)。
 * @returns {{ ms: number, playhead: boolean, source: string }} source ∈ frame|edge|playhead|marker
 */
export function snapCandidateEx(ms, excludeIds = []) {
  if (!uiStore.get().magnet) return { ms: Math.max(0, Math.round(ms)), playhead: false, source: "frame" };
  const radius = snapRadiusPx() / PX_PER_MS;
  const strength = pref("snapStrength", "standard");
  // 1) 播放头(半径优先,旧口径)
  const ph = selectionStore.get().playheadMs || 0;
  if (strength !== "loose" && ph > 0 && Math.abs(msAdd(ms, -ph)) <= radius) {
    return { ms: Math.max(0, Math.round(ph)), playhead: true, source: "playhead" };
  }
  // 2) 会话标记(strong 档)
  if (strength === "strong") {
    for (const m of ephemeralStore.get().markers || []) {
      if (Math.abs(msAdd(ms, -m)) <= radius) {
        return { ms: Math.max(0, Math.round(m)), playhead: false, source: "marker" };
      }
    }
  }
  // 3) 片段边缘(全部投影行的 start/end;排除拖拽自身与隐藏轨)
  if (strength !== "loose") {
    const hidden = new Set(ephemeralStore.get().hiddenTracks || []);
    const rows = timelineStore.get().clips || [];
    let best = null;
    let bestDist = radius;
    for (const c of rows) {
      if (excludeIds.includes(c.id) || hidden.has(c.track)) continue;
      for (const edge of [c.startMs, c.endMs]) {
        const d = Math.abs(msAdd(ms, -edge));
        if (d <= bestDist) { bestDist = d; best = edge; }
      }
    }
    if (best !== null) return { ms: Math.max(0, Math.round(best)), playhead: false, source: "edge" };
  }
  // 4) 帧网格(旧缺省)
  return { ms: Math.max(0, snapAt(ms)), playhead: false, source: "frame" };
}

/** 旧签名兼容:单参版本(拖拽移动/标尺 scrub 沿用;无排除集)。 */
export function snapCandidate(ms) {
  const c = snapCandidateEx(ms, []);
  return { ms: c.ms, playhead: c.source === "playhead" };
}

/** 对齐脉冲(磁性提示):吸附值变化时 80ms 一次;限频防频闪。 */
export function makePulser() {
  let lastValue = null;
  let lastAt = 0;
  return (cand) => {
    const now = performance.now();
    if (cand.ms !== lastValue && now - lastAt > PULSE_MIN_GAP_MS) {
      pulseSnap(cand.ms);
      lastAt = now;
    }
    lastValue = cand.ms;
  };
}
