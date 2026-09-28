/* 预览/播放循环(T2.4 播放解耦,修 W2/W6/永久循环):
 *
 * - rAF 循环每帧只更新:播放头 transform、时间码文本、当前媒体元素与 canvas 帧、
 *   标尺标记——绝不触发时间线 DOM 重渲染;
 * - 暂停且视口静止(最后一次活动 > QUIET_MS)后循环自动休眠;任何 seek/投影变化唤醒;
 * - 时钟:playing 时挂钟推算(baseMs + now - t0),与旧壳口径一致;
 * - 播放头是纯客户端会话态(selection.playheadMs 只在离散事件落 store)。
 */
import { $ } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore, playbackStore } from "../core/store.js";
import { timelineEndMsOf, frameMsOf, sourceSecAt, rowCovers } from "../core/model.js";
import { ensurePool, poolEntries, hasEntry } from "./media-pool.js";
import { drawRuler } from "./ruler.js";
import { setPlayheadMs, smoothSeek } from "./playhead.js";

const QUIET_MS = 600;        // 暂停后静默多久进入休眠
const DRIFT_CORRECT_MS = 250; // 播放中媒体漂移校正节流(旧壳口径)
const DRIFT_TOL_PLAY = 0.12;

let baseMs = 0;
let t0 = 0;
let rafId = 0;
let running = false;
let lastActivity = 0;
let lastDrift = 0;
let renderer = null; // preview.js 注册的 canvas 代理绘制

/** 播放控制面(commands 与面板经此驱动;时钟权威持有者)。 */
export const playback = {
  clockMs() {
    return playbackStore.get().playing
      ? baseMs + now_scaled()
      : baseMs;
  },
  seek(ms) {
    const end = timelineEndMsOf(timelineStore.get().clips);
    const clamped = Math.max(0, Math.min(ms, end));
    if (!playbackStore.get().playing) smoothSeek(); // 离散 seek 平滑(播放路径零过渡,直接跟帧)
    baseMs = clamped;
    t0 = performance.now();
    selectionStore.set({ playheadMs: clamped });
    syncMedia(true);
    wake();
  },
  /**
   * 播放态切换。finalMs:播到结尾自动停时的精确停点(避免挂钟外推越过端点)。
   */
  setPlaying(on, finalMs) {
    if (on && !playbackStore.get().playing) {
      baseMs = playback.clockMs();
      t0 = performance.now();
    } else if (!on && playbackStore.get().playing) {
      baseMs = finalMs !== undefined ? finalMs : baseMs + (now_scaled());
      selectionStore.set({ playheadMs: baseMs });
    }
    if (!on) playbackStore.set({ speed: 1 }); // 停即复位倍速(J/K/L 链语义)
    playbackStore.set({ playing: on });
    updatePlayButton();
    syncMedia(true);
    if (on) wake();
    else drawPreviewFrame(); // 暂停瞬间的最后一帧
  },
  /** 倍速/方向(J/K/L 链;speed<0 = 倒放)。播放中改速率需重置时钟基准。 */
  setRate(speed) {
    const wasPlaying = playbackStore.get().playing;
    if (wasPlaying) {
      baseMs = playback.clockMs();
      t0 = performance.now();
    }
    playbackStore.set({ speed });
    updatePlayButton();
    if (wasPlaying) {
      syncMedia(true);
      wake();
    }
  },
  /** 预览 canvas 代理绘制注册(preview.js 装配时调用)。 */
  registerRenderer(fn) { renderer = fn; },
};

/** 挂钟位移(带方向与倍速;speed=1 正放时与旧口径逐位一致)。 */
function now_scaled() {
  const { speed } = playbackStore.get();
  const sp = Math.abs(speed || 1);
  const dir = (speed || 1) < 0 ? -1 : 1;
  return (performance.now() - t0) * dir * sp;
}

/** 唤醒循环(至少画一轮;播放态则持续跑)。 */
export function wake() {
  lastActivity = performance.now();
  if (!running) {
    running = true;
    rafId = requestAnimationFrame(tick);
  }
}

function tick(now) {
  const playing = playbackStore.get().playing;
  if (playing) {
    const ms = baseMs + (now - t0) * dirSpeed();
    const end = timelineEndMsOf(timelineStore.get().clips);
    if (ms >= end && dirSpeed() > 0) {
      playback.setPlaying(false, end); // 播到结尾自动停(精确停点,旧壳口径的修正版)
      return;
    }
    if (ms <= 0 && dirSpeed() < 0) {
      playback.setPlaying(false, 0); // 倒放到开头自动停
      return;
    }
    advanceTo(Math.max(0, ms));
    if (now - lastDrift > DRIFT_CORRECT_MS) {
      lastDrift = now;
      syncMedia(false);
    }
    lastActivity = now;
    rafId = requestAnimationFrame(tick);
    return;
  }
  // 暂停:静默窗口过后休眠(修永久 rAF 循环)
  advanceTo(baseMs);
  if (now - lastActivity < QUIET_MS) {
    rafId = requestAnimationFrame(tick);
  } else {
    running = false;
  }
}

