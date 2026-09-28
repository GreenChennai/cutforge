/* 波形层(ADR-0012 重绘层成员):音频片段内嵌小画布。
 * 诚实口径:当前内核未下发 PCM/波形数据,画布绘制「装饰性纹理」(icon-wave 同款
 * 竖条纹理),不冒充真实波形;attachWaveformData() 是未来真数据的接入点
 * (候册四 T4.1 代理转码一并供数)。 */
import { clipKindOf } from "../core/model.js";

/** 片段元素宽度变化后(重)画纹理;非音频片段不画。 */
export function drawWaveIfAudio(clipEl, row, widthPx) {
  if (clipKindOf(row) !== "audio") return;
  let cv = clipEl.querySelector("canvas.clip-wave");
  const w = Math.max(0, Math.floor(widthPx) - 4);
  if (w < 12) {
    if (cv) cv.remove();
    return;
  }
  if (!cv) {
    cv = document.createElement("canvas");
    cv.className = "clip-wave";
    cv.setAttribute("aria-hidden", "true");
    cv.style.cssText = "position:absolute;inset:0;width:100%;height:100%;opacity:.5;pointer-events:none;";
    clipEl.insertBefore(cv, clipEl.firstChild);
  }
  if (cv.width !== w) {
    cv.width = w;
    cv.height = 28;
    const ctx = cv.getContext("2d");
    ctx.clearRect(0, 0, w, 28);
    // 装饰性竖条纹理(非数据;真波形接入点见文件头注释)
    ctx.fillStyle = "rgba(255,255,255,.35)";
    for (let x = 2; x < w; x += 6) {
      const hgt = 6 + ((x * 2654435761) % 14);
      ctx.fillRect(x, (28 - hgt) / 2, 2, hgt);
    }
  }
}
