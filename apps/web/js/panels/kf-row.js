/* 时间线关键帧行(册五 T5.1 FE;选中片段下方菱形标记行)。
 *
 * 交互映射(计划书 T5.1 壳侧):
 * - 菱形 per 关键帧(全部属性;横位 = timeMs 在片段内的相对位置,悬停给属性/时刻/值);
 * - 菱形拖拽 = 移动(邻居间夹取保严格递增;拖拽全程零 Op,松手整组写回单 Op,Esc 取消);
 * - 双击菱形 = 删除该帧(单 Op);
 * - 行空白双击 = 在播放头时刻打当前值(属性 = 关键帧编辑器当前属性,ephemeral.kfProp);
 * - 行内一切交互 stopPropagation:不触发轨道框选/片段拖拽/右键菜单。
 */
import { h, clear } from "../ui/dom.js";
import { timelineStore, selectionStore, ephemeralStore, projectStore } from "../core/store.js";
import { updateClip, playheadMs } from "../core/commands.js";
import { PX_PER_MS, snapMs, frameMsOf } from "../core/model.js";
import {
  PROP_META, currentValueOf, kfTimeAt,
  draftOf as kfDraftOf, upsertKf, dropKf, moveKf, kfPatch, emptyBlocked, CLEAR_BLOCKED_MSG,
} from "../core/kf-model.js";
import { runGesture, showBubble, hideBubble } from "../render/gesture-kit.js";
import { toast } from "../ui/toast.js";

/** 在轨道行内同步关键帧行(仅选中片段;timeline-view.renderTimelineView 每轮调用)。 */
export function syncKfRow(laneEl, row, selected) {
  const prev = laneEl.querySelector(".kf-row");
  if (!selected || !row || !hasAnyKf(row)) {
    if (prev) prev.remove();
    return;
  }
  let el = prev;
  if (!el || el.dataset.clipId !== row.id) {
    if (prev) prev.remove();
    el = buildRow(row);
    laneEl.appendChild(el);
  }
  layoutRow(el, row);
}

function hasAnyKf(row) {
  return Array.isArray(row.keyframes) && row.keyframes.length > 0;
}

function buildRow(row) {
  const el = h("div", {
    class: "kf-row", testid: "kf-row", dataset: { clipId: row.id },
    title: "关键帧行:拖菱形移动 / 双击删除 / 空白双击在播放头打当前值",
  });
  // 行级隔离:不触发轨道框选/片段手势/右键
  el.addEventListener("pointerdown", (e) => e.stopPropagation());
  el.addEventListener("dblclick", (e) => {
    e.stopPropagation();
    if (/** @type {HTMLElement} */ (e.target).closest(".kf-diamond, .kf-row-add")) return;
    addAtPlayhead(row.id);
  });
  el.addEventListener("contextmenu", (e) => { e.stopPropagation(); e.preventDefault(); });
  el.addEventListener("click", (e) => e.stopPropagation());
  el.appendChild(h("button", {
    class: "mini kf-row-add", type: "button", testid: "kw-row-add",
    title: "在播放头时刻打当前值(属性 = 编辑器当前属性;单 Op)",
    onclick: (e) => { e.stopPropagation(); addAtPlayhead(row.id); },
  }, ["+打点"]));
  el.appendChild(h("div", { class: "kf-diamonds" }));
  return el;
}

function layoutRow(el, row) {
  const dur = Math.max(1, Math.round(row.endMs - row.startMs));
  const width = Math.max(6, Math.round((row.endMs - row.startMs) * PX_PER_MS));
  el.style.left = `${Math.round(row.startMs * PX_PER_MS)}px`;
  el.style.width = `${width}px`;
  const zone = el.querySelector(".kf-diamonds");
  if (!zone) return;
  clear(zone);
  const kfs = (row.keyframes || []).slice()
    .sort((a, b) => (a.property === b.property
      ? a.timeMs - b.timeMs
      : (a.property < b.property ? -1 : 1)));
  kfs.forEach((k, idx) => {
    const x = (Math.max(0, Math.min(dur, k.timeMs)) / dur) * width;
    const d = h("button", {
      class: "kf-diamond", type: "button", testid: "kw-diamond",
      dataset: { prop: k.property, timeMs: String(k.timeMs), idx: String(idx) },
      title: `${propLabel(k.property)} @${k.timeMs}ms = ${round4(k.value)}(拖动移动 / 双击删除)`,
      "aria-label": `关键帧 ${propLabel(k.property)} ${k.timeMs}ms`,
    }, []);
    d.style.left = `${Math.round(x) - 5}px`;
    mountDrag(d, row.id, k);
    d.addEventListener("dblclick", (e) => {
      e.stopPropagation();
      delKf(row.id, k.property, k.timeMs);
    });
    zone.appendChild(d);
  });
}

