/* 视图操作(T3.4 视图键):缩放/适应窗口/预览全屏/面板开合/导出面板聚焦。
 * 全部为纯客户端显示映射操作(不产 Op、不触碰投影);PX_PER_MS 红线:
 * 只在用户显式按键时改值,装配路径永不触碰。 */
import { $ } from "./dom.js";
import { uiStore } from "../core/store.js";
import { PX_PER_MS, PX_PER_MS_MIN, PX_PER_MS_MAX, setPxPerMs, timelineEndMsOf } from "../core/model.js";
import { timelineStore } from "../core/store.js";
import { renderTimelineView } from "../render/timeline-view.js";

/** 视口中心锚定缩放:中心时刻在新映射下保持屏中(与滚轮缩放同一手感)。 */
export function zoomBy(factor) {
  const wrap = $("timeline-wrap");
  if (!wrap) return;
  const oldPx = PX_PER_MS;
  if (!setPxPerMs(oldPx * factor)) return;
  const centerMs = (wrap.scrollLeft + wrap.clientWidth / 2) / oldPx;
  renderTimelineView();
  wrap.scrollLeft = Math.max(0, centerMs * PX_PER_MS - wrap.clientWidth / 2);
}

/** 适应窗口(\):整条时间线装进视口;内容比视口短时回默认 0.06(e2e 红线值)。 */
export function fitTimeline() {
  const wrap = $("timeline-wrap");
  if (!wrap || !wrap.clientWidth) return;
  const end = timelineEndMsOf(timelineStore.get().clips);
  const px = wrap.clientWidth / end;
  const target = px > PX_PER_MS_MAX ? PX_PER_MS_MAX
    : px < PX_PER_MS_MIN ? PX_PER_MS_MIN
      : 0.06; // 足够短:回显示映射缺省(不写死为最小档)
  if (!setPxPerMs(target)) return;
  renderTimelineView();
  wrap.scrollLeft = 0;
}

/** 预览全屏(F):Fullscreen API;Esc 由浏览器原生退出。 */
export function togglePreviewFullscreen() {
  const el = $("preview");
  if (!el) return;
  if (document.fullscreenElement) {
    document.exitFullscreen().catch(() => { /* 已退出 */ });
  } else {
    el.requestFullscreen().catch(() => { /* 策略拒绝:静默 */ });
  }
}

/** 侧面板开合(Tab):media/inspector 两列 display 切换(css .panels-collapsed)。 */
export function togglePanels() {
  uiStore.set({ panelsOpen: !uiStore.get().panelsOpen });
}

/** 导出面板聚焦(Ctrl+S):滚动到位 + 一次性闪烁框(结构性提示,非颜色单线索)。 */
export function focusExportPanel() {
  const el = $("export");
  if (!el) return;
  el.scrollIntoView({ block: "nearest", inline: "nearest" });
  el.classList.remove("flash");
  void el.offsetWidth;
  el.classList.add("flash");
  setTimeout(() => el.classList.remove("flash"), 1200);
}
