/* 播放头(ADR-0012 决策 3):DOM 单线 + transform(合成器路径,零重排);
 * 另持有 #tl-overlay 视口画布:拖拽吸附线等瞬时指示(仅手势期重画)。 */
import { $, h } from "../ui/dom.js";
import { PX_PER_MS } from "../core/model.js";

let line = null;
let overlay = null;
let overlayCtx = null;

export function mountPlayhead() {
  const wrap = $("timeline-wrap");
  line = $("playhead");
  overlay = $("tl-overlay");
  overlayCtx = overlay.getContext("2d");
  // 旧壳把 playhead 放在 wrap 之下由 js 定位;这里保证挂载在 wrap 内(绝对定位坐标系)
  if (line.parentElement !== wrap) wrap.appendChild(line);
  if (overlay.parentElement !== wrap) wrap.appendChild(overlay);
  window.addEventListener("resize", resizeOverlay);
  resizeOverlay();
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

/** 视口画布重画:横向伪 sticky(translateX 跟随滚动)+ 吸附线。 */
export function drawOverlay(snapMsVal = null) {
  const wrap = $("timeline-wrap");
  resizeOverlay();
  const sl = wrap.scrollLeft;
  overlay.style.transform = `translateX(${sl}px)`;
  overlayCtx.clearRect(0, 0, overlay.width, overlay.height);
  if (snapMsVal !== null && snapMsVal !== undefined) {
    const x = Math.round(snapMsVal * PX_PER_MS - sl) + 0.5;
    if (x >= 0 && x <= overlay.width) {
      overlayCtx.strokeStyle = "rgba(77,163,255,.8)";
      overlayCtx.setLineDash([4, 3]);
      overlayCtx.beginPath();
      overlayCtx.moveTo(x, 0);
      overlayCtx.lineTo(x, overlay.height);
      overlayCtx.stroke();
      overlayCtx.setLineDash([]);
    }
  }
}
