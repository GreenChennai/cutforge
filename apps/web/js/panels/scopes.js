/* 示波器面板(册五 T5.2 FE;波形/矢量/直方图三画布,数据 JSON 来自 scope_data)。
 *
 * 数据链:render_frame(含调色的精确帧,工程内相对路径)→ scope_data(src=帧)→
 * 三类数据 canvas 绘制;render_frame 不可用时诚实降级 = 直吃覆盖片段源素材
 * (标注「源域,未调色」);两者都失败 → 服务端错误如实呈现,不做假图。
 * 性能预算:只在面板开启时采样(关 = 停订阅去抖;开 = 手动按钮 + 播放头静止
 * 1.2s 去抖自动采样);绘制全部经 theme token,零硬编码色。
 */
import { $, h } from "../ui/dom.js";
import { call } from "../core/api.js";
import { timelineStore, selectionStore } from "../core/store.js";
import { playback } from "../render/preview-loop.js";
import { sourceSecAt } from "../core/model.js";
import { projectStore } from "../core/store.js";
import { precisePreview } from "../core/render-commands.js";
import { messageOf } from "../core/errors.js";
import { toast } from "../ui/toast.js";
import { cssVar } from "../render/theme.js";

const WAVE_W = 264;
const WAVE_H = 110;
const VEC_SIZE = 120;
const DEBOUNCE_MS = 1200;

let open = false;
let sampling = false;
let debounceTimer = 0;
let host = null;
let statusEl = null;
let noteEl = null;
let waveCv = null;
let vecCv = null;
let histCv = null;
let unsubPlayhead = null;

/** 装配(挂到 #pv-col,预览之下;main.js 调用)。 */
export function mountScopes() {
  const col = $("pv-col");
  if (!col || !$("preview")) return;
  host = h("div", { id: "scope-panel", class: "panel scope-panel", testid: "scopes-panel", hidden: true });
  const toggle = h("button", {
    id: "pv-scopes-toggle", testid: "scopes-toggle",
    title: "示波器(亮度波形/矢量/直方图;开启才采样)",
    "aria-pressed": "false",
    onclick: () => setOpen(!open),
  }, ["示波器"]);
  const bar = h("div", { class: "scope-bar" }, [
    h("b", null, ["示波器"]),
    h("button", {
      class: "mini", type: "button", testid: "scopes-sample",
      title: "采样播放头所在帧(render_frame 精确帧 → scope_data)",
      onclick: () => sample(),
    }, ["采样当前帧"]),
    h("span", { class: "dim", testid: "scopes-status" }, ["未采样"]),
  ]);
  host.appendChild(bar);
  noteEl = h("div", { class: "hint", testid: "scopes-note" }, [""]);
  host.appendChild(noteEl);
  waveCv = h("canvas", { width: String(WAVE_W), height: String(WAVE_H), testid: "scope-wave", "aria-label": "亮度波形" });
  vecCv = h("canvas", { width: String(VEC_SIZE), height: String(VEC_SIZE), testid: "scope-vector", "aria-label": "矢量示波器" });
  histCv = h("canvas", { width: String(WAVE_W), height: String(WAVE_H), testid: "scope-hist", "aria-label": "RGB 直方图" });
  const row = h("div", { class: "scope-canvases" }, [
    h("figure", null, [waveCv, h("figcaption", { class: "dim" }, ["亮度波形(luma 0..255)"])]),
    h("figure", null, [vecCv, h("figcaption", { class: "dim" }, ["矢量(UV 64×64)"])]),
    h("figure", null, [histCv, h("figcaption", { class: "dim" }, ["RGB 直方图(64 桶)"])]),
  ]);
  host.appendChild(row);
  statusEl = bar.querySelector('[data-testid="scopes-status"]');
  col.insertBefore(host, $("bgm-panel"));
  // 顶栏入口按钮(预览传输行尾追加;保持预览面板不动)
  const transport = $("pv-transport");
  if (transport) transport.appendChild(toggle);
}

