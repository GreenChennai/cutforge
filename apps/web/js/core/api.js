/* API 客户端层(T2.3):壳内唯一的 fetch 收口。
 *
 * - /rpc 统一封装:AbortController 超时(默认 10s,渲染类豁免)、幂等查询指数退避重试、
 *   401/5xx 分流、request_id 幂等键透传(键由调用方经 model.newRequestId() 生成);
 * - 数据面 GET 白名单(/session、/ui-fields、/events):无 RPC 等价物的数据面端点,
 *   全部经 dataGet() 走本文件(/media 走媒体元素 src,不经 fetch);
 * - 事件长轮询降级(core/event-bus.js)经 pollOnce() 走本文件,壳内其余模块零裸 fetch。
 *
 * @typedef {{ ok: boolean, code: string, message?: string, data?: Object, ns?: string, net?: boolean }} Envelope
 */
import { netFail } from "./errors.js";

/** @type {string} */
let token = "";

/** 渲染类工具:豁免 10s 默认超时(渲染以分钟计)。 */
const RENDER_CLASS = new Set(["render", "render_run", "export_jianying", "render_frame"]);
const RENDER_TIMEOUT_MS = 300000;
const DEFAULT_TIMEOUT_MS = 10000;

/** 只读(幂等查询)工具:网络失败/5xx 可安全重试。 */
const READONLY = new Set([
  "project_get", "timeline_get", "oplog_tail", "conflict_list", "media_browse",
  "media_probe", "render_probe", "notes_list", "render_progress", "stage_status",
  "capability_matrix", "wordline_get", "cutlist_get", "sync_check",
]);

/** 数据面 GET 白名单(壳内 fetch 只允许这些路径 + /rpc)。 */
const DATA_GET_WHITELIST = new Set(["/session", "/ui-fields", "/events"]);

const RETRY_MAX = 3;
const RETRY_BASE_MS = 300;

/** @type {Set<() => void>} */
const authFailCbs = new Set();
/** @type {Set<(msg: string) => void>} */
const netDownCbs = new Set();

export function setToken(t) { token = t || ""; }
export function onAuthFail(cb) { authFailCbs.add(cb); return () => authFailCbs.delete(cb); }
export function onNetDown(cb) { netDownCbs.add(cb); return () => netDownCbs.delete(cb); }

function authHeaders(extra) {
  const h = { ...(extra || {}) };
  if (token) h["Authorization"] = `Bearer ${token}`;
  return h;
}

