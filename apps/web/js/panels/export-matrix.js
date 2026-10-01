/* 导出矩阵面板组(册六 T6.3 壳侧,F3):格式七出口/画幅预设/清晰度档/码率档/
 * 区域导出(入出点回填)/仅视频/批量多画幅/export_preflight 检查清单门。
 *
 * 只对 cutforge 内核生效(ffmpeg 后端 = CutFlow rs_render 编排,不消费本组参数,
 * 切后端时如实提示被忽略——与编码设置组同口径)。检查清单为启发式预估面
 * (heuristic:true 服务端如实标注):有问题项不静默拦截,给「仍要导出」显式越过。
 */
import { h } from "../ui/dom.js";
import { selectionStore, uiStore } from "../core/store.js";
import { selectField, numberField, toggleField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";
import { exportPreflight, exportAllVariants } from "../core/library-commands.js";
import { timelineEndMs } from "../core/commands.js";

/** 七出口(frame-png 走精确预览/单帧,不入整片导出面,如实不列)。 */
const FORMATS = [
  ["", "mp4-h264(缺省;与既有编码面同出口)"],
  ["mp4-h265", "mp4-h265(HEVC hvc1;软件编码)"],
  ["mov", "mov(QuickTime 容器)"],
  ["gif", "gif(12fps 固定;palettegen 两段)"],
  ["m4a", "m4a(仅音频;不渲视频链)"],
  ["mp3", "mp3(192k;仅音频)"],
  ["png-seq", "png-seq(序列帧 %04d)"],
];
const FORMAT_HINT = {
  "": "缺省:mp4-h264(remux 直写,零代损;显式编码选项才重编码)",
  "mp4-h265": "libx265 + hvc1 标签(软件编码;硬件请求只 WARN 不冒充)",
  mov: "容器随扩展名(MOV);编码面同 mp4",
  gif: "12fps 固定(如实声明);调色板两段生成,产物为动图(无音轨)",
  m4a: "纯音频短路:probe→mix→copy,不渲视频链;时长 = 工程窗内音频",
  mp3: "libmp3lame 192k;仅音频",
  "png-seq": "image2 %04d 序列帧;帧数清点入进度(无音轨)",
};

let fmtSel = null;
let presetSel = null;
let tierSel = null;
let brTierSel = null;
let inF = null;
let outF = null;
let voT = null;
let pfBox = null;
let pfList = null;
let batchOut = null;

export function mountExportMatrix(container) {
  fmtSel = selectField({
    id: "exp-format", testid: "export-format", options: FORMATS,
    onChange: () => syncFormatHint(),
  });
  presetSel = selectField({
    id: "exp-preset", testid: "export-preset",
    options: [
      ["", "缺省(工程画幅)"], ["vertical", "vertical(9:16)"], ["horizontal", "horizontal(16:9)"],
      ["square", "square(1:1)"], ["9x16", "9x16"], ["16x9", "16x9"], ["1x1", "1x1"],
      ["3x4", "3x4"], ["4x5", "4x5"],
    ],
  });
  tierSel = selectField({
    id: "exp-tier", testid: "export-quality-tier",
    options: [["", "缺省(不缩放)"], ["1080p", "1080p(短边 1080)"], ["720p", "720p(短边 720)"], ["480p", "480p(短边 480)"]],
  });
  brTierSel = selectField({
    id: "exp-brtier", testid: "export-bitrate-tier",
    options: [["", "缺省(不用码率档)"], ["high", "high(≈12Mbps@1080p)"], ["medium", "medium(≈8Mbps@1080p)"], ["low", "low(≈4Mbps@1080p)"]],
  });
  inF = numberField({ id: "exp-in", testid: "export-in-ms", step: 100, min: 0 });
  outF = numberField({ id: "exp-out", testid: "export-out-ms", step: 100, min: 0 });
  voT = toggleField({
    testid: "export-video-only", label: "仅视频(音轨静音 + BGM 摘除)",
  });
  pfBox = h("fieldset", { class: "insp-group exp-pf", testid: "export-preflight", hidden: true }, [
    h("legend", { title: "导出前检查(启发式预估,heuristic 如实标注)" }, ["导出前检查"]),
    h("div", { class: "insp-fields-wrap", testid: "export-pf-list" }, []),
  ]);
  pfList = pfBox.querySelector('[data-testid="export-pf-list"]');
  batchOut = h("span", { class: "hint", testid: "export-batch-out" }, []);
  const box = h("fieldset", { class: "insp-group", testid: "export-matrix" }, [
    h("legend", { "data-tip": "导出矩阵(册六 T6.3;仅 cutforge 内核消费)" }, ["导出矩阵(仅 cutforge 内核)"]),
    h("div", { class: "insp-fields-wrap" }, [
      h("label", { class: "v2-field" }, ["格式 ", fmtSel.root]),
      h("div", { class: "hint", testid: "export-format-hint" }, [FORMAT_HINT[""]]),
      h("label", { class: "v2-field" }, ["画幅预设 ", presetSel.root]),
      h("label", { class: "v2-field" }, ["清晰度档 ", tierSel.root]),
      h("label", { class: "v2-field" }, ["码率档(估算,显式码率覆盖) ", brTierSel.root]),
      h("div", { class: "exp-actions" }, [
        h("label", null, ["入点 ms ", inF.root]),
        h("label", null, ["出点 ms ", outF.root]),
        h("button", {
          class: "mini", testid: "export-range-fill", title: "用时间线入出点(I/O 键设的会话入出点)回填;留空 = 到片尾",
          onclick: () => fillRange(),
        }, ["回填入出点"]),
      ]),
      voT.root,
      h("div", { class: "exp-actions" }, [
        h("button", {
          class: "mini", testid: "export-preflight-run", title: "export_preflight:缺失素材/黑帧风险/静音段/时长/响度(轻探测,不渲全片)",
          onclick: () => runPreflight(null),
        }, ["跑导出前检查"]),
        h("button", {
          class: "mini", testid: "export-batch", title: "export_all_variants:按画幅变体逐个入渲染队列(进度到「渲染队列」页看)",
          onclick: () => runBatch(),
        }, ["多画幅批量"]),
      ]),
      batchOut,
      pfBox,
    ]),
  ]);
  container.appendChild(box);
}

function syncFormatHint() {
  const hint = document.querySelector('[data-testid="export-format-hint"]');
  if (hint) hint.textContent = FORMAT_HINT[fmtSel.get()] || "";
}

/** 区域导出回填:selectionStore 入出点(I/O 键;缺省出点 = 时间线末)。 */
function fillRange() {
  const sel = selectionStore.get();
  if (sel.inMs === null && sel.outMs === null) {
    toast("还没有入出点:播放头处按 I / O 设置(会话级)", false);
    return;
  }
  inF.set(sel.inMs ?? 0);
  outF.set(sel.outMs ?? timelineEndMs());
  toast(`已回填区域:${Math.round(inF.get() || 0)}–${Math.round(outF.get() || 0)}ms(可手改)`);
}

/** 当前矩阵参数(只带已提供的键;全部缺省 = 既有渲染路径逐字不变)。 */
export function matrixArgs() {
  const num = (f) => {
    const v = Number(f.get());
    return f.get() !== "" && !Number.isNaN(v) && v >= 0 ? Math.round(v) : undefined;
  };
  const args = {
    format: fmtSel.get() || undefined,
    preset: presetSel.get() || undefined,
    qualityTier: tierSel.get() || undefined,
    bitrateTier: brTierSel.get() || undefined,
    inMs: num(inF),
    outMs: num(outF),
    videoOnly: voT.get() ? true : undefined,
  };
  for (const k of Object.keys(args)) if (args[k] === undefined) delete args[k];
  return args;
}

/* ---------------- export_preflight 检查清单 ---------------- */

/** 手动跑检查(展示面);auto = 导出前的门(返回 Promise<boolean> 是否放行)。 */
export async function runPreflight(autoResolve) {
  const args = matrixArgs();
  pfBox.hidden = false;
  pfList.textContent = "检查中(轻探测:缺失素材/黑帧抽样/静音段…)…";
  const env = await exportPreflight({ inMs: args.inMs, outMs: args.outMs });
  while (pfList.firstChild) pfList.removeChild(pfList.firstChild);
  if (!env.ok) {
    pfList.appendChild(h("div", { class: "hint" }, [
      `检查不可用:${env.message || env.code}(不阻塞导出;缺失素材面以渲染报错为准)`,
    ]));
    return true;
  }
  const pf = env.data;
  const issues = [];
  renderPfRow("时长", `${pf.totalMs}ms · ${pf.clips} 视频段 · ${pf.audioEvents} 音频事件`
    + (pf.window ? ` · 窗口 ${pf.window.inMs}–${pf.window.outMs ?? "片尾"}ms` : ""), false);
  if ((pf.missingAssets || []).length) {
    issues.push("missing");
    renderPfRow("缺失素材", `${pf.missingAssets.length} 个(渲染必炸):${pf.missingAssets.join("、")}`, true);
  } else {
    renderPfRow("缺失素材", "无", false);
  }
  const bf = pf.blackFrame || {};
  renderPfRow("黑帧风险(启发式)", bf.risk
    ? `有(首帧 luma ${bf.firstLuma} / 末帧 luma ${bf.lastLuma},<16 判风险)`
    : `无(首帧 luma ${bf.firstLuma} / 末帧 luma ${bf.lastLuma})`, Boolean(bf.risk));
  if (bf.risk) issues.push("black");
  const silent = pf.silentRanges || [];
  renderPfRow("静音段(启发式)", silent.length
    ? `${silent.length} 段:${silent.slice(0, 5).map((r) => `${r.startMs}–${r.endMs}ms`).join("、")}${silent.length > 5 ? "…" : ""}`
    : "无", silent.length > 0);
  if (silent.length) issues.push("silent");
  const loud = pf.loudness && pf.loudness.ok ? pf.loudness.data : null;
  renderPfRow("响度预估", loud && loud.inputI != null
    ? `${loud.inputI} LUFS${loud.deviation !== undefined ? `(偏差 ${loud.deviation} LU${loud.within1LU ? ",达标" : ""})` : ""}(成片实测复用)`
    : "尚无成片产物,缺位不虚标", false);
  for (const w of pf.warnings || []) renderPfRow("提示", String(w).replace(/;$/, ""), false);
  renderPfRow("口径", "启发式预估面(heuristic),质量不作硬验收", false);
  if (!autoResolve || !issues.length) return true;
  // 有问题项 + 导出前自动门:展示清单,给「仍要导出」显式越过
  return await new Promise((resolve) => {
    const actions = h("div", { class: "exp-actions" }, [
      h("button", {
        testid: "export-pf-anyway", title: "知道风险,仍要导出",
        onclick: () => { pfBox.hidden = true; resolve(true); },
      }, ["仍要导出"]),
      h("button", {
        testid: "export-pf-cancel", title: "回到编辑处理问题项",
        onclick: () => { pfBox.hidden = true; resolve(false); },
      }, ["先不导出"]),
    ]);
    pfList.appendChild(actions);
    actions.querySelector('[data-testid="export-pf-anyway"]').focus();
  });
}

function renderPfRow(label, text, warn) {
  pfList.appendChild(h("div", {
    class: `exp-pf-row${warn ? " warn" : ""}`, testid: "export-pf-item",
    "data-warn": warn ? "1" : "0",
  }, [h("span", { class: "exp-pf-k" }, [label]), h("span", null, [` ${text}`])]));
}

/* ---------------- 多画幅批量(export_all_variants) ---------------- */

async function runBatch() {
  const args = matrixArgs();
  if ((args.preset || "").trim()) {
    toast("批量导出按变体逐个覆写画幅预设;请把「画幅预设」留空(缺省)", false);
    return;
  }
  batchOut.textContent = "批量入队中…";
  const env = await exportAllVariants({ ...args });
  if (env.ok) {
    const n = (env.data.runs || []).length;
    batchOut.textContent = `已入队 ${n} 个变体(parentRunId ${String(env.data.parentRunId).slice(0, 10)}…;进度见「渲染队列」页)`;
    toast(`多画幅批量:已入队 ${n} 个变体任务`);
    uiStore.set({ tab: "queue" });
  } else {
    batchOut.textContent = `批量失败:${env.message || env.code}`;
  }
}