/** 方向×倍速系数(正放 1x 恒为 1:与旧壳挂钟口径逐位一致)。 */
function dirSpeed() {
  const { speed } = playbackStore.get();
  const sp = Math.abs(speed || 1);
  const dir = (speed || 1) < 0 ? -1 : 1;
  return dir * sp;
}

function updatePlayButton() {
  const btn = $("pv-play");
  if (!btn) return;
  const { playing, speed } = playbackStore.get();
  const tag = playing && Math.abs(speed) !== 1 ? ` ${Math.abs(speed)}x${speed < 0 ? "↩" : ""}` : "";
  btn.textContent = playing ? `⏸ 暂停${tag}` : "▶ 播放";
}

/** 每帧写入面:transform + 文本 + 标尺 + 预览画布(零时间线 DOM 重渲染)。 */
function advanceTo(ms) {
  setPlayheadMs(ms);
  const phEl = $("playhead-ms");
  if (phEl) phEl.textContent = String(Math.round(ms));
  const tb = $("tb-time");
  if (tb) tb.textContent = `${Math.round(ms)}ms`;
  const pv = $("pv-time");
  if (pv) pv.textContent = `${(ms / 1000).toFixed(3)}s${playbackStore.get().playing ? " ▶" : ""}`;
  drawRuler(ms);
  drawPreviewFrame();
}

/** 媒体同步:可见行 seek/play/pause(播放映射,非时间线运算;旧壳 syncPreview 口径)。
 * 倍速扩展(J/K/L 链):正放 |speed|>1 → el.playbackRate 跟随;倒放无法原生倒播,
 * 元素保持暂停、按漂移阈值逐次 seek(画质代理口径,诚实降级)。 */
export function syncMedia(forceSeek = false) {
  const t = playback.clockMs();
  const { playing, speed } = playbackStore.get();
  const rate = Math.abs(speed || 1);
  const reverse = playing && (speed || 1) < 0;
  // T3.5 池有界化:覆盖行缺元素(被 LRU 淘汰)→ 即时补建(播放头附近行锚定)
  const clips = timelineStore.get().clips;
  if (clips.some((row) => row.src && rowCovers(row, t) && !hasEntry(row.id))) {
    ensurePool(clips, projectStore.get().token, t);
  }
  for (const { row, el } of poolEntries()) {
    if (!rowCovers(row, t)) {
      if (!el.paused) el.pause();
      continue;
    }
    const want = sourceSecAt(row, t);
    const drift = Math.abs(el.currentTime - want);
    const tol = playing ? Math.max(DRIFT_TOL_PLAY, 0.25 * (rate - 1)) : 1 / 1000;
    if (forceSeek || drift > tol) {
      try {
        el.currentTime = want;
      } catch { /* 元素未就绪,下一帧再试 */ }
    }
    try {
      el.playbackRate = Math.min(4, Math.max(0.25, rate)); // 元素支持范围 0.25–4
    } catch { /* 旧内核忽略 */ }
    if (playing && !reverse && el.paused) {
      el.play().catch(() => { /* 自动播放被策略拦截:画面仍走 seek 代理 */ });
    }
    if (!playing || reverse) {
      if (!el.paused) el.pause();
    }
  }
}

/** 预览 canvas 代理帧(preview.js 注册;旧壳 drawPreview 口径:overlay/position/scale)。 */
export function drawPreviewFrame() {
  if (renderer) renderer(poolEntries());
}

/* ---------------- 装配 ---------------- */

export function mountPreviewLoop() {
  // 投影变化 → 池增量对齐 + 预览画布尺寸/端点对齐 + 唤醒画一轮
  timelineStore.subscribe(() => {
    ensurePool(timelineStore.get().clips, projectStore.get().token, selectionStore.get().playheadMs || 0);
    syncMedia(false);
    wake();
  });
  projectStore.subscribe(() => {
    ensurePool(timelineStore.get().clips, projectStore.get().token, selectionStore.get().playheadMs || 0);
    wake();
  });
  selectionStore.subscribe((patch) => {
    if (patch.playheadMs !== undefined) {
      ensurePool(timelineStore.get().clips, projectStore.get().token, patch.playheadMs);
      wake();
    }
  });
  // 首轮池对齐(boot 时投影到达前先空跑一次建立宿主)
  ensurePool(timelineStore.get().clips, projectStore.get().token, 0);
  // 启动静默画一轮(空工程也画黑底 + 时间码)
  wake();
}

/** 供面板读取帧毫秒(逐帧步进按钮用)。 */
export function currentFrameMs() {
  return frameMsOf(projectStore.get().project);
}
