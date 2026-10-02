/* 检查器(T2.5;册四 FE2 重构):分组字段由 /ui-fields 单一真相源驱动(壳不读文件),
 * 控件形态与组级附加面见 insp-groups.js(本文件只留装配/草稿/应用流)。
 * 行为红线不变:基础三字段沿用旧 id(insp-start/insp-dur/insp-vol,e2e 兼容);
 * 交互模式 = 暂存草稿、点「应用」才发 clip_update(fill→apply 流程对齐)。
 * 整对象字段口径:textStyle/crop 带点草稿 → 现值克隆 + 草稿覆盖后整对象替换
 * (内核 Option 语义:局部对象会整替,必须带全量);transition/motion 维持按字段合并。
 */
import { $, h, clear } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore, uiStore } from "../core/store.js";
import { nestedGet } from "../core/model.js";
import { updateClip, duplicateSelectedToPlayhead, addTrack, selectClip } from "../core/commands.js";
import { batchUpdateClips } from "../core/edit-commands.js";
import { buildGroup, WHOLE_OBJECT_PARENTS, META } from "./insp-groups.js";
import { collapseGroup } from "../ui/controls.js";
import { watchForField } from "./kf-watch.js";
import { toast } from "../ui/toast.js";
import * as plan from "../core/plan.js";

/** field 名 → 控件实例 */
const fields = new Map();
/** 组级附加面(自持草稿编辑器/提示;fillFromSelection 时 __cfRefresh(row)) */
const extras = [];
let groupsHost = null;
let fieldsInfo = null;
let readonlyBox = null;
let applyBtn = null;
let dupBtn = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["检查器(分组)"]));
  fieldsInfo = h("div", { id: "insp-fields", testid: "insp-fields" }, ["未选中片段:点击时间线片段,或从左侧素材面板双击插入"]);
  container.appendChild(fieldsInfo);
  groupsHost = h("div", { id: "insp-groups", testid: "insp-groups" });
  container.appendChild(groupsHost);
  applyBtn = h("button", {
    id: "insp-apply", testid: "insp-apply", disabled: true,
    title: "改动生成可撤销的 Op(clip_update)",
    onclick: () => applyInspector(),
  }, ["应用(clip_update)"]);
  dupBtn = h("button", {
    id: "insp-dup", testid: "insp-dup", disabled: true,
    title: "复制选中片段到播放头(clip_duplicate)",
    onclick: () => duplicateSelectedToPlayhead(),
  }, ["复制到播放头"]);
  container.appendChild(h("div", { class: "insp-actions" }, [applyBtn, dupBtn]));
  readonlyBox = h("div", { id: "insp-readonly", class: "insp-readonly", hidden: true, testid: "insp-readonly" });
  container.appendChild(readonlyBox);
  container.appendChild(h("div", { class: "insp-tracks" }, [
    h("button", { id: "track-add-video", testid: "track-add-video", onclick: () => addTrack("video") }, ["+视频轨"]),
    h("button", { id: "track-add-audio", testid: "track-add-audio", onclick: () => addTrack("audio") }, ["+音频轨"]),
    h("button", { id: "track-add-text", testid: "track-add-text", onclick: () => addTrack("text") }, ["+文本轨"]),
  ]));
  container.appendChild(buildBatchOps());

  uiStore.subscribe((patch, st) => {
    if (patch.uiFields !== undefined || patch.__reset__) buildGroups(st.uiFields);
  });
  buildGroups(uiStore.get().uiFields);
  selectionStore.subscribe(() => fillFromSelection());
}

function buildGroups(uiFields) {
  clear(groupsHost);
  fields.clear();
  extras.length = 0;
  if (!uiFields || !uiFields.editable) {
    fieldsInfo.textContent = "ui-fields 下发失败:检查器退化为空分组";
    return;
  }
  const ctx = {
    rowOf: () => {
      const id = selectionStore.get().clipId;
      return id ? timelineStore.get().clips.find((c) => c.id === id) || null : null;
    },
  };
  for (const [group, list] of Object.entries(uiFields.editable)) {
    const { controls, extras: groupExtras } = buildGroup(group, list, ctx);
    for (const control of controls) fields.set(control.field, control);
    const nodes = controls.map((c) => wrapField(c));
    for (const ex of groupExtras) {
      extras.push(ex);
      nodes.push(ex);
    }
    groupsHost.appendChild(collapseGroup(group, nodes, { testid: `insp-group-${group}` }));
  }
  fillFromSelection();
}

