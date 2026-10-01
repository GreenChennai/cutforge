/* 导出面板(T2.5,E5 对拍 + 新增):双后端(cutforge 异步 / ffmpeg 经 CutFlow)
 * + 剪映草稿(export_jianying)+ 产物清单(render_probe)。
 * 进度面:轮询为主(确定性),render.progress SSE 事件作为提速信号触发即查。
 * T3.5:进度回调 Hz 记录(perf.recordExportTick;预算「导出进度 ≥2Hz」面板可视)。
 * A5-FE2(T5.6/T5.5)增:编码设置组(encoder/quality/encode_probe 探测/高级折叠
 * crf/bitrate/gop/pixFmt/loudnormTarget/verboseCmd,render_run 显式选项才重编码,
 * 缺省 remux+bt709 零代损)+ OTIO/EDL 导出 + OTIO 导入(新工程)。 */
import { h, clear } from "../ui/dom.js";
import { projectStore, ephemeralStore } from "../core/store.js";
import { runExportCutforge, runExportFfmpeg, exportJianying, refreshRenderOutputs } from "../core/render-commands.js";
import { otioExport, otioImport, probeEncoders } from "../core/pro-commands.js";
import { selectField, numberField, textField, toggleField, collapseGroup } from "../ui/controls.js";
import { openDialog } from "../ui/dialog.js";
import { toast } from "../ui/toast.js";
import { subscribe } from "../core/event-bus.js";
import { recordExportTick } from "../ui/perf.js";
import { markHistory } from "../core/edit-commands.js";
import { pref, setPref } from "../ui/prefs.js";
import { mountExportMatrix, matrixArgs, runPreflight } from "./export-matrix.js";

let backendSel = null;
let ratioSel = null;
let proxyToggle = null;
let progressEl = null;
let progressBar = null;
let progressText = null;
let filesEl = null;
let encSel = null;
let qualitySel = null;
let probeOut = null;
let advFields = null;
let verboseT = null;
let otioSel = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["导出(编辑器内出片,E5)"]));
  backendSel = selectField({
    id: "exp-backend", testid: "export-backend",
    options: [["cutforge", "cutforge 内核(不依赖 CutFlow)"], ["ffmpeg", "ffmpeg(CutFlow rs_render)"]],
    onChange: () => syncProxyEnabled(),
  });
  container.appendChild(h("label", null, ["后端 ", backendSel.root]));
  ratioSel = selectField({
    id: "exp-ratio", testid: "export-ratio",
    options: [["9x16", "9x16"], ["3x4", "3x4"], ["16x9", "16x9"]],
  });
  container.appendChild(h("label", null, ["画幅(仅 ffmpeg 后端) ", ratioSel.root]));
  // T4.1 代理预览:明示「预览走代理 / 导出默认原片」;显式 opt-in 不悄悄降质
  proxyToggle = h("input", {
    type: "checkbox", id: "exp-proxy", testid: "export-proxy",
    title: "用代理渲染(cutforge 内核;缺失代理的片段回落原片)。缺省不勾 = 原片导出",
    "data-tip": "用代理预览(1/2 分辨率,快);不勾 = 原片导出(默认)",
  });
  proxyToggle.addEventListener("change", syncProxyEnabled);
  container.appendChild(h("label", { class: "toggle", title: "用代理渲染(cutforge 内核)" }, [
    proxyToggle, " 用代理预览(缺省原片导出)",
  ]));
  buildEncodeGroup(container); // T5.6 编码设置(缺省 remux 零代损;显式选项才重编码)
  mountExportMatrix(container); // 册六 T6.3 导出矩阵(格式/档位/区域/仅视频/批量/预检)
  buildOtioRow(container);     // T5.5 OTIO/EDL 导出 + OTIO 导入(新工程)
  container.appendChild(h("div", { class: "exp-actions" }, [
    h("button", { id: "exp-run", testid: "export-run", title: "按当前后端导出成片(Ctrl+S 回到这里)", onclick: () => runExport() }, ["导出成片"]),
    h("button", { id: "exp-jy", testid: "export-jianying", title: "生成剪映草稿(export_jianying;不落盘成片)", "data-tip": "生成剪映草稿(export_jianying;不落盘成片)", onclick: () => runJianying() }, ["导出剪映草稿"]),
  ]));
  progressEl = h("div", { id: "exp-progress", class: "exp-progress", testid: "export-progress" }, [
    progressBar = h("span", { class: "exp-bar idle", "aria-hidden": "true" }, [
      h("span", { class: "exp-bar-fill" }),
    ]),
    progressText = h("span", { class: "exp-text" }),
  ]);
  container.appendChild(progressEl);
  container.appendChild(h("h3", null, ["产物(06_成片输出)"]));
  filesEl = h("div", { id: "exp-files", testid: "export-files" });
  container.appendChild(filesEl);

  // render.progress SSE:渲染状态变化 → 立即拉产物清单(提速信号;轮询仍是主路径)
  subscribe("render.progress", (data) => {
    if (data && (data.state === "ok" || data.state === "fail")) refreshFiles();
  });
  refreshFiles();
}

