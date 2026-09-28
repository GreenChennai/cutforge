/* 快捷键调度器(T2.5;册三 T3.4 完整体系预置)。
 * 约定:INPUT/TEXTAREA/SELECT 或对话框打开时不触发;组合键归一化 ctrlOrMeta。 */

/** @type {Map<string, (e: KeyboardEvent) => void>} */
const routes = new Map();
let bound = false;

/** @param {string} combo 例如 " ", "s", "ArrowLeft", "Delete", "ctrl+z", "shift+Delete", "Home" */
export function registerShortcut(combo, fn) {
  routes.set(normalize(combo), fn);
  if (!bound) {
    bound = true;
    document.addEventListener("keydown", dispatchKey);
  }
}

export function unregisterShortcut(combo) {
  routes.delete(normalize(combo));
}

function normalize(combo) {
  return combo.toLowerCase().replace(/\s+/g, "");
}

function keyOf(e) {
  const parts = [];
  if (e.ctrlKey || e.metaKey) parts.push("ctrl");
  if (e.shiftKey) parts.push("shift");
  if (e.altKey) parts.push("alt");
  parts.push(e.key === " " ? "space" : e.key);
  return normalize(parts.join("+"));
}

function dispatchKey(e) {
  const t = /** @type {HTMLElement} */ (e.target);
  if (t && ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName)) return;
  if (t && t.isContentEditable) return;
  const fn = routes.get(keyOf(e));
  if (fn) {
    e.preventDefault();
    fn(e);
  }
}
