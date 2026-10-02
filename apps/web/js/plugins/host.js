/* JS Worker 插件宿主(册七 T7.2/ADR-0024):插件代码只跑在 Web Worker(独立全局,
 * 无 DOM、无 token)——宿主桥接 postMessage 单通道:
 *
 *   插件 → 宿主:{t:"call",id,tool,args}(API 代理请求)
 *   宿主 → 插件:{t:"res",id,envelope}(宿主持 token 调 /api/v1 后原样回传)
 *   宿主 → 插件:{t:"invoke",seq,kind,id,payload}(贡献点回调:命令/菜单)
 *   插件 → 宿主:{t:"invokeres",seq,ok,message} / {t:"panel",panelId,html}
 *               / {t:"toast",msg,ok} / {t:"log",msg} / {t:"ready"}
 *
 * 安全面:插件代码经 Blob URL 进 Worker 全局(不落主线程作用域);token 只在宿主,
 * 插件拿不到;每次 API 代理请求先过 manifest 权限裁决镜像(manifest.js::authorize,
 * 与服务端 plugin-call 同口径),越权 GUARD_FAILED/FORBIDDEN 原样回传——服务端
 * plugin-call 通道构成第二道保险;崩溃隔离:onerror → 终止 Worker + 自动禁用 + toast
 * (禁用态持久,重启需人工启用)。
 */
import { rpcArgs, call } from "../core/api.js";
import { toast } from "../ui/toast.js";
import { authorize, getRecord, listInstalled, setEnabled, ensureKinds } from "./manifest.js";
import { registerContribs, unregisterContribs, renderPanelFragment } from "./contributes.js";

const CALL_TIMEOUT_MS = 15000;

/** pid → { worker, manifest, state:"starting"|"running", blobUrl } */
const running = new Map();

export function pluginState(pid) {
  const r = running.get(pid);
  return r ? r.state : null;
}
export function runningIds() {
  return [...running.keys()];
}

/* 运行态变化回调(管理面板徽标重渲;host 不反向 import manager 防环)。 */
/** @type {Set<() => void>} */
const stateCbs = new Set();
export function onPluginStateChange(cb) { stateCbs.add(cb); return () => stateCbs.delete(cb); }
function fireStateChange() { for (const cb of stateCbs) { try { cb(); } catch (e) { console.error("[plugins] 状态回调异常", e); } } }

/* ---------------- Worker 引导代码(注入插件全局 cutforge 桥)---------------- */

function bridgeBootstrap() {
  return `/* CutForge 插件桥(宿主注入;Worker 全局无 DOM/token,一切能力经此代理)。 */
(function () {
  var seq = 0;
  var pending = new Map();
  var commands = new Map();
  function post(m) { self.postMessage(m); }
  self.addEventListener("message", (ev) => {
    var m = ev.data || {};
    if (m.t === "res" && pending.has(m.id)) {
      var p = pending.get(m.id);
      pending.delete(m.id);
      if (m.envelope && m.envelope.ok === false) {
        p.reject(Object.assign(new Error(m.envelope.message || m.envelope.code), { envelope: m.envelope }));
      } else {
        p.resolve(m.envelope ? m.envelope.data : null);
      }
    } else if (m.t === "invoke") {
      var fn = commands.get(String(m.id));
      if (!fn) {
        post({ t: "invokeres", seq: m.seq, ok: false, message: "未注册的处理函数: " + m.id });
        return;
      }
      Promise.resolve(fn(m.payload || {}))
        .then((r) => post({ t: "invokeres", seq: m.seq, ok: true, message: r && typeof r === "string" ? r : "" }))
        .catch((e) => post({ t: "invokeres", seq: m.seq, ok: false, message: String((e && e.message) || e) }));
    }
  });
  var bridge = {
    call(tool, args) {
      var id = ++seq;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        post({ t: "call", id, tool: String(tool), args: args || {} });
        setTimeout(() => {
          if (pending.has(id)) {
            pending.delete(id);
            reject(new Error("桥调用超时(${CALL_TIMEOUT_MS / 1000}s;宿主无回执)"));
          }
        }, ${CALL_TIMEOUT_MS});
      });
    },
    onCommand(id, fn) { commands.set(String(id), fn); },
    setPanel(panelId, html) { post({ t: "panel", panelId: String(panelId), html: String(html) }); },
    toast(msg, ok) { post({ t: "toast", msg: String(msg), ok: ok !== false }); },
    log(msg) { post({ t: "log", msg: String(msg) }); },
    ready() { post({ t: "ready" }); },
  };
  self.cutforge = bridge;
})();
`;
}

/* ---------------- 生命周期 ---------------- */

/**
 * 启动 Worker 形态插件(record.enabled 由调用方保证;首启确认在管理面板完成)。
 * @returns {{ ok: boolean, message?: string }}
 */
