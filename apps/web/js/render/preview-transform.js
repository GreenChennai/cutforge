/* 预览区变换交互(册四 T4.9 FE2):选中片段时在画布代理上叠加变换把手层。
 *
 * - 画中画/变换片段(overlay 或 scale≠1 / rotation≠0 / flip):角柄=缩放、顶柄=旋转,
 *   松手各一笔 clip_update(拖拽全程零 Op,Esc/失焦取消);九宫格吸附说明见下;
 * - 文本片段:画布内直接拖拽 → patch.textStyle.{x,y}(PlayRes=画布像素,所见即所得;
 *   换算经预览缩放比,canvas 坐标 = 客户坐标 × canvas.width/stageRect.width);
 * - position 诚实只读:ui-fields readonly(内核仅 overlay_add 通道可设),把手层只展示
 *   数值并标注「只读」,不假实现拖移;
 * - 变换可视化:canvas 代理按投影绘制 overlay/position/scale(preview.js 既有);
 *   flip/crop 为渲染链效果,代理不呈现(诚实标注于 HUD)。
 */
import { h } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore } from "../core/store.js";
import { updateClip } from "../core/commands.js";
import { clipKindOf } from "../core/model.js";
import { runGesture } from "./gesture-kit.js";
import { toast } from "../ui/toast.js";

const SCALE_MIN = 0.05;
const SCALE_MAX = 4;

let stage = null;
let layer = null;
let hud = null;
let ghost = null;
let curId = null;
/** 把手层元素引用(手势期直改几何,免重建)。 */
let frameEl = null;
let cornerEl = null;
let rotEl = null;

export function mountPreviewTransform() {
  stage = document.getElementById("pv-stage");
  if (!stage) return;
  stage.classList.add("pv-stage-rel");
  layer = h("div", { class: "pv-transform-layer", "aria-hidden": "true" });
  hud = h("div", { class: "pv-hud", testid: "pv-transform-hud" });
  ghost = h("div", { class: "pv-text-ghost", testid: "pv-text-ghost", hidden: true });
  stage.appendChild(layer);
  stage.appendChild(ghost);
  stage.insertAdjacentElement("afterend", hud);
  layer.addEventListener("pointerdown", onLayerDown);
  selectionStore.subscribe(() => refresh());
  timelineStore.subscribe(() => refresh());
  refresh();
}

function selectedRow() {
  const id = selectionStore.get().clipId;
  return id ? timelineStore.get().clips.find((c) => c.id === id) || null : null;
}

/** 画布坐标 → stage 客户像素的缩放比(pv-canvas max-width/height 自适应)。 */
function stageScale() {
  const canvas = document.getElementById("pv-canvas");
  if (!canvas || !stage) return { sx: 1, sy: 1, rect: null };
  const rect = canvas.getBoundingClientRect();
  return {
    sx: rect.width / (canvas.width || 1),
    sy: rect.height / (canvas.height || 1),
    rect,
  };
}

/** 片段在画布中的显示矩形(canvas 像素;与 preview.drawProxy 同一口径)。 */
function frameRectOf(row, canvasW, canvasH) {
  const ov = row.overlay;
  const pos = row.position;
  const scale = row.scale || 1;
  if (ov) return { x: ov.x, y: ov.y, w: ov.w, h: ov.h };
  const w = canvasW * scale;
  const hgt = canvasH * scale;
  const dx = pos ? (pos.x / 100) * (canvasW - w) : (canvasW - w) / 2;
  const dy = pos ? (pos.y / 100) * (canvasH - hgt) : (canvasH - hgt) / 2;
  return { x: dx, y: dy, w, h: hgt };
}

