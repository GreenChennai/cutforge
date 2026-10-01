/* 检查器分组构建(册四 FE2 重构;从 inspector.js 分册,单文件 ≤400 纪律)。
 *
 * - 字段集与分组仍以 /ui-fields 单一真相源驱动(inspector.buildGroups 逐组调本模块);
 * - 平字段升级人话控件:变速滑杆+预设、降噪四档、变调居中滑杆、旋转/翻转/裁剪、
 *   文本 14 样式字段(字体/字号/颜色板/描边/底衬/阴影/对齐九宫格/行距/透明度/位置/卡拉OK);
 * - 自持草稿的整对象编辑器以「附加面」嵌入本组:速度曲线(curve.js)/特效栈(fxlib.js)/
 *   花字(textool.js),各自单 Op commit,不进检查器统一草稿;
 * - 转场钳制提示:草稿 durMs > 片段时长 → 提示「后端将钳制」(不阻塞,诚实口径)。
 */
import { h } from "../ui/dom.js";
import {
  numberField, textField, selectField, toggleField, sliderField, colorField,
} from "../ui/controls.js";
import { buildCurveHost } from "./curve.js";
import { buildFxStackHost } from "./fxlib.js";
import { buildHuaziHost } from "./textool.js";
import { buildKeyframeHost } from "./kf-editor.js";
import { positionWatchRow } from "./kf-watch.js";
import { buildGradeHost } from "./grade.js";
import { ensureCatalogs, motionLists } from "../core/catalogs.js";
import { clearTransition, applyMotionAlias } from "../core/edit-commands.js";
import { toast } from "../ui/toast.js";

/** 整对象替换的带点父字段(patch.x = 现值克隆 + 草稿覆盖;inspector.computePatch 消费)。 */
export const WHOLE_OBJECT_PARENTS = ["textStyle", "crop"];

/** 平字段展示元数据:type=控件形态;label=人话;legacy=旧 id 双锚点(e2e 红线)。 */
export const META = {
  startMs: { type: "number", step: 1, min: 0, legacy: "insp-start", label: "起点(ms)" },
  durationMs: { type: "number", step: 1, min: 1, legacy: "insp-dur", label: "时长(ms)" },
  sourceInMs: { type: "number", step: 1, min: 0, label: "源入点(ms)" },
  volume: { type: "slider", min: 0, max: 2, step: 0.05, legacy: "insp-vol", label: "音量" },
  denoise: { type: "select", label: "降噪(afftdn 四档)", enum: [
    ["", "(工程默认)"], ["off", "关"], ["low", "轻(本底噪声)"],
    ["mid", "中(人声推荐)"], ["high", "强(重降噪,可能伤音质)"]] },
  pitch: { type: "slider", min: -12, max: 12, step: 1, label: "变调(半音,0=原调)" },
  speed: { type: "slider", min: 0.25, max: 4, step: 0.05, label: "速度(x)" },
  reverse: { type: "toggle", label: "倒放(先于变速)" },
  scale: { type: "slider", min: 0, max: 4, step: 0.05, label: "缩放(画中画/构图)" },
  opacity: { type: "slider", min: 0, max: 1, step: 0.05, label: "不透明度" },
  rotation: { type: "slider", min: -180, max: 180, step: 5, label: "旋转(°)" },
  flip: { type: "select", label: "翻转", enum: [
    ["", "(不变)"], ["none", "不翻转"], ["h", "水平镜像"], ["v", "垂直镜像"]] },
  "crop.x": { type: "number", step: 1, min: 0, label: "裁剪 X(源像素)" },
  "crop.y": { type: "number", step: 1, min: 0, label: "裁剪 Y(源像素)" },
  "crop.w": { type: "number", step: 1, min: 1, label: "裁剪宽(必填)" },
  "crop.h": { type: "number", step: 1, min: 1, label: "裁剪高(必填)" },
  text: { type: "textarea", label: "文本内容" },
  freezeMs: { type: "number", step: 1, min: 0, label: "定格时长(ms)" },
  "textStyle.fontFamily": { type: "text", label: "字体" },
  "textStyle.fontSize": { type: "number", step: 1, min: 8, max: 500, label: "字号(px)" },
  "textStyle.color": { type: "color", label: "填充色" },
  "textStyle.outlineColor": { type: "color", label: "描边色" },
  "textStyle.outlineWidth": { type: "slider", min: 0, max: 20, step: 0.5, label: "描边宽" },
  "textStyle.borderStyle": { type: "select", label: "底衬模式", enum: [
    ["", "(默认)"], ["1", "1 · 描边"], ["3", "3 · 底衬框"]] },
  "textStyle.backColor": { type: "color", label: "底衬色(模式 3)" },
  "textStyle.shadow": { type: "slider", min: 0, max: 20, step: 0.5, label: "阴影距离" },
  "textStyle.align": { type: "align", label: "对齐(锚点九宫格)" },
  "textStyle.lineSpacing": { type: "number", step: 1, min: 0, max: 200, label: "行距(px)" },
  "textStyle.opacity": { type: "slider", min: 0, max: 1, step: 0.05, label: "文本不透明度" },
  "textStyle.x": { type: "number", step: 1, min: 0, label: "X(画布像素;画布内可拖)" },
  "textStyle.y": { type: "number", step: 1, min: 0, label: "Y(画布像素;画布内可拖)" },
  "textStyle.karaoke": { type: "toggle", label: "卡拉OK逐字填色" },
  "transition.type": { type: "select", label: "转场类型(旧枚举;全量走 fx)", enum: [
    ["", "(不变)"], ["fade", "叠化"], ["wipeleft", "左划"], ["wipeup", "上划"],
    ["slideleft", "左滑"], ["circleopen", "圆形展开"], ["cut", "硬切"], ["none", "关闭"]] },
  "transition.durMs": { type: "slider", min: 0, max: 2000, step: 10, label: "转场时长(ms)" },
  "transition.fx": { type: "text", label: "转场 fx(tr.<目录id>)" },
  "motion.in": { type: "select", label: "入场", enum: "dyn-in" },
  "motion.inMs": { type: "number", step: 50, min: 0, label: "入场时长(ms)" },
  "motion.out": { type: "select", label: "出场", enum: "dyn-out" },
  "motion.outMs": { type: "number", step: 50, min: 0, label: "出场时长(ms)" },
};

