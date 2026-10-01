/* 场景检测壳侧(T5.4):scene_detect(frame-diff 启发式,硬切级跳变)→ 检测点列表
 * → 可选 autoSplit 单 Op 自动切段。挂「多机位」页签下方(同属专业编辑工具簇);
 * degraded/阈值语义按工具输出如实标注。 */
import { h, clear } from "../ui/dom.js";
import { projectStore, ephemeralStore } from "../core/store.js";
import { sceneDetect } from "../core/pro-commands.js";
import { numberField, toggleField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";

let srcInput = null;
let sensField = null;
let autoT = null;
let trackSel = null;
let out = null;
const EMPTY = "(未检测)engine=frame-diff,只收硬切级跳变(溶解/渐变检出候 ffmpeg scdet,登记)";

export function mount(container) {
  sensField = numberField({ testid: "scene-sens", step: 0.1, min: 0, max: 1 });
  sensField.set(0.5);
  autoT = toggleField({
    testid: "scene-auto", label: "检测后自动切段(单 Op)", checked: false,
    onChange: () => syncTrackRow(),
  });
  trackSel = h("select", { testid: "scene-track", "aria-label": "切段目标轨" });
  srcInput = h("input", {
    type: "text", testid: "scene-src", placeholder: "素材路径(工程内相对;或素材右键复制路径粘贴)",
    "aria-label": "检测素材路径",
  });
  out = h("div", { class: "mc-sync-out", testid: "scene-out" });

  const trackRow = h("div", { class: "mc-row", testid: "scene-track-row", hidden: true }, [
    h("label", null, ["切段目标轨 ", trackSel]),
  ]);
  const box = h("fieldset", { class: "insp-group", testid: "scene-tool" }, [
    h("legend", { "data-tip": "scene_detect:抽帧差分剪切点检测(启发式;可选自动切段单 Op)" }, ["场景检测(scene_detect)"]),
    h("div", { class: "insp-fields-wrap" }, [
      srcInput,
      h("div", { class: "mc-row" }, [
        h("button", {
          class: "mini", testid: "scene-run", title: "scene_detect:frame-diff 硬切检测(纯计算;autoSplit 勾选时单 Op 切段)",
          onclick: () => run(),
        }, ["检测剪切点"]),
        h("label", null, ["灵敏度 ", sensField.root]),
        autoT.root,
      ]),
      trackRow,
      out,
    ]),
  ]);
  container.appendChild(box);
  projectStore.subscribe((patch) => { if (patch.project !== undefined) refreshTracks(); });
  ephemeralStore.subscribe((patch) => { if (patch.scene !== undefined) renderOut(); });
  refreshTracks();
  renderOut();
}

function syncTrackRow() {
  trackRow().hidden = !autoT.get();
}
function trackRow() {
  return /** @type {HTMLElement} */ (document.querySelector('[data-testid="scene-track-row"]'));
}

async function run() {
  const src = String(srcInput.value || "").trim();
  if (!src) { toast("先填素材路径", false); return; }
  const split = autoT.get() ? trackSel.value : null;
  if (autoT.get() && !split) { toast("工程没有视频轨,无法自动切段(可取消勾选只检测)", false); return; }
  out.textContent = "检测中…(抽帧差分,时长随素材)";
  const env = await sceneDetect(src, Number(sensField.get()), split);
  if (!env || !env.ok) out.textContent = "(检测失败,见 toast)";
}

function refreshTracks() {
  const tracks = (projectStore.get().project?.tracks || []).filter((t) => (t.kind || "video") === "video");
  clear(trackSel);
  if (!tracks.length) {
    trackSel.appendChild(h("option", { value: "" }, ["(无视频轨)"]));
    trackSel.value = "";
    return;
  }
  for (const t of tracks) trackSel.appendChild(h("option", { value: t.id }, [`${t.id}${t.name ? ` ${t.name}` : ""}`]));
  trackSel.value = tracks[0].id;
}

function renderOut() {
  const d = ephemeralStore.get().scene;
  clear(out);
  if (!d) {
    out.appendChild(h("span", { class: "dim", testid: "scene-empty" }, [EMPTY]));
    return;
  }
  out.appendChild(h("div", { class: "mc-sync-head", testid: "scene-summary" }, [
    h("b", null, [`${d.cutCount ?? (d.cuts || []).length} 剪切点`]),
    h("span", { class: "dim" }, [` · ${d.frames ?? "?"} 帧@${d.sampleFps ?? 5}fps · 灵敏度 ${d.sensitivity ?? "-"}`]),
    d.degraded ? h("span", { class: "dim" }, [" · 启发式(frame-diff),如实标注"]) : null,
  ]));
  const chips = h("div", { class: "beats-list", testid: "scene-cuts" });
  for (const c of (d.cuts || []).slice(0, 64)) {
    chips.appendChild(h("span", {
      class: "beats-chip", testid: "scene-cut",
      title: `tMs=${c.tMs} 置信度=${c.confidence}`,
    }, [`${(c.tMs / 1000).toFixed(2)}s`]));
  }
  out.appendChild(chips);
  if ((d.cuts || []).length > 64) {
    out.appendChild(h("span", { class: "dim" }, [`…共 ${d.cuts.length} 点`]));
  }
}