/**
 * 进度呈现(T3.2 平滑不跳变):文本走 .exp-text;状态条走 .exp-bar 状态类
 * (running=不确定进度循环/ok=满格/err=红满格;内核暂无百分比下发,诚实口径)。
 * 文本节点整体替换会扫掉进度条,故 #exp-progress 固定含 bar+text 两子节点(e2e 文本兼容)。
 */
function setProgress(state, text) {
  progressText.textContent = text;
  const mode = state === "done" ? "ok"
    : state === "fail" ? "err"
      : (state === "running" || state === "submit") ? "active" : "idle";
  progressBar.className = `exp-bar ${mode}`;
}

/** render-commands 的 onProgress(state, text) → 面板只展示人话文本 + Hz 记录(T3.5)。 */
function onProgress(state, text) {
  recordExportTick(state);
  setProgress(state, text);
}

/** ffmpeg 后端(CutFlow rs_render)不消费 useProxy:切换后端时同步开关可用态。 */
function syncProxyEnabled() {
  const isCutforge = backendSel.get() === "cutforge";
  proxyToggle.disabled = !isCutforge;
  if (!isCutforge) proxyToggle.checked = false;
  proxyToggle.title = isCutforge
    ? "用代理渲染(缺失代理的片段回落原片)。缺省不勾 = 原片导出"
    : "ffmpeg 后端不支持代理(useProxy 仅 cutforge 内核)";
}

async function runExport() {
  // 导出前自动打历史快照标记(T4.3;会话态,不落盘)
  markHistory("导出前");
  if (backendSel.get() === "cutforge") {
    // 册六 T6.3:导出前自动跑 export_preflight 检查(有问题项给「仍要导出」显式越过)
    const go = await runPreflight(true);
    if (!go) return;
    runExportCutforge(onProgress, proxyToggle.checked, encOpts(), matrixArgs());
  } else {
    // 编码设置/导出矩阵仅 cutforge 内核消费;填了选项切到 ffmpeg 时如实提示被忽略
    if (Object.keys(encOpts()).length || Object.keys(matrixArgs()).length) {
      toast("编码设置/导出矩阵仅 cutforge 内核消费(ffmpeg 后端忽略)", false);
    }
    runExportFfmpeg(ratioSel.get(), onProgress);
  }
}

/* ---------------- T5.6 编码设置组 ---------------- */

/** 编码设置面(缺省全空 = render_run 现行为:remux+bt709 标签,零重编码零代损;
 * 任一显式编码选项给值才重编码——后端语义,本组只透传)。 */
