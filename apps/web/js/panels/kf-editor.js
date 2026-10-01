/* 关键帧编辑器(册五 T5.1 FE;双面板:关键帧列表 + 贝塞尔画布)。
 *
 * - 数据:读投影 keyframes → 本地草稿 → 整组写回(clip_update patch.keyframes 单 Op);
 * - 画布形状唯一来源 = 投影 keyframeSamples(kf-curve.js;ADR-0018 壳零插值);
 * - 列表:增删改(timeMs/value/interp);画布:拖锚点改时刻/值、选中帧调 interp 预设、
 *   bezier 双柄拖拽;拖拽全程零 Op,松手提交,Esc 取消(草稿回滚投影值);
 * - speed 关键帧与 speedCurve 互斥(内核契约):互斥时打点前端拦截并如实说明。
 */
import { h, clear } from "../ui/dom.js";
import { timelineStore, selectionStore, ephemeralStore } from "../core/store.js";
import { updateClip, playheadMs } from "../core/commands.js";
import { snapMs, frameMsOf } from "../core/model.js";
import { projectStore } from "../core/store.js";
import {
  WATCHABLE, PROP_META, INTERP_PRESETS, EASE_CP,
  kfsOfProp, samplesOf, domainOf, currentValueOf, kfTimeAt,
  draftOf as kfDraftOf, upsertKf, dropKf, moveKf, kfPatch,
} from "../core/kf-model.js";
import { createCurveCanvas } from "./kf-curve.js";
import { toast } from "../ui/toast.js";

/**
 * 构建关键帧编辑器宿主(检查器「画面」组附加面;__cfRefresh(row) 随选中刷新)。
 * @param {() => Object|null} rowOf
 */