/** 底层 fetch:超时(AbortController)+ 401 分流;返回 {status, json} 或抛网络错。 */
async function rawFetch(path, { method = "GET", body = null, timeoutMs = DEFAULT_TIMEOUT_MS, headers = {} } = {}) {
  const ctrl = new AbortController();
  const timer = timeoutMs > 0
    ? setTimeout(() => ctrl.abort("timeout"), timeoutMs)
    : null;
  try {
    const resp = await fetch(path, {
      method,
      headers: authHeaders(headers),
      body: body === null ? undefined : JSON.stringify(body),
      signal: ctrl.signal,
    });
    if (resp.status === 401) {
      for (const cb of authFailCbs) cb();
      return { status: 401, json: null };
    }
    const json = await resp.json().catch(() => null);
    return { status: resp.status, json };
  } finally {
    if (timer) clearTimeout(timer);
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * 数据面 GET(白名单校验;事件轮询经 pollOnce)。
 * @returns {Promise<*|Envelope>} JSON 或 netFail envelope
 */
export async function dataGet(path, params = {}) {
  if (!DATA_GET_WHITELIST.has(path)) {
    return netFail("FORBIDDEN_PATH", `数据面白名单外路径:${path}`);
  }
  const qs = Object.entries(params)
    .filter(([, v]) => v !== undefined && v !== null && v !== "")
    .map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`)
    .join("&");
  const url = qs ? `${path}?${qs}` : path;
  for (let attempt = 0; ; attempt += 1) {
    try {
      const { status, json } = await rawFetch(url, { timeoutMs: DEFAULT_TIMEOUT_MS });
      if (status === 401) return netFail("UNAUTHORIZED", "token 鉴权失败(服务重启会换新 token)");
      if (status >= 500 && attempt < RETRY_MAX - 1) {
        await sleep(RETRY_BASE_MS * 2 ** attempt);
        continue;
      }
      if (status !== 200) return netFail(`HTTP_${status}`, `数据面 ${path} 非 200`);
      return json;
    } catch (e) {
      if (attempt >= RETRY_MAX - 1) {
        for (const cb of netDownCbs) cb(String(e && e.message || e));
        return netFail("NETWORK", `数据面 ${path} 网络失败`);
      }
      await sleep(RETRY_BASE_MS * 2 ** attempt);
    }
  }
}

/** 事件长轮询单次调用(events.js 降级路径专用;长轮询自身挂 2s 上限)。 */
export async function pollOnce(since) {
  const params = since > 0 ? { since } : {};
  const qs = Object.entries(params).map(([k, v]) => `${k}=${v}`).join("&");
  const url = qs ? `/events?${qs}` : "/events";
  const { status, json } = await rawFetch(url, { timeoutMs: 2000 });
  if (status !== 200 || !json) throw new Error(`events ${status}`);
  return json;
}

/**
 * /rpc 统一入口。envelope(ok:false)原样返回,不抛异常;
 * 网络异常收敛为 netFail envelope。
 * @returns {Promise<Envelope>}
 */
export async function call(name, args = {}, { timeoutMs } = {}) {
  const effectiveTimeout = timeoutMs
    ?? (RENDER_CLASS.has(name) ? RENDER_TIMEOUT_MS : DEFAULT_TIMEOUT_MS);
  const canRetry = READONLY.has(name);
  const attempts = canRetry ? RETRY_MAX : 1;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      const { status, json } = await rawFetch("/rpc", {
        method: "POST",
        timeoutMs: effectiveTimeout,
        body: {
          jsonrpc: "2.0", id: 1, method: "tools/call",
          params: { name, arguments: rpcArgs(args) },
        },
      });
      if (status === 401) return netFail("UNAUTHORIZED", "token 鉴权失败(数据面)");
      if (status >= 500) {
        if (canRetry && attempt < attempts - 1) {
          await sleep(RETRY_BASE_MS * 2 ** attempt);
          continue;
        }
        return netFail(`HTTP_${status}`, `${name}: 服务端 5xx`);
      }
      if (status !== 200 || !json) return netFail(`HTTP_${status}`, `${name}: 非 200 响应`);
      const contentText = json?.result?.content?.[0]?.text;
      if (typeof contentText !== "string") {
        return netFail("INTERNAL", `${name}: MCP 响应缺 content(jsonrpc error?)`);
      }
      try {
        return JSON.parse(contentText);
      } catch {
        return netFail("INTERNAL", `${name}: envelope 非法 JSON`);
      }
    } catch (e) {
      const aborted = e && (e.name === "AbortError" || String(e).includes("timeout"));
      const env = netFail(aborted ? "TIMEOUT" : "NETWORK", `${name}: ${aborted ? "超时" : "网络失败"}`);
      if (canRetry && attempt < attempts - 1) {
        await sleep(RETRY_BASE_MS * 2 ** attempt);
        continue;
      }
      for (const cb of netDownCbs) cb(env.message);
      return env;
    }
  }
  return netFail("INTERNAL", "unreachable"); // 循环必然 return,防御兜底
}

/** rpc args 的 root 注入(工程根,来自 /session;未就绪时由调用方显式传)。 */
let sessionRoot = "";
export function setSessionRoot(root) { sessionRoot = root || ""; }
export function rpcArgs(args = {}) {
  return { root: sessionRoot, ...args };
}