function wrapField(control) {
  const meta = control.meta || {};
  // 开关控件自带 wrapping label(toggleField),不得再嵌套 label(label 嵌套会使
  // 关联失效 → axe label critical;册四 FE2 axe 实测)
  if (meta.type === "toggle") return control.root;
  const label = meta.label || control.field;
  // 秒表(T5.1):可动画属性(scale/opacity/rotation/volume)旁打点开关;
  // 草稿值经 control.get() 读(打点取当前输入值)
  const watch = watchForField(control.field, () => selectedRow(), () => control.get());
  return h("label", {
    class: meta.type === "select" || meta.type === "slider" ? "v2-field" : null,
    "data-tip": control.field,
  }, watch ? [label, control.root, watch] : [label, control.root]);
}

function selectedRow() {
  const id = selectionStore.get().clipId;
  if (!id) return null;
  return timelineStore.get().clips.find((c) => c.id === id) || null;
}

function fillFromSelection() {
  const row = selectedRow();
  for (const ex of extras) {
    if (typeof ex.__cfRefresh === "function") ex.__cfRefresh(row);
  }
  if (!row) {
    fieldsInfo.textContent = "未选中片段:点击时间线片段,或从左侧素材面板双击插入";
    readonlyBox.hidden = true;
    readonlyBox.textContent = "";
    applyBtn.disabled = true;
    dupBtn.disabled = true;
    for (const control of fields.values()) control.set(null);
    return;
  }
  const track = (projectStore.get().project?.tracks || []).find((t) => t.id === row.track);
  const multi = (selectionStore.get().clipIds || []).length;
  fieldsInfo.textContent = multi > 1
    ? `已多选 ${multi} 段(主选中 ${row.id}):点「应用」将批量修改所有选中片段(逐笔 Op)`
    : `id=${row.id} track=${row.track} src=${row.src ?? "-"}`;
  const selInfo = $("sel-info");
  if (selInfo) selInfo.textContent = `选中 ${row.id}(${track ? track.kind : "?"})`;
  for (const [f, control] of fields) {
    control.set(nestedGet(row, f));
  }
  renderReadonly(row);
  applyBtn.disabled = false;
  dupBtn.disabled = false;
}

/** 只读展示(E4-3):投影里有、ClipPatch 未承接的字段(ui-fields.readonly 列表)。 */
function renderReadonly(row) {
  const ro = (uiStore.get().uiFields && uiStore.get().uiFields.readonly) || [];
  const vals = [];
  for (const f of ro) {
    const v = row[f];
    if (v === undefined || v === null) continue;
    vals.push(`${f}=${JSON.stringify(v)}`);
  }
  if (!vals.length) {
    readonlyBox.hidden = true;
    readonlyBox.textContent = "";
    return;
  }
  readonlyBox.hidden = false;
  readonlyBox.textContent = "只读(内核 ClipPatch 暂未承接,E4-3):" + vals.join("  ");
}

/** 应用:草稿 → patch(只含变更字段)→ clip_update。批量口径不变(T4.2)。 */
async function applyInspector() {
  const row = selectedRow();
  if (!row) return;
  const selIds = selectionStore.get().clipIds || [];
  if (selIds.length > 1) {
    const done = await batchUpdateClips(selIds, (r) => computePatch(r, row).patch);
    if (!done) toast("选中片段已等于目标值,无改动");
    return;
  }
  const { patch, warning } = computePatch(row, null);
  if (warning) toast(warning, false);
  if (!Object.keys(patch).length) {
    if (!warning) toast("无改动");
    return;
  }
  await updateClip(row.id, patch, `已应用 ${Object.keys(patch).join("/")}(可撤销)`);
}

/** 草稿字段 → patch。anchor=null(单选):与本行现值比对只提交变更;
 * anchor=锚点行(批量):只提交「与锚点不同」的字段。
 * 返回 { patch, warning? }:warning = 前端拦截(裁剪缺 w/h)说明,不阻塞其余字段。 */
