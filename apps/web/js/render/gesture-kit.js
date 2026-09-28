/* 手势基座(T3.3):通用 pointer 管线 + 拖拽气泡 + 边缘卷入 + 吸附候选。
 * 与具体手势解耦:clip 移动/trim/框选/标尺 scrub 在 gestures.js,素材拖放走 HTML5 DnD。 */

import { h } from "../ui/dom.js";
import { PX_PER_MS, snapMs, frameMsOf } from "../core/model.js";
import { projectStore, uiStore, selectionStore } from "../core/store.js";
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

/* ---------------- 吸附候选(播放头半径优先,否则帧磁吸)+ 对齐脉冲 ---------------- */

export function snapCandidate(ms) {
  const ph = selectionStore.get().playheadMs || 0;
  if (ph > 0 && Math.abs(ms - ph) <= PLAYHEAD_SNAP_PX / PX_PER_MS) {
    return { ms: Math.max(0, Math.round(ph)), playhead: true };
  }
  return { ms: Math.max(0, snapAt(ms)), playhead: false };
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
