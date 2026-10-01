/* 复合片段壳侧(T5.4,ADR-0019):右键打包/解包 + 时间线徽标数据 + 双击说明卡。
 * 诚实口径(内核语义如此):复合片段的「内部编辑」= 解包(compound_unbind)→ 编辑
 * 子片段 → 重新打包(compound_create)的引导流——子 clips 不随投影下发(载荷纪律),
 * 说明卡只呈投影概要(clipCount/durationMs/canvas),不假造子片段列表。
 * 菜单动作从这里出(pro-commands 承接命令),timeline-view 双击只调 openCompoundCard。 */
import { h } from "../ui/dom.js";
import { selectionStore, timelineStore } from "../core/store.js";
import { compoundCreate, compoundUnbind, compoundPrecheck } from "../core/pro-commands.js";
import { openDialog } from "../ui/dialog.js";
import { toast } from "../ui/toast.js";

/** 当前多选集是否可打包(菜单禁用态与 why 文案;前端拦截与后端 GUARD 同向)。
 * 主选中不在框选集内(右键点了他处片段)→ 视为单选:按通用约定不沿用旧框选集。 */
export function packState() {
  const sel = selectionStore.get();
  const ids = (sel.clipIds || []).includes(sel.clipId)
    ? sel.clipIds
    : (sel.clipId ? [sel.clipId] : []);
  const pre = compoundPrecheck(ids);
  return { ids, ok: !pre.err, why: pre.err || `compound_create:${ids.length} 段 → 单壳片段(单 Op,可撤销)` };
}

/** 打包(取当前多选集;前置校验失败 toast 说明)。 */
export function packCompound() {
  const { ids } = packState();
  return compoundCreate(ids);
}

/** 解包(选中片段为复合时可用)。 */
export function unbindCompound(clipId) {
  return compoundUnbind(clipId);
}

/**
 * 复合片段说明卡(时间线双击弹;右键「复合片段说明」同源)。
 * 内容 = 投影 compound 概要 + 解包按钮 + 「内部编辑=解包流」引导(诚实标注)。
 * @param {string} clipId
 */
export function openCompoundCard(clipId) {
  const row = timelineStore.get().clips.find((c) => c.id === clipId);
  if (!row || !row.compound) {
    toast("该片段不是复合片段", false);
    return;
  }
  const cp = row.compound;
  openDialog({
    id: "compound-card",
    title: `复合片段 ${clipId}(T5.4;ADR-0019 内联子时间线)`,
    build: (body) => {
      body.appendChild(h("div", { class: "cpd-grid", testid: "cpd-summary" }, [
        h("span", { class: "dim" }, ["子片段数"]),
        h("b", null, [String(cp.clipCount ?? "-")]),
        h("span", { class: "dim" }, ["子时间线总时长"]),
        h("b", { class: "mono" }, [`${cp.durationMs ?? "-"}ms`]),
        h("span", { class: "dim" }, ["子画幅 canvas"]),
        h("b", { class: "mono" }, [cp.canvas ? `${cp.canvas.width}×${cp.canvas.height}` : "(随工程)"]),
        h("span", { class: "dim" }, ["主时间线落位"]),
        h("b", { class: "mono" }, [`${row.startMs}ms · 轨 ${row.track}`]),
      ]));
      body.appendChild(h("div", { class: "hint", testid: "cpd-hint" }, [
        "内部编辑 = 解包流(后端语义如此,如实标注):「解包」把子片段平移回主时间线"
        + "(id 重分配,单 Op 可撤销)→ 正常编辑 → 重新框选「打包为复合片段」。"
        + "复合不可嵌套(深度上限两级);子 clips 全量不随投影下发(载荷纪律),本卡只呈概要。",
      ]));
      const actions = h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "cpd-unbind", title: "compound_unbind:解包还原(单 Op,可撤销)",
          onclick: () => {
            document.getElementById("compound-card")?.remove();
            unbindCompound(clipId);
          },
        }, ["解包(compound_unbind)"]),
        h("button", { testid: "cpd-close", onclick: () => document.getElementById("compound-card")?.remove() }, ["关闭"]),
      ]);
      body.appendChild(actions);
    },
  });
}
