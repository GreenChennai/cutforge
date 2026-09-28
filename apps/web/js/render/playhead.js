/* 播放头(ADR-0012 决策 3):DOM 单线 + transform(合成器路径,零重排);
 * 另持有 #tl-overlay 视口画布:拖拽吸附线/会话标记/入出点带等瞬时指示(仅变化期重画)。
 * T3.2:#snap-pulse 吸附对齐脉冲(80ms 一次性)与 .smooth 离散 seek 平滑(120ms)。
 * T3.4:overlay 追加画「会话标记刻度 + 入出点区间带」(纯会话态,ADR-0013 口径)。 */
import { $, h } from "../ui/dom.js";
import { PX_PER_MS } from "../core/model.js";
import { selectionStore, ephemeralStore } from "../core/store.js";
import { cssVar } from "./theme.js";

let line = null;
let overlay = null;
let overlayCtx = null;
let snapEl = null;
let smoothTimer = 0;

export function mountPlayhead() {
  const wrap = $("timeline-wrap");
  line = $("playhead");
  overlay = $("tl-overlay");
  overlayCtx = overlay.getContext("2d");
  // 旧壳把 playhead 放在 wrap 之下由 js 定位;这里保证挂载在 wrap 内(绝对定位坐标系)
  if (line.parentElement !== wrap) wrap.appendChild(line);
  if (overlay.parentElement !== wrap) wrap.appendChild(overlay);
  if (!snapEl) {
    snapEl = h("div", { id: "snap-pulse", "aria-hidden": "true" });
    wrap.appendChild(snapEl);
  }
  window.addEventListener("resize", resizeOverlay);
  resizeOverlay();
  // 标记变化 → 只重画 overlay(拖拽期 ephemeral.* 高频 patch 不走此路径,零 DOM 变更)
  ephemeralStore.subscribe((patch) => {
    if (patch.markers !== undefined) drawOverlay(ephemeralStore.get().snapMs);
  });
  selectionStore.subscribe((patch) => {
    if (patch.inMs !== undefined || patch.outMs !== undefined) drawOverlay(null);
  });
}

function resizeOverlay() {
  const wrap = $("timeline-wrap");
  const w = Math.max(1, wrap.clientWidth);
  const hpx = Math.max(1, wrap.clientHeight - 22);
  if (overlay.width !== w) overlay.width = w;
  if (overlay.height !== hpx) overlay.height = hpx;
  overlay.style.width = `${w}px`;
  overlay.style.height = `${hpx}px`;
}

/** 播放循环每帧调用:只碰 transform,不触发任何布局。 */
export function setPlayheadMs(ms) {
  line.style.transform = `translateX(${Math.round(ms * PX_PER_MS)}px)`;
}

/** 直接以像素定位(虚拟化/滚动重算用;内容坐标 px)。 */
export function setPlayheadPx(x) {
  line.style.transform = `translateX(${Math.round(x)}px)`;
}

/** 离散 seek 平滑(非播放路径):一次性挂 .smooth(120ms transform 过渡)后摘除;
 * 播放/scrub 的每帧写入路径绝不挂载(reduced-motion 由 motion.css 全量降级)。 */
export function smoothSeek() {
  if (!line) return;
  line.classList.add("smooth");
  clearTimeout(smoothTimer);
  smoothTimer = setTimeout(() => line.classList.remove("smooth"), 140);
}

/** 吸附对齐脉冲(T3.2):定位走 transform,重放 80ms 动画一次。 */
export function pulseSnap(ms) {
  if (!snapEl) return;
  snapEl.style.setProperty("--cf-snap-x", `${Math.round(ms * PX_PER_MS)}px`);
  snapEl.classList.remove("pulse-on");
  void snapEl.offsetWidth; // 强制回流以重放动画
  snapEl.classList.add("pulse-on");
}

/** 清除脉冲(手势收尾;避免残影停在 opacity 0 之外的异常态)。 */
export function clearSnapPulse() {
  if (snapEl) snapEl.classList.remove("pulse-on");
}

/** 视口画布重画:横向伪 sticky(translateX 跟随滚动)+ 吸附线/标记/入出点带
 * (色值经 theme token,零硬编码)。 */
export function drawOverlay(snapMsVal = null) {
  const wrap = $("timeline-wrap");
  resizeOverlay();
  const sl = wrap.scrollLeft;
  overlay.style.transform = `translateX(${sl}px)`;
  overlayCtx.clearRect(0, 0, overlay.width, overlay.height);
  // 入出点区间带(T3.4:selection.inMs/outMs;会话态)
  const { inMs, outMs } = selectionStore.get();
  if (inMs !== null && outMs !== null && outMs > inMs) {
    const x0 = Math.round(inMs * PX_PER_MS - sl);
    const w = Math.round((outMs - inMs) * PX_PER_MS);
    overlayCtx.fillStyle = cssVar("--cf-select-glow");
    overlayCtx.fillRect(x0, 0, w, overlay.height);
  }
  // 会话标记刻度(T3.4:ephemeral.markers;旗标小三角)
  overlayCtx.fillStyle = cssVar("--cf-marker");
  for (const m of ephemeralStore.get().markers || []) {
    const x = Math.round(m * PX_PER_MS - sl) + 0.5;
    if (x < -6 || x > overlay.width + 6) continue;
    overlayCtx.beginPath();
    overlayCtx.moveTo(x - 4, 0);
    overlayCtx.lineTo(x + 4, 0);
    overlayCtx.lineTo(x, 6);
    overlayCtx.closePath();
    overlayCtx.fill();
  }
  if (snapMsVal !== null && snapMsVal !== undefined) {
    const x = Math.round(snapMsVal * PX_PER_MS - sl) + 0.5;
    if (x >= 0 && x <= overlay.width) {
      overlayCtx.strokeStyle = cssVar("--cf-snap-line");
      overlayCtx.setLineDash([4, 3]);
      overlayCtx.beginPath();
      overlayCtx.moveTo(x, 0);
      overlayCtx.lineTo(x, overlay.height);
      overlayCtx.stroke();
      overlayCtx.setLineDash([]);
    }
  }
}