function propLabel(p) {
  return (PROP_META[p] && PROP_META[p].label) || p;
}

function round4(v) {
  return Math.round(v * 10000) / 10000;
}

function rowById(clipId) {
  return timelineStore.get().clips.find((c) => c.id === clipId) || null;
}

/** 菱形拖拽(零 Op → 松手单 Op;Esc 取消回投影)。 */
function mountDrag(el, clipId, kf) {
  el.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const row0 = rowById(clipId);
    if (!row0) return;
    const draft = kfDraftOf(row0);
    const target = draft.find((k) => k.property === kf.property && k.timeMs === kf.timeMs);
    if (!target) return;
    const dur = Math.max(1, Math.round(row0.endMs - row0.startMs));
    const applyMove = (ev) => {
      const wrap = document.getElementById("timeline-wrap");
      if (!wrap) return;
      const absMs = (ev.clientX - wrap.getBoundingClientRect().left + wrap.scrollLeft) / PX_PER_MS;
      const t = moveKf(draft, target, absMs - row0.startMs, dur);
      const width = Math.max(6, Math.round(dur * PX_PER_MS));
      el.style.left = `${Math.round((t / dur) * width) - 5}px`;
      showBubble(ev.clientX, ev.clientY, `${propLabel(kf.property)} @${t}ms`);
    };
    runGesture(el, e, {
      move: applyMove,
      end: (ev) => {
        if (ev) applyMove(ev); // 补最后一帧(rAF 合帧可能吞掉收尾 move)
        hideBubble();
        updateClip(clipId, kfPatch(draft),
          `关键帧已移动:${propLabel(kf.property)} → ${target.timeMs}ms(可撤销)`);
        layoutRefresh();
      },
      cancel: () => { hideBubble(); layoutRefresh(); },
    });
  });
}

function layoutRefresh() {
  // 轻量自愈:投影未到前先把行几何复位(渲染订阅到达后会整体重排)
  const sel = selectionStore.get().clipId;
  if (!sel) return;
  const row = rowById(sel);
  const lane = document.querySelector(`[data-testid="track-lane-${row ? row.track : ""}"]`);
  if (lane && row) syncKfRow(lane, row, true);
}

function addAtPlayhead(clipId) {
  const row = rowById(clipId);
  if (!row) return;
  const prop = ephemeralStore.get().kfProp || "opacity";
  const t = snapMs(kfTimeAt(row, playheadMs()), true, frameMsOf(projectStore.get().project));
  const value = round4(currentValueOf(row, prop, null));
  const draft = kfDraftOf(row);
  upsertKf(draft, prop, t, value);
  updateClip(clipId, kfPatch(draft), `已打关键帧 ${propLabel(prop)} @${t}ms = ${value}(可撤销)`);
}

function delKf(clipId, propK, timeMs) {
  const row = rowById(clipId);
  if (!row) return;
  const draft = kfDraftOf(row);
  const target = draft.find((k) => k.property === propK && k.timeMs === timeMs);
  if (!target) return;
  dropKf(draft, target);
  if (emptyBlocked(draft)) {
    // 内核 minItems=1 拒空数组(实测):不发注定失败的写,诚实提示
    toast(CLEAR_BLOCKED_MSG, false);
    return;
  }
  updateClip(clipId, kfPatch(draft), `已删除关键帧 ${propLabel(propK)} @${timeMs}ms(可撤销)`);
}
