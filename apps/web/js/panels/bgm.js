/* BGM 面板(T2.5,v0.6 对拍):bgm_set / bgm_clear(工程级,可撤销)。
 * 册四 T4.8 增:卡点(audio_beats 纯计算 → 会话节拍;列表/置信度诚实展示;
 * 节拍入统一吸附候选 + 标尺 scrub 播放头吸附,见 gesture-kit/gestures)。 */
import { h, clear } from "../ui/dom.js";
import { projectStore, ephemeralStore } from "../core/store.js";
import { setBgm, clearBgm } from "../core/commands.js";
import { detectBeats, clearBeats } from "../core/edit-commands.js";
import { numberField, toggleField } from "../ui/controls.js";

let srcInput = null;
let gainField = null;
let duckT = null;
let loopT = null;
let stateEl = null;
let beatsBox = null;

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

  /* ---- 卡点(audio_beats;纯计算不落盘,节拍为会话态) ---- */
  const sens = numberField({ testid: "beats-sensitivity", min: 0, max: 1, step: 0.1 });
  sens.set(0.5);
  beatsBox = h("div", { class: "beats-box", testid: "beats-box" });
  container.appendChild(h("div", { class: "bgm-row" }, [
    h("button", {
      id: "bgm-beats", testid: "bgm-beats", title: "audio_beats:onset-energy 启发式检测(纯计算,不产 Op)",
      onclick: () => {
        const src = srcInput.value.trim();
        if (!src) { beatsBox.textContent = "先填 BGM 音频路径(或素材面板点 BGM)"; return; }
        detectBeats(src, Number(sens.get())).then((env) => {
          if (!env || !env.ok) renderBeats();
        });
      },
    }, ["检测节拍"]),
    h("button", {
      id: "bgm-beats-clear", testid: "beats-clear", title: "清除会话节拍(不落盘)",
      onclick: () => { clearBeats(); renderBeats(); },
    }, ["清除节拍"]),
    h("label", null, ["灵敏度 ", sens.root]),
  ]));
  container.appendChild(beatsBox);
  container.appendChild(h("div", { class: "hint" }, [
    "节拍检测为启发式(onset-energy,置信度如实);节拍入统一吸附候选(磁吸 standard 档),"
    + "标尺 scrub 自动吸附最近拍。会话态,刷新即失。",
  ]));

  // 投影到达 → 回填工程 BGM 态(旧壳 refreshBgm 口径:每次 refresh 均回填)。
  // A4-L15 竞态根治(焦点守卫):上一笔 op 的 reproject 恰落在「填草稿 → 点应用」
  // 窗口内时,fillFrom 曾以工程值覆盖输入框 → 草稿被清空 → 本笔点击发空值。
  // 口径:草稿编辑中(面板任一控件持焦点)的投影回填跳过——草稿归用户,投影只回填
  // 非编辑态;点应用后焦点移出控件,回填恢复,应用值照常进框。__reset__(切工程)
  // 不守卫:旧工程草稿无保护价值,强制回填。
  projectStore.subscribe((patch, st) => {
    if (patch.__reset__ || (patch.project !== undefined && !userEditing())) fillFrom(st.project);
  });
  ephemeralStore.subscribe((patch) => { if (patch.beats !== undefined) renderBeats(); });
  fillFrom(projectStore.get().project);
  renderBeats();
}

/** 面板任一输入控件持焦点 = 用户草稿编辑中(fillFrom 不得覆盖)。 */
function userEditing() {
  const ae = document.activeElement;
  return !!ae && (ae === srcInput || ae === gainField.input
    || ae === duckT.input || ae === loopT.input);
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

/** 节拍列表(bpm/拍数/置信度/启发式标注 + 前 16 拍时间)。 */
function renderBeats() {
  if (!beatsBox) return;
  clear(beatsBox);
  const beats = ephemeralStore.get().beats;
  if (!beats || !beats.beats || !beats.beats.length) {
    beatsBox.appendChild(h("span", { class: "dim", testid: "beats-empty" }, ["(未检测节拍)"]));
    return;
  }
  const conf = typeof beats.confidence === "number" ? `${Math.round(beats.confidence * 100)}%` : "-";
  beatsBox.appendChild(h("div", { class: "beats-head", testid: "beats-summary" }, [
    h("b", null, [`BPM ${beats.bpm}`]),
    h("span", { class: "dim" }, [` · ${beats.beats.length} 拍 · 置信度 ${conf}`]),
    h("span", { class: "dim" }, [beats.degraded ? " · 启发式(onset-energy),非节拍追踪,如实标注" : ""]),
  ]));
  beatsBox.appendChild(h("div", { class: "beats-list", testid: "beats-list" }, [
    ...beats.beats.slice(0, 16).map((b) => h("span", { class: "beats-chip" }, [`${(b / 1000).toFixed(2)}s`])),
    beats.beats.length > 16 ? h("span", { class: "dim" }, [`…共 ${beats.beats.length} 拍`]) : null,
  ]));
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
