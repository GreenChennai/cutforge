/* 混音台视图(T5.3 壳侧):每轨条(静音/独奏 + EQ/动态折叠组)+ 总线(响度目标 +
 * 成片响度单)+ 电平表(静态)。诚实口径(登记,不假实现):
 * - 轨道级音量/声像:TrackPatch 无 volume/pan 字段(IR 缺口)→ 推子禁用占位 +
 *   「候 BE」标注;片段级音量仍走检查器·音频组(volume);
 * - 实时电平表:播放链无采样口 → audio_loudness 按需测量,电平表以最近一次测量
 *   静态呈现 + 「实时候播放链采样口(登记)」标注;
 * - EQ/动态参数面按 mcp-tools inputSchema 渲染(track_update.eq/dyn,单 Op);
 * - 响度目标 loudnormTarget 与导出面板双入口:同写 prefs("loudnormTarget")。 */
import { h, clear } from "../ui/dom.js";
import { projectStore, timelineStore, ephemeralStore } from "../core/store.js";
import { updateTrack } from "../core/edit-commands.js";
import { measureLoudness } from "../core/pro-commands.js";
import { refreshRenderOutputs } from "../core/render-commands.js";
import { numberField, textField, collapseGroup } from "../ui/controls.js";
import { pref, setPref } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";

const EQ_TYPES = [
  ["peaking", "peaking(峰值)"],
  ["lowshelf", "lowshelf(低架)"],
  ["highshelf", "highshelf(高架)"],
];
const EQ_LIM = { freq: [20, 20000], gain: [-24, 24], q: [0.1, 16] };
const DYN_FIELDS = [
  ["thresholdDb", "阈值 dB(-60..0)"], ["ratio", "比例(1..20)"],
  ["attackMs", "启动 ms"], ["releaseMs", "释放 ms"], ["limitDb", "限幅 dB(-24..0)"],
];

/** @type {Map<string, Object>} trackId → strip 记录(控件引用;草稿自持,重建只在轨集变化) */
const strips = new Map();
let host = null;
let busOut = null;
let targetField = null;
let busSrc = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["混音台(T5.3;track_update.eq/dyn + audio_loudness)"]));
  container.appendChild(h("div", { class: "hint" }, [
    "诚实口径:轨道级音量/声像无 IR 支撑(候 BE,推子禁用占位;片段音量走检查器·音频组);"
    + "实时电平候播放链采样口(登记)——本页电平表 = 最近一次 audio_loudness 测量的静态呈现。",
  ]));
  buildBus(container);
  host = h("div", { id: "mixer-tracks", testid: "mixer-tracks" });
  container.appendChild(host);
  projectStore.subscribe((patch) => { if (patch.project !== undefined) renderTracks(); });
  ephemeralStore.subscribe((patch) => {
    if (patch.loudness !== undefined) renderBusLoudness(ephemeralStore.get().loudness);
  });
  renderTracks();
  renderBusLoudness(null);
  prefillBusSrc();
}

/* ---------------- 总线:响度目标(双入口)+ 成片响度单 ---------------- */

function buildBus(container) {
  const box = h("fieldset", { class: "insp-group", testid: "mix-bus" }, [
    h("legend", { "data-tip": "总线响度目标与成片响度单(audio_loudness;与导出面板 loudnormTarget 双入口)" },
      ["总线 · 响度(loudnorm)"]),
    h("div", { class: "insp-fields-wrap" }, [
      (targetField = textField({
        testid: "mix-target", ariaLabel: "响度目标 I[:TP]",
        placeholder: "-14:-1.0(格式 I[:TP])", onInput: (v) => setPref("loudnormTarget", v.trim()),
      })).root,
      (busSrc = textField({ testid: "mix-bus-src", ariaLabel: "成片路径", placeholder: "成片(工程内相对路径;可用「取最新成片」)" })).root,
      h("div", { class: "mix-actions" }, [
        h("button", {
          class: "mini", testid: "mix-bus-latest", title: "取 render_probe 最新产物路径(工程内相对)",
          onclick: () => prefillBusSrc(true),
        }, ["取最新成片"]),
        h("button", {
          class: "mini", testid: "mix-bus-measure",
          title: "audio_loudness:对成片测量 LUFS/TP/LRA(loudnorm 与导出双 pass 同源)",
          onclick: () => measureBus(),
        }, ["测量成片响度"]),
      ]),
      busOut = h("div", { id: "mix-bus-out", testid: "mix-bus-out", class: "mix-loud-out" }),
    ]),
  ]);
  targetField.set(pref("loudnormTarget", "-14:-1.0"));
  container.appendChild(box);
}

function targetLufs() {
  const m = /^(-?\d+(?:\.\d+)?)/.exec(String(targetField.get() || "").trim());
  return m ? Number(m[1]) : undefined;
}

