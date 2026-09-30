/* 曲线变速编辑器(册四 T4.4 FE2;简化版:点表 + 折线小图)。
 *
 * 语义对拍内核 speed_segments(crates/cutforge-core/src/model.rs):
 * - 点 {atMs, speed},atMs 相对片段起点;点间线性插值,首点速度前延到 0、末点后延到
 *   durationMs(端点常速外延);单点曲线 ≡ 常速;
 * - patch.speedCurve = 整组替换单 Op;有曲线时优先于 speed。
 * 本编辑器自持草稿、「应用曲线」一次 commit(不进检查器统一草稿——数组整组替换与
 * 逐字段草稿模型不同构);折线为线性插值示意图(区间渲染取均值常速,示意足够)。
 */
import { h, clear } from "../ui/dom.js";
import { updateClip } from "../core/commands.js";
import { toast } from "../ui/toast.js";
import { cssVar } from "../render/theme.js";

const SPEED_MIN = 0.25;
const SPEED_MAX = 4;

/**
 * 构建曲线编辑器宿主(检查器「变速」组附加面;选中变化时由 inspector 调 __cfRefresh)。
 * @param {() => Object|null} rowOf 当前选中片段投影行
 * @returns {HTMLElement}
 */
export function buildCurveHost(rowOf) {
  /** @type {Array<{atMs:number,speed:number}>} */
  let draft = [];
  let loadedId = null;
  const host = h("div", { class: "curve-host", testid: "curve-editor" });
  const plot = h("canvas", { class: "curve-plot", testid: "curve-plot", width: 260, height: 64, "aria-hidden": "true" });
  const rowsEl = h("div", { class: "curve-rows", testid: "curve-rows" });
  host.appendChild(h("div", { class: "curve-head" }, [
    h("b", null, ["速度曲线"]),
    h("button", {
      class: "mini", testid: "curve-add", title: "在末尾追加一个速度点",
      onclick: () => {
        const row = rowOf();
        const dur = row ? row.endMs - row.startMs : 3000;
        const last = draft[draft.length - 1];
        const at = last ? Math.min(dur, last.atMs + Math.round(dur / 4)) : 0;
        draft.push({ atMs: at, speed: 1 });
        renderRows();
        drawPlot();
      },
    }, ["+ 加点"]),
    h("button", {
      class: "mini", testid: "curve-apply", title: "整组替换写回(clip_update.speedCurve,单 Op)",
      onclick: () => commit(false),
    }, ["应用曲线"]),
    h("button", {
      class: "mini", testid: "curve-reset", title: "清回匀速 1x(单点曲线 ≡ 常速)",
      onclick: () => commit(true),
    }, ["重置匀速"]),
  ]));
  host.appendChild(plot);
  host.appendChild(rowsEl);
  host.appendChild(h("div", { class: "hint" }, [
    "点间线性插值;首点前延到片段起点、末点后延到终点(内核分段积分,渲染取区间均值常速)。",
  ]));

  function load(row) {
    if (!row) { draft = []; loadedId = null; renderRows(); drawPlot(); return; }
    if (loadedId === row.id) return; // 同一片段:保留未应用的草稿(应用/切换片段才重载)
    loadedId = row.id;
    const dur = row.endMs - row.startMs;
    draft = (row.speedCurve || []).map((p) => ({
      atMs: Math.min(dur, Math.max(0, Number(p.atMs) || 0)),
      speed: Math.min(SPEED_MAX, Math.max(SPEED_MIN, Number(p.speed) || 1)),
    }));
    renderRows();
    drawPlot();
  }

  function commit(reset) {
    const row = rowOf();
    if (!row) { toast("先选中片段", false); return; }
    if (reset) {
      updateClip(row.id, { speedCurve: [{ atMs: 0, speed: 1 }], speed: 1 }, "已重置匀速(可撤销)");
      return;
    }
    if (!draft.length) { toast("曲线为空:先加点,或用「重置匀速」", false); return; }
    const dur = row.endMs - row.startMs;
    const pts = draft
      .map((p) => ({ atMs: Math.min(dur, Math.max(0, Math.round(p.atMs))), speed: p.speed }))
      .sort((a, b) => a.atMs - b.atMs);
    updateClip(row.id, { speedCurve: pts }, `已应用速度曲线(${pts.length} 点,可撤销)`);
  }

  function renderRows() {
    clear(rowsEl);
    if (!draft.length) {
      rowsEl.appendChild(h("div", { class: "hint" }, ["未启用曲线(匀速走「变速·速度」字段)"]));
      return;
    }
    draft.forEach((p, i) => {
      const at = h("input", {
        type: "number", min: 0, step: 10, class: "curve-at",
        "aria-label": `点 ${i + 1} 时刻(ms)`,
      });
      at.value = String(p.atMs);
      at.addEventListener("input", () => {
        p.atMs = Math.max(0, Number(at.value) || 0);
        drawPlot();
      });
      const sp = h("input", {
        type: "number", min: SPEED_MIN, max: SPEED_MAX, step: 0.05, class: "curve-sp",
        "aria-label": `点 ${i + 1} 速度(x)`,
      });
      sp.value = String(p.speed);
      sp.addEventListener("input", () => {
        const v = Number(sp.value);
        if (Number.isNaN(v)) return;
        const was = p.speed;
        p.speed = Math.min(SPEED_MAX, Math.max(SPEED_MIN, v)); // 越界前端钳制
        if (p.speed !== was) sp.value = String(p.speed);
        drawPlot();
      });
      rowsEl.appendChild(h("div", { class: "curve-row", dataset: { idx: String(i) } }, [
        h("span", { class: "dim" }, [`#${i + 1}`]),
        h("label", null, ["t ", at, "ms"]),
        h("label", null, ["速 ", sp, "x"]),
        h("button", {
          class: "mini", title: "删除该点", "aria-label": `删除点 ${i + 1}`,
          onclick: () => { draft.splice(i, 1); renderRows(); drawPlot(); },
        }, ["✕"]),
      ]));
    });
  }

  /** 折线小图:x=atMs/dur,y=速度(0..4 对数感压缩为线性即可;token 取色,零硬编码)。 */
  function drawPlot() {
    const ctx = plot.getContext("2d");
    const W = plot.width;
    const H = plot.height;
    const bg = cssVar("--cf-panel-deep");
    const line = cssVar("--cf-accent");
    const grid = cssVar("--cf-line");
    ctx.fillStyle = bg || "";
    ctx.fillRect(0, 0, W, H);
    ctx.strokeStyle = grid || "";
    ctx.lineWidth = 1;
    for (const fy of [0.25, 0.5, 0.75]) {
      ctx.beginPath();
      ctx.moveTo(0, H * fy);
      ctx.lineTo(W, H * fy);
      ctx.stroke();
    }
    const row = rowOf();
    const dur = Math.max(1, row ? row.endMs - row.startMs : 3000);
    if (!draft.length) return;
    const pts = [...draft].sort((a, b) => a.atMs - b.atMs);
    const xOf = (at) => (at / dur) * (W - 8) + 4;
    const yOf = (sp) => H - 4 - (Math.min(SPEED_MAX, Math.max(0, sp)) / SPEED_MAX) * (H - 8);
    // 端点外延(与内核口径一致的示意)
    ctx.strokeStyle = line || "";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(xOf(0), yOf(pts[0].speed));
    for (const p of pts) ctx.lineTo(xOf(p.atMs), yOf(p.speed));
    ctx.lineTo(xOf(dur), yOf(pts[pts.length - 1].speed));
    ctx.stroke();
    ctx.fillStyle = line || "";
    for (const p of pts) ctx.fillRect(xOf(p.atMs) - 2, yOf(p.speed) - 2, 4, 4);
  }

  host.__cfRefresh = (row) => { load(row); drawPlot(); };
  return host;
}