const LEGACY_MOTION = {
  "motion.in": [["", "(无)"], ["fadeIn", "淡入"], ["slideInLeft", "左侧滑入"],
    ["slideInRight", "右侧滑入"], ["scaleIn", "缩放入场"], ["zoomIn", "推进入场"]],
  "motion.out": [["", "(无)"], ["fadeOut", "淡出"], ["slideOutLeft", "左滑出"],
    ["slideOutRight", "右滑出"]],
};

/** ui-fields 下发的整对象字段 → 展开的带点子字段(转场/动效/文本样式/裁剪)。 */
const EXPAND = {
  transition: ["transition.type", "transition.durMs", "transition.fx"],
  motion: ["motion.in", "motion.inMs", "motion.out", "motion.outMs"],
  textStyle: [
    "textStyle.fontFamily", "textStyle.fontSize", "textStyle.color",
    "textStyle.outlineColor", "textStyle.outlineWidth", "textStyle.borderStyle",
    "textStyle.backColor", "textStyle.shadow", "textStyle.align",
    "textStyle.lineSpacing", "textStyle.opacity", "textStyle.x", "textStyle.y",
    "textStyle.karaoke",
  ],
  crop: ["crop.x", "crop.y", "crop.w", "crop.h"],
};

/**
 * 构建一组控件与附加面。
 * @param {string} group 组名(ui-fields editable 键)
 * @param {string[]} list 字段名列表
 * @param {{ rowOf: () => Object|null }} ctx
 * @returns {{ controls: Object[], extras: HTMLElement[] }}
 */
export function buildGroup(group, list, ctx) {
  const controls = [];
  const extras = [];
  for (const f of list) {
    const expanded = EXPAND[f];
    if (expanded) {
      for (const k of expanded) controls.push(mkControl(k, META[k] || { type: "text" }));
      continue;
    }
    // fx / huazi / speedCurve / grade:整对象数组编辑器走组级附加面(自持草稿,单 Op commit)
    if (f === "fx" || f === "huazi" || f === "speedCurve" || f === "grade") continue;
    controls.push(mkControl(f, META[f] || { type: "text" }));
  }
  // 组级附加面(自持草稿的整对象编辑器 / 提示面 / 预设条)
  if (group === "变速") extras.push(speedPresets(controls), buildCurveHost(ctx.rowOf));
  if (group === "文本") extras.push(buildHuaziHost(ctx.rowOf));
  if (group === "特效") extras.push(buildFxStackHost(ctx.rowOf));
  if (group === "转场") extras.push(transitionExtras(controls, ctx));
  if (group === "动效") extras.push(motionExtras(ctx));
  if (group === "画面") extras.push(positionWatchRow(ctx.rowOf), buildKeyframeHost(ctx.rowOf));
  if (group === "调色") extras.push(buildGradeHost(ctx.rowOf));
  if (group === "画面") extras.push(h("div", { class: "hint" }, [
    "变换链(内核口径):裁剪→翻转→旋转→画幅归一;punchIn/position 为只读(ui-fields readonly)。",
  ]));
  return { controls, extras };
}