async function measureBus() {
  const src = String(busSrc.get() || "").trim();
  if (!src) { toast("先填成片路径(或点「取最新成片」)", false); return; }
  await measureLoudness(src, targetLufs());
}

async function prefillBusSrc(force = false) {
  if (!force && String(busSrc.get() || "").trim()) return;
  const files = await refreshRenderOutputs();
  const hit = files.find((f) => String(f.file || "").includes("final")) || files[0];
  if (hit && hit.file) busSrc.set(hit.file);
}

/** 响度单渲染:LUFS/TP/LRA + 目标偏差 + 静态电平条(LUFS −60..0 → 0..100%)。 */
function renderBusLoudness(d) {
  if (!busOut) return;
  clear(busOut);
  busOut.appendChild(h("div", { class: "lvl-bar", testid: "mix-bus-meter", role: "img", "aria-label": "总线电平(静态)" }, [
    h("span", { class: "lvl-fill", style: `width:${lufsPct(d)}%` }),
  ]));
  if (!d) {
    busOut.appendChild(h("span", { class: "dim" }, ["(未测量)静态电平 · 实时候播放链采样口(登记)"]));
    return;
  }
  const dev = d.deviation;
  busOut.appendChild(h("div", { class: "mix-loud-nums", testid: "mix-bus-nums" }, [
    h("b", { class: "mono" }, [`${fmtNum(d.inputI)} LUFS`]),
    h("span", { class: "dim" }, [` TP ${fmtNum(d.inputTp)} · LRA ${fmtNum(d.inputLra)}`]),
    d.target !== undefined ? h("span", {
      class: `badge ${d.within1LU ? "ok" : "warn"}`, testid: "mix-bus-dev",
      title: "目标偏差(AC-5.3 判定面:≤1LU 达标)",
    }, [d.within1LU ? `达标 ≤1LU(偏差 ${dev})` : `偏差 ${dev} LU`]) : null,
  ]));
  busOut.appendChild(h("span", { class: "dim" }, [`源:${d.src} · 静态电平 · 实时候播放链采样口(登记)`]));
}

/* ---------------- 每轨条 ---------------- */

/** 轨道条重建(轨集变化时;切到本页签也会调用一次兜底)。 */
export function renderTracks() {
  if (!host) return;
  const tracks = (projectStore.get().project?.tracks || [])
    .filter((t) => t.kind === "video" || t.kind === "audio");
  const want = new Set(tracks.map((t) => t.id));
  for (const [id, st] of [...strips]) {
    if (!want.has(id)) { st.root.remove(); strips.delete(id); }
  }
  for (const t of tracks) {
    let st = strips.get(t.id);
    if (!st) { st = buildStrip(t); strips.set(t.id, st); host.appendChild(st.root); }
    syncStrip(st, t);
  }
}

function syncStrip(st, t) {
  st.mute.setAttribute("aria-pressed", t.mute ? "true" : "false");
  st.solo.setAttribute("aria-pressed", t.solo ? "true" : "false");
  st.name.textContent = t.name || t.id;
  const eqCount = Array.isArray(t.eq) ? t.eq.length : 0;
  st.eqLegend.textContent = `EQ${eqCount ? `(${eqCount} 段)` : ""}(≤8 段;整组替换)`;
  const dynKeys = t.dyn && typeof t.dyn === "object" ? Object.keys(t.dyn).length : 0;
  st.dynLegend.textContent = `动态 acompressor+alimiter${dynKeys ? `(已设 ${dynKeys} 项)` : ""}`;
}