/** 重建把手层与 HUD(选中/投影变化)。 */
function refresh() {
  if (!layer) return;
  while (layer.firstChild) layer.removeChild(layer.firstChild);
  frameEl = null; cornerEl = null; rotEl = null;
  const row = selectedRow();
  const canvas = document.getElementById("pv-canvas");
  if (!row || !canvas) { hud.textContent = ""; curId = null; return; }
  curId = row.id;
  const cw = canvas.width;
  const ch = canvas.height;
  const kind = clipKindOf(row);
  const isText = kind === "text" || Boolean(row.textStyle) || row.text != null;
  const transformed = row.overlay || (row.scale && row.scale !== 1)
    || (row.rotation && row.rotation !== 0) || row.flip || row.crop;
  if (isText) {
    hud.textContent = "文本:画布内按住拖动 = 改位置(textStyle.x/y,松手提交);样式在检查器·文本组";
    hud.dataset.mode = "text";
    // 拖拽面:图层空置时 CSS :empty 断接(pointer-events:none)——文本选中态铺全画布
    // 拖拽面(画布代理无点击交互,遮挡无害;点选不拖 = 松手无变更不产 Op)
    layer.appendChild(h("div", {
      class: "pv-text-dragzone", testid: "pv-text-dragzone", role: "button",
      title: "按住拖动改文本位置(松手一笔 clip_update,可撤销;Esc 取消)",
      "aria-label": "文本位置拖拽区(画布全域)",
    }));
    return;
  }
  if (!transformed) {
    hud.textContent = "选中片段无变换;画中画/缩放/旋转:检查器·画面组,或 overlay 片段出现把手";
    hud.dataset.mode = "none";
    return;
  }
  const { sx, sy } = stageScale();
  const r = frameRectOf(row, cw, ch);
  hud.dataset.mode = "transform";
  hud.textContent = `缩放 ${Number(row.scale || 1).toFixed(2)}x · 旋转 ${Math.round(row.rotation || 0)}°`
    + (row.flip ? ` · 翻转${row.flip === "h" ? "水平" : "垂直"}` : "")
    + (row.position ? ` · 位置 ${Math.round(row.position.x)}/${Math.round(row.position.y)}%(只读)` : "")
    + "(flip/crop 走渲染链,代理不呈现)";
  // 角柄(右下)= 缩放;顶柄 = 旋转;框 = 当前显示矩形
  frameEl = h("div", { class: "pv-transform-frame" });
  cornerEl = h("div", {
    class: "pv-handle pv-handle-scale", testid: "pv-handle-scale", role: "button",
    title: "拖动缩放(松手一笔 clip_update)",
  });
  rotEl = h("div", {
    class: "pv-handle pv-handle-rot", testid: "pv-handle-rot", role: "button",
    title: "拖动旋转(松手一笔 clip_update)",
  });
  layer.appendChild(frameEl);
  layer.appendChild(cornerEl);
  layer.appendChild(rotEl);
  layoutTransform(r, sx, sy);
}

/** 按显示矩形(canvas px)摆放把手(手势期复用)。 */
function layoutTransform(r, sx, sy) {
  if (!frameEl) return;
  frameEl.style.left = `${r.x * sx}px`;
  frameEl.style.top = `${r.y * sy}px`;
  frameEl.style.width = `${r.w * sx}px`;
  frameEl.style.height = `${r.h * sy}px`;
  cornerEl.style.left = `${r.x * sx + r.w * sx}px`;
  cornerEl.style.top = `${r.y * sy + r.h * sy}px`;
  rotEl.style.left = `${r.x * sx + (r.w * sx) / 2}px`;
  rotEl.style.top = `${r.y * sy - 14}px`;
}

function onLayerDown(e) {
  const row = selectedRow();
  if (!row) return;
  const kind = clipKindOf(row);
  // 文本片段:画布全域即拖拽面(位置);变换片段:按把手命中分发
  if (kind === "text" || row.text != null) { runTextDrag(row, e); return; }
  const handle = /** @type {HTMLElement} */ (e.target);
  if (handle.classList.contains("pv-handle-scale")) runScale(row, e);
  else if (handle.classList.contains("pv-handle-rot")) runRotate(row, e);
}

function canvasPoint(ev) {
  const canvas = document.getElementById("pv-canvas");
  const rect = canvas.getBoundingClientRect();
  return {
    x: (ev.clientX - rect.left) * (canvas.width / rect.width),
    y: (ev.clientY - rect.top) * (canvas.height / rect.height),
  };
}

