/* 手势层(册三 T3.3;T4.2 片段四件套分册至 clip-gestures.js):
 * 本文件持有 轨道行手势(点空白取消/框选多选)、素材拖放(HTML5 DnD)、
 * 标尺 scrub、时间轴缩放。通用管线/气泡/卷入/吸附在 gesture-kit.js。纪律:
 * - 拖拽零动画(直接跟手):几何每帧直写,不挂 transition(C-R1);
 * - 拖拽过程不产 Op:候选值只写 ephemeral.dragGhost / ephemeral.snapMs(ADR-0013),
 *   松手一次手势一个命令,提交后以重投影收敛;Esc/失焦取消不提交。
 */
import { $, h } from "../ui/dom.js";
import { ephemeralStore, mediaStore, selectionStore } from "../core/store.js";
import { PX_PER_MS, msAdd, setPxPerMs } from "../core/model.js";
import { selectClip, insertMedia } from "../core/commands.js";
import { toast } from "../ui/toast.js";
import { renderTimelineView } from "./timeline-view.js";
import { trackLockedOf, onClipPointerDown } from "./clip-gestures.js";
import { runGesture, snapAt, fmtSec, showBubble, hideBubble, edgeScroll } from "./gesture-kit.js";

/* ---------------- 轨道行手势(点空白取消选中 / 空白拉框多选) ---------------- */

/** 轨道空白 pointerdown:未过阈值 = 取消选中(旧壳口径);过阈值 = 框选多选。
 * 锁定轨的空白仍可框选(锁定只拦片段编辑,不拦视图选择)。 */
export function onLanePointerDown(e) {
  if (e.target !== e.currentTarget || e.button !== 0) return;
  const laneEl = /** @type {HTMLElement} */ (e.currentTarget);
  const wrap = $("timeline-wrap");
  const additive = e.shiftKey || e.ctrlKey || e.metaKey;
  const baseIds = additive ? new Set(selectionStore.get().clipIds || []) : new Set();
  let box = null;
  let scroller = null;
  let bx0 = 0;
  let by0 = 0;

  /** 框选矩形(内容坐标直写,跟手零动画)+ 可见片段相交命中。 */
  const applyMarquee = (x1, y1) => {
    const left = Math.min(bx0, x1);
    const top = Math.min(by0, y1);
    box.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
    box.style.width = `${Math.max(1, Math.round(Math.abs(x1 - bx0)))}px`;
    box.style.height = `${Math.max(1, Math.round(Math.abs(y1 - by0)))}px`;
    const r = wrap.getBoundingClientRect();
    const cx0 = r.left + left - wrap.scrollLeft;
    const cy0 = r.top + top - wrap.scrollTop;
    const ids = new Set(baseIds);
    let anchor = selectionStore.get().clipId;
    for (const el of wrap.querySelectorAll(".clip:not(.ghost)")) {
      const cr = el.getBoundingClientRect();
      if (cr.right > cx0 && cr.left < cx0 + Math.abs(x1 - bx0)
        && cr.bottom > cy0 && cr.top < cy0 + Math.abs(y1 - by0)) {
        ids.add(el.dataset.id);
        if (!anchor) anchor = el.dataset.id;
      }
    }
    selectionStore.set({ clipIds: [...ids], clipId: anchor });
  };
  const teardown = () => {
    if (scroller) { scroller.stop(); scroller = null; }
    if (box) { box.remove(); box = null; }
  };

  runGesture(laneEl, e, {
    start: (ev) => {
      const r = wrap.getBoundingClientRect();
      bx0 = ev.clientX - r.left + wrap.scrollLeft;
      by0 = ev.clientY - r.top + wrap.scrollTop;
      box = h("div", { id: "marquee-box", "aria-hidden": "true" });
      wrap.appendChild(box);
      box.style.display = "block";
      if (!additive) selectClip(null);
      scroller = edgeScroll(wrap);
    },
    move: (ev) => {
      if (!box) return;
      if (scroller) scroller.move(ev.clientX);
      const r = wrap.getBoundingClientRect();
      applyMarquee(ev.clientX - r.left + wrap.scrollLeft, ev.clientY - r.top + wrap.scrollTop);
    },
    end: teardown,
    cancel: teardown,
    click: () => { if (!additive) selectClip(null); },
  });
}