/** 单字段控件(与 inspector 旧 buildField 同构:{root,input,get,set,field,meta})。
 * legacy 双锚点:基础三字段 id 与 testid 都用旧 id(e2e 兼容红线,旧 inspector 口径)。 */
export function mkControl(f, meta) {
  const testid = meta.legacy || `field-${f.replace(/\./g, "-")}`;
  let control;
  switch (meta.type) {
    case "select":
      control = selectField({
        id: meta.legacy || `insp-f-${f}`, testid,
        options: (meta.enum === "dyn-in" || meta.enum === "dyn-out") ? LEGACY_MOTION[f] : meta.enum,
      });
      break;
    case "slider":
      control = sliderField({ id: meta.legacy || `insp-f-${f}`, testid, min: meta.min, max: meta.max, step: meta.step });
      break;
    case "color":
      control = colorField({ id: meta.legacy || `insp-f-${f}`, testid });
      break;
    case "toggle":
      control = toggleField({ id: meta.legacy || `insp-f-${f}`, testid, label: meta.label || "" });
      break;
    case "textarea": {
      const ta = h("textarea", { id: `insp-f-${f}`, testid, rows: "2", class: "insp-textarea" });
      control = { root: ta, input: ta, get: () => ta.value, set: (v) => { ta.value = v == null ? "" : String(v); } };
      break;
    }
    case "align":
      control = alignGrid(testid);
      break;
    default:
      control = numberField({ id: meta.legacy || `insp-f-${f}`, testid, step: meta.step, min: meta.min, max: meta.max });
  }
  control.field = f;
  control.meta = meta;
  // 动效下拉:目录到达后换成真实渲染目录(19 项;legacy 枚举逐字保留在表内)
  if (meta.enum === "dyn-in" || meta.enum === "dyn-out") {
    ensureCatalogs().then((cat) => {
      if (!cat) return;
      const lists = motionLists();
      const rows = meta.enum === "dyn-in" ? lists.in : lists.out;
      const opts = [["", "(无)"], ...rows.map((m) => [m.id, `${m.name}`])];
      refillSelect(control.input, opts);
    });
  }
  return control;
}

function refillSelect(sel, opts) {
  const cur = sel.value;
  while (sel.firstChild) sel.removeChild(sel.firstChild);
  for (const [v, label] of opts) sel.appendChild(h("option", { value: v }, [label]));
  sel.value = opts.some(([v]) => v === cur) ? cur : "";
}

/** 对齐九宫格(3x3;点选即锚点;再点同格取消)。 */
function alignGrid(testid) {
  const VALUES = [
    ["topLeft", "↖"], ["topCenter", "↑"], ["topRight", "↗"],
    ["middleLeft", "←"], ["center", "·"], ["middleRight", "→"],
    ["bottomLeft", "↙"], ["bottomCenter", "↓"], ["bottomRight", "↘"],
  ];
  let cur = "";
  const btns = [];
  const root = h("div", { class: "align-grid", testid, role: "group", "aria-label": "文本对齐九宫格" });
  for (const [v, glyph] of VALUES) {
    const b = h("button", {
      type: "button", class: "align-cell", dataset: { align: v }, title: v,
      "aria-label": `对齐 ${v}`,
      onclick: () => {
        cur = cur === v ? "" : v;
        for (const x of btns) x.classList.toggle("on", x.dataset.align === cur);
      },
    }, [glyph]);
    btns.push(b);
    root.appendChild(b);
  }
  return {
    root, input: /** @type {HTMLInputElement} */ (/** @type {*} */ (root)),
    get: () => cur,
    set(v) {
      cur = v ? String(v) : "";
      for (const x of btns) x.classList.toggle("on", x.dataset.align === cur);
    },
  };
}

