/* 差异面板(T2.5):OpLog 逐字段审计(actor 过滤/limit/勾选批量撤销 undo ×N)。
 * T3.7:批量撤销前确认弹窗(设置可关;prefs.confirmBatch)。
 * 册七 T7.5:AI 计划批准流——preview_plan 逐项卡片(字段级 before/after;大值折叠
 * 提示经 oplog_tail 查)→ 逐项批准/拒绝 + 全批/全拒 → apply_plan 回执逐项 toast +
 * OpLog 归因入口(causedBy 链含 planId)。三入口(检查器批量区/脚本页签/粘贴)共用
 * core/plan.js 同一份 draft,此处是唯一审批台。 */
import { h, clear } from "../ui/dom.js";
import { selectField, textField } from "../ui/controls.js";
import { undoBatch } from "../core/commands.js";
import { call } from "../core/api.js";
import { reproject } from "../core/projector.js";
import { openDialog } from "../ui/dialog.js";
import { confirmBatch } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";
import * as plan from "../core/plan.js";

let actorSel = null;
let limitField = null;
let rowsEl = null;
let planHost = null;

export function mount(container) {
  const panel = h("div", { class: "panel tab-page" });
  panel.appendChild(h("h3", null, ["OpLog(AI 改了什么,逐字段可审计)"]));
  actorSel = selectField({
    id: "diff-actor", testid: "diff-actor",
    options: [["", "全部"], ["agent", "agent(AI)"], ["user", "user(人)"], ["script", "script(脚本)"], ["plugin", "plugin(插件)"]],
  });
  panel.appendChild(h("label", { class: "filter-row" }, ["actor ", actorSel.root]));
  limitField = textField({ id: "diff-limit", testid: "diff-limit", onEnter: () => refresh() });
  limitField.set("50");
  panel.appendChild(h("label", { class: "filter-row" }, ["limit ", limitField.root]));
  panel.appendChild(h("div", { class: "filter-row" }, [
    h("button", { id: "diff-refresh", testid: "diff-refresh", onclick: () => refresh() }, ["刷新"]),
    h("button", { id: "diff-undo-batch", testid: "diff-undo-batch", onclick: () => batchUndo() }, ["一键撤销该批(undo ×N)"]),
  ]));
  rowsEl = h("div", { id: "diff-rows", testid: "diff-rows" });
  panel.appendChild(rowsEl);
  container.appendChild(panel);
  // 册七 T7.5:计划批准流(唯一审批台;状态在 core/plan.js)
  const planPanel = h("div", { class: "panel tab-page" });
  planPanel.appendChild(h("h3", null, ["AI 计划批准流(preview_plan → 人审 → apply_plan)"]));
  planHost = h("div", { testid: "plan-host" });
  planPanel.appendChild(planHost);
  planPanel.appendChild(h("div", { class: "hint" }, [
    "预演在服务端副本工程 dry-run(真工程不落盘);应用只执行显式批准项(默认拒绝,未决/被拒零写入)。", " ",
    "入口:检查器「批量操作」粘贴 plan、脚本页签「以 plan 提交」,三方共用同一份待审计划。",
  ]));
  container.appendChild(planPanel);
  plan.subscribe(renderPlan);
  renderPlan();
}

export async function refresh() {
  const actor = actorSel.get() || undefined;
  const limit = Number(limitField.get()) || 50;
  const env = await call("oplog_tail", { limit, ...(actor ? { actor } : {}) });
  if (!env.ok) return;
  clear(rowsEl);
  for (const op of env.data.ops || []) {
    const pick = h("input", { type: "checkbox", class: "pick", testid: "diff-pick" });
    rowsEl.appendChild(h("div", { class: "row", testid: "diff-row" }, [
      pick,
      h("span", { class: "oid" }, [op.op_id ?? op.opId ?? ""]),
      h("span", { class: `badge ${op.actor.kind}` }, [op.actor.kind]),
      h("span", { class: "bd" }, [
        String(op.summary ?? ""),
        causedByChip(op),
        h("div", { class: "delta" }, [
          `${JSON.stringify(op.before).slice(0, 80)} → ${JSON.stringify(op.after).slice(0, 80)}`,
        ]),
      ]),
    ]));
  }
}

/** causedBy 链入口(T7.5):Op 带 causedBy 时显式成行(planId 可从此对账)。 */
function causedByChip(op) {
  const chain = op.causedBy || op.caused_by;
  if (!Array.isArray(chain) || !chain.length) return document.createTextNode("");
  const kids = ["causedBy: "];
  chain.forEach((c, i) => {
    if (i) kids.push(" ← ");
    kids.push(h("code", { class: c.startsWith("plan-") ? "plan-id" : null, testid: c.startsWith("plan-") ? "diff-plan-id" : null }, [c]));
  });
  return h("div", { class: "plan-causedby", testid: "diff-causedby" }, kids);
}

/* ---------------- 计划批准流(核心状态见 core/plan.js)---------------- */

