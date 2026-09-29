/* 波形层(T4.1 真 peaks;ADR-0012 重绘层成员):音频片段内嵌小画布 + 素材卡迷你波形。
 * 数据:media_peaks 多级分辨率 min/max 桶(经 core/media-cache 缓存);密度档随缩放
 * (peaksLevelFor)联动。诚实口径:peaks 不可用(无 ffmpeg/无音轨)时回落装饰性纹理,
 * 不冒充真数据。色值经 theme token(零硬编码)。 */
import { clipKindOf, PX_PER_MS } from "../core/model.js";
import { cssVar } from "./theme.js";
import { peaksFor, peaksLevelFor } from "../core/media-cache.js";
import { projectStore } from "../core/store.js";

/** 峰值桶采样:把 [0,1) 的像素列映射到桶下标,取窗口内最大幅度。 */
function bucketAmp(peaks, buckets, from, to) {
  const a = Math.max(0, Math.min(buckets - 1, Math.floor(from * buckets)));
  const b = Math.max(a, Math.min(buckets - 1, Math.ceil(to * buckets)));
  let amp = 0;
  for (let i = a; i <= b; i += 1) {
    const v = Math.abs(peaks.min[i] ?? 0) > Math.abs(peaks.max[i] ?? 0)
      ? Math.abs(peaks.min[i]) : Math.abs(peaks.max[i] ?? 0);
    if (v > amp) amp = v;
  }
  return amp;
}

/** 在 ctx 上画一版波形(pxPerCol 列宽;sourceFrom/sourceTo 为素材源时间秒区间)。 */
function paintPeaks(ctx, w, hgt, peaks, sourceFrom, sourceTo) {
  const buckets = peaks.buckets || peaks.min.length;
  const span = Math.max(1e-6, sourceTo - sourceFrom);
  ctx.fillStyle = cssVar("--cf-wave-line");
  for (let x = 0; x < w; x += 2) {
    const f0 = sourceFrom + (x / w) * span;
    const f1 = sourceFrom + ((x + 2) / w) * span;
    const b0 = f0 / (peaks.durationMs / 1000 || span);
    const b1 = f1 / (peaks.durationMs / 1000 || span);
    const amp = bucketAmp(peaks, buckets, b0, b1);
    const hpx = Math.max(2, Math.min(hgt, amp * hgt));
    ctx.fillRect(x, (hgt - hpx) / 2, 2, hpx);
  }
}

/** 装饰性纹理(peaks 不可用时的诚实回落:与旧纹理同观感)。 */
function paintTexture(ctx, w, hgt) {
  ctx.fillStyle = cssVar("--cf-wave-line");
  for (let x = 2; x < w; x += 6) {
    const hpx = 6 + ((x * 2654435761) % 14);
    ctx.fillRect(x, (hgt - hpx) / 2, 2, hpx);
  }
}

/** 音频片段内嵌波形。宽变即重画;peaks 到达时若节点已断开/换源则放弃本次绘制。
 * row:投影行(widthPx 片段像素宽;sourceInMs/speed 决定素材源窗口)。 */
export function drawWaveIfAudio(clipEl, row, widthPx) {
  if (clipKindOf(row) !== "audio") return;
  let cv = clipEl.querySelector("canvas.clip-wave");
  const w = Math.max(0, Math.floor(widthPx) - 4);
  if (w < 12) {
    if (cv) cv.remove();
    return;
  }
  const hgt = 28;
  if (!cv) {
    cv = document.createElement("canvas");
    cv.className = "clip-wave";
    cv.setAttribute("aria-hidden", "true");
    cv.style.cssText = "position:absolute;inset:0;width:100%;height:100%;opacity:.5;pointer-events:none;";
    clipEl.insertBefore(cv, clipEl.firstChild);
  }
  const token = projectStore.get().token || "";
  const level = peaksLevelFor(PX_PER_MS); // ESM live binding:缩放后重画自动升档
  const src = row.src || "";
  // 先落一版纹理保底,peaks 到达后整幅替换(异步不阻塞时间线渲染路径)
  if (cv.width !== w) {
    cv.width = w;
    cv.height = hgt;
    paintTexture(cv.getContext("2d"), w, hgt);
  }
  const myToken = token;
  peaksFor(src, level).then((p) => {
    if (!p.ok || !cv.isConnected) return;
    if ((projectStore.get().token || "") !== myToken) return;
    const spd = row.speed || 1;
    const from = (row.sourceInMs || 0) / 1000;
    const to = from + ((row.endMs - row.startMs) * spd) / 1000;
    const ctx = cv.getContext("2d");
    ctx.clearRect(0, 0, w, hgt);
    paintPeaks(ctx, w, hgt, p, from, to);
  }).catch(() => { /* peaks 失败:保持纹理回落 */ });
}

/**
 * 素材卡迷你波形(T4.1:音频卡小图标位真波形;卡片销毁后静默放弃)。
 * @param {HTMLCanvasElement} cv 画布(调用方建好尺寸)
 * @param {string} src 工程内相对路径
 */
export function drawMiniWave(cv, src) {
  const w = cv.width || 64;
  const hgt = cv.height || 20;
  paintTexture(cv.getContext("2d"), w, hgt);
  peaksFor(src, "coarse").then((p) => {
    if (!p.ok || !cv.isConnected) return;
    const ctx = cv.getContext("2d");
    ctx.clearRect(0, 0, w, hgt);
    paintPeaks(ctx, w, hgt, p, 0, p.durationMs / 1000 || 1);
  }).catch(() => { /* 保持纹理 */ });
}
