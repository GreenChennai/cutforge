/* 媒体元素池(T2.4,修 W12/「播放中被销毁→currentTime 归零」):
 * <video>/<audio> 按投影行 id 池化——workspace.changed 增量 diff(新增/移除/换 src),
 * 绝不销毁重建未变化的元素;宿主 #pv-media(e2e 锚点)。
 *
 * 池化键说明:按「行 id」而非 src——同一 src 的两行(如 V/A 双轨同源)各自需要
 * 独立的 currentTime(sourceInMs 不同),按 src 共享元素在语义上不可行;
 * e2e_preview 的「≥4 元素」前提亦依赖逐行元素。 */
import { $, h } from "../ui/dom.js";
import { clipKindOf, mediaUrlFor } from "../core/model.js";

/** clipId → { el, src, kind, row } */
const pool = new Map();
let host = null;

export function mountMediaPool() {
  host = $("pv-media");
  if (!host) {
    host = h("div", { id: "pv-media", hidden: true, testid: "pv-media" });
    document.body.appendChild(host);
  }
}

/**
 * 增量对齐池与投影(只在集合/src/kind 变化时碰 DOM)。
 * @returns {Array<{row: Object, el: HTMLVideoElement|HTMLAudioElement}>}
 */
export function ensurePool(clips, token) {
  if (!host) mountMediaPool();
  const want = new Set();
  for (const row of clips || []) {
    if (!row.src) continue;
    const kind = clipKindOf(row);
    if (kind === "text") continue;
    want.add(row.id);
    let entry = pool.get(row.id);
    if (!entry) {
      // 视频轨元素静音:同源画面/声音常被切成 V+A 双轨,audible 交给音频轨元素,避免双声
      const el = h(kind === "audio" ? "audio" : "video", { preload: "auto" });
      el.muted = kind !== "audio"; // 属性 muted 只定默认态;属性赋值才可靠
      el.src = mediaUrlFor(row.src, token);
      host.appendChild(el);
      entry = { el, src: row.src, kind, row };
      pool.set(row.id, entry);
    } else if (entry.src !== row.src || entry.kind !== kind) {
      entry.el.muted = kind !== "audio";
      entry.el.src = mediaUrlFor(row.src, token);
      entry.src = row.src;
      entry.kind = kind;
    }
    entry.row = row;
  }
  for (const [id, entry] of [...pool]) {
    if (!want.has(id)) {
      try {
        entry.el.pause();
      } catch { /* 已释放 */ }
      entry.el.removeAttribute("src");
      entry.el.load();
      entry.el.remove();
      pool.delete(id);
    }
  }
  return [...pool.values()];
}

/** 当前池内全部条目(预览循环用)。 */
export function poolEntries() {
  return [...pool.values()];
}

/** 调试/自测:池规模。 */
export function poolSize() {
  return pool.size;
}