function setOpen(v) {
  open = v;
  if (host) host.hidden = !v;
  const btn = $("pv-scopes-toggle");
  if (btn) {
    btn.setAttribute("aria-pressed", v ? "true" : "false");
    btn.classList.toggle("on", v);
  }
  if (unsubPlayhead) { unsubPlayhead(); unsubPlayhead = null; }
  if (v) {
    // 只在开启时订阅播放头(静止 1.2s 去抖采样;关闭即摘除)
    let lastMs = -1;
    unsubPlayhead = selectionStore.subscribe((patch) => {
      if (patch.playheadMs === undefined || patch.playheadMs === lastMs) return;
      lastMs = patch.playheadMs;
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => sample(), DEBOUNCE_MS);
    });
    sample();
  } else {
    clearTimeout(debounceTimer);
    if (statusEl) statusEl.textContent = "已关闭(不再采样)";
  }
}

/** 采样:精确帧(含调色)优先;降级 = 覆盖片段源素材(未调色,如实标注)。 */
async function sample() {
  if (!open || sampling) return;
  sampling = true;
  const atMs = Math.round(playback.clockMs());
  try {
    if (statusEl) statusEl.textContent = `采样中 @${atMs}ms…`;
    const frame = await precisePreview(atMs);
    let src = "";
    let srcAtMs = 0;
    let graded = true;
    if (frame.ok) {
      src = frame.data && (frame.data.media || frame.data.path);
    } else {
      const cov = coveringClip(atMs);
      if (!cov) {
        if (statusEl) statusEl.textContent = `不可采样:${messageOf(frame, "render_frame")}`;
        return;
      }
      src = cov.row.src;
      srcAtMs = Math.round(sourceSecAt(cov.row, atMs) * 1000);
      graded = false;
    }
    const env = await call("scope_data", { src, atMs: srcAtMs || undefined });
    if (!env.ok) {
      if (statusEl) statusEl.textContent = `scope_data 失败:${env.code}`;
      toast(`示波器:${messageOf(env, "scope_data")}`, false);
      return;
    }
    const scope = env.data && env.data.scope;
    if (!scope) {
      if (statusEl) statusEl.textContent = "scope_data 无数据";
      return;
    }
    drawAll(scope);
    noteEl.textContent = graded
      ? `数据域:精确帧 @${atMs}ms(含调色;render_frame${env.data.cached ? ",缓存命中" : ""})`
      : `数据域:源素材 ${cov2label(src)}(render_frame 不可用,降级;未调色,诚实口径)`;
    if (statusEl) statusEl.textContent = `已采样 @${atMs}ms`;
  } finally {
    sampling = false;
  }
}

function cov2label(src) {
  return String(src).split("/").pop() || src;
}

function coveringClip(tMs) {
  for (const row of timelineStore.get().clips || []) {
    if (row.src && row.track && row.track.charAt(0) !== "A" && tMs >= row.startMs && tMs < row.endMs) {
      return { row };
    }
  }
  return null;
}

/* ---------------- 三画布绘制(token 取色,零硬编码) ---------------- */

function drawAll(scope) {
  drawWave(scope.waveform || {});
  drawVector(scope.vectorscope || {});
  drawHist(scope.histogram || {});
}

/** 亮度波形:每列 [min,max,avg](0..255)→ 竖线(min-max)+ avg 点。 */
function drawWave(wf) {
  const ctx = waveCv.getContext("2d");
  ctx.fillStyle = cssVar("--cf-sunken") || "";
  ctx.fillRect(0, 0, WAVE_W, WAVE_H);
  const cols = wf.columns || [];
  if (!cols.length) return;
  const n = cols.length;
  const bw = WAVE_W / n;
  ctx.strokeStyle = cssVar("--cf-gray-500") || "";
  ctx.beginPath();
  for (let i = 0; i < n; i += 1) {
    const [mn, mx] = cols[i];
    const x = i * bw + bw / 2;
    ctx.moveTo(x, WAVE_H - (mx / 255) * WAVE_H);
    ctx.lineTo(x, WAVE_H - (mn / 255) * WAVE_H);
  }
  ctx.stroke();
  ctx.fillStyle = cssVar("--cf-accent") || "";
  for (let i = 0; i < n; i += 1) {
    const avg = cols[i][2];
    ctx.fillRect(i * bw + bw / 2 - 1, WAVE_H - (avg / 255) * WAVE_H - 1, 2, 2);
  }
}

