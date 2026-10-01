/* 一级校色面板(检查器「调色」组升级;册五 T5.2 FE)。
 *
 * - 一级:七杆滑杆(controls 工厂)+ Lift/Gamma/Gain 三色轮(grade-wheel.js,角度=色相
 *   半径=强度 → RGB 三值);二级(折叠):RGB/亮度曲线点集(kf-curve.js points 模式,
 *   写 grade.curves)+ LUT 管理(lut_import 导入/会话库/清除;.cube 非媒体扩展,
 *   media_browse 不列 → 库 = 会话导入记录 ∪ 工程 grade.lut 引用,诚实口径);
 * - 写通道:clip.grade 整对象替换(patch.grade;空 = 清除即 grade_clear),单 Op;
 * - 实时性诚实口径:预览画布是画质代理不含调色;拖动只改本地草稿(零 Op),
 *   松手提交;调色效果看「精确预览」(render_frame)——面板恒显提示。
 */
import { h, clear } from "../ui/dom.js";
import { call } from "../core/api.js";
import { timelineStore, selectionStore, ephemeralStore } from "../core/store.js";
import { updateClip } from "../core/commands.js";
import { sliderField } from "../ui/controls.js";
import { messageOf } from "../core/errors.js";
import { toast } from "../ui/toast.js";
import { createColorWheel } from "./grade-wheel.js";
import { createCurveCanvas } from "./kf-curve.js";
import { runPrecisePreview } from "./preview.js";

/** 一级滑杆参数(field → [min,max,step,label])。 */
const PRIMARY = [
  ["temperature", -100, 100, 1, "色温(暖+)"],
  ["tint", -100, 100, 1, "色调(品红+)"],
  ["exposure", -3, 3, 0.05, "曝光(档)"],
  ["contrast", -100, 100, 1, "对比"],
  ["highlights", -100, 100, 1, "高光"],
  ["shadows", -100, 100, 1, "阴影"],
  ["saturation", 0, 3, 0.05, "饱和度(1=不变)"],
];

const WHEELS = [
  ["lift", "Lift(阴影)", 0, 1, -1, 1],
  ["gamma", "Gamma(中间调)", 1, 1.5, 0.2, 5],
  ["gain", "Gain(高光)", 1, 2, 0, 4],
];

const CURVE_CHANNELS = [
  ["master", "亮度(master)"], ["red", "红"], ["green", "绿"], ["blue", "蓝"],
];


/** 当前选中片段行。 */
function rowNow(rowOf) {
  const row = rowOf ? rowOf() : null;
  if (row) return row;
  const id = selectionStore.get().clipId;
  return id ? timelineStore.get().clips.find((c) => c.id === id) || null : null;
}

const round4 = (v) => Math.round(v * 10000) / 10000;

/**
 * 构建调色面板宿主(检查器「调色」组附加面;__cfRefresh(row) 随选中刷新)。
 * @param {() => Object|null} rowOf
 */