/** 变速预设条(0.5/1/2/4x;写检查器草稿,应用才提交)。 */
function speedPresets(controls) {
  const speedCtl = controls.find((c) => c.field === "speed");
  if (!speedCtl) return h("span");
  return h("div", { class: "preset-row", testid: "speed-presets" }, [
    h("span", { class: "dim" }, ["预设"]),
    ...[0.5, 1, 2, 4].map((v) => h("button", {
      class: "mini", type: "button", dataset: { speed: String(v) },
      title: `速度预填 ${v}x(检查器「应用」提交)`,
      onclick: () => speedCtl.set(v),
    }, [`${v}x`])),
  ]);
}

/** 转场组附加面:钳制提示(不阻塞)+ 预设时长 + 清除转场。 */
function transitionExtras(controls, ctx) {
  const durCtl = controls.find((c) => c.field === "transition.durMs");
  const hint = h("div", { class: "hint warn-hint", testid: "insp-transition-clamp", hidden: true });
  const update = () => {
    const row = ctx.rowOf();
    if (!row) { hint.hidden = true; return; }
    const durCtlEl = durCtl;
    const draft = Number(durCtlEl && durCtlEl.get ? durCtlEl.get() : "") || 0;
    const cur = (row.transition && row.transition.durMs) || 0;
    const eff = draft || cur;
    const clipDur = row.endMs - row.startMs;
    if (eff > clipDur) {
      hint.textContent = `⚠ 转场时长 ${Math.round(eff)}ms > 片段时长 ${Math.round(clipDur)}ms:`
        + "渲染端将钳制到片段时长(服务端 WARN,不阻塞)";
      hint.hidden = false;
    } else {
      hint.hidden = true;
    }
  };
  if (durCtl && durCtl.input) {
    durCtl.input.addEventListener("input", update);
    durCtl.input.addEventListener("change", update);
  }
  const wrap = h("div", { class: "trans-extras" });
  wrap.appendChild(h("div", { class: "preset-row", testid: "trans-dur-presets" }, [
    h("span", { class: "dim" }, ["时长预设"]),
    ...[250, 500, 1000].map((v) => h("button", {
      class: "mini", type: "button", title: `时长预填 ${v}ms(应用提交)`,
      onclick: () => { durCtl.set(v); update(); },
    }, [`${v}`])),
    h("button", {
      class: "mini", type: "button", testid: "trans-clear", title: '关闭转场(transition_set type="none",单 Op)',
      onclick: () => {
        const row = ctx.rowOf();
        if (row) clearTransition(row.id);
      },
    }, ["清除转场"]),
  ]));
  wrap.appendChild(hint);
  wrap.__cfRefresh = update;
  return wrap;
}

/** 动效组附加面:mo.<id> 直通别名(高级,可折叠;motion_set inFx/outFx)。 */
function motionExtras(ctx) {
  const aliasIn = textField({ testid: "field-motion-aliasIn", ariaLabel: "入场直通别名", placeholder: "mo.<id>(高级)" });
  const aliasOut = textField({ testid: "field-motion-aliasOut", ariaLabel: "出场直通别名", placeholder: "mo.<id>(高级)" });
  const box = h("fieldset", { class: "insp-group collapsed", testid: "motion-alias-advanced" }, [
    h("legend", {
      onclick: () => box.classList.toggle("collapsed"),
      "data-tip": "mo.* 直通别名:优先于枚举,未注册渲染降级 WARN(写入后投影无回读)",
    }, ["直通别名(高级)"]),
    h("div", { class: "insp-fields-wrap" }, [
      h("label", { class: "v2-field" }, ["入场别名 ", aliasIn.root]),
      h("label", { class: "v2-field" }, ["出场别名 ", aliasOut.root]),
      h("button", {
        class: "mini", type: "button", testid: "motion-alias-apply",
        title: "motion_set inFx/outFx(各一笔 Op)",
        onclick: () => {
          const row = ctx.rowOf();
          if (!row) { toast("先选中片段", false); return; }
          const a = aliasIn.get().trim();
          const b = aliasOut.get().trim();
          if (a) applyMotionAlias(row.id, "in", a);
          if (b) applyMotionAlias(row.id, "out", b);
          if (!a && !b) toast("先填 mo.<id> 别名(如 mo.bounceIn)", false);
        },
      }, ["应用别名"]),
      h("div", { class: "hint" }, ["别名输入为只写(服务端 motion.fx 直通,投影不回显)。"]),
    ]),
  ]);
  return box;
}
