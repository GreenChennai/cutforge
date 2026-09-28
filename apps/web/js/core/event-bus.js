/* 事件订阅层(T2.3):/events 封装为 events.subscribe(topic, handler)。
 *
 * - 主通道 SSE(EventSource;Accept: text/event-stream,服务端按头分流,ADR-0009 面);
 * - 降级长轮询:EventSource 致命关闭(CLOSED,如服务不支持 SSE)时切入,
 *   经 api.pollOnce()(壳内 fetch 收口);
 * - 事件面(FLOW.md §5.6):workspace.changed / notes.changed / cutlist.changed /
 *   render.progress / resync(补发窗滚出)→ 全部按 topic 分发;
 * - M10-2 e2e 会 abort /events*:两通道失败都必须静默重试,绝不阻塞装配。
 */
import { pollOnce } from "./api.js";

/** @type {Map<string, Set<(data: Object) => void>>} */
const handlers = new Map();
/** @type {EventSource|null} */
let es = null;
let lpRunning = false;
let lpBackoffMs = 1000;
let lastSeq = 0;
let started = false;

export const TOPICS = ["workspace.changed", "notes.changed", "cutlist.changed", "render.progress"];

/** @param {string} topic @param {(data: Object) => void} fn @returns {() => void} 取消函数 */
export function subscribe(topic, fn) {
  if (!handlers.has(topic)) handlers.set(topic, new Set());
  handlers.get(topic).add(fn);
  return () => handlers.get(topic)?.delete(fn);
}

function dispatch(topic, data) {
  if (data && typeof data.seq === "number" && data.seq > lastSeq) lastSeq = data.seq;
  for (const fn of handlers.get(topic) || []) {
    try {
      fn(data);
    } catch (e) {
      console.error(`[events] handler ${topic} 异常`, e);
    }
  }
}

/** 启动事件通道(SSE 主 + 长轮询降级;幂等)。EventSource 无法带 Authorization 头,
 * 走数据面查询参数通道(服务端 first_line 含 token= 即鉴权,同 /media 契约)。 */
export function startEvents(token, bootstrapSeq = 0) {
  if (started) return;
  started = true;
  lastSeq = bootstrapSeq;
  connectSse(token);
}

function connectSse(token) {
  try {
    es = new EventSource(`/events?token=${encodeURIComponent(token || "")}`);
  } catch {
    enterLongPoll();
    return;
  }
  for (const topic of TOPICS) {
    es.addEventListener(topic, (ev) => {
      const data = safeParse(ev.data);
      dispatch(topic, data);
    });
  }
  es.addEventListener("resync", () => {
    // 补发窗滚出:服务端要求全量刷新(语义同 workspace.changed)
    dispatch("workspace.changed", { event: "workspace.changed", resync: true });
  });
  es.addEventListener("bye", () => {
    // 流寿命到顶:服务端优雅收流,EventSource 自动重连;无需处理
  });
  es.onerror = () => {
    // readyState CLOSED = 致命(非网络抖动,如 MIME/策略)→ 降级长轮询;
    // CONNECTING = 浏览器自带重连(e2e route.abort 场景),静默等待即可。
    if (es && es.readyState === EventSource.CLOSED) enterLongPoll();
  };
}

function enterLongPoll() {
  if (lpRunning) return;
  lpRunning = true;
  longPollLoop();
}

async function longPollLoop() {
  for (;;) {
    try {
      const v = await pollOnce(lastSeq);
      if (v && typeof v.seq === "number") lastSeq = Math.max(lastSeq, v.seq);
      if (v && v.event && v.event !== "none") {
        dispatch(v.event === "workspace.changed" ? "workspace.changed" : v.event, v);
      }
      lpBackoffMs = 1000;
    } catch {
      await new Promise((r) => setTimeout(r, lpBackoffMs));
      lpBackoffMs = Math.min(lpBackoffMs * 2, 8000);
    }
  }
}

function safeParse(text) {
  try {
    return JSON.parse(text || "{}");
  } catch {
    return {};
  }
}

/** 诊断面(e2e/控制台观察通道状态)。 */
export function eventsStatus() {
  return { sse: es ? es.readyState : null, longPoll: lpRunning, seq: lastSeq };
}