export function buildGradeHost(rowOf) {
  let draft = {};
  let loadedId = null;
  let channel = "master";
  let dragging = false;

  const host = h("div", { class: "grade-host", testid: "grade-panel" });
  host.appendChild(h("div", { class: "hint warn-hint", testid: "grade-preview-hint" }, [
    "预览画布是画质代理,不含调色效果;调色结果以「◎ 精确预览」(render_frame)与成片为准。",
  ]));
  const preciseBtn = h("button", {
    class: "mini", type: "button", testid: "grade-precise",
    title: "内核逐帧渲染播放头所在帧(含调色/转场/特效最终效果)",
    onclick: () => runPrecisePreview(),
  }, ["◎ 精确预览调色效果"]);
  host.appendChild(h("div", { class: "grade-actions" }, [preciseBtn]));

  const primaryBox = h("div", { class: "grade-primary" });
  host.appendChild(primaryBox);
  const wheelBox = h("div", { class: "grade-wheels", testid: "grade-wheels" });
  host.appendChild(wheelBox);

  // —— 二级(折叠):曲线 + LUT ——
  const curvePane = h("div", { class: "grade-curve-pane" });
  const lutPane = h("div", { class: "grade-lut-pane" });
  const curveHost = h("div", { class: "grade-curve-host", testid: "grade-curves" });
  const lutBox = h("div", { class: "grade-lut-box", testid: "grade-lut" });
  curvePane.appendChild(curveHost);
  lutPane.appendChild(lutBox);
  const advanced = h("fieldset", { class: "insp-group", testid: "grade-advanced" }, [
    h("legend", { onclick: () => advanced.classList.toggle("collapsed"), "data-tip": "点击折叠/展开" }, ["二级:曲线 / LUT"]),
    h("div", { class: "insp-fields-wrap" }, [curvePane, lutPane]),
  ]);
  host.appendChild(advanced);

  // —— 拷贝/粘贴/清除 ——
  const copyBtn = h("button", {
    class: "mini", type: "button", testid: "grade-copy",
    title: "复制当前片段调色(会话态;不落盘不进撤销)",
    onclick: () => {
      const row = rowNow(rowOf);
      const g = row && row.grade;
      if (!g || !Object.keys(g).length) { toast("当前片段无调色可复制", false); return; }
      ephemeralStore.set({ gradeClipboard: JSON.parse(JSON.stringify(g)) });
      toast("调色已复制(会话态;选中另一片段粘贴)");
      refreshButtons();
    },
  }, ["复制调色"]);
  const pasteBtn = h("button", {
    class: "mini", type: "button", testid: "grade-paste",
    title: "粘贴已复制的调色(clip_update patch.grade 整对象,单 Op)",
    onclick: () => {
      const row = rowNow(rowOf);
      const clip = ephemeralStore.get().gradeClipboard;
      if (!row) { toast("先选中片段", false); return; }
      if (!clip) { toast("剪贴板为空:先在别的片段「复制调色」", false); return; }
      updateClip(row.id, { grade: clip }, "已粘贴调色(可撤销)");
    },
  }, ["粘贴调色"]);
  const clearBtn = h("button", {
    class: "mini", type: "button", testid: "grade-clear",
    title: "清除全部调色(patch.grade=null → grade_clear,单 Op)",
    onclick: () => {
      const row = rowNow(rowOf);
      if (!row) return;
      draft = {};
      updateClip(row.id, { grade: null }, "已清除调色(可撤销)");
    },
  }, ["清除调色"]);
  const actions2 = h("div", { class: "grade-actions", testid: "grade-clipboard" }, [copyBtn, pasteBtn, clearBtn]);
  host.appendChild(actions2);

  function refreshButtons() {
    pasteBtn.disabled = !ephemeralStore.get().gradeClipboard;
  }

  /* ---------------- 草稿与提交 ---------------- */

  function load(row, force = false) {
    if (!row) { draft = {}; loadedId = null; renderAll(); return; }
    if (loadedId === row.id && !force) return;
    loadedId = row.id;
    draft = row.grade && typeof row.grade === "object" ? JSON.parse(JSON.stringify(row.grade)) : {};
    renderAll();
  }

  function commit(msg) {
    const row = rowNow(rowOf);
    if (!row) { toast("先选中片段", false); return; }
    const clean = sanitize(draft);
    if (!Object.keys(clean).length) {
      updateClip(row.id, { grade: null }, msg || "已清除调色(可撤销)");
      return;
    }
    updateClip(row.id, { grade: clean }, msg || "调色已应用(整对象,可撤销)");
  }

  function sanitize(g) {
    const out = {};
    for (const [k, v] of Object.entries(g || {})) {
      if (v === undefined || v === null || v === "") continue;
      if (Array.isArray(v)) {
        if (v.length && v.every((x) => x !== null && x !== undefined)) out[k] = v;
        continue;
      }
      if (typeof v === "object") {
        if (Object.keys(v).length) out[k] = v;
        continue;
      }
      out[k] = v;
    }
    return out;
  }

  /* ---------------- 一级滑杆 + 三色轮 ---------------- */

  const controls = {};
  for (const [key, min, max, step, label] of PRIMARY) {
    const f = sliderField({
      min, max, step, testid: `grade-${key}`,
      onChange: (v) => {
        const n = Number(v);
        draft[key] = n === 0 ? 0 : n; // 0 参与写回(与「未调」区分靠整对象语义,0 值合法)
        commit();
      },
    });
    controls[key] = f;
    primaryBox.appendChild(h("label", { class: "v2-field" }, [label, f.root]));
  }

  const wheels = {};
  for (const [key, label, neutral, strength, lo, hi] of WHEELS) {
    wheels[key] = createColorWheel({
      label, testid: `grade-wheel-${key}`, neutral, strength, lo, hi,
      get: () => Array.isArray(draft[key]) ? draft[key] : null,
      set: (rgb) => { draft[key] = rgb.map(round4); },
      reset: () => { delete draft[key]; },
      onCommit: () => commit(),
      onCancel: () => {
        const row = rowNow(rowOf);
        if (row) { draft = row.grade ? JSON.parse(JSON.stringify(row.grade)) : {}; renderAll(); }
      },
    });
    wheelBox.appendChild(wheels[key].root);
  }

  /* ---------------- 二级:曲线(点集模式) ---------------- */

  const chanSel = /** @type {HTMLSelectElement} */ (h("select", { testid: "grade-curve-channel", "aria-label": "曲线通道" }));
  for (const [v, label] of CURVE_CHANNELS) chanSel.appendChild(h("option", { value: v }, [label]));
  chanSel.addEventListener("change", () => { channel = chanSel.value; renderCurve(); });

  const curve = createCurveCanvas(curveHost, {
    mode: "points", width: 300, height: 150,
    testid: "grade-curve-canvas", ariaLabel: "调色曲线画布(0..1 归一域)",
    frameOf: () => ({
      dur: 1, lo: 0, hi: 1,
      anchors: curvePoints().map(([x, y]) => ({ t: x, v: y })),
      samples: [], sel: -1,
    }),
    onAnchorDrag: (i, x, y) => {
      dragging = true;
      const pts = curvePoints();
      const nx = clampBetween(x, i, pts);
      pts[i] = [round4(nx), round4(Math.min(1, Math.max(0, y)))];
      pts.sort((a, b) => a[0] - b[0]);
      setCurvePoints(pts);
    },
    onAnchorDel: (i) => {
      const pts = curvePoints();
      pts.splice(i, 1);
      setCurvePoints(pts);
      renderCurve();
      if (!pts.length) commit(`已清除曲线(${channel})`);
      else curveCommitIfReady(`已删除曲线点(${channel})`);
    },
    onCanvasDbl: (x, y) => {
      const pts = curvePoints();
      pts.push([round4(x), round4(y)]);
      pts.sort((a, b) => a[0] - b[0]);
      setCurvePoints(pts);
      renderCurve();
      curveCommitIfReady(`已加曲线点(${channel} ${round4(x)}/${round4(y)})`);
    },
    onCommit: () => {
      dragging = false;
      curveCommitIfReady("曲线已更新(拖拽落点,可撤销)");
    },
    onCancel: () => {
      dragging = false;
      const row = rowNow(rowOf);
      if (row) { draft = row.grade ? JSON.parse(JSON.stringify(row.grade)) : {}; renderAll(); }
    },
  });

  function curvePoints() {
    const c = draft.curves && typeof draft.curves === "object" ? draft.curves : {};
    return Array.isArray(c[channel]) ? c[channel] : [];
  }

  function setCurvePoints(pts) {
    draft.curves = draft.curves && typeof draft.curves === "object" ? draft.curves : {};
    if (pts.length) draft.curves[channel] = pts;
    else delete draft.curves[channel];
    if (!Object.keys(draft.curves).length) delete draft.curves;
  }

  function clampBetween(x, i, pts) {
    const lo = i > 0 ? pts[i - 1][0] + 0.005 : 0;
    const hi = i < pts.length - 1 ? pts[i + 1][0] - 0.005 : 1;
    return Math.min(hi, Math.max(lo, x));
  }

  const curveHint = h("span", { class: "dim", testid: "grade-curve-hint" });
  const curveClear = h("button", {
    class: "mini", type: "button", testid: "grade-curve-clear",
    title: "清除本通道曲线点集",
    onclick: () => {
      setCurvePoints([]);
      renderCurve();
      commit(`已清除曲线(${channel})`);
    },
  }, ["清除本通道"]);
  const curveBar = h("div", { class: "grade-curve-bar" }, [chanSel, curveClear, curveHint]);
  curvePane.insertBefore(curveBar, curveHost);
  curvePane.appendChild(h("div", { class: "hint" }, [
    "点集 0..1 归一域(渲染编译为 ffmpeg curves;master≈亮度):双击加点 / 拖点改 / 双击点删;",
    "后端契约:每通道至少 2 点(minItems 2)才整对象提交。",
  ]));

  /** 曲线提交闸:每通道 ≥2 点才写(内核 minItems 2 实测);不足时挂草稿并如实提示。 */
  const ONE_POINT_HINT = "已挂 1 点(草稿):再补 1 点后提交(后端至少 2 点)";
  function curveCommitIfReady(msg) {
    const pts = curvePoints();
    curveHint.textContent = pts.length === 1 ? ONE_POINT_HINT : "";
    if (pts.length >= 2) commit(msg || "曲线已更新(可撤销)");
  }

  function renderCurve() {
    curve.draw();
    curveHint.textContent = curvePoints().length === 1 ? ONE_POINT_HINT : "";
  }

  /* ---------------- 二级:LUT ---------------- */

  const lutInput = /** @type {HTMLInputElement} */ (h("input", {
    type: "text", testid: "grade-lut-src", placeholder: ".cube 工程内相对路径",
    "aria-label": "LUT 源路径(.cube,工程内相对路径)", spellcheck: "false",
  }));
  const lutSel = /** @type {HTMLSelectElement} */ (h("select", { testid: "grade-lut-select", "aria-label": "LUT 库(会话导入记录 ∪ 工程引用)" }));
  lutSel.addEventListener("change", () => {
    if (!lutSel.value) return;
    draft.lut = lutSel.value;
    renderLut();
    commit(`LUT 已选择:${lutSel.value}`);
  });
  const lutImportBtn = h("button", {
    class: "mini", type: "button", testid: "grade-lut-import",
    title: "lut_import:校验 .cube 并拷入 .cutforge/luts/(返回路径写 grade.lut)",
    onclick: async () => {
      const src = lutInput.value.trim();
      if (!src) { toast("先填 .cube 工程内相对路径(先把文件放进工程目录)", false); return; }
      lutImportBtn.disabled = true;
      const env = await call("lut_import", { src });
      lutImportBtn.disabled = false;
      if (!env.ok) { toast(`LUT 导入失败:${messageOf(env, "lut_import")}`, false); return; }
      const rel = env.data && env.data.lut;
      noteLutLib(rel);
      draft.lut = rel;
      lutInput.value = "";
      renderLut();
      commit(`LUT 已导入并应用:${rel}(${env.data ? env.data.size3d : ""}³)`);
    },
  }, ["导入 LUT"]);
  const lutClearBtn = h("button", {
    class: "mini", type: "button", testid: "grade-lut-clear",
    title: "清除 LUT 引用(grade.lut 移除;库文件保留)",
    onclick: () => {
      delete draft.lut;
      renderLut();
      commit("已清除 LUT(可撤销)");
    },
  }, ["清除 LUT"]);
  lutBox.appendChild(h("div", { class: "grade-lut-row" }, [lutInput, lutImportBtn]));
  lutBox.appendChild(h("div", { class: "grade-lut-row" }, [lutSel, lutClearBtn]));
  lutBox.appendChild(h("div", { class: "hint" }, [
    "LUT 库 = 本会话导入记录 ∪ 工程内 grade.lut 引用(media_browse 不列 .cube,诚实口径);",
    "导入需先把 .cube 放进工程目录再填相对路径。",
  ]));

/** 会话 LUT 库(ephemeral;导入记录 ∪ 工程全片段 grade.lut 引用)。 */
function lutLib(row) {
  const set = new Set(ephemeralStore.get().lutLib || []);
  for (const c of timelineStore.get().clips || []) {
    if (c && c.grade && c.grade.lut) set.add(c.grade.lut);
  }
  if (row && row.grade && row.grade.lut) set.add(row.grade.lut);
  return [...set];
}
  function noteLutLib(rel) {
    const cur = ephemeralStore.get().lutLib || [];
    if (rel && !cur.includes(rel)) ephemeralStore.set({ lutLib: [...cur, rel] });
  }

  function renderLut() {
    const row = rowNow(rowOf);
    const lib = lutLib(row);
    clear(lutSel);
    lutSel.appendChild(h("option", { value: "" }, [lib.length ? "(选择 LUT)" : "(库为空:先导入)"]));
    for (const p of lib) lutSel.appendChild(h("option", { value: p }, [p]));
    lutSel.value = draft.lut || "";
  }

  /* ---------------- 渲染与刷新 ---------------- */

  function renderAll() {
    for (const [key] of PRIMARY) {
      const v = draft[key];
      controls[key].set(v === undefined || v === null ? "" : String(v));
    }
    for (const w of Object.values(wheels)) w.draw();
    renderCurve();
    renderLut();
    refreshButtons();
  }

  host.__cfRefresh = (row) => {
    const id = row ? row.id : null;
    if (id !== loadedId) load(row);
  };
  timelineStore.subscribe(() => {
    if (dragging) return;
    const row = rowNow(rowOf);
    // 同片段:提交后投影收敛回读(草稿重置为服务端状态)
    if (row && loadedId === row.id) {
      draft = row.grade && typeof row.grade === "object" ? JSON.parse(JSON.stringify(row.grade)) : {};
    }
    renderAll();
  });
  load(rowNow(rowOf));
  return host;
}