function buildStrip(t) {
  const root = h("div", { class: "mix-strip", testid: `mix-strip-${t.id}` });
  const mute = h("button", {
    class: "mini", testid: `mix-mute-${t.id}`, "aria-pressed": "false",
    title: "静音(track_update.mute;只作用音频面)", onclick: () => toggleMs(t.id, "mute"),
  }, ["M"]);
  const solo = h("button", {
    class: "mini", testid: `mix-solo-${t.id}`, "aria-pressed": "false",
    title: "独奏:仅独奏轨出声(track_update.solo)", onclick: () => toggleMs(t.id, "solo"),
  }, ["S"]);
  const name = h("span", { class: "mix-name", testid: `mix-name-${t.id}` }, [t.name || t.id]);
  root.appendChild(h("div", { class: "mix-head" }, [
    h("span", { class: "badge" }, [t.id]), name,
    h("span", { class: "mix-ms" }, [mute, solo]),
  ]));
  // 轨道音量/声像:IR 缺口,禁用占位(诚实,候 BE)
  const fader = h("input", { type: "range", min: 0, max: 2, step: 0.05, disabled: true, class: "mix-fader", testid: `mix-fader-${t.id}`, "aria-label": `轨道 ${t.id} 音量(候 BE)` });
  root.appendChild(h("div", { class: "mix-row" }, [
    h("label", { class: "mix-lab" }, ["音量 ", fader]),
    h("span", { class: "hint" }, ["轨道级音量/声像候 BE(登记);片段音量走检查器·音频组"]),
  ]));

  const st = { root, mute, solo, name, eqLegend: null, dynLegend: null, eqRows: null, dynInputs: null };

  /* EQ 组(草稿自持:整组替换单 Op;空数组不用,清除走 null) */
  const eqRows = h("div", { class: "mix-eq-rows" });
  const eqBox = collapseGroup("EQ", [eqRows, eqActions(t.id, eqRows)], { open: false, testid: `mix-eq-${t.id}` });
  st.eqLegend = /** @type {HTMLElement} */ (eqBox.querySelector("legend"));
  st.eqRows = eqRows;
  root.appendChild(eqBox);

  /* 动态组(整对象替换;留空字段按后端缺省) */
  const dynInputs = DYN_FIELDS.map(([k, lab]) => {
    const f = numberField({ testid: `mix-dyn-${k}-${t.id}` });
    return [k, lab, f];
  });
  const dynBox = collapseGroup("动态", [
    ...dynInputs.map(([, lab, f]) => h("label", { class: "v2-field" }, [lab, f.root])),
    h("div", { class: "mix-actions" }, [
      h("button", { class: "mini", testid: `mix-dyn-apply-${t.id}`, title: "track_update patch.dyn(整对象替换,单 Op)", onclick: () => applyDyn(t.id, dynInputs) }, ["应用动态"]),
      h("button", { class: "mini", testid: `mix-dyn-clear-${t.id}`, title: "patch.dyn = null(清除,单 Op)", onclick: () => clearTrackPatch(t.id, "dyn") }, ["清除动态"]),
    ]),
    h("div", { class: "hint" }, ["整对象替换语义:留空字段按后端缺省(非保留现值)。"]),
  ], { open: false, testid: `mix-dyn-${t.id}` });
  st.dynLegend = /** @type {HTMLElement} */ (dynBox.querySelector("legend"));
  st.dynInputs = dynInputs;
  root.appendChild(dynBox);

  /* 源响度(按需测量;静态呈现) */
  const loudOut = h("span", { class: "mix-loud-out", testid: `mix-loud-out-${t.id}` }, ["(未测量)"]);
  root.appendChild(h("div", { class: "mix-row" }, [
    h("button", {
      class: "mini", testid: `mix-loud-${t.id}`,
      title: "audio_loudness 测该轨最后片段源素材(源域,未含轨道 EQ/动态;诚实标注)",
      onclick: () => measureTrackSrc(t.id, loudOut),
    }, ["测源响度"]),
    loudOut,
  ]));
  fillEq(t.id, eqRows);
  fillDyn(t.id, dynInputs);
  return st;
}

function eqActions(trackId, rows) {
  return h("div", { class: "mix-actions" }, [
    h("button", {
      class: "mini", testid: `mix-eq-add-${trackId}`, title: "加一段(≤8;peaking/freq/gain/q)",
      onclick: () => rows.appendChild(eqBandRow(trackId, null).row),
    }, ["+ 加段"]),
    h("button", {
      class: "mini", testid: `mix-eq-apply-${trackId}`, title: "track_update patch.eq 整组替换(单 Op)",
      onclick: () => applyEq(trackId, rows),
    }, ["应用 EQ"]),
    h("button", {
      class: "mini", testid: `mix-eq-clear-${trackId}`, title: "patch.eq = null(清除,单 Op)",
      onclick: () => clearTrackPatch(trackId, "eq"),
    }, ["清除"]),
  ]);
}

/** 一段 EQ 行:type/freq/gain/q(+删除);band 缺省 peaking/1000/0/1。
 * 返回 {row, bandOf} — bandOf() 从控件读钳制后的一段(applyEq 消费)。 */
