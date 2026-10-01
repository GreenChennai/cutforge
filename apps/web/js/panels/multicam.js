/* 多机位壳侧(T5.4,ADR-0019 展开方案):同步集(素材右键维护/手输)→ multicam_sync
 * 分析(角度列表 + 置信度 + 偏移;pcm-xcorr 启发式如实标注)→ 切换点收集(播放头逐点
 * 「打切换点」+ 角度选择,会话态)→ multicam_cut 单 Op 生成序列。
 * 诚实口径:多机位 = 展开的普通片段序列,不是实体;切换点 tMs 相对序列起点,
 * 壳自动以首点归一(首点必须 0/严格递增由归一保证);offset 语义按工具 hint 原文呈现。
 * 场景检测(scenetool.js)挂同页签下方。 */
import { h, clear } from "../ui/dom.js";
import { projectStore, ephemeralStore } from "../core/store.js";
import { playheadMs } from "../core/commands.js";
import {
  mcAddAngle, mcRemoveAngle, mcClearAngles, multicamSync,
  mcAddSwitch, mcRemoveSwitch, mcClearSwitches, multicamCut,
} from "../core/pro-commands.js";
import { numberField, textField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";

let angleHost = null;
let angleInput = null;
let syncOut = null;
let windowField = null;
let angleSel = null;
let switchHost = null;
let durField = null;
let trackSel = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["多机位(T5.4;pcm-xcorr 启发式,置信度如实)"]));
  container.appendChild(h("div", { class: "hint" }, [
    "素材右键「加入多机位同步集」(或下方手输)→「同步分析」→ 播放头逐点「打切换点」→"
    + "「生成序列」。多机位 = 展开的普通片段序列(单 Op,可撤销),不是实体。",
  ]));

  /* ① 同步集 */
  angleInput = textField({ testid: "mc-angle-src", ariaLabel: "角度素材路径", placeholder: "01_原始素材/cam2.mp4(工程内相对;或素材右键加入)" });
  angleInput.root.addEventListener("keydown", (e) => {
    if (e.key === "Enter") commitAngle();
  });
  container.appendChild(h("div", { class: "mc-row" }, [
    angleInput.root,
    h("button", { class: "mini", testid: "mc-angle-add", title: "加入会话同步集(angles[0]=基准)", onclick: commitAngle }, ["加入"]),
    h("button", { class: "mini", testid: "mc-angle-clear", title: "清空会话同步集", onclick: () => { mcClearAngles(); } }, ["清空"]),
  ]));
  angleHost = h("div", { class: "mc-angles", testid: "mc-angles" });
  container.appendChild(angleHost);

  /* ② 同步分析 */
  windowField = numberField({ testid: "mc-window", step: 500, min: 200, max: 30000 });
  windowField.set(5000);
  syncOut = h("div", { class: "mc-sync-out", testid: "mc-sync-out" });
  container.appendChild(h("div", { class: "mc-row" }, [
    h("button", {
      class: "mini", testid: "mc-sync", title: "multicam_sync:波形互相关对齐(纯计算不落盘;angles[0]=基准)",
      onclick: () => runSync(),
    }, ["同步分析"]),
    h("label", null, ["搜索窗 ±ms ", windowField.root]),
  ]));
  container.appendChild(syncOut);

  /* ③ 切换点收集(播放头逐点;会话态) */
  angleSel = h("select", { testid: "mc-switch-angle", "aria-label": "切到的角度" });
  container.appendChild(h("div", { class: "mc-row" }, [
    h("button", {
      class: "mini", testid: "mc-switch-add", title: "在播放头处打一个切换点(相对序列起点自动归一)",
      onclick: () => {
        const mc = ephemeralStore.get().multicam;
        if (!mc) { toast("先「同步分析」取得角度清单", false); return; }
        const a = Number(angleSel.value);
        mcAddSwitch(playheadMs(), Number.isNaN(a) ? 0 : a);
      },
    }, ["播放头打切换点"]),
    h("label", null, ["切到 ", angleSel]),
    h("button", { class: "mini", testid: "mc-switch-clear", title: "清空切换点(会话态)", onclick: () => mcClearSwitches() }, ["清空"]),
  ]));
  switchHost = h("div", { class: "mc-switches", testid: "mc-switches" });
  container.appendChild(switchHost);

  /* ④ 生成序列(单 Op) */
  durField = numberField({ testid: "mc-duration", step: 500, min: 1 });
  durField.set(3000);
  trackSel = h("select", { testid: "mc-track", "aria-label": "目标视频轨" });
  container.appendChild(h("div", { class: "mc-row" }, [
    h("button", {
      class: "mini", testid: "mc-cut", title: "multicam_cut:切换点展开为普通片段序列(单 Op 原子)",
      onclick: () => runCut(),
    }, ["生成序列(multicam_cut)"]),
    h("label", null, ["目标轨 ", trackSel]),
    h("label", null, ["总时长 ms ", durField.root]),
  ]));
  container.appendChild(h("div", { class: "hint" }, [
    "生成口径:序列起点 = 第一个切换点的播放头位置(后续点相对它归一,首点恒 0);"
    + "角度 offsets 取自最近一次同步分析(重新分析会清空切换点,防陈旧偏移落序列)。",
  ]));

  projectStore.subscribe((patch) => { if (patch.project !== undefined) refreshTracks(); });
  ephemeralStore.subscribe((patch) => {
    if (patch.mcAngles !== undefined || patch.multicam !== undefined || patch.mcSwitches !== undefined) renderAll();
  });
  renderAll();
}

function commitAngle() {
  const p = String(angleInput.get() || "").trim();
  if (!p) { toast("先填素材路径(或素材右键「加入多机位同步集」)", false); return; }
  mcAddAngle(p);
  angleInput.set("");
}

