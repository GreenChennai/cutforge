/* AI 计划批准流核心(册七 T7.5;人审在环的唯一会合点)。
 *
 * 数据面:preview_plan(副本 dry-run,真工程不动)→ 人逐项批准/拒绝 →
 * apply_plan(approvals 显式批准面,默认拒绝:未决项一律 skipped,零落地)。
 * 状态纪律:纯会话态(plan/预演回执/批准面都不落盘,刷新即失——计划必须当次人审);
 * 本模块不发 toast、不建 DOM,只持状态与订阅(视图 = diff 面板批准流 / 检查器批量区 /
 * 脚本页签共用同一份 draft,三入口一处审批)。
 */
import { call } from "./api.js";
import { reproject } from "./projector.js";

/** @typedef {{ tool: string, args: Object, id?: string }} PlanItem */

/** 会话态(非持久):draft → preview → approvals → applyResult 的单向流转。 */
let draft = null;      // { plan: PlanItem[], source: string }
let preview = null;    // preview_plan data:{revFrom, revTo, items[], summary}
let approvals = [];    // per index:"approve" | "reject" | null(未决 = 不批)
let applyResult = null;// apply_plan data:{planId, items[], applied, skipped, rejected, revFrom, revTo}
let busy = false;

/** @type {Set<() => void>} */
const listeners = new Set();

/** 订阅状态变化(diff 批准流/脚本页签重渲)。@returns {() => void} 取消函数 */
export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function notify() { for (const fn of listeners) { try { fn(); } catch (e) { console.error("[plan] 订阅者异常", e); } } }

/* ---------------- 读面 ---------------- */

export function getDraft() { return draft; }
export function getPreview() { return preview; }
export function getApprovals() { return approvals.slice(); }
export function getApplyResult() { return applyResult; }
export function isBusy() { return busy; }

/* ---------------- 解析(AI 产出 plan JSON → plan 数组)---------------- */

/**
 * 解析粘贴的 plan JSON。接受两形:{plan:[…]} 或裸数组 […];
 * 逐项要求 {tool, args?};未知/非对象/缺 tool 一律拒绝(诚实报错,不猜)。
 * @returns {{ plan: PlanItem[] } | { error: string }}
 */
export function parsePlanText(text) {
  let v;
  try {
    v = JSON.parse(text);
  } catch (e) {
    return { error: `plan 不是合法 JSON:${String(e && e.message || e)}` };
  }
  const items = Array.isArray(v) ? v : (v && Array.isArray(v.plan) ? v.plan : null);
  if (!items) return { error: "plan 结构不对:要 {plan:[{tool,args}]} 或裸数组 [{tool,args}]" };
  if (!items.length) return { error: "plan 为空(至少一项)" };
  const plan = [];
  for (let i = 0; i < items.length; i += 1) {
    const it = items[i];
    if (!it || typeof it !== "object" || typeof it.tool !== "string" || !it.tool) {
      return { error: `plan[${i}] 缺 tool(字符串)` };
    }
    const args = it.args && typeof it.args === "object" && !Array.isArray(it.args) ? it.args : {};
    plan.push({ tool: it.tool, args, id: typeof it.id === "string" ? it.id : undefined });
  }
  return { plan };
}

/* ---------------- 写面 ---------------- */

/** 入口(检查器批量区 / 脚本页签共用):载入新 draft(未预演态),清旧预演与批准面。 */
export function submitDraft(plan, source) {
  draft = { plan: plan.map((p) => ({ tool: p.tool, args: p.args || {}, ...(p.id ? { id: p.id } : {}) })), source };
  preview = null;
  approvals = [];
  applyResult = null;
  notify();
}

/** 预演当前 draft(preview_plan;副本 dry-run,真工程零写入)。 */
export async function previewDraft() {
  if (!draft || busy) return null;
  busy = true;
  notify();
  try {
    const env = await call("preview_plan", { plan: draft.plan });
    preview = env.ok ? env.data : { error: env.message || env.code, code: env.code };
    approvals = (preview && preview.items) ? preview.items.map(() => null) : [];
    applyResult = null;
  } finally {
    busy = false;
  }
  notify();
  return preview;
}

/** 逐项批准/拒绝/撤销决定(未决 = null;apply_plan 默认拒绝未决项)。 */
export function setApproval(index, decision) {
  if (!preview || !preview.items || index < 0 || index >= approvals.length) return;
  approvals[index] = approvals[index] === decision ? null : decision;
  notify();
}

/** 全批(预演 ok 项)/ 全拒(全部)。返回受影响数。 */
export function approveAllOk() {
  let n = 0;
  preview.items.forEach((it, i) => { if (it.ok) { approvals[i] = "approve"; n += 1; } });
  notify();
  return n;
}
export function rejectAll() {
  approvals = approvals.map(() => "reject");
  notify();
  return approvals.length;
}
export function clearApprovals() {
  approvals = approvals.map(() => null);
  notify();
}

/** 当前批准面(apply_plan approvals 契约形)。 */
export function approvalsOf() {
  const approve = [];
  const reject = [];
  approvals.forEach((a, i) => {
    if (a === "approve") approve.push(i);
    else if (a === "reject") reject.push(i);
  });
  return { approve, reject };
}

/** 应用批准项(apply_plan;逐项原 Op 通道,causedBy 绑 planId;真工程落地)。 */
export async function applyDraft() {
  if (!draft || !preview || !preview.items || busy) return null;
  busy = true;
  notify();
  try {
    const env = await call("apply_plan", {
      plan: draft.plan,
      approvals: approvalsOf(),
      planId: `plan-web-${Date.now().toString(36)}`,
    });
    applyResult = env.ok ? env.data : { error: env.message || env.code, code: env.code };
    if (applyResult && applyResult.applied > 0) await reproject(); // 落地后投影对齐(rev 翻牌)
  } finally {
    busy = false;
  }
  notify();
  return applyResult;
}
