/* 视口虚拟化(T2.4):时间线只渲染视口内片段(1k clips 只渲染可见 ≈ 数十个)。 */
import { PX_PER_MS } from "../core/model.js";

const SCROLL_PAD_PX = 240; // 两侧预渲染余量(px),滚动不闪断

/**
 * @param {HTMLElement} wrap 滚动容器(#timeline-wrap)
 */
export function createVirtualizer(wrap) {
  /** @type {Set<() => void>} */
  const cbs = new Set();
  let raf = 0;

  function emit() {
    for (const cb of cbs) cb();
  }

  function onScroll() {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      emit();
    });
  }

  wrap.addEventListener("scroll", onScroll, { passive: true });
  window.addEventListener("resize", onScroll, { passive: true });

  return {
    /** 当前可见窗口(内容坐标 ms + 视口 px)。 */
    window() {
      const scrollLeft = wrap.scrollLeft;
      const viewW = wrap.clientWidth;
      const padMs = SCROLL_PAD_PX / PX_PER_MS;
      // 页签隐藏(display:none)时 clientWidth=0 → t1 落到负值,renderClips 会把全部
      // 片段当"视口外"裁除,而切回页签无 scroll/resize 事件、不重渲 → 时间线空面板。
      // 隐藏期退化为全量窗口(用户看不见,渲染成本一次性;切回即所见即所得)。
      if (!viewW) {
        return {
          scrollLeft,
          viewW,
          viewH: wrap.clientHeight,
          scrollTop: wrap.scrollTop,
          t0: 0,
          t1: Number.MAX_SAFE_INTEGER,
        };
      }
      return {
        scrollLeft,
        viewW,
        viewH: wrap.clientHeight,
        scrollTop: wrap.scrollTop,
        t0: Math.max(0, scrollLeft / PX_PER_MS - padMs),
        t1: (scrollLeft + viewW) / PX_PER_MS + padMs,
      };
    },
    onChange(cb) {
      cbs.add(cb);
      return () => cbs.delete(cb);
    },
  };
}