function computePatch(row, anchor = null) {
  const patch = {};
  let warning = "";
  const flat = [];
  for (const [f, control] of fields) {
    const v = control.get();
    const isToggle = control.meta && control.meta.type === "toggle";
    // 开关恒参与(关→开/开→关都要可比对);其余空串 = 未填,不参与
    if (f.indexOf(".") >= 0 || isToggle || v !== "") flat.push([f, control, v]);
  }
  const isIntent = (f, raw, meta) => {
    if (!anchor) return true;
    const cur = nestedGet(anchor, f);
    const curN = (cur === undefined || cur === null) ? null : cur;
    if (meta.type === "number" || meta.type === "slider") return Number(raw) !== Number(curN ?? NaN);
    if (meta.type === "toggle") return Boolean(raw) !== Boolean(curN);
    return String(raw) !== String(curN ?? "");
  };
  // 带点字段:textStyle/crop = 整对象(现值克隆+覆盖);transition/motion = 按字段合并(null=不改)
  for (const [f, control, raw] of flat) {
    const dot = f.indexOf(".");
    if (dot < 0) continue;
    const obj = f.slice(0, dot);
    const key = f.slice(dot + 1);
    const meta = control.meta || {};
    let v = null;
    if (meta.type === "select") v = raw !== "" ? raw : null;
    else if (meta.type === "toggle") v = Boolean(raw);
    else if (meta.type === "align") v = raw !== "" ? raw : null;
    else if (raw !== "" && raw !== false) {
      v = meta.type === "number" || meta.type === "slider" ? Number(raw) : raw;
      if (typeof v === "number" && Number.isNaN(v)) v = null;
    }
    if (!isIntent(f, raw, meta)) continue;
    if (WHOLE_OBJECT_PARENTS.includes(obj)) {
      const base = row[obj] && typeof row[obj] === "object" ? { ...row[obj] } : {};
      if (v === null || v === "" || v === undefined) delete base[key];
      else base[key] = meta.type === "number" || meta.type === "slider" ? Number(v) : v;
      patch[obj] = base;
    } else {
      patch[obj] = patch[obj] || {};
      patch[obj][key] = v;
    }
  }
  for (const obj of Object.keys(patch)) {
    if (!Object.keys(patch[obj]).length) delete patch[obj];
  }
  // 裁剪前端拦截:w/h 必填(schema required);缺 → 丢弃 crop 并说明
  if (patch.crop && (!patch.crop.w || !patch.crop.h)) {
    delete patch.crop;
    warning = "裁剪需要宽与高(源像素):两个字段都填后应用,本次未含 crop";
  }
  // 平字段:与本行现值比对,只提交变更
  for (const [f, control, raw] of flat) {
    if (f.indexOf(".") >= 0) continue;
    const meta = control.meta || {};
    if (raw === "" && meta.type !== "toggle") continue;
    if (!isIntent(f, raw, meta)) continue;
    let v = raw;
    if (meta.type === "number" || meta.type === "slider") {
      v = Number(raw);
      if (Number.isNaN(v)) continue;
    } else if (meta.type === "toggle") {
      v = Boolean(raw);
    }
    const cur = row[f];
    const curN = (cur === undefined || cur === null) ? null : cur;
    const differs = typeof v === "number"
      ? v !== Number(curN ?? NaN)
      : meta.type === "toggle"
        ? Boolean(v) !== Boolean(curN)
        : String(v) !== String(curN ?? "");
    if (differs) patch[f] = v;
  }
  return { patch, warning };
}

/** 供 main 装配:取消选中走这里(保持旧壳「点空白即取消」文案一致性)。 */
export function inspectorDeselectHint() {
  selectClip(null);
}

/** 批量操作区(册七 T7.5):粘贴 AI 产出的 plan JSON → preview_plan 预演 →
 * 送差异面板批准流(与脚本页签「以 plan 提交」共用 core/plan.js 同一 draft)。 */
function buildBatchOps() {
  const box = h("textarea", {
    class: "insp-textarea plan-paste", testid: "batch-plan-src", spellcheck: "false",
    "aria-label": "批量操作 plan JSON 粘贴区",
    placeholder: "{\"plan\":[{\"tool\":\"clip_update\",\"args\":{…}}]}(AI 产出,人审后落地)",
    style: "width:100%;min-height:64px",
  });
  const run = async () => {
    const r = plan.parsePlanText(box.value);
    if (r.error) {
      toast(r.error, false);
      return;
    }
    plan.submitDraft(r.plan, "检查器批量");
    toast(`已送批准流(${r.plan.length} 项):到「差异面板」预演并逐项人审`);
  };
  return collapseGroup("批量操作(plan 批准流)", [
    h("div", { class: "hint" }, ["粘贴 plan JSON → 进差异面板批准流(preview_plan 预演,人审后 apply_plan;默认拒绝未批准项)。"]),
    box,
    h("div", { class: "filter-row" }, [
      h("button", { testid: "batch-plan-submit", onclick: run, title: "送差异面板批准流(不直接执行)" }, ["送批准流"]),
    ]),
  ], { open: false, testid: "insp-batch-ops" });
}

// META 仅供调试/对表(e2e 可经控制台读);防 tree-shake 语义导出
export { META };
