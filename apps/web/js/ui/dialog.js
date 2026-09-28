/* 对话框(W8 修复):role=dialog + aria-modal + 焦点陷阱 + Esc/遮罩关闭 + 焦点归还。 */
import { h, clear } from "./dom.js";

let openStack = [];

/**
 * 打开模态对话框。
 * @param {{ id?: string, title: string, build: (body: HTMLElement) => void, onClose?: () => void }} opts
 * @returns {{ close: () => void, root: HTMLElement }}
 */
export function openDialog(opts) {
  const prevFocus = /** @type {HTMLElement} */ (document.activeElement);
  const mask = h("div", {
    class: "cf-mask wizard-mask", id: opts.id || null, testid: opts.id || "dialog",
    role: "presentation",
  });
  const card = h("div", {
    class: "cf-dialog wizard-card panel", role: "dialog", "aria-modal": "true",
    "aria-label": opts.title, tabindex: "-1",
  });
  card.appendChild(h("h3", null, [opts.title]));
  const body = h("div", { class: "cf-dialog-body" });
  card.appendChild(body);
  mask.appendChild(card);

  const api = { root: mask, closed: false };
  function close() {
    if (api.closed) return;
    api.closed = true;
    mask.remove();
    openStack = openStack.filter((d) => d !== api);
    document.removeEventListener("keydown", onKey, true);
    if (prevFocus && prevFocus.isConnected) prevFocus.focus();
    if (opts.onClose) opts.onClose();
  }
  api.close = close;

  function onKey(e) {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
      return;
    }
    if (e.key === "Tab") trapFocus(e, card);
  }

  // 点击遮罩空白处关闭(点击卡片内部不关)
  mask.addEventListener("mousedown", (e) => {
    if (e.target === mask) close();
  });
  document.addEventListener("keydown", onKey, true);
  opts.build(body);
  document.body.appendChild(mask);
  openStack.push(api);
  // 初始焦点:第一个可聚焦元素;一个都没有 → 卡片自身(tabindex=-1 兜底,焦点不出模态)
  const first = card.querySelector("input, select, textarea, button");
  if (first) first.focus();
  else card.focus();
  return api;
}

/** 焦点陷阱:Tab 循环限制在对话框内。 */
function trapFocus(e, card) {
  const focusables = [...card.querySelectorAll("button, input, select, textarea, a[href]")];
  if (!focusables.length) return;
  const first = focusables[0];
  const last = focusables[focusables.length - 1];
  const active = /** @type {HTMLElement} */ (document.activeElement);
  if (e.shiftKey && (active === first || !card.contains(active))) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && active === last) {
    e.preventDefault();
    first.focus();
  }
}

/** 当前是否有对话框打开(快捷键抑制用)。 */
export function dialogOpen() {
  return openStack.length > 0;
}
