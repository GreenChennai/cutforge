/* 标尺 canvas 层(ADR-0012):刻度/网格/播放头三角标记按视口窗口绘制;
 * #ruler 元素本身撑内容宽度(滚动几何与旧壳一致),canvas sticky-left 只画可见段。
 * 点击/拖拽 seek 的手势在 gestures.js(#ruler 是手势锚点,e2e 兼容红线)。
 * 色值一律经 theme token(零硬编码,ADR-0014/R5)。 */
import { $, h } from "../ui/dom.js";
import { PX_PER_MS } from "../core/model.js";
import { cssVar } from "./theme.js";

let canvas = null;
let ctx = null;
let contentW = 0;

export function mountRuler() {
  const ruler = $("ruler");
  canvas = h("canvas", { testid: "ruler-canvas" });
  ruler.appendChild(canvas);
  ctx = canvas.getContext("2d");
  window.addEventListener("resize", () => drawRuler());
}

/** 投影端点变化时设定内容宽度(#ruler 宽度即时间线滚动几何)。 */
export function setRulerContent(contentWidth) {
  contentW = contentWidth;
  $("ruler").style.width = `${contentWidth}px`;
  drawRuler();
}

function ensureSize() {
  const wrap = $("timeline-wrap");
  const w = Math.max(1, Math.min(wrap.clientWidth, contentW || wrap.clientWidth));
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== 22) canvas.height = 22;
  canvas.style.width = `${w}px`;
  return w;
}

/** 重画(滚动/播放头推进/投影变化时调用;只画视口窗口,成本与可见像素相关)。 */
export function drawRuler(playheadMs = null, scrollLeft = null) {
  if (!ctx) return;
  const wrap = $("timeline-wrap");
  const w = ensureSize();
  const sl = scrollLeft === null ? wrap.scrollLeft : scrollLeft;
  ctx.clearRect(0, 0, w, 22);
  ctx.fillStyle = cssVar("--cf-ruler-bg");
  ctx.fillRect(0, 0, w, 22);
  const t0 = Math.max(0, sl / PX_PER_MS);
  const t1 = (sl + w) / PX_PER_MS;
  // 次刻度 100ms、主刻度 1s(视觉层;无 DOM 刻度节点)
  ctx.strokeStyle = cssVar("--cf-ruler-minor");
  ctx.beginPath();
  for (let t = Math.floor(t0 / 100) * 100; t <= t1; t += 100) {
    const x = Math.round(t * PX_PER_MS - sl) + 0.5;
    ctx.moveTo(x, 15);
    ctx.lineTo(x, 22);
  }
  ctx.stroke();
  ctx.strokeStyle = cssVar("--cf-ruler-major");
  ctx.fillStyle = cssVar("--cf-ruler-text");
  ctx.font = `10px ${cssVar("--cf-font-mono")}`;
  ctx.beginPath();
  for (let t = Math.max(0, Math.floor(t0 / 1000) * 1000); t <= t1; t += 1000) {
    const x = Math.round(t * PX_PER_MS - sl) + 0.5;
    ctx.moveTo(x, 8);
    ctx.lineTo(x, 22);
    ctx.fillText(`${t / 1000}s`, x + 3, 9);
  }
  ctx.stroke();
  // 播放头三角标记
  if (playheadMs !== null) {
    const x = Math.round(playheadMs * PX_PER_MS - sl);
    if (x >= -6 && x <= w + 6) {
      ctx.fillStyle = cssVar("--cf-playhead");
      ctx.beginPath();
      ctx.moveTo(x - 5, 0);
      ctx.lineTo(x + 5, 0);
      ctx.lineTo(x, 7);
      ctx.closePath();
      ctx.fill();
    }
  }
}
