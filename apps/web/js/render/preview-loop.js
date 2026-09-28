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
import { ensurePool, poolEntries } from "./media-pool.js";
import { drawRuler } from "./ruler.js";
import { setPlayheadMs } from "./playhead.js";

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
      ? baseMs + (performance.now() - t0)
      : baseMs;
  },
  seek(ms) {
    const end = timelineEndMsOf(timelineStore.get().clips);
    const clamped = Math.max(0, Math.min(ms, end));
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
      baseMs = finalMs !== undefined ? finalMs : baseMs + (performance.now() - t0);
      selectionStore.set({ playheadMs: baseMs });
    }
    playbackStore.set({ playing: on });
    updatePlayButton();
    syncMedia(true);
    if (on) wake();
    else drawPreviewFrame(); // 暂停瞬间的最后一帧
  },
  /** 预览 canvas 代理绘制注册(preview.js 装配时调用)。 */
  registerRenderer(fn) { renderer = fn; },
};

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
    const ms = baseMs + (now - t0);
    const end = timelineEndMsOf(timelineStore.get().clips);
    if (ms >= end) {
      playback.setPlaying(false, end); // 播到结尾自动停(精确停点,旧壳口径的修正版)
      return;
    }
    advanceTo(ms);
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

function updatePlayButton() {
  const btn = $("pv-play");
  if (btn) btn.textContent = playbackStore.get().playing ? "⏸ 暂停" : "▶ 播放";
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

/** 媒体同步:可见行 seek/play/pause(播放映射,非时间线运算;旧壳 syncPreview 口径)。 */
export function syncMedia(forceSeek = false) {
  const t = playback.clockMs();
  const playing = playbackStore.get().playing;
  for (const { row, el } of poolEntries()) {
    if (!rowCovers(row, t)) {
      if (!el.paused) el.pause();
      continue;
    }
    const want = sourceSecAt(row, t);
    const drift = Math.abs(el.currentTime - want);
    if (forceSeek || drift > (playing ? DRIFT_TOL_PLAY : 1 / 1000)) {
      try {
        el.currentTime = want;
      } catch { /* 元素未就绪,下一帧再试 */ }
    }
    if (playing && el.paused) el.play().catch(() => { /* 自动播放被策略拦截:画面仍走 seek 代理 */ });
    if (!playing && !el.paused) el.pause();
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
    ensurePool(timelineStore.get().clips, projectStore.get().token);
    syncMedia(false);
    wake();
  });
  projectStore.subscribe(() => {
    ensurePool(timelineStore.get().clips, projectStore.get().token);
    wake();
  });
  selectionStore.subscribe((patch) => {
    if (patch.playheadMs !== undefined) wake();
  });
  // 首轮池对齐(boot 时投影到达前先空跑一次建立宿主)
  ensurePool(timelineStore.get().clips, projectStore.get().token);
  // 启动静默画一轮(空工程也画黑底 + 时间码)
  wake();
}

/** 供面板读取帧毫秒(逐帧步进按钮用)。 */
export function currentFrameMs() {
  return frameMsOf(projectStore.get().project);
}