function buildEncodeGroup(container) {
  encSel = selectField({
    id: "exp-encoder", testid: "export-encoder",
    options: [
      ["", "缺省(remux 直写,零代损)"],
      ["auto", "auto(libx264 确定性基线)"],
      ["hw", "硬件优先(试编失败优雅降级 libx264)"],
      ["sw", "强制软件(libx264)"],
    ],
  });
  qualitySel = selectField({
    id: "exp-quality", testid: "export-quality",
    options: [
      ["", "(质量预设缺省)"],
      ["fast", "fast(草稿:crf28/veryfast)"],
      ["balanced", "balanced(均衡:crf23/medium)"],
      ["quality", "quality(高质:crf18/slow)"],
    ],
  });
  const crf = numberField({ id: "exp-crf", testid: "export-crf", step: 1, min: 0, max: 51 });
  const bitrate = numberField({ id: "exp-bitrate", testid: "export-bitrate", step: 100, min: 100 });
  const gop = numberField({ id: "exp-gop", testid: "export-gop", step: 1, min: 1 });
  const pixFmt = textField({ id: "exp-pixfmt", testid: "export-pixfmt", ariaLabel: "像素格式", placeholder: "缺省 yuv420p" });
  const ln = textField({
    id: "exp-loudnorm", testid: "export-loudnorm", ariaLabel: "响度目标 I[:TP]",
    placeholder: "-14:-1.0(混音台同源双入口)",
    onInput: (v) => setPref("loudnormTarget", String(v).trim()),
  });
  ln.set(pref("loudnormTarget", ""));
  verboseT = toggleField({ testid: "export-verbose", label: "进度附命令原文(含素材路径,安全口径缺省关)" });
  advFields = { crf, bitrate, gop, pixFmt, ln };
  probeOut = h("span", { class: "hint", testid: "export-probe-out" }, ["(未探测)"]);
  const advBox = collapseGroup("高级(crf/bitrate/gop/pixFmt)", [
    h("label", { class: "v2-field" }, ["crf(0..51,覆盖预设) ", crf.root]),
    h("label", { class: "v2-field" }, ["码率 kbps(给值时 crf 让位) ", bitrate.root]),
    h("label", { class: "v2-field" }, ["GOP(关键帧间隔,帧) ", gop.root]),
    h("label", { class: "v2-field" }, ["像素格式 ", pixFmt.root]),
  ], { open: false });
  advBox.classList.add("collapsed");
  const box = h("fieldset", { class: "insp-group", testid: "export-encode" }, [
    h("legend", { "data-tip": "编码设置(T5.6):缺省 remux 零代损;显式选项才重编码" }, ["编码设置(仅 cutforge 内核)"]),
    h("div", { class: "insp-fields-wrap" }, [
      h("label", { class: "v2-field" }, ["编码器 ", encSel.root]),
      h("label", { class: "v2-field" }, ["质量预设 ", qualitySel.root]),
      h("div", { class: "exp-actions" }, [
        h("button", {
          class: "mini", testid: "export-probe", title: "encode_probe:-encoders 清单 + 试编可用性(毫秒级)",
          onclick: () => runProbe(),
        }, ["探测本机编码器"]),
        probeOut,
      ]),
      advBox,
      h("label", { class: "v2-field" }, ["响度目标 I[:TP](导出/混音台双入口) ", ln.root]),
      verboseT.root,
    ]),
  ]);
  container.appendChild(box);
}

async function runProbe() {
  probeOut.textContent = "探测中…(-encoders + 试编)";
  const env = await probeEncoders();
  if (env.ok && env.data) {
    const enc = ephemeralStore.get().encodeProbe?.encoders || {};
    const usable = Object.entries(enc).filter(([, v]) => v && v.usable).map(([k]) => `h264_${k}`);
    const listed = Object.entries(enc).filter(([, v]) => v && v.listed && !v.usable).map(([k]) => `h264_${k}`);
    probeOut.textContent = usable.length
      ? `可用硬件:${usable.join(", ")}(选「硬件优先」即用)`
      : `无可用硬件${listed.length ? `(在位但试编不过:${listed.join(",")})` : ""};auto/缺省=libx264`;
  } else {
    probeOut.textContent = "(探测失败,见 toast)";
  }
}

/** 收集编码选项(只带已提供的键;全部缺省 = remux 现行为零代损;
 * 不按后端过滤——ffmpeg 后端由调用方拿键数判断「填了但被忽略」)。 */
function encOpts() {
  const num = (f) => { const v = Number(f.get()); return f.get() !== "" && !Number.isNaN(v) ? v : undefined; };
  return {
    encoder: encSel.get() || undefined,
    quality: qualitySel.get() || undefined,
    crf: num(advFields.crf),
    bitrate: num(advFields.bitrate),
    gop: num(advFields.gop),
    pixFmt: String(advFields.pixFmt.get() || "").trim() || undefined,
    loudnormTarget: String(advFields.ln.get() || "").trim() || undefined,
    verboseCmd: verboseT.get() ? true : undefined,
  };
}

/* ---------------- T5.5 OTIO/EDL 互操作 ---------------- */

function buildOtioRow(container) {
  otioSel = selectField({
    id: "exp-otio-format", testid: "export-otio-format",
    options: [["otio", "OTIO JSON(往返语义等价)"], ["edl", "EDL CMX3600(视频轨)"]],
  });
  container.appendChild(h("div", { class: "exp-actions", testid: "export-otio" }, [
    h("label", null, ["格式 ", otioSel.root]),
    h("button", {
      class: "mini", testid: "export-otio-run", title: "otio_export:工程 → 交换文件(派生物落 06_成片输出,不产 Op)",
      onclick: () => otioExport(otioSel.get()),
    }, ["导出 OTIO/EDL"]),
    h("button", {
      class: "mini", testid: "export-otio-import", title: "otio_import:从 OTIO 新建工程(目标目录必须不存在)",
      onclick: () => openOtioImport(),
    }, ["导入 OTIO(新工程)"]),
    h("a", {
      class: "hint", href: "https://github.com/GreenChennai/cutforge/blob/main/docs/PROJECT-FORMAT.md",
      target: "_blank", rel: "noopener", title: "公开工程格式(OTIO 子集范围)",
    }, ["格式文档"]),
  ]));
}