function renderPlan() {
  if (!planHost) return;
  clear(planHost);
  const draft = plan.getDraft();
  if (!draft) {
    planHost.appendChild(h("div", { class: "empty-hint", testid: "plan-empty" }, [
      "暂无待审计划:在检查器「批量操作」或脚本页签提交 plan JSON 后,这里逐项人审。",
    ]));
    return;
  }
  planHost.appendChild(h("div", { class: "filter-row", testid: "plan-draft-head" }, [
    h("span", { class: "badge" }, [`来源 ${draft.source}`]),
    h("span", null, [`${draft.plan.length} 项`]),
  ]));
  const pv = plan.getPreview();
  if (!pv) {
    planHost.appendChild(h("div", { class: "filter-row" }, [
      h("button", { testid: "plan-preview", disabled: plan.isBusy(), onclick: () => plan.previewDraft() },
        [plan.isBusy() ? "预演中…" : "① 预演(preview_plan,副本 dry-run)"]),
    ]));
    return;
  }
  if (pv.error) {
    planHost.appendChild(h("div", { class: "plan-err", testid: "plan-preview-error" }, [
      `预演失败:${pv.code || ""} ${pv.error}`,
    ]));
    planHost.appendChild(retryRow());
    return;
  }
  const sum = pv.summary || {};
  planHost.appendChild(h("div", { class: "filter-row", testid: "plan-preview-summary" }, [
    h("span", null, [`预演:${sum.total ?? "?"} 项,ok ${sum.ok ?? "?"} / 错 ${sum.err ?? "?"};rev ${pv.revFrom} → ${pv.revTo}(副本,真工程未动)`]),
  ]));
  const list = h("div", { testid: "plan-items" });
  pv.items.forEach((item, i) => list.appendChild(planCard(item, i)));
  planHost.appendChild(list);
  planHost.appendChild(actionRow());
  renderApplyResult();
}

function retryRow() {
  return h("div", { class: "filter-row" }, [
    h("button", { testid: "plan-repreview", disabled: plan.isBusy(), onclick: () => plan.previewDraft() }, ["重新预演"]),
  ]);
}

/** 逐项卡片:批准/拒绝双钮(再点撤销回未决)+ 字段级 before/after(大值折叠)。 */
function planCard(item, i) {
  const dec = plan.getApprovals()[i];
  const card = h("div", { class: `plan-card ${dec || ""}${item.ok ? "" : " err"}`, testid: "plan-item", dataset: { index: String(i) } });
  const head = h("div", { class: "filter-row" }, [
    h("span", { class: "badge" }, [`#${i}${item.id ? ` · ${item.id}` : ""}`]),
    h("code", null, [item.tool]),
    item.ok
      ? h("span", { class: "badge ok" }, ["预演 ok"])
      : h("span", { class: "badge err", title: String(item.message || "") }, [`预演失败 ${item.code}`]),
    h("span", { class: "plan-rev", title: "该项在副本上的 rev 推进" }, [`rev ${item.revBefore}→${item.revAfter}`]),
    h("span", { class: "spacer" }),
    decBtn(i, "approve", dec, "批准(应用时执行此项)"),
    decBtn(i, "reject", dec, "拒绝(应用时显式跳过此项)"),
  ]);
  card.appendChild(head);
  card.appendChild(h("div", { class: "plan-decision", testid: "plan-item-decision" }, [
    dec === "approve" ? "已批准" : dec === "reject" ? "已拒绝" : "未决(默认拒绝,不落地)",
  ]));
  const changes = h("div", { class: "plan-changes", testid: "plan-item-changes" });
  for (const ch of item.changes || []) changes.appendChild(changeRow(ch));
  if (!(item.changes || []).length) {
    changes.appendChild(h("span", { class: "hint" }, [item.ok ? "(无字段级变更预览:该工具回执未产 Op 变更)" : ""]));
  }
  card.appendChild(changes);
  return card;
}

function decBtn(i, kind, dec, tip) {
  return h("button", {
    class: `plan-dec ${kind}${dec === kind ? " on" : ""}`,
    testid: kind === "approve" ? "plan-item-approve" : "plan-item-reject",
    "aria-pressed": dec === kind ? "true" : "false",
    title: tip,
    onclick: () => plan.setApproval(i, kind),
  }, [kind === "approve" ? "批准" : "拒绝"]);
}

/** 字段级 before/after:小值直显;slim 折叠值({elided,bytes})提示经 oplog_tail 查。 */
function changeRow(ch) {
  const row = h("div", { class: "plan-change", testid: "plan-change" }, [
    h("code", { class: "plan-opid", title: "副本预演 Op id" }, [ch.opId]),
    h("span", { class: "badge" }, [ch.kind]),
    h("span", { class: "plan-path" }, [`${ch.file} · ${ch.path}`]),
    h("span", { class: "hint" }, [String(ch.summary ?? "")]),
  ]);
  row.appendChild(deltaOf(ch.before, "before"));
  row.appendChild(deltaOf(ch.after, "after"));
  return row;
}

