/* 检查器(T2.5):分组字段由 /ui-fields 单一真相源驱动(壳不读文件)。
 * 行为与旧壳对拍:基础三字段沿用旧 id(insp-start/insp-dur/insp-vol,e2e 兼容红线);
 * 转场/动效按 v0.6 口径展开为子字段控件;readonly 面如实标注「内核未承接」。
 * 交互模式:暂存草稿、点「应用」才发 clip_update(与 e2e fill→apply 流程对齐)。
 */
import { $, h, clear } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore, uiStore } from "../core/store.js";
import { nestedGet } from "../core/model.js";
import { updateClip, duplicateSelectedToPlayhead, addTrack, selectClip } from "../core/commands.js";
import { numberField, textField, selectField, collapseGroup } from "../ui/controls.js";
import { toast } from "../ui/toast.js";

// 展示元数据:仅输入控件形态(step/min 等),字段全集与分组以 ui-fields 为准(旧壳口径)
const FIELD_META = {
  startMs: { type: "number", step: 1, min: 0, legacy: "insp-start" },
  durationMs: { type: "number", step: 1, min: 1, legacy: "insp-dur" },
  sourceInMs: { type: "number", step: 1, min: 0 },
  speed: { type: "number", step: 0.05, min: 0.25, max: 4 },
  volume: { type: "number", step: 0.1, min: 0, max: 2, legacy: "insp-vol" },
  opacity: { type: "number", step: 0.05, min: 0, max: 1 },
  scale: { type: "number", step: 0.05, min: 0 },
  text: { type: "text" },
  freezeMs: { type: "number", step: 1, min: 0 },
};
// v0.6 NLE 化:枚举下拉与嵌套对象字段(转场/动效)。label=人话;enum=下拉选项;
// 值写入 clip_update 的嵌套 patch(transition/motion)。
const FIELD_META_V2 = {
  "transition.type": { type: "select", label: "转场类型",
    enum: [["", "(无)"], ["fade", "叠化"], ["wipeleft", "左划"], ["wipeup", "上划"],
      ["slideleft", "左滑"], ["circleopen", "圆形展开"], ["cut", "硬切"], ["none", "关闭"]] },
  "transition.durMs": { type: "number", label: "转场时长(ms)", step: 10, min: 0 },
  "transition.fx": { type: "text", label: "转场 fx(效果目录 id,可空)" },
  "motion.in": { type: "select", label: "入场",
    enum: [["", "(无)"], ["fadeIn", "淡入"], ["slideInLeft", "左侧滑入"], ["slideInRight", "右侧滑入"],
      ["scaleIn", "缩放入场"], ["zoomIn", "推近入场"]] },
  "motion.inMs": { type: "number", label: "入场时长(ms)", step: 50, min: 0 },
  "motion.out": { type: "select", label: "出场",
    enum: [["", "(无)"], ["fadeOut", "淡出"], ["slideOutLeft", "左滑出"], ["slideOutRight", "右滑出"]] },
  "motion.outMs": { type: "number", label: "出场时长(ms)", step: 50, min: 0 },
};

/** field 名 → 控件实例 */
const fields = new Map();
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

  uiStore.subscribe((patch, st) => {
    if (patch.uiFields !== undefined || patch.__reset__) buildGroups(st.uiFields);
  });
  buildGroups(uiStore.get().uiFields);
  selectionStore.subscribe(() => fillFromSelection());
}

function buildGroups(uiFields) {
  clear(groupsHost);
  fields.clear();
  if (!uiFields || !uiFields.editable) {
    fieldsInfo.textContent = "ui-fields 下发失败:检查器退化为空分组";
    return;
  }
  for (const [group, list] of Object.entries(uiFields.editable)) {
    const controls = [];
    for (const f of list) {
      // 后端对嵌套对象(transition/motion)下发收敛后的对象键——展开为子字段控件
      const subs = Object.keys(FIELD_META_V2).filter((k) => k.startsWith(`${f}.`));
      if (!FIELD_META_V2[f] && subs.length) {
        for (const k of subs) controls.push(buildField(k, FIELD_META_V2[k]));
        continue;
      }
      const v2 = FIELD_META_V2[f];
      if (v2) {
        controls.push(buildField(f, v2));
        continue;
      }
      const meta = FIELD_META[f] || { type: "text" };
      controls.push(buildField(f, meta));
    }
    groupsHost.appendChild(collapseGroup(group, controls.map((c) => wrapField(c, c.field)), { testid: `insp-group-${group}` }));
  }
  fillFromSelection();
}

