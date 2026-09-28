/* 用户偏好存取(T3.4/T3.7):localStorage 命名空间收口。
 * 纪律:这里只放「用户偏好」(键位表/界面开关),绝不放工程数据(IR/Oplog 落盘
 * 由内核负责);JSON 解析失败按缺省值兜底(隐私模式 localStorage 可能抛异常)。 */

const NS = "cutforge.prefs.v1";
/** @type {Object<string, *>} */
let cache = null;

function load() {
  if (cache) return cache;
  try {
    cache = JSON.parse(localStorage.getItem(NS) || "{}") || {};
  } catch {
    cache = {};
  }
  return cache;
}

function save() {
  try {
    localStorage.setItem(NS, JSON.stringify(cache));
  } catch { /* 隐私模式/配额:偏好退化为会话内生效 */ }
}

/** 读偏好(key 缺省返回 fallback)。 */
export function pref(key, fallback) {
  const v = load()[key];
  return v === undefined ? fallback : v;
}

/** 写偏好(持久化;跨刷新生效)。 */
export function setPref(key, value) {
  load()[key] = value;
  save();
}

/* ---- 具名偏好(T3.4 键位 + T3.7 界面开关)---- */

/** 键位覆盖表:id → combo(空串 = 已停用);不在表内 = 用默认。 */
export function keyOverrides() { return pref("keybinds", {}); }
export function setKeyOverrides(map) { setPref("keybinds", map); }

/** 批量操作前确认(T3.7;缺省开)。 */
export function confirmBatch() { return pref("confirmBatch", true); }
export function setConfirmBatch(on) { setPref("confirmBatch", Boolean(on)); }
