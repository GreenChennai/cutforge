/* 键位注册表(T3.4):绑定即数据——默认表 + localStorage 覆盖 + 冲突检测 + 全表导出。
 * 消费方:js/ui/keymap.js(安装调度)、帮助面板(全表+搜索)、设置面板(重绑定)、
 * 下波 e2e(遍历全表;window.__cfKeymap 挂导出面)。
 * 纪律:本模块不 import 任何带副作用的模块(prefs 除外),避免装配环。 */
import { keyOverrides, setKeyOverrides } from "./prefs.js";

/** 单条绑定:{ id, group, label, combo(默认), run }。combo="" 表示默认不绑。 */
const defs = [];

/** 注册一条绑定定义(keymap.js 装配期调用)。 */
export function defineBinding(id, group, label, combo, run) {
  defs.push({ id, group, label, combo, run });
}

/** 全表导出(帮助面板/e2e 遍历):[{id, group, label, defaultCombo, combo, keys}]。 */
export function exportTable() {
  return defs.map((d) => ({
    id: d.id,
    group: d.group,
    label: d.label,
    defaultCombo: d.combo,
    combo: comboOf(d.id),
  }));
}

/** 生效组合(用户覆盖 > 默认;空串 = 已停用)。 */
export function comboOf(id) {
  const ov = keyOverrides()[id];
  return ov !== undefined ? ov : (defs.find((d) => d.id === id)?.combo ?? "");
}

/** 展示形(格式化 kbd 文本):combo → "Ctrl+Z" 之类。 */
export function displayCombo(combo) {
  if (!combo) return "—";
  return combo.split("+").map((p) => {
    if (p === "ctrl") return "Ctrl";
    if (p === "shift") return "Shift";
    if (p === "alt") return "Alt";
    if (p === "space") return "Space";
    if (p === "arrowleft") return "←";
    if (p === "arrowright") return "→";
    if (p === "arrowup") return "↑";
    if (p === "arrowdown") return "↓";
    if (p === "enter") return "Enter";
    if (p === "escape") return "Esc";
    if (p === "tab") return "Tab";
    if (p === "delete") return "Del";
    if (p === "backspace") return "Bksp";
    if (p === "home") return "Home";
    if (p === "end") return "End";
    return p.toUpperCase();
  }).join("+");
}

/** 同组合的其他绑定 id(冲突检测;combo 空串不参与)。 */
export function findConflicts(combo, exceptId) {
  if (!combo) return [];
  const want = combo.toLowerCase();
  return defs
    .filter((d) => d.id !== exceptId && comboOf(d.id) === want)
    .map((d) => d.id);
}

/**
 * 写入一条覆盖(设置面板「仍要绑定」路径;冲突时调用方决定是否强制)。
 * force=true:同组合的既有绑定自动停用(组合唯一性由注册表保证)。
 * @returns {{ ok: boolean, conflicts: string[], disabled: string[] }}
 */
export function rebind(id, combo, force = false) {
  const conflicts = findConflicts(combo, id);
  const map = { ...keyOverrides() };
  const disabled = [];
  if (force) {
    for (const other of conflicts) {
      map[other] = "";
      disabled.push(other);
    }
  }
  map[id] = String(combo || "");
  setKeyOverrides(map);
  fireChange();
  return { ok: true, conflicts: force ? [] : conflicts, disabled };
}

/** 恢复全部默认(清覆盖表)。 */
export function resetAll() {
  setKeyOverrides({});
  fireChange();
}

/* ---- 变更通知(keymap 重装调度;设置面板免 import keymap 防装配环)---- */
/** @type {Set<() => void>} */
const changeCbs = new Set();
export function onOverridesChange(cb) { changeCbs.add(cb); return () => changeCbs.delete(cb); }
function fireChange() { for (const cb of changeCbs) cb(); }