function deltaOf(v, tag) {
  if (v === undefined || v === null) return document.createTextNode("");
  if (v && v.elided) {
    return h("span", { class: "plan-delta elided", testid: `plan-delta-${tag}` },
      [`${tag}: 值过大(${v.bytes} bytes,完整值经 oplog_tail 查)`]);
  }
  return h("span", { class: "plan-delta", testid: `plan-delta-${tag}` },
    [`${tag}: ${JSON.stringify(v)}`]);
}

function actionRow() {
  const ap = plan.approvalsOf();
  return h("div", { class: "filter-row plan-actions" }, [
    h("button", { testid: "plan-approve-all", onclick: () => plan.approveAllOk(), title: "预演 ok 的项全部批准(失败项仍不批)" }, ["全批(ok 项)"]),
    h("button", { testid: "plan-reject-all", onclick: () => plan.rejectAll() }, ["全拒"]),
    h("button", { testid: "plan-clear", onclick: () => plan.clearApprovals() }, ["清 approvals"]),
    h("span", { class: "badge", testid: "plan-approval-count" }, [`批准 ${ap.approve.length} · 拒绝 ${ap.reject.length}`]),
    h("button", {
      class: "primary", testid: "plan-apply", disabled: plan.isBusy() || !ap.approve.length,
      title: ap.approve.length ? "apply_plan:仅执行批准项,causedBy 绑 planId(可审计可撤销)" : "先至少批准一项",
      onclick: () => plan.applyDraft(),
    }, [plan.isBusy() ? "应用中…" : "② 应用批准项(apply_plan)"]),
  ]);
}

/** apply_plan 回执:逐项 toast + 汇总行(rev 链/planId 归因入口)。 */
function renderApplyResult() {
  const r = plan.getApplyResult();
  if (!r) return;
  if (r.error) {
    planHost.appendChild(h("div", { class: "plan-err", testid: "plan-apply-error" }, [`应用失败:${r.code || ""} ${r.error}`]));
    return;
  }
  for (const it of r.items || []) {
    if (it.skipped) {
      toast(`#${it.index} ${it.tool} 未落地(${it.reason === "rejected_by_caller" ? "人已拒绝" : "未批准,默认拒绝"})`, false);
    } else if (it.ok) {
      toast(`#${it.index} ${it.tool} 已落地(rev ${it.rev}${(it.opIds || []).length ? `,op ${(it.opIds || []).join(",")}` : ""};OpLog 归因 causedBy=${r.planId})`);
    } else {
      toast(`#${it.index} ${it.tool} 应用失败:${it.code} ${it.message || ""}`, false);
    }
  }
  planHost.appendChild(h("div", { class: "filter-row plan-result", testid: "plan-apply-result" }, [
    h("span", { class: "badge ok" }, [`落地 ${r.applied}`]),
    h("span", { class: "badge" }, [`跳过 ${r.skipped}`]),
    h("span", { class: "badge" }, [`拒绝 ${r.rejected}`]),
    h("span", null, [`rev ${r.revFrom} → ${r.revTo}`]),
    h("span", { class: "plan-causedby" }, [
      "OpLog 归因链入口:causedBy=",
      h("code", { class: "plan-id", testid: "plan-apply-planid" }, [r.planId]),
      "(在上方 OpLog 面板逐条 diff-causedby 可见)",
    ]),
    h("button", { testid: "plan-result-refresh", onclick: () => refresh() }, ["刷新 OpLog 对账"]),
  ]));
}

async function batchUndo() {
  const n = rowsEl.querySelectorAll(".pick:checked").length;
  if (!n) {
    toast("先勾选要撤销的 Op 行", false);
    return;
  }
  if (confirmBatch()) {
    const yes = await confirmBatchDialog(n);
    if (!yes) return;
  }
  await undoBatch(n);
  await refresh();
  await reproject();
}

/** 批量确认弹窗(T3.7;testid=confirm-dialog;批量影响 N 笔 Op,先问一句)。 */
function confirmBatchDialog(n) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v) => { if (!settled) { settled = true; resolve(v); } };
    const dlg = openDialog({
      id: "confirm-dialog",
      title: `确认撤销 ${n} 笔操作?`,
      onClose: () => done(false), // Esc/遮罩关闭 = 取消
      build: (body) => {
        body.appendChild(h("p", null, [`将按 OpLog 顺序批量撤销 ${n} 笔(可重做)。`]));
        const ok = h("button", { testid: "confirm-ok" }, ["撤销这批"]);
        const cancel = h("button", { testid: "confirm-cancel" }, ["取消"]);
        ok.addEventListener("click", () => { done(true); dlg.close(); });
        cancel.addEventListener("click", () => { done(false); dlg.close(); });
        body.appendChild(h("div", { class: "wizard-actions" }, [ok, cancel]));
        ok.focus();
      },
    });
  });
}