function eqBandRow(trackId, band) {
  const type = h("select", { testid: `mix-eq-type-${trackId}`, "aria-label": "段类型" });
  for (const [v, label] of EQ_TYPES) type.appendChild(h("option", { value: v }, [label]));
  type.value = band && band.type ? String(band.type) : "peaking";
  const freq = numberField({ step: 1, min: EQ_LIM.freq[0], max: EQ_LIM.freq[1], testid: `mix-eq-freq-${trackId}` });
  const gain = numberField({ step: 0.5, min: EQ_LIM.gain[0], max: EQ_LIM.gain[1], testid: `mix-eq-gain-${trackId}` });
  const q = numberField({ step: 0.1, min: EQ_LIM.q[0], max: EQ_LIM.q[1], testid: `mix-eq-q-${trackId}` });
  freq.set(band && band.freq !== undefined ? band.freq : 1000);
  gain.set(band && band.gain !== undefined ? band.gain : 0);
  q.set(band && band.q !== undefined ? band.q : 1);
  const row = h("div", { class: "mix-eq-band", testid: "mix-eq-band" }, [
    type,
    h("label", null, ["f ", freq.root]), h("label", null, ["g ", gain.root]), h("label", null, ["q ", q.root]),
    h("button", { class: "mini", "aria-label": "删除该段", title: "从草稿移除该段(应用才提交)", onclick: () => row.remove() }, ["✕"]),
  ]);
  const bandOf = () => ({
    type: type.value,
    freq: clampOf(freq.get(), EQ_LIM.freq),
    gain: clampOf(gain.get(), EQ_LIM.gain),
    q: clampOf(q.get(), EQ_LIM.q),
  });
  const rec = { row, bandOf };
  eqBandRows.set(row, rec);
  return rec;
}

function fillEq(trackId, rows) {
  clear(rows);
  const t = trackOf(trackId);
  for (const band of (t && Array.isArray(t.eq) ? t.eq.slice(0, 8) : [])) {
    rows.appendChild(eqBandRow(trackId, band).row);
  }
}

async function applyEq(trackId, rows) {
  const bands = [...rows.querySelectorAll(".mix-eq-band")].map((row) => {
    const hit = eqBandRows.get(row);
    return hit ? hit.bandOf() : null;
  }).filter(Boolean);
  if (!bands.length) { toast("至少一段;清空走「清除」(null)", false); return; }
  const env = await updateTrack(trackId, { eq: bands });
  if (env && env.ok) fillEq(trackId, strips.get(trackId)?.eqRows || rows);
  return env;
}

function fillDyn(trackId, dynInputs) {
  const dyn = (trackOf(trackId) || {}).dyn || {};
  for (const [k, , f] of dynInputs) f.set(dyn[k] !== undefined ? dyn[k] : "");
}

async function applyDyn(trackId, dynInputs) {
  const patch = {};
  for (const [k, , f] of dynInputs) {
    const v = Number(f.get());
    if (f.get() !== "" && !Number.isNaN(v)) patch[k] = v;
  }
  if (!Object.keys(patch).length) { toast("先填至少一项动态参数(清除走「清除动态」)", false); return; }
  const env = await updateTrack(trackId, { dyn: patch });
  if (env && env.ok) fillDyn(trackId, strips.get(trackId)?.dynInputs || dynInputs);
  return env;
}

async function clearTrackPatch(trackId, key) {
  const env = await updateTrack(trackId, { [key]: null });
  if (env && env.ok && key === "eq") fillEq(trackId, strips.get(trackId)?.eqRows);
  if (env && env.ok && key === "dyn") fillDyn(trackId, strips.get(trackId)?.dynInputs || []);
  return env;
}

async function toggleMs(trackId, key) {
  const cur = Boolean((trackOf(trackId) || {})[key]);
  await updateTrack(trackId, { [key]: !cur });
}

/** 一段 EQ 行的读取器(WeakMap:行元素 → {row,bandOf};删行自动回收)。 */
const eqBandRows = new WeakMap();

async function measureTrackSrc(trackId, out) {
  const src = lastSrcOf(trackId);
  if (!src) { toast(`${trackId} 轨没有带源片段`, false); return; }
  out.textContent = "测量中…(loudnorm 全片扫描)";
  const env = await measureLoudness(src, targetLufs());
  if (env && env.ok && env.data) {
    const d = env.data;
    out.textContent = `${String(src).split(/[\\/]/).pop()} ${fmtNum(d.inputI)} LUFS · TP ${fmtNum(d.inputTp)}(源域,未含轨道 EQ/动态)`;
  } else {
    out.textContent = "(测量失败,见 toast)";
  }
}

/* ---------------- 小工具 ---------------- */

function trackOf(id) {
  return (projectStore.get().project?.tracks || []).find((t) => t.id === id) || null;
}
function lastSrcOf(trackId) {
  const list = timelineStore.get().clips.filter((c) => c.track === trackId && c.src);
  return list.length ? list[list.length - 1].src : null;
}
function clampOf(raw, [lo, hi]) {
  const v = Number(raw);
  if (Number.isNaN(v)) return lo;
  return Math.min(hi, Math.max(lo, v));
}
function fmtNum(v) {
  return v === undefined || v === null ? "-" : String(v);
}
function lufsPct(d) {
  const v = d && Number.parseFloat(String(d.inputI));
  if (v === undefined || Number.isNaN(v)) return 0;
  return Math.round(Math.min(100, Math.max(0, ((v + 60) / 60) * 100)));
}