export function startPlugin(record) {
  const pid = record.id;
  if (running.has(pid)) return { ok: true };
  if (!record.code || typeof record.code !== "string") {
    return { ok: false, message: "入口代码缺失(注册表损坏?请重新安装)" };
  }
  let url = "";
  try {
    const blob = new Blob([bridgeBootstrap(), "\n", record.code], { type: "application/javascript" });
    url = URL.createObjectURL(blob);
    const worker = new Worker(url);
    running.set(pid, { worker, manifest: record.manifest, state: "starting", blobUrl: url });
    worker.onmessage = (ev) => onPluginMessage(pid, ev.data);
    worker.onerror = (e) => crashPlugin(pid, (e && e.message) || "Worker 运行错误");
    worker.onmessageerror = () => crashPlugin(pid, "消息反序列化失败");
  } catch (e) {
    if (url) URL.revokeObjectURL(url);
    running.delete(pid);
    return { ok: false, message: `Worker 创建失败:${String(e && e.message || e)}` };
  }
  return { ok: true };
}

/** 禁用/卸载/崩溃路径:终止 Worker + 摘贡献点 + 撤销 Blob。 */
export function stopPlugin(pid) {
  const handle = running.get(pid);
  if (!handle) return;
  running.delete(pid);
  fireStateChange();
  try { handle.worker.terminate(); } catch { /* 已死 */ }
  if (handle.blobUrl) URL.revokeObjectURL(handle.blobUrl);
  unregisterContribs(pid);
}

/** 崩溃隔离:Worker error → 终止 + 持久禁用 + toast(重启需人工启用)。 */
function crashPlugin(pid, why) {
  if (!running.has(pid)) return;
  const name = running.get(pid).manifest?.name || pid;
  stopPlugin(pid);
  setEnabled(pid, false);
  console.error(`[plugins] ${pid} 崩溃:`, why);
  toast(`插件 ${name} 已崩溃并被自动禁用:${why}(管理面板可重新启用)`, false);
}

/** 装配期:拉起全部 enabled 插件(已确认过的重启自动启用,不重复确认)。
 * 先取工具分类(GET /api/v1/tools):裁决镜像 fail-closed,分类缺失会拒一切代理
 * 调用;清单拉取失败也照常拉起(插件面板如实显示调用被拒,分类可经下次调用重试)。 */
export function startAllEnabled() {
  ensureKinds().finally(() => {
    for (const rec of listInstalled()) {
      if (rec.enabled && rec.manifest?.form === "worker") startPlugin(rec);
    }
  });
}

/* ---------------- 桥消息处理 ---------------- */

function onPluginMessage(pid, m) {
  if (!m || typeof m !== "object") return;
  const handle = running.get(pid);
  if (!handle) return;
  switch (m.t) {
    case "ready": {
      handle.state = "running";
      fireStateChange();
      const rec = getRecord(pid);
      registerContribs(pid, handle.manifest, {
        menuOff: rec?.menuOff || [],
      });
      toast(`插件已启用:${handle.manifest.name || pid}`);
      break;
    }
    case "call": {
      // API 代理:权限裁决镜像先行(服务端 plugin-call 通道=第二道保险)
      const verdict = authorize(handle.manifest, m.tool, m.args || {});
      if (!verdict.ok) {
        console.warn(`[plugins] ${pid} 越权拦截:`, verdict.message);
        postTo(pid, { t: "res", id: m.id, envelope: { ok: false, code: verdict.code, message: verdict.message, data: {} } });
        return;
      }
      // root 注入与 /rpc 数据面同源;causedBy 归因链标明插件(OpLog 审计面)
      const args = rpcArgs({ ...(m.args || {}) });
      const chain = [`plugin:${pid}`];
      if (Array.isArray(args.causedBy)) chain.push(...args.causedBy.filter((x) => typeof x === "string"));
      args.causedBy = chain;
      call(m.tool, args)
        .then((envelope) => postTo(pid, { t: "res", id: m.id, envelope }))
        .catch((e) => postTo(pid, {
          t: "res", id: m.id,
          envelope: { ok: false, code: "NETWORK", message: String(e && e.message || e), data: {} },
        }));
      break;
    }
    case "panel":
      renderPanelFragment(pid, m.panelId, m.html);
      break;
    case "toast":
      toast(`[${handle.manifest.name || pid}] ${m.msg}`, m.ok !== false);
      break;
    case "log":
      console.info(`[plugins:${pid}]`, m.msg);
      break;
    default:
      break; // 未知消息忽略(协议演进面)
  }
}

function postTo(pid, msg) {
  const handle = running.get(pid);
  if (!handle) return;
  try { handle.worker.postMessage(msg); } catch (e) { console.error("[plugins] 回传失败", e); }
}

let invokeSeq = 0;

/** 贡献点回调(命令/菜单):invoke → 插件执行 → invokeres 逐请求回执。 */
export function invokeContribution(pid, kind, id, payload) {
  const handle = running.get(pid);
  if (!handle || handle.state !== "running") {
    toast(`插件 ${pid} 未运行(已禁用或崩溃)`, false);
    return;
  }
  const seq = ++invokeSeq;
  const onRes = (ev) => {
    const m = ev.data || {};
    if (m.t !== "invokeres" || m.seq !== seq) return;
    handle.worker.removeEventListener("message", onRes);
    const name = handle.manifest.name || pid;
    if (m.ok && m.message) toast(`[${name}] ${m.message}`);
    else if (!m.ok) toast(`[${name}] 失败:${m.message}`, false);
  };
  handle.worker.addEventListener("message", onRes);
  postTo(pid, { t: "invoke", seq, kind, id, payload: payload || {} });
}
