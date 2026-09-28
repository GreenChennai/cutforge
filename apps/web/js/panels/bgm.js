/* BGM 面板(T2.5,v0.6 对拍):bgm_set / bgm_clear(工程级,可撤销)。 */
import { h } from "../ui/dom.js";
import { projectStore } from "../core/store.js";
import { setBgm, clearBgm } from "../core/commands.js";
import { numberField, toggleField } from "../ui/controls.js";

let srcInput = null;
let gainField = null;
let duckT = null;
let loopT = null;
let stateEl = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["背景乐 BGM(bgm_set,可撤销)"]));
  srcInput = h("input", { type: "text", id: "bgm-src", testid: "bgm-src", placeholder: "01_原始素材/bgm.mp3 或素材面板点 BGM" });
  container.appendChild(h("label", null, ["音频路径 ", srcInput]));
  gainField = numberField({ id: "bgm-gain", testid: "bgm-gain", step: 1, min: null, max: 0 });
  gainField.set(-18);
  duckT = toggleField({ id: "bgm-duck", testid: "bgm-duck", label: "人声闪避", checked: true });
  loopT = toggleField({ id: "bgm-loop", testid: "bgm-loop", label: "循环", checked: true });
  container.appendChild(h("div", { class: "bgm-row" }, [
    h("label", null, ["音量(dB) ", gainField.root]),
    duckT.root,
    loopT.root,
  ]));
  stateEl = h("span", { id: "bgm-state", class: "dim", testid: "bgm-state" }, ["(未设置)"]);
  container.appendChild(h("div", { class: "bgm-row" }, [
    h("button", { id: "bgm-apply", testid: "bgm-apply", onclick: () => apply(false) }, ["应用 BGM"]),
    h("button", { id: "bgm-clear", testid: "bgm-clear", onclick: () => apply(true) }, ["清除"]),
    stateEl,
  ]));

  // 投影到达 → 回填工程 BGM 态(旧壳 refreshBgm 口径:每次 refresh 均回填)
  projectStore.subscribe((patch, st) => {
    if (patch.project !== undefined || patch.__reset__) fillFrom(st.project);
  });
  fillFrom(projectStore.get().project);
}

function fillFrom(project) {
  const bgm = project && project.bgm;
  srcInput.value = (bgm && bgm.src) || "";
  gainField.set(bgm && bgm.gainDb !== undefined ? bgm.gainDb : -18);
  duckT.set(bgm ? bgm.ducking !== false : true);
  loopT.set(bgm ? bgm.loop !== false : true);
  stateEl.textContent = bgm && bgm.src
    ? `当前:${String(bgm.src).split(/[\\/]/).pop()}(${bgm.gainDb ?? -18}dB)`
    : "(未设置)";
}

function apply(clear_) {
  if (clear_) {
    clearBgm();
    return;
  }
  setBgm({
    src: srcInput.value.trim(),
    gainDb: Number(gainField.get()),
    ducking: duckT.get(),
    loop: loopT.get(),
  });
}