/** 导入对话框:src 预填最近导出产物(工程内相对 → 绝对拼接;浏览器无 fs,路径手输诚实标注)。 */
function openOtioImport() {
  openDialog({
    id: "otio-import-dialog",
    title: "导入 OTIO(otio_import;从零新建工程,不动已开工作区)",
    build: (body) => {
      const last = ephemeralStore.get().otioLast;
      const root = projectStore.get().root || "";
      const sep = root.includes("\\") ? "\\" : "/";
      const prefill = last && last.file ? `${root.replace(/[\\/]+$/, "")}${sep}${last.file}` : "";
      const src = textField({ testid: "otio-src", ariaLabel: "OTIO 文件路径", placeholder: "OTIO JSON 路径(服务端可读;导出产物预填)" });
      src.set(prefill);
      const name = h("input", { type: "text", testid: "otio-name", value: `${projectStore.get().project?.slug || "cutforge"}-imported` });
      const out = h("div", { class: "exp-progress", testid: "otio-progress" });
      body.appendChild(h("label", null, ["OTIO 文件路径 ", src.root]));
      body.appendChild(h("label", null, ["新工程目录名(在当前工程旁创建) ", name]));
      body.appendChild(h("div", { class: "hint" }, [
        "目标目录必须不存在(拒绝覆盖);导入后需单独启动服务打开新工程。子集外元素 WARN 留痕不静默丢。",
      ]));
      body.appendChild(out);
      const run = h("button", {
        testid: "otio-run",
        onclick: async () => {
          const target = siblingRoot(root, String(name.value || "").trim());
          const srcV = String(src.get() || "").trim();
          if (!target || !srcV) { toast("工程目录名与 OTIO 路径都必填", false); return; }
          run.disabled = true;
          const env = await otioImport(target, srcV);
          out.textContent = env.ok ? `完成:${target}` : `(失败:${env.code})`;
          run.disabled = false;
        },
      }, ["导入"]);
      const close = h("button", { testid: "otio-cancel" }, ["关闭"]);
      close.addEventListener("click", () => document.getElementById("otio-import-dialog")?.remove());
      body.appendChild(h("div", { class: "wizard-actions" }, [run, close]));
    },
  });
}

/** 当前工程根的同名父目录下拼新工程目录(与 commands.createProject 同口径,壳只拼接)。 */
function siblingRoot(root, name) {
  if (!root || !name || /[\\/:*?"<>|]/.test(name)) return "";
  const sep = root.includes("\\") ? "\\" : "/";
  const parts = root.replace(/[\\/]+$/, "").split(/[\\/]/);
  parts.pop();
  return parts.join(sep) + sep + name;
}

function runJianying() {
  const slug = projectStore.get().project?.slug || "cutforge";
  openDialog({
    id: "jy-dialog",
    title: "导出剪映草稿(export_jianying)",
    build: (body) => {
      const nameField = h("input", { type: "text", testid: "jy-name", value: `${slug}-draft` });
      const out = h("div", { class: "exp-progress", testid: "jy-progress" });
      const run = h("button", {
        testid: "jy-run",
        onclick: async () => {
          const name = /** @type {HTMLInputElement} */ (nameField).value.trim();
          if (!name) {
            toast("草稿名必填", false);
            return;
          }
          run.disabled = true;
          await exportJianying(name, (state, text) => {
            out.textContent = text;
            if (state === "done" || state === "fail") run.disabled = false;
          });
        },
      }, ["生成"]);
      const close = h("button", { testid: "jy-cancel" }, ["关闭"]);
      close.addEventListener("click", () => document.getElementById("jy-dialog")?.remove());
      body.appendChild(h("label", null, ["草稿名 ", nameField]));
      body.appendChild(out);
      body.appendChild(h("div", { class: "wizard-actions" }, [run, close]));
    },
  });
}

async function refreshFiles() {
  const files = await refreshRenderOutputs();
  clear(filesEl);
  if (!files.length) {
    filesEl.appendChild(h("div", { class: "dim" }, ["(暂无产物)"]));
    return;
  }
  for (const f of files) {
    filesEl.appendChild(h("div", { class: "row", testid: "export-file" }, [
      h("span", { class: "oid" }, [String(f.file)]),
      h("span", { class: "bd" }, [`${f.bytes} B`]),
    ]));
  }
}