/** 缩放手势:角柄拖动,比例 = 距中心距离比(与投影 frameRect 同心)。 */
function runScale(row0, e) {
  const canvas = document.getElementById("pv-canvas");
  const cw = canvas.width;
  const ch = canvas.height;
  const center = { x: cw / 2, y: ch / 2 };
  const p0 = canvasPoint(e);
  const d0 = Math.max(8, Math.hypot(p0.x - center.x, p0.y - center.y));
  const s0 = row0.scale || 1;
  let value = s0;
  runGesture(/** @type {HTMLElement} */ (e.target), e, {
    move: (ev) => {
      const p = canvasPoint(ev);
      const d = Math.max(8, Math.hypot(p.x - center.x, p.y - center.y));
      value = Math.min(SCALE_MAX, Math.max(SCALE_MIN, s0 * (d / d0)));
      hud.textContent = `缩放 ${value.toFixed(2)}x(松手提交)`;
      const { sx, sy } = stageScale();
      layoutTransform(frameRectOf({ ...row0, scale: value }, cw, ch), sx, sy);
    },
    end: () => {
      const row = selectedRow();
      if (!row) return;
      if (Math.abs(value - s0) < 0.01) { refresh(); return; }
      updateClip(row.id, { scale: Number(value.toFixed(3)) }, `缩放 ${value.toFixed(2)}x(可撤销)`);
    },
    cancel: refresh,
  });
}

/** 旋转手势:顶柄绕中心角度;投影 frameRect 中心为轴(渲染口径:画布内旋转)。 */
function runRotate(row0, e) {
  const canvas = document.getElementById("pv-canvas");
  const center = { x: canvas.width / 2, y: canvas.height / 2 };
  const p0 = canvasPoint(e);
  const a0 = Math.atan2(p0.y - center.y, p0.x - center.x);
  const r0 = row0.rotation || 0;
  let value = r0;
  runGesture(/** @type {HTMLElement} */ (e.target), e, {
    move: (ev) => {
      const p = canvasPoint(ev);
      const a = Math.atan2(p.y - center.y, p.x - center.x);
      const deg = (a - a0) * 180 / Math.PI;
      value = Math.max(-180, Math.min(180, Math.round(r0 + deg)));
      hud.textContent = `旋转 ${value}°(松手提交;±90 精确互换宽高,其余画布内旋转)`;
    },
    end: () => {
      const row = selectedRow();
      if (!row) return;
      if (value === r0) { refresh(); return; }
      updateClip(row.id, { rotation: value }, `旋转 ${value}°(可撤销)`);
    },
    cancel: refresh,
  });
}

/** 文本拖拽(由文本选中态在 layer 上直接下笔;layer 覆盖 stage 全域)。 */
function runTextDrag(row0, e) {
  const p0 = canvasPoint(e);
  const ts = row0.textStyle || {};
  const x0 = ts.x != null ? ts.x : p0.x;
  const y0 = ts.y != null ? ts.y : p0.y;
  let value = { x: x0, y: y0 };
  runGesture(stage, e, {
    start: () => {
      ghost.hidden = false;
      ghost.textContent = row0.text || "文本";
    },
    move: (ev) => {
      const p = canvasPoint(ev);
      const canvas = document.getElementById("pv-canvas");
      value = {
        x: Math.max(0, Math.min(canvas.width, Math.round(x0 + (p.x - p0.x)))),
        y: Math.max(0, Math.min(canvas.height, Math.round(y0 + (p.y - p0.y)))),
      };
      ghost.style.left = `${(value.x / canvas.width) * 100}%`;
      ghost.style.top = `${(value.y / canvas.height) * 100}%`;
      ghost.dataset.xy = `${value.x},${value.y}`;
    },
    end: () => {
      ghost.hidden = true;
      const row = selectedRow();
      if (!row) return;
      if (value.x === x0 && value.y === y0) return;
      updateClip(row.id, {
        textStyle: { ...(row.textStyle || {}), x: value.x, y: value.y },
      }, `文本位置 ${value.x},${value.y}(可撤销;精确预览可见)`);
    },
    cancel: () => { ghost.hidden = true; },
  });
}
