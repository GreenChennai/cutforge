/* 媒体缓存面(T4.1):缩略图 / 波形 peaks / 代理状态 的会话级缓存与请求编排。
 *
 * - 三类产物全部由后端工具落盘缓存(.cutforge/{thumb-cache,peaks-cache,proxy}/,
 *   内容寻址),本模块只做「调用去重 + 在途合并 + 内存上限」;
 * - 缩略/peaks 生成是 ffmpeg 子进程:并发队列限流(缩略 2 路),避免媒体池首屏
 *   一次视口内几十张卡把 ffmpeg 打爆(懒加载由 media-card 的 IntersectionObserver 负责);
 * - peaks 内存缓存按条目上限淘汰(LRU;2000 桶 ×2 数组 ≈ 数十 KB/条,上限 60 条);
 * - 失败负缓存:DEP_MISSING(无 ffmpeg)等不重试,同会话内直接返回失败 envelope。
 */
import { call, mediaGet } from "./api.js";

/* ---------------- 通用 promise 去重表 ---------------- */

/** @type {Map<string, Promise<*>>} */
const inflight = new Map();

function dedupe(key, maker) {
  const hit = inflight.get(key);
  if (hit) return hit;
  const p = maker().finally(() => inflight.delete(key));
  inflight.set(key, p);
  return p;
}

/* ---------------- 缩略图(media_thumbnail;排队限流) ---------------- */

const THUMB_WIDTH = 160;          // 卡片缩略宽(卡面 ≈ 76px,DPR 2 余量)
const THUMB_QUEUE_MAX = 2;        // ffmpeg 并发上限(首屏视口内卡片排队消化)
const THUMB_FAIL_MS = 30000;      // 失败负缓存窗口(同会话内不反复打 ffmpeg)

/** @type {Array<() => void>} */
const thumbQueue = [];
let thumbRunning = 0;
/** @type {Map<string, number>} src → 失败时刻 */
const thumbFails = new Map();

function pumpThumb() {
  while (thumbRunning < THUMB_QUEUE_MAX && thumbQueue.length) {
    const job = thumbQueue.shift();
    thumbRunning += 1;
    job().finally(() => {
      thumbRunning -= 1;
      pumpThumb();
    });
  }
}

/**
 * 素材缩略图(缓存命中零 ffmpeg;失败负缓存)。resolve 为 `{ ok, url?, cached?, message? }`,
 * url 直接可挂 <img src>(经 /media 数据面)。
 * @param {string} src 工程内相对路径
 * @param {number} [atMs] 抽帧点(缺省服务端取时长 10%)
 */
export function thumbnailFor(src, atMs) {
  return dedupe(`thumb:${src}@${atMs || ""}`, () => new Promise((resolve) => {
    const failAt = thumbFails.get(src);
    if (failAt && Date.now() - failAt < THUMB_FAIL_MS) {
      resolve({ ok: false, message: "缩略图刚失败(本会话内暂不重试)" });
      return;
    }
    thumbQueue.push(async () => {
      const args = { src, width: THUMB_WIDTH };
      if (atMs !== undefined) args.atMs = atMs;
      const env = await call("media_thumbnail", args);
      if (env.ok && env.data && env.data.file) {
        resolve({ ok: true, cached: Boolean(env.data.cached), file: env.data.file });
      } else {
        thumbFails.set(src, Date.now());
        resolve({ ok: false, message: (env && env.message) || "缩略图不可用" });
      }
    });
    pumpThumb();
  }));
}

/* ---------------- 波形 peaks(media_peaks;LRU 内存上限) ---------------- */

export const PEAKS_LEVEL_BUCKETS = { coarse: 1000, standard: 2000, fine: 4000 };
const PEAKS_MAX = 60;
/** @type {Map<string, { min: number[], max: number[], buckets: number, durationMs: number, usedAt: number }>} */
const peaksMem = new Map();
/** @type {Map<string, number>} */
const peaksFails = new Map();
let peaksEvicted = 0;

/** 缩放 → 分辨率档(密度联动:越放大越细;PX_PER_MS 0.02–0.30 三段)。 */
export function peaksLevelFor(pxPerMs) {
  if (pxPerMs < 0.08) return "coarse";
  if (pxPerMs < 0.18) return "standard";
  return "fine";
}

/**
 * 素材波形 peaks(后端缓存 + 壳内存 LRU)。resolve `{ok, min, max, buckets, durationMs}`。
 * @param {string} src
 * @param {"coarse"|"standard"|"fine"} [level]
 */
export function peaksFor(src, level = "standard") {
  const key = `peaks:${src}@${level}`;
  return dedupe(key, async () => {
    const hit = peaksMem.get(key);
    if (hit) {
      hit.usedAt = performance.now();
      return hit;
    }
    const failAt = peaksFails.get(key);
    if (failAt && Date.now() - failAt < THUMB_FAIL_MS) {
      return { ok: false, message: "peaks 不可用(近期已失败)" };
    }
    const env = await call("media_peaks", { src, level });
    if (!env.ok || !env.data || !env.data.file) {
      peaksFails.set(key, Date.now());
      return { ok: false, message: (env && env.message) || "peaks 不可用" };
    }
    const doc = await mediaGet(env.data.file);
    if (!doc || !doc.ok || !Array.isArray(doc.min)) {
      peaksFails.set(key, Date.now());
      return { ok: false, message: (doc && doc.message) || "peaks 数据不可读" };
    }
    if (peaksMem.size >= PEAKS_MAX) {
      // LRU:淘汰 usedAt 最老一条(波形重画时可再生成,后端缓存兜底)
      let oldest = null;
      let oldestAt = Infinity;
      for (const [k, v] of peaksMem) {
        if (v.usedAt < oldestAt) { oldestAt = v.usedAt; oldest = k; }
      }
      if (oldest) { peaksMem.delete(oldest); peaksEvicted += 1; }
    }
    const entry = {
      ok: true,
      min: doc.min, max: doc.max,
      buckets: doc.buckets || doc.min.length,
      durationMs: doc.durationMs || 0,
      usedAt: performance.now(),
    };
    peaksMem.set(key, entry);
    return entry;
  });
}

/** peaks 缓存规模(性能面板可视候选)。 */
export function peaksStats() {
  return { size: peaksMem.size, cap: PEAKS_MAX, evicted: peaksEvicted };
}

/* ---------------- 代理(media_proxy;状态会话缓存) ---------------- */

/** @type {Map<string, string>} src → "ready"|"missing" */
const proxyState = new Map();

/** 查询/生成代理,resolve `{ok, state, file}`;state ∈ ready/missing。 */
export function proxyFor(src, generate = false) {
  return dedupe(`proxy:${src}@${generate}`, async () => {
    const env = await call("media_proxy", { src, generate });
    if (!env.ok || !env.data) {
      return { ok: false, state: "unknown", message: (env && env.message) || "代理查询失败" };
    }
    const state = env.data.state || (env.data.file ? "ready" : "missing");
    if (state === "ready") proxyState.set(src, "ready");
    return { ok: true, state, file: env.data.file || "", cached: Boolean(env.data.cached) };
  });
}

/** 已知代理状态(卡片渲染免重复查询)。 */
export function proxyStateOf(src) {
  return proxyState.get(src) || "";
}