async function runSync() {
  const angles = ephemeralStore.get().mcAngles || [];
  if (angles.length < 2) { toast("同步集至少 2 个角度(angles[0]=基准)", false); return; }
  syncOut.textContent = "分析中…(PCM 解码 + 互相关,时长随素材)";
  const env = await multicamSync(angles, Number(windowField.get()));
  if (!env || !env.ok) syncOut.textContent = "(分析失败,见 toast)";
}

async function runCut() {
  const trackId = trackSel.value;
  if (!trackId) { toast("工程没有视频轨", false); return; }
  await multicamCut(trackId, Number(durField.get()));
}

/* ---------------- 渲染 ---------------- */

function renderAll() {
  renderAngles();
  renderSync();
  renderAngleSel();
  renderSwitches();
}

function renderAngles() {
  const list = ephemeralStore.get().mcAngles || [];
  clear(angleHost);
  if (!list.length) {
    angleHost.appendChild(h("span", { class: "dim", testid: "mc-angles-empty" }, ["(同步集为空:素材右键加入,或上方手输)"]));
    return;
  }
  list.forEach((p, i) => {
    angleHost.appendChild(h("span", { class: "mc-angle-chip", testid: "mc-angle", title: p }, [
      h("b", null, [i === 0 ? "基准" : `角${i}`]),
      ` ${String(p).split(/[\\/]/).pop()}`,
      h("button", { class: "mini", "aria-label": `移出 ${p}`, title: "移出同步集", onclick: () => mcRemoveAngle(p) }, ["✕"]),
    ]));
  });
}

function renderSync() {
  const mc = ephemeralStore.get().multicam;
  clear(syncOut);
  if (!mc) {
    syncOut.appendChild(h("span", { class: "dim", testid: "mc-sync-empty" }, ["(未分析)"]));
    return;
  }
  const conf = `${Math.round((mc.confidence ?? 0) * 100)}%`;
  syncOut.appendChild(h("div", { class: "mc-sync-head", testid: "mc-sync-summary" }, [
    h("b", null, [`最小置信度 ${conf}`]),
    h("span", { class: "dim" }, [` · ${mc.angles.length} 角度 · 窗 ±${mc.windowMs}ms · engine=pcm-xcorr(启发式,如实标注)`]),
  ]));
  const rows = h("div", { class: "mc-sync-rows", testid: "mc-sync-angles" });
  for (const a of mc.angles) {
    rows.appendChild(h("div", { class: "mc-sync-row", testid: "mc-sync-angle" }, [
      h("b", null, [a.index === 0 ? "基准" : `角${a.index}`]),
      h("span", { class: "mono", title: a.src }, [String(a.src).split(/[\\/]/).pop()]),
      h("span", { class: "mono" }, [`偏移 ${a.offsetMs}ms`]),
      h("span", { class: "dim" }, [`置信度 ${Math.round((a.confidence ?? 0) * 100)}%`]),
    ]));
  }
  syncOut.appendChild(rows);
  syncOut.appendChild(h("span", { class: "dim" }, [
    "offset 语义 = 角度源时间轴相对基准的滞后(读角度 i 于时间线 T:sourceInMs = T + offsetMs)。",
  ]));
}

function renderAngleSel() {
  const mc = ephemeralStore.get().multicam;
  const list = mc && mc.angles ? mc.angles : (ephemeralStore.get().mcAngles || []).map((src, index) => ({ index, src }));
  const cur = angleSel.value;
  clear(angleSel);
  if (!list.length) {
    angleSel.appendChild(h("option", { value: "" }, ["(先同步)"]));
    angleSel.value = "";
    return;
  }
  list.forEach((a) => {
    angleSel.appendChild(h("option", { value: String(a.index) }, [`角${a.index} · ${String(a.src).split(/[\\/]/).pop()}`]));
  });
  angleSel.value = list.some((a) => String(a.index) === cur) ? cur : "0";
}

function renderSwitches() {
  const list = ephemeralStore.get().mcSwitches || [];
  clear(switchHost);
  if (!list.length) {
    switchHost.appendChild(h("span", { class: "dim", testid: "mc-switches-empty" }, ["(无切换点:移播放头后点「播放头打切换点」)"]));
    return;
  }
  const base = Math.min(...list.map((s) => s.tAbs));
  list.forEach((s, i) => {
    switchHost.appendChild(h("span", { class: "mc-switch-chip", testid: "mc-switch" }, [
      h("b", null, [`#${i}`]),
      ` ${(s.tAbs / 1000).toFixed(2)}s → 角${s.angle}`,
      i === 0 ? h("span", { class: "dim" }, ["(序列起点)"]) : null,
      h("button", { class: "mini", "aria-label": `删除切换点 ${i}`, title: "删除该切换点", onclick: () => mcRemoveSwitch(i) }, ["✕"]),
    ]));
  });
  switchHost.appendChild(h("span", { class: "dim" }, [`共 ${list.length} 点 · 起点 ${base}ms`]));
}

/** 视频轨下拉(projectStore 驱动;装配期与投影到达各刷一次)。 */
function refreshTracks() {
  const tracks = (projectStore.get().project?.tracks || []).filter((t) => (t.kind || "video") === "video");
  const cur = trackSel.value;
  clear(trackSel);
  if (!tracks.length) {
    trackSel.appendChild(h("option", { value: "" }, ["(无视频轨)"]));
    trackSel.value = "";
    return;
  }
  for (const t of tracks) trackSel.appendChild(h("option", { value: t.id }, [`${t.id}${t.name ? ` ${t.name}` : ""}`]));
  trackSel.value = tracks.some((t) => t.id === cur) ? cur : tracks[0].id;
}
