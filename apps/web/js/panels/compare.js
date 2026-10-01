/* 分屏对比(册五 T5.2 FE 登记项;预览区拖分割线左右对比)。
 *
 * 实现口径(诚实):内核 render_frame 无「不带 grade」开关,而壳零 Op 纪律禁止
 * 「清 grade→渲染→还原」的临时写;故 A/B 走「基准帧快照」——
 *   左 = 基准帧(点「抓取基准帧」时刻的工程状态,典型用法:调色前抓基准)
 *   右 = 当前帧(带当前 grade 的 render_frame,可随时刷新)
 * 两帧均经 render_frame 拉取(/media 加载),image 叠放 + clip-path 分割,拖线跟手
 * (零动画,transform/clip-path 直写;Esc 复位 50%)。面板恒显口径说明。
 */
import { $, h } from "../ui/dom.js";
import { playback } from "../render/preview-loop.js";
import { mediaUrlFor } from "../core/model.js";
import { projectStore } from "../core/store.js";
import { precisePreview } from "../core/render-commands.js";
import { messageOf } from "../core/errors.js";
import { toast } from "../ui/toast.js";
import { runGesture } from "../render/gesture-kit.js";

let open = false;
let pct = 50;
let host = null;
let overlay = null;
let imgBase = null;
let imgCur = null;
let divider = null;
let statusEl = null;
let baseSet = false;

/** 装配(main.js 调用;控件条挂预览面板之下,叠层挂 #pv-stage)。 */
export function mountCompare() {
  const col = $("pv-col");
  const stage = $("pv-stage");
  if (!col || !stage) return;
  stage.classList.add("pv-stage-rel");
  imgBase = h("img", {
    class: "compare-img", testid: "compare-img-base", alt: "基准帧", draggable: "false",
  });
  imgCur = h("img", {
    class: "compare-img", testid: "compare-img-cur", alt: "当前帧", draggable: "false",
  });
  divider = h("div", { class: "compare-divider", testid: "compare-divider", title: "拖动分割线(Esc 复位 50%)", role: "separator", "aria-label": "对比分割线" }, [
    h("span", { class: "compare-grip", "aria-hidden": "true" }, ["⇔"]),
  ]);
  overlay = h("div", { class: "compare-overlay", testid: "compare-overlay", hidden: true }, [
    imgBase, imgCur,
    h("span", { class: "compare-tag compare-tag-l" }, ["基准"]),
    h("span", { class: "compare-tag compare-tag-r" }, ["当前"]),
    divider,
  ]);
  stage.appendChild(overlay);
  host = h("div", { class: "compare-bar", testid: "compare-bar", hidden: true }, [
    h("button", {
      class: "mini", type: "button", testid: "compare-baseline",
      title: "render_frame 抓当前帧为对比基准(典型用法:调色前抓基准)",
      onclick: () => captureBase(),
    }, ["抓取基准帧"]),
    h("button", {
      class: "mini", type: "button", testid: "compare-refresh",
      title: "render_frame 刷新右半当前帧(含当前调色)",
      onclick: () => refreshCurrent("已刷新当前帧"),
    }, ["刷新当前帧"]),
    h("span", { class: "dim", testid: "compare-status" }, ["基准未抓取"]),
  ]);
  col.insertBefore(host, $("bgm-panel"));
  const transport = $("pv-transport");
  const toggle = h("button", {
    id: "pv-compare-toggle", testid: "compare-toggle",
    title: "A/B 分屏对比(左=基准帧快照,右=当前帧;两帧均 render_frame)",
    "aria-pressed": "false",
    onclick: () => setOpen(!open),
  }, ["A/B 对比"]);
  if (transport) transport.appendChild(toggle);
  mountDividerDrag();
}

function setOpen(v) {
  open = v;
  overlay.hidden = !v;
  host.hidden = !v;
  const btn = $("pv-compare-toggle");
  if (btn) {
    btn.setAttribute("aria-pressed", v ? "true" : "false");
    btn.classList.toggle("on", v);
  }
  if (v) {
    if (!baseSet) captureBase();
    refreshCurrent("");
  }
}

/** 抓基准帧:render_frame 播放头帧(工程状态快照;零 Op,不进撤销)。 */
async function captureBase() {
  const atMs = Math.round(playback.clockMs());
  if (statusEl) statusEl.textContent = "抓取基准中…";
  const env = await precisePreview(atMs);
  if (!env.ok) {
    if (statusEl) statusEl.textContent = `基准帧失败:${env.code}`;
    toast(`对比基准:${messageOf(env, "render_frame")}`, false);
    return;
  }
  const media = env.data && env.data.media;
  if (!media) {
    if (statusEl) statusEl.textContent = "render_frame 未返回帧路径";
    return;
  }
  imgBase.src = mediaUrlFor(media, projectStore.get().token);
  baseSet = true;
  if (statusEl) statusEl.textContent = `基准 @${atMs}ms(抓取时刻工程状态)`;
}

/** 刷新当前帧(右半)。 */
async function refreshCurrent(okMsg) {
  const atMs = Math.round(playback.clockMs());
  const env = await precisePreview(atMs);
  if (!env.ok) {
    if (statusEl) statusEl.textContent = `当前帧失败:${env.code}`;
    toast(`对比当前帧:${messageOf(env, "render_frame")}`, false);
    return;
  }
  const media = env.data && env.data.media;
  if (!media) return;
  imgCur.src = mediaUrlFor(media, projectStore.get().token);
  if (okMsg && statusEl) statusEl.textContent = `${okMsg} @${atMs}ms`;
}

function applyPct() {
  imgCur.style.clipPath = `inset(0 0 0 ${pct}%)`;
  divider.style.left = `${pct}%`;
}

function mountDividerDrag() {
  divider.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const start = pct;
    const applyMove = (ev) => {
      const rect = overlay.getBoundingClientRect();
      if (!rect.width) return;
      pct = Math.min(98, Math.max(2, ((ev.clientX - rect.left) / rect.width) * 100));
      applyPct();
    };
    runGesture(divider, e, {
      move: applyMove,
      end: (ev) => {
        if (ev) applyMove(ev); // 补最后一帧(rAF 合帧可能吞掉收尾 move)
      },
      cancel: () => { pct = start; applyPct(); },
    });
  });
  divider.addEventListener("dblclick", (e) => {
    e.stopPropagation();
    pct = 50;
    applyPct();
  });
  applyPct();
}
