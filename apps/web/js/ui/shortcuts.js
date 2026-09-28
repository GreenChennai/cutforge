/* 快捷键调度器(T2.5 基座,册三 T3.4 升级):组合键归一化 + 输入态/模态屏蔽。
 * 约定:
 * - INPUT/TEXTAREA/SELECT/contentEditable 聚焦时不触发(既有行为,e2e 兼容);
 * - 对话框打开时不触发(dialog.js 声明的既有约定,此版落实);
 * - Enter/Space 是按钮激活键:焦点在交互元素上时放行原生激活(可访问性);
 * - shift 修饰:命名键与字母键记 shift(Shift+D、Shift+Del、Shift+←);
 *   移位符号按产出字符记账(Shift+/ 即 "?"),字母恒小写(无大小写双绑定);
 * - 每次按键把屏蔽原因写进 #shortcut-gate(testid=shortcut-gate,e2e 可断言):
 *   gate="input"|"dialog"|"native"|"hit:<组合>"|"miss:<组合>"。
 */
import { dialogOpen } from "./dialog.js";

/** @type {Map<string, (e: KeyboardEvent) => void>} */
const routes = new Map();
let bound = false;
let gateEl = null;

function gate() {
  if (!gateEl || !gateEl.isConnected) {
    gateEl = document.querySelector('[data-testid="shortcut-gate"]');
  }
  return gateEl;
}

function setGate(reason) {
  const el = gate();
  if (el) el.setAttribute("data-gate", reason);
}

/** @param {string} combo 例如 " ", "s", "ArrowLeft", "Delete", "ctrl+z", "shift+Delete", "?" */
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

/** 清空全部路由(重绑定/恢复默认后整体重装)。 */
export function clearRoutes() {
  routes.clear();
}

/** 当前已注册组合键数(自测/帮助面板对账)。 */
export function routeCount() {
  return routes.size;
}

export function normalize(combo) {
  return String(combo).toLowerCase().replace(/\s+/g, "");
}

/** KeyboardEvent → 归一化组合串(与 registerShortcut 同一口径;键位捕获控件复用)。
 * shift 修饰规则:命名键与字母键记 shift(Shift+D → "shift+d"、Shift+Del → "shift+delete");
 * 移位符号按产出字符记账(Shift+/ 即 "?"),字母恒小写(无大小写双绑定)。
 * 兼容口径:Playwright/部分合成事件对 Shift+/ 上报 key="/" 而非 "?",此处归一到 "?"。 */
export function keyOf(e) {
  const parts = [];
  if (e.ctrlKey || e.metaKey) parts.push("ctrl");
  if (e.altKey) parts.push("alt");
  const k = e.key;
  const isLetter = k.length === 1 && /[a-z]/i.test(k);
  if (e.shiftKey && (k.length > 1 || isLetter)) parts.push("shift");
  let base = k === " " ? "space" : k.toLowerCase();
  if (e.shiftKey && base === "/") base = "?"; // Shift+/ 统一记作 "?"
  parts.push(base);
  return normalize(parts.join("+"));
}

/** 焦点是否在交互元素上(按钮/链接等;Enter/Space 放行原生激活用)。 */
function onActivable(t) {
  return Boolean(t && t.closest && t.closest("button, a, select, summary, [role='button']"));
}

function dispatchKey(e) {
  const key = keyOf(e);
  const t = /** @type {HTMLElement} */ (e.target);
  if (t && ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName)) return setGate("input");
  if (t && t.isContentEditable) return setGate("input");
  if (dialogOpen()) return setGate("dialog");
  // 激活键让位:焦点在按钮/链接上时 Enter/Space 走原生激活(键盘可操作性优先)
  if ((key === "enter" || key === "space") && onActivable(t)) return setGate("native");
  const fn = routes.get(key);
  if (fn) {
    e.preventDefault();
    setGate(`hit:${key}`);
    fn(e);
  } else {
    setGate(`miss:${key}`);
  }
}