/** 矢量:UV 64×64 计数网格(行优先 V 外 U 内;UV ±0.5)→ 密度热图 + 十字准线。 */
function drawVector(vs) {
  const ctx = vecCv.getContext("2d");
  const S = VEC_SIZE;
  ctx.fillStyle = cssVar("--cf-sunken") || "";
  ctx.fillRect(0, 0, S, S);
  const grid = vs.grid || [];
  const bins = vs.bins || 64;
  if (!grid.length) return;
  let maxC = 1;
  for (const c of grid) maxC = Math.max(maxC, c);
  const cell = S / bins;
  for (let row = 0; row < bins; row += 1) {
    for (let u = 0; u < bins; u += 1) {
      const c = grid[row * bins + u] || 0;
      if (!c) continue;
      const t = Math.min(1, c / maxC);
      // 密度 → 灰阶插值(深底 → 亮),数值计算的非主题展示色(数据面,同取色器光谱)
      const g = grayCss(0.05 + 0.95 * t);
      ctx.fillStyle = g;
      const cx = ((u + 0.5) / bins - 0.5) * S;
      const cy = ((row + 0.5) / bins - 0.5) * S;
      ctx.fillRect(S / 2 + cx - cell / 2, S / 2 + cy - cell / 2, Math.ceil(cell), Math.ceil(cell));
    }
  }
  ctx.strokeStyle = cssVar("--cf-line-strong") || "";
  ctx.beginPath();
  ctx.moveTo(S / 2, 0); ctx.lineTo(S / 2, S);
  ctx.moveTo(0, S / 2); ctx.lineTo(S, S / 2);
  ctx.stroke();
}

/** 密度 t(0..1)→ 灰阶色串(hex 由数值拼出;源码零色值字面量,R5 纪律)。 */
function grayCss(t) {
  const v = Math.round(255 * Math.min(1, Math.max(0, t)));
  const hx = v.toString(16).padStart(2, "0");
  return `#${hx}${hx}${hx}`;
}

/** RGB 直方图:三通道 64 桶计数 → 归一折线下面积(通道色 = token)。 */
function drawHist(hs) {
  const ctx = histCv.getContext("2d");
  ctx.fillStyle = cssVar("--cf-sunken") || "";
  ctx.fillRect(0, 0, WAVE_W, WAVE_H);
  const chans = [
    ["r", cssVar("--cf-red-450") || ""],
    ["g", cssVar("--cf-green-450") || ""],
    ["b", cssVar("--cf-blue-500") || ""],
  ];
  for (const [key, color] of chans) {
    const arr = hs[key] || [];
    if (!arr.length) continue;
    let maxC = 1;
    for (const c of arr) maxC = Math.max(maxC, c);
    const n = arr.length;
    const bw = WAVE_W / n;
    ctx.globalAlpha = 0.55;
    ctx.fillStyle = color;
    ctx.strokeStyle = color;
    ctx.beginPath();
    ctx.moveTo(0, WAVE_H);
    for (let i = 0; i < n; i += 1) {
      ctx.lineTo(i * bw + bw / 2, WAVE_H - (arr[i] / maxC) * (WAVE_H - 4));
    }
    ctx.lineTo(WAVE_W, WAVE_H);
    ctx.closePath();
    ctx.fill();
    ctx.globalAlpha = 1;
  }
}
