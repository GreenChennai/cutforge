/* 检查器秒表(册五 T5.1 关键帧 UI · 打点面)。
 *
 * 交互契约(计划书 T5.1 壳侧):
 * - 可动画属性旁秒表按钮:开启即打「播放头时刻」关键帧(值 = 检查器草稿值,缺省投影值);
 * - 再点取消:确认弹窗 → 移除该属性全部关键帧(整组写回,单 Op);
 * - position.x/y 无独立输入字段(只读投影):专属行展示现值 + 两枚秒表(值换算自投影);
 * - testid=kw-toggle-<prop>;全程零插值、零额外 Op(一笔 clip_update)。
 */
import { h } from "../ui/dom.js";
import { timelineStore, selectionStore, projectStore } from "../core/store.js";
import { updateClip, playheadMs } from "../core/commands.js";
import { snapMs, frameMsOf } from "../core/model.js";
import {
  WATCHABLE, PROP_META, propAnimated, kfsOfProp, currentValueOf, kfTimeAt,
  draftOf as kfDraftOf, upsertKf, removePropKfs, kfPatch,
} from "../core/kf-model.js";
import { openDialog } from "../ui/dialog.js";
import { toast } from "../ui/toast.js";

/** @type {Set<() => void>} 秒表状态刷新器(投影/选中变化时统一刷新;模块级单订阅)。 */
const refreshers = new Set();
let subscribed = false;

function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  const refreshAll = () => { for (const fn of refreshers) fn(); };
  timelineStore.subscribe(refreshAll);
  selectionStore.subscribe(refreshAll);
}

/** 当前选中片段行。 */
function rowNow(rowOf) {
  const row = rowOf ? rowOf() : null;
  if (row) return row;
  const id = selectionStore.get().clipId;
  return id ? timelineStore.get().clips.find((c) => c.id === id) || null : null;
}

/** 打点时刻(帧磁吸;磁吸关 = 原值圆整)。 */
function snappedKfTime(row) {
  const raw = kfTimeAt(row, playheadMs());
  return snapMs(raw, true, frameMsOf(projectStore.get().project));
}

/** 秒表开关主逻辑:on=打点 / off=确认后整属性移除(整组清空受阻时诚实告知)。 */
async function toggleWatch(prop, rowOf, draftOf) {
  const row = rowNow(rowOf);
  if (!row) { toast("先选中片段(点时间线片段)", false); return; }
  const list = kfsOfProp(row, prop);
  if (!list.length) {
    const value = currentValueOf(row, prop, draftOf);
    const draft = kfDraftOf(row);
    upsertKf(draft, prop, snappedKfTime(row), value);
    const t = kfTimeAt(row, playheadMs());
    updateClip(row.id, kfPatch(draft), `已打关键帧 ${propLabel(prop)} @${Math.round(t)}ms = ${round4(value)}(可撤销)`);
    return;
  }
  const isLast = kfsOfProp(row, prop).length === ((row.keyframes || []).length);
  const yes = await confirmKfOff(prop, list.length, isLast);
  if (!yes) return;
  const draft = kfDraftOf(row);
  removePropKfs(draft, prop);
  // 空数组 = 清除全部关键帧(册五收口:schema minItems 已移除,内核接受 [])
  updateClip(row.id, kfPatch(draft), `已移除 ${propLabel(prop)} 全部关键帧(${list.length} 帧,可撤销)`);
}

function propLabel(prop) {
  return (PROP_META[prop] && PROP_META[prop].label) || prop;
}

function round4(v) {
  return Math.round(v * 10000) / 10000;
}

function confirmKfOff(prop, count, isLast) {
  return new Promise((resolve) => {
    let settled = false;
    const done = (v) => { if (!settled) { settled = true; resolve(v); } };
    const dlg = openDialog({
      id: "confirm-dialog",
      title: `移除「${propLabel(prop)}」的全部关键帧?`,
      onClose: () => done(false),
      build: (body) => {
        body.appendChild(h("p", null, [`该属性共 ${count} 个关键帧,移除后按整组替换写回(单笔 Op,可撤销)。`]));
        if (isLast) {
          body.appendChild(h("p", { class: "warn-hint" }, [
            "注意:这是工程内最后一个含关键帧的属性——移除后该片段将没有任何关键帧(整组清空 = 空数组,单笔 Op 可撤销)。",
          ]));
        }
        const ok = h("button", { testid: "confirm-ok" }, [isLast ? "知道了(仍不生效)" : "移除"]);
        const cancel = h("button", { testid: "confirm-cancel" }, ["取消"]);
        ok.addEventListener("click", () => { done(!isLast); dlg.close(); });
        cancel.addEventListener("click", () => { done(false); dlg.close(); });
        body.appendChild(h("div", { class: "wizard-actions" }, [ok, cancel]));
        ok.focus();
      },
    });
  });
}

/**
 * 秒表按钮(检查器字段旁挂载;状态随投影/选中自动刷新)。
 * @param {string} prop 白名单属性键
 * @param {() => Object|null} rowOf
 * @param {() => *} [draftOf] 检查器草稿值读取(打点取当前输入值)
 * @returns {HTMLElement}
 */
export function watchButton(prop, rowOf, draftOf) {
  ensureSubscribed();
  const btn = h("button", {
    type: "button",
    class: "kw-watch",
    testid: `kw-toggle-${prop.replace(/\./g, "-")}`,
    title: `关键帧:开启即打播放头时刻关键帧(值=当前输入);再点移除该属性全部关键帧`,
    "aria-label": `关键帧秒表 ${propLabel(prop)}`,
    onclick: () => toggleWatch(prop, rowOf, draftOf),
  }, ["⏱"]);
  const refresh = () => {
    const row = rowNow(rowOf);
    const on = Boolean(row && propAnimated(row, prop));
    btn.classList.toggle("on", on);
    btn.setAttribute("aria-pressed", on ? "true" : "false");
    btn.dataset.state = on ? "on" : "off";
  };
  refreshers.add(refresh);
  refresh();
  return btn;
}

/**
 * 位置打点行(position.x/y 只读投影 → 专属秒表行;画面组附加面)。
 * @param {() => Object|null} rowOf
 */
export function positionWatchRow(rowOf) {
  const readout = h("span", { class: "dim kw-pos-read", testid: "kw-position-read" }, ["-"]);
  const row = h("div", { class: "kw-pos-row", testid: "kw-position-row" }, [
    h("span", { class: "dim" }, ["位置(画布 %)"]),
    readout,
    watchButton("position.x", rowOf),
    watchButton("position.y", rowOf),
  ]);
  const refresh = () => {
    const r = rowNow(rowOf);
    readout.textContent = r && r.position
      ? `x=${round4(Number(r.position.x))} y=${round4(Number(r.position.y))}`
      : "x=50 y=50(缺省居中)";
  };
  refreshers.add(refresh);
  refresh();
  return row;
}

/** 检查器字段 → 秒表挂载(字段名在 WATCHABLE 内则返回秒表按钮,否则 null)。 */
export function watchForField(field, rowOf, draftOf) {
  if (!WATCHABLE.includes(field)) return null;
  return watchButton(field, rowOf, draftOf);
}