export function buildKeyframeHost(rowOf) {
  let draft = [];
  let loadedId = null;
  let prop = "opacity";
  let sel = -1;
  let dragging = false;

  const host = h("div", { class: "kw-host", testid: "kw-editor" });
  const propSel = /** @type {HTMLSelectElement} */ (h("select", { testid: "kw-prop-select", "aria-label": "关键帧属性" }));
  propSel.addEventListener("change", () => {
    prop = propSel.value;
    sel = -1;
    ephemeralStore.set({ kfProp: prop }); // 时间线关键帧行打点共用「当前属性」
    renderAll();
  });
  const head = h("div", { class: "kw-head" }, [
    h("b", null, ["关键帧"]),
    propSel,
    h("button", {
      class: "mini", type: "button", testid: "kw-add",
      title: "在播放头时刻打当前值(整组写回,单 Op)",
      onclick: () => addAtPlayhead(),
    }, ["+ 打点"]),
    h("span", { class: "dim", testid: "kw-count" }, [""]),
  ]);
  host.appendChild(head);
  const panes = h("div", { class: "kw-panes" });
  const listPane = h("div", { class: "kw-list", testid: "kw-kf-list" });
  const canvasPane = h("div", { class: "kw-canvas-pane" });
  panes.appendChild(listPane);
  panes.appendChild(canvasPane);
  host.appendChild(panes);
  const interpRow = h("div", { class: "kw-interp-row", testid: "kw-interp" });
  canvasPane.appendChild(interpRow);
  host.appendChild(h("div", { class: "hint" }, [
    "画布点云 = 服务端采样(keyframeSamples,壳零插值);拖锚点为直线示意,松手提交后刷新真曲线;Esc 取消拖拽。",
  ]));

  const curve = createCurveCanvas(canvasPane, {
    mode: "kf",
    testid: "kf-curve-canvas",
    ariaLabel: "关键帧贝塞尔画布",
    frameOf: () => {
      const row = rowOf();
      const dur = row ? Math.max(1, Math.round(row.endMs - row.startMs)) : 3000;
      const [lo, hi] = row ? domainOf(row, prop) : [0, 1];
      const mine = draft.filter((k) => k.property === prop);
      return {
        dur, lo, hi,
        anchors: mine.map((k) => ({ t: k.timeMs, v: k.value, interp: k.interp, bezier: k.bezier })),
        samples: row ? samplesOf(row, prop) : [],
        sel,
        playheadT: row ? Math.max(0, Math.min(dur, playheadMs() - row.startMs)) : 0,
      };
    },
    snapTimeOf: (t) => snapMs(t, true, frameMsOf(projectStore.get().project)),
    onSelect: (i) => { sel = i; renderInterp(); curve.draw(); renderList(); },
    onAnchorDrag: (i, t, v) => {
      dragging = true;
      const row = rowOf();
      const dur = row ? Math.max(1, Math.round(row.endMs - row.startMs)) : 3000;
      const mine = draft.filter((k) => k.property === prop);
      const kf = mine[i];
      if (!kf) return;
      moveKf(draft, kf, t, dur); // 邻居间夹取,保同属性严格递增(内核契约)
      kf.value = round4(v);
      renderList();
    },
    onHandleDrag: (i, cp) => {
      dragging = true;
      const mine = draft.filter((k) => k.property === prop);
      if (mine[i]) mine[i].bezier = cp;
    },
    onAnchorDel: (i) => {
      const mine = draft.filter((k) => k.property === prop);
      if (!mine[i]) return;
      dropKf(draft, mine[i]);
      sel = -1;
      commit(`已删除关键帧(${propLabel()},可撤销)`);
    },
    onCanvasDbl: (t, v) => {
      addKf(t, round4(v));
    },
    onCommit: () => {
      dragging = false;
      commit("关键帧已更新(拖拽落点,可撤销)");
    },
    onCancel: () => {
      dragging = false;
      const row = rowOf();
      if (row) { draft = kfDraftOf(row); sel = -1; }
      renderAll();
    },
  });

  function propLabel(p) {
    return (PROP_META[p] && PROP_META[p].label) || p;
  }

  function round4(v) {
    return Math.round(v * 10000) / 10000;
  }

  function load(row, force = false) {
    if (!row) { draft = []; loadedId = null; renderAll(); return; }
    if (loadedId === row.id && !force && !dragging) {
      // 同片段:投影变化(外部改/本编辑器提交后)→ 重载草稿,保持服务端收敛
      draft = kfDraftOf(row);
      renderAll();
      return;
    }
    if (loadedId !== row.id) {
      loadedId = row.id;
      sel = -1;
      const mine = kfsOfProp(row, "opacity");
      prop = mine.length || !kfsOfProp(row, "scale").length ? "opacity" : "scale";
      if (!kfsOfProp(row, prop).length) {
        const first = (row.keyframes || []).find((k) => WATCHABLE.includes(k.property) || k.property.startsWith("fx."));
        if (first) prop = first.property;
      }
    }
    draft = kfDraftOf(row);
    renderAll();
  }

  function addAtPlayhead() {
    const row = rowOf();
    if (!row) { toast("先选中片段", false); return; }
    addKf(kfTimeAt(row, playheadMs()), round4(currentValueOf(row, prop, null)));
  }

  function addKf(t, v) {
    const row = rowOf();
    if (!row) { toast("先选中片段", false); return; }
    if (prop === "speed" && row.speedCurve) {
      toast("speed 关键帧与 speedCurve 互斥:先「重置匀速」清掉速度曲线,或改用速度曲线表达", false);
      return;
    }
    upsertKf(draft, prop, t, v);
    sel = draft.filter((k) => k.property === prop).findIndex((k) => k.timeMs === Math.round(t));
    commit(`已打关键帧 ${propLabel(prop)} @${Math.round(t)}ms = ${round4(v)}(可撤销)`);
  }

  function commit(msg) {
    const row = rowOf();
    if (!row) return;
    // 空数组 = 清除全部关键帧(册五收口:内核已接受 [])
    updateClip(row.id, kfPatch(draft), msg);
  }

  function refreshProps(row) {
    const props = WATCHABLE.slice();
    for (const k of (row && row.keyframes) || []) {
      if (!props.includes(k.property)) props.push(k.property);
    }
    const cur = props.includes(prop) ? prop : (props[0] || "opacity");
    clear(propSel);
    for (const p of props) {
      propSel.appendChild(h("option", { value: p }, [propLabel(p)]));
    }
    propSel.value = cur;
    prop = cur;
    ephemeralStore.set({ kfProp: cur });
  }

  function renderAll() {
    const row = rowOf();
    refreshProps(row);
    const mine = draft.filter((k) => k.property === prop);
    const countEl = host.querySelector('[data-testid="kw-count"]');
    if (countEl) countEl.textContent = row ? `${mine.length} 帧` : "";
    renderList();
    renderInterp();
    curve.draw();
  }

  function renderList() {
    clear(listPane);
    const mine = draft.filter((k) => k.property === prop);
    if (!mine.length) {
      listPane.appendChild(h("div", { class: "hint" }, [
        `「${propLabel(prop)}」无关键帧:秒表打点、画布双击或「+ 打点」开始。`,
      ]));
      return;
    }
    mine.forEach((kf, i) => {
      const tIn = h("input", { type: "number", min: 0, step: 10, class: "kw-t", "aria-label": `帧 ${i + 1} 时刻(ms)`, testid: `kw-time-${i}` });
      tIn.value = String(kf.timeMs);
      tIn.addEventListener("input", () => {
        const row = rowOf();
        const dur = row ? Math.max(1, Math.round(row.endMs - row.startMs)) : 3000;
        moveKf(draft, kf, Number(tIn.value) || 0, dur);
        curve.draw();
      });
      tIn.addEventListener("change", () => { renderList(); commit(`关键帧时刻已改(${propLabel(prop)},可撤销)`); });
      const vIn = h("input", { type: "number", step: PROP_META[prop] ? PROP_META[prop].step : 0.05, class: "kw-v", "aria-label": `帧 ${i + 1} 值`, testid: `kw-value-${i}` });
      vIn.value = String(kf.value);
      vIn.addEventListener("input", () => { kf.value = round4(Number(vIn.value) || 0); curve.draw(); });
      vIn.addEventListener("change", () => commit(`关键帧值已改(${propLabel(prop)},可撤销)`));
      listPane.appendChild(h("div", {
        class: `kw-kf-row${i === sel ? " sel" : ""}`, dataset: { idx: String(i) },
        onclick: () => { sel = i; renderList(); renderInterp(); curve.draw(); },
      }, [
        h("span", { class: "dim" }, [`#${i + 1}`]),
        h("label", null, ["t ", tIn]),
        h("label", null, ["值 ", vIn]),
        h("button", {
          class: "mini", title: "删除该帧", "aria-label": `删除帧 ${i + 1}`, testid: `kw-kf-del-${i}`,
          onclick: (e) => {
            e.stopPropagation();
            dropKf(draft, kf);
            sel = -1;
            commit(`已删除关键帧(${propLabel(prop)},可撤销)`);
          },
        }, ["✕"]),
      ]));
    });
  }

  function renderInterp() {
    clear(interpRow);
    const mine = draft.filter((k) => k.property === prop);
    if (sel < 0 || !mine[sel]) {
      interpRow.appendChild(h("span", { class: "dim" }, ["选中关键帧后调插值"]));
      return;
    }
    const kf = mine[sel];
    interpRow.appendChild(h("span", { class: "dim" }, [`#${sel + 1} 插值`]));
    for (const [value, label] of INTERP_PRESETS) {
      interpRow.appendChild(h("button", {
        class: `mini${kf.interp === value ? " on" : ""}`, type: "button",
        testid: `kw-interp-${value}`, title: `插值预设:${label}(本帧→下一帧区间)`,
        onclick: () => {
          kf.interp = value;
          if (value === "bezier" && !Array.isArray(kf.bezier)) kf.bezier = EASE_CP.easeInOut.slice();
          if (value !== "bezier") delete kf.bezier;
          renderInterp();
          curve.draw();
          commit(`插值已设为 ${label}(${propLabel(prop)} #${sel + 1},可撤销)`);
        },
      }, [label]));
    }
    if (kf.interp === "bezier") {
      interpRow.appendChild(h("span", { class: "dim kw-bezier-cp", testid: "kw-bezier-cp" },
        [`柄 (${kf.bezier || []})`]));
    }
  }

  host.__cfRefresh = (row) => {
    // 选中变化才强制重载;播放头 seek(selectionStore 高频)不打断编辑中的草稿
    const id = row ? row.id : null;
    if (id !== loadedId) load(row);
  };
  timelineStore.subscribe(() => {
    const row = rowOf();
    if (row && !dragging) load(row);
  });
  selectionStore.subscribe((patch) => {
    // 仅选中片段变化时重载;playheadMs 高频更新忽略(投影收敛走 timelineStore 订阅)
    if (patch.clipId !== undefined && !dragging) load(rowOf());
  });
  load(rowOf());
  return host;
}
