/* 媒体元素池(T2.4,修 W12/「播放中被销毁→currentTime 归零」;册三 T3.5 有界化):
 * <video>/<audio> 按投影行 id 池化——workspace.changed 增量 diff(新增/移除/换 src),
 * 绝不销毁重建未变化的元素;宿主 #pv-media(e2e 锚点)。
 *
 * 有界纪律(T3.5):池上限 POOL_MAX=24 个媒体元素——超出时按 LRU 淘汰「非锚定」条目,
 * 淘汰计数仅供性能面板可视;播放头覆盖窗口(±PIN_WINDOW_MS)内的行永远锚定不淘,
 * 播放中若覆盖行缺元素(被淘)由 preview-loop.syncMedia 即时补建,长跑内存有界。
 *
 * 池化键说明:按「行 id」而非 src——同一 src 的两行(如 V/A 双轨同源)各自需要
 * 独立的 currentTime(sourceInMs 不同),按 src 共享元素在语义上不可行;
 * e2e_preview 的「≥4 元素」前提亦依赖逐行元素。 */
import { $, h } from "../ui/dom.js";
import { clipKindOf, mediaUrlFor } from "../core/model.js";
import { selectionStore } from "../core/store.js";

/** 池上限(常量即预算;4h 长跑内存 e2e 候下波,本波保证池有界)。 */
export const POOL_MAX = 24;
/** 播放头锚定窗口:覆盖 [t-2s, t+8s] 的行不被 LRU 淘汰(播放连续性优先)。 */
const PIN_BACK_MS = 2000;
const PIN_FWD_MS = 8000;

/** clipId → { el, src, kind, row, usedAt } */
const pool = new Map();
let host = null;
let evictedCount = 0;

export function mountMediaPool() {
  host = $("pv-media");
  if (!host) {
    host = h("div", { id: "pv-media", hidden: true, testid: "pv-media" });
    document.body.appendChild(host);
  }
}

/**
 * 增量对齐池与投影(只在集合/src/kind 变化时碰 DOM)。
 * @param {Object[]} clips 投影行
 * @param {string} token 数据面 token
 * @param {number} [playheadMs] 当前播放头(ms;锚定窗口用,缺省 0)
 * @returns {Array<{row: Object, el: HTMLVideoElement|HTMLAudioElement}>}
 */
export function ensurePool(clips, token, playheadMs = 0) {
  if (!host) mountMediaPool();
  const want = new Set();
  const rows = [];
  for (const row of clips || []) {
    if (!row.src) continue;
    const kind = clipKindOf(row);
    if (kind === "text") continue;
    want.add(row.id);
    rows.push({ row, kind, pinned: rowCoversWindow(row, playheadMs) });
  }
  // 锚定行优先创建,其余按离播放头就近排序(超限时先建近处)
  rows.sort((a, b) => (b.pinned - a.pinned)
    || (Math.abs(a.row.startMs - playheadMs) - Math.abs(b.row.startMs - playheadMs)));
  for (const { row, kind } of rows) {
    let entry = pool.get(row.id);
    if (!entry) {
      if (pool.size >= POOL_MAX) evictLru(row.id);
      if (pool.size >= POOL_MAX) break; // 全是锚定行仍超限:保守不再新建
      // 视频轨元素静音:同源画面/声音常被切成 V+A 双轨,audible 交给音频轨元素,避免双声
      const el = h(kind === "audio" ? "audio" : "video", { preload: "auto" });
      el.muted = kind !== "audio"; // 属性 muted 只定默认态;属性赋值才可靠
      el.src = mediaUrlFor(row.src, token);
      host.appendChild(el);
      entry = { el, src: row.src, kind, row, usedAt: performance.now() };
      pool.set(row.id, entry);
    } else if (entry.src !== row.src || entry.kind !== kind) {
      entry.el.muted = kind !== "audio";
      entry.el.src = mediaUrlFor(row.src, token);
      entry.src = row.src;
      entry.kind = kind;
    }
    entry.row = row;
    entry.usedAt = performance.now();
  }
  // 投影已不含的行:立即释放(优先于 LRU,语义同旧口径)
  for (const [id, entry] of [...pool]) {
    if (!want.has(id)) release(id, entry);
  }
  return [...pool.values()];
}

/** 行是否落在播放头锚定窗口。 */
function rowCoversWindow(row, t) {
  return t >= row.startMs - PIN_BACK_MS && t < row.endMs + PIN_FWD_MS;
}

/** LRU 淘汰:仅在非锚定集合里挑 usedAt 最老者;返回是否淘汰成功。 */
function evictLru(keepId) {
  let oldestId = null;
  let oldestAt = Infinity;
  for (const [id, entry] of pool) {
    if (id === keepId) continue;
    if (rowCoversWindow(entry.row, currentPlayhead())) continue; // 播放头附近不淘
    if (entry.usedAt < oldestAt) {
      oldestAt = entry.usedAt;
      oldestId = id;
    }
  }
  if (oldestId === null) return false;
  release(oldestId, pool.get(oldestId));
  evictedCount += 1;
  return true;
}

/** 播放头读取(selectionStore 会话态;store 与 render 无环)。 */
function currentPlayhead() {
  const ph = selectionStore.get().playheadMs;
  return typeof ph === "number" ? ph : 0;
}

function release(id, entry) {
  try {
    entry.el.pause();
  } catch { /* 已释放 */ }
  entry.el.removeAttribute("src");
  entry.el.load();
  entry.el.remove();
  pool.delete(id);
}

/** 当前池内全部条目(预览循环用)。 */
export function poolEntries() {
  return [...pool.values()];
}

/** 池内是否存在该行元素(preview-loop 补建判断)。 */
export function hasEntry(id) {
  return pool.has(id);
}

/** 调试/自测:池规模。 */
export function poolSize() {
  return pool.size;
}

/** 性能面板可视(T3.5):规模/上限/累计淘汰数。 */
export function poolStats() {
  return { size: pool.size, cap: POOL_MAX, evicted: evictedCount };
}
