/* toast + 诊断锚点(E8 口径):顶部 toast 顶替单行 footer;#status 保留作 e2e 诊断锚点。 */
import { $, h } from "./dom.js";

const TOAST_MAX = 4;
const TOAST_TTL_MS = 4000;

/**
 * 反馈一条消息:写 #status(读屏/诊断)+ 弹 toast。
 * @param {string} msg
 * @param {boolean} ok false = 错误样式
 */
export function toast(msg, ok = true) {
  const statusEl = $("status");
  if (statusEl) statusEl.textContent = msg;
  const box = $("toasts");
  if (!box) return;
  const t = h("div", { class: ok ? "toast" : "toast err", testid: ok ? "toast" : "toast-err" }, [msg]);
  box.appendChild(t);
  while (box.children.length > TOAST_MAX) box.removeChild(box.firstChild);
  setTimeout(() => t.remove(), TOAST_TTL_MS);
}