/* ---------------- 素材拖放(HTML5 DnD,旧壳契约) ---------------- */

export function onLaneDragOver(e) {
  e.preventDefault();
  /** @type {HTMLElement} */ (e.currentTarget).classList.add("drop-hint");
}

export function onLaneDragLeave(e) {
  /** @type {HTMLElement} */ (e.currentTarget).classList.remove("drop-hint");
}

/** 素材拖放到轨道落点(px→ms 显示映射 + 统一吸附;插入由 clip_add 命令提交)。
 * 锁定轨拒绝落点(诚实提示,不产 Op)。 */
export function onLaneDrop(e) {
  e.preventDefault();
  const lane = /** @type {HTMLElement} */ (e.currentTarget);
  lane.classList.remove("drop-hint");
  const src = e.dataTransfer.getData("text/cutforge-media");
  if (!src) return;
  const trackId = lane.dataset.trackId;
  if (trackLockedOf(trackId)) {
    toast(`轨道 ${trackId} 已锁定,拒绝插入(轨头解锁后再拖)`, false);
    return;
  }
  const rect = lane.getBoundingClientRect();
  const at = snapAt(Math.max(0, (e.clientX - rect.left) / PX_PER_MS));
  // durationMs 优先用素材面板浏览元信息(避免无谓的二次探测)
  const item = mediaStore.get().files.find((f) => f.path === src);
  insertMedia(src, trackId, at, item && item.durationMs);
}

/* ---------------- 标尺手势(e2e 锚点:#ruler 点击坐标 ÷ 0.06 = ms) ---------------- */

/** 标尺 seek:按下即跳 + 连续 scrub(rAF 节流 = 预览抽帧节流)+ 悬停时间气泡。 */
export function mountRulerGestures(seekFn) {
  const ruler = document.getElementById("ruler");
  const msOf = (ev) => Math.max(0, (ev.clientX - ruler.getBoundingClientRect().left) / PX_PER_MS);
  ruler.addEventListener("pointermove", (e) => {
    if (e.buttons) return; // 拖拽中的气泡由 scrub 路径接管
    showBubble(e.clientX, e.clientY + 14, fmtSec(msOf(e)));
  });
  ruler.addEventListener("pointerleave", hideBubble);
  ruler.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    let raf = 0;
    let pendingMs = null;
    const scrub = (ms) => { // 连续 seek 合帧:预览抽帧至多 60Hz
      pendingMs = ms;
      if (!raf) {
        raf = requestAnimationFrame(() => {
          raf = 0;
          seekFn(snapAt(pendingMs));
        });
      }
    };
    seekFn(snapAt(msOf(e))); // 按下即跳(不经阈值)
    runGesture(ruler, e, {
      move: (ev) => {
        showBubble(ev.clientX, ev.clientY + 14, fmtSec(msOf(ev)));
        scrub(msOf(ev));
      },
      end: () => { if (raf) cancelAnimationFrame(raf); raf = 0; hideBubble(); },
      cancel: () => { if (raf) cancelAnimationFrame(raf); raf = 0; hideBubble(); },
    });
  });
}

/* ---------------- 时间轴缩放(Ctrl/⌘/Alt+滚轮;触控板捏合=Ctrl+滚轮) ---------------- */

/** 视口中心锚定缩放:中心时刻在新映射下保持屏中;0.06 缺省永不被装配路径触碰(e2e 红线)。 */
export function mountTimelineZoom() {
  const wrap = $("timeline-wrap");
  wrap.addEventListener("wheel", (e) => {
    if (!(e.ctrlKey || e.metaKey || e.altKey)) return; // 纯滚轮/双指 = 原生滚动
    e.preventDefault();
    const oldPx = PX_PER_MS;
    const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    if (!setPxPerMs(oldPx * factor)) return;
    const centerMs = (wrap.scrollLeft + wrap.clientWidth / 2) / oldPx;
    renderTimelineView();
    wrap.scrollLeft = Math.max(0, msAdd(centerMs * PX_PER_MS, -wrap.clientWidth / 2));
  }, { passive: false });
}
