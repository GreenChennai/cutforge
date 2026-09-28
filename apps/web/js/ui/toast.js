/* toast + 诊断锚点(E8 口径):顶部 toast 顶替单行 footer;#status 保留作 e2e 诊断锚点。
 * T3.2 微交互:滑入(css rise-in)+ 自动淡出(TTL 前 160ms 挂 .out,transform/opacity)。
 * T3.7 升级:action toast——带「撤销」按钮的真撤销入口(点按钮调既有 undo),
 * 带 action 时 TTL 延长到 5s(预算:5s 内可撤销);aria-live 由容器承担。 */
import { $, h } from "./dom.js";

const TOAST_MAX = 4;
const TOAST_TTL_MS = 4000;
const TOAST_ACTION_TTL_MS = 5000; // T3.7:可撤销 toast 的 5s 窗口
const FADE_MS = 180; // 与 .toast.out 过渡时长匹配(css 160ms + 余量)

/**
 * 反馈一条消息:写 #status(读屏/诊断)+ 弹 toast。
 * @param {string} msg
 * @param {boolean} ok false = 错误样式
 * @param {{ action?: { label: string, fn: () => void }, testid?: string }} [opts]
 * @returns {HTMLElement|null}
 */
export function toast(msg, ok = true, opts = null) {
  const statusEl = $("status");
  if (statusEl) statusEl.textContent = msg;
  const box = $("toasts");
  if (!box) return null;
  const t = h("div", { class: ok ? "toast" : "toast err", testid: opts?.testid || (ok ? "toast" : "toast-err") }, [msg]);
  if (opts && opts.action) {
    const btn = h("button", {
      class: "toast-action", testid: "toast-action",
      onclick: () => {
        clearTimeouts(t);
        t.remove();
        opts.action.fn();
      },
    }, [opts.action.label]);
    t.appendChild(btn);
    t.classList.add("has-action");
  }
  t.__cfTimers = [
    setTimeout(() => t.classList.add("out"), (opts?.action ? TOAST_ACTION_TTL_MS : TOAST_TTL_MS) - FADE_MS),
    setTimeout(() => t.remove(), opts?.action ? TOAST_ACTION_TTL_MS : TOAST_TTL_MS),
  ];
  box.appendChild(t);
  while (box.children.length > TOAST_MAX) box.removeChild(box.firstChild);
  return t;
}

function clearTimeouts(t) {
  for (const id of t.__cfTimers || []) clearTimeout(id);
  t.__cfTimers = [];
}