function buildField(f, meta) {
  const testid = `field-${f.replace(/\./g, "-")}`;
  let control;
  if (meta.type === "select") {
    control = selectField({ id: `insp-f-${f}`, testid, options: meta.enum });
  } else if (meta.type === "text") {
    control = textField({ id: meta.legacy || `insp-f-${f}`, testid: meta.legacy || testid });
  } else {
    control = numberField({
      id: meta.legacy || `insp-f-${f}`, testid: meta.legacy || testid,
      step: meta.step, min: meta.min, max: meta.max,
    });
  }
  control.field = f;
  control.meta = meta;
  fields.set(f, control);
  return control;
}

function wrapField(control, f) {
  const label = h("label", { class: control.meta && control.meta.type === "select" ? "v2-field" : null, "data-tip": f }, [
    control.meta && control.meta.label ? control.meta.label : f,
    control.root,
  ]);
  return label;
}

function selectedRow() {
  const id = selectionStore.get().clipId;
  if (!id) return null;
  return timelineStore.get().clips.find((c) => c.id === id) || null;
}

function fillFromSelection() {
  const row = selectedRow();
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
  fieldsInfo.textContent = `id=${row.id} track=${row.track} src=${row.src ?? "-"}`;
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

/** 应用:草稿 → patch(只含变更字段;带点字段收拢为嵌套对象)→ clip_update。 */
async function applyInspector() {
  const row = selectedRow();
  if (!row) return;
  const patch = {};
  const flat = [];
  for (const [f, control] of fields) {
    const v = control.get();
    if (f.indexOf(".") >= 0 || v !== "") flat.push([f, control, v]);
  }
  // 带点字段收拢:patch.transition / patch.motion(空串/无效 → 显式 null 清空)
  for (const [f, control, raw] of flat) {
    const dot = f.indexOf(".");
    if (dot < 0) continue;
    const obj = f.slice(0, dot);
    const key = f.slice(dot + 1);
    const meta = control.meta || {};
    let v = null;
    if (meta.type === "select") {
      if (raw !== "") v = raw;
    } else if (raw !== "") {
      v = Number(raw);
      if (Number.isNaN(v)) v = null;
    }
    patch[obj] = patch[obj] || {};
    patch[obj][key] = v;
  }
  for (const obj of Object.keys(patch)) {
    if (!Object.keys(patch[obj]).length) delete patch[obj];
  }
  // 平字段:与投影现值比对,只提交变更
  for (const [f, control, raw] of flat) {
    if (f.indexOf(".") >= 0) continue;
    const meta = control.meta || {};
    if (raw === "") continue;
    const v = meta.type === "number" ? Number(raw) : raw;
    if (Number.isNaN(v)) continue;
    const cur = row[f];
    const curN = (cur === undefined || cur === null) ? null : cur;
    const differs = meta.type === "number"
      ? Number(v) !== Number(curN ?? NaN)
      : String(v) !== String(curN ?? "");
    if (differs) patch[f] = v;
  }
  if (!Object.keys(patch).length) {
    toast("无改动");
    return;
  }
  await updateClip(row.id, patch, `已应用 ${Object.keys(patch).map((k) => (typeof patch[k] === "object" ? k : k)).join("/")}(可撤销)`);
}

/** 供 main 装配:取消选中走这里(保持旧壳「点空白即取消」文案一致性)。 */
export function inspectorDeselectHint() {
  selectClip(null);
}
