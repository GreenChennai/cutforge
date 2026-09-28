/* 工具提示(T2.5 三件套之一):data-tip 属性 → 单例浮层(原生 title 的样式化替代)。 */
import { h } from "./dom.js";

let tip = null;
let timer = null;

export function mountTooltip() {
  // 空单例常驻态挂 aria-hidden,免被读屏当作无名字提示原语;show() 解除,hide() 复挂。
  tip = h("div", {
    class: "cf-tooltip", role: "tooltip", "aria-hidden": "true", testid: "tooltip",
  });
  document.body.appendChild(tip);
  document.addEventListener("mouseover", onOver);
  document.addEventListener("mousedown", hide, true);
  document.addEventListener("scroll", hide, true);
}

function onOver(e) {
  const el = /** @type {HTMLElement} */ (e.target);
  const text = el && el.closest && el.closest("[data-tip]")?.getAttribute("data-tip");
  if (!text) {
    hide();
    return;
  }
  clearTimeout(timer);
  timer = setTimeout(() => show(text, /** @type {HTMLElement} */ (el.closest("[data-tip]"))), 350);
}

function show(text, anchor) {
  if (!tip) return;
  tip.textContent = text;
  tip.removeAttribute("aria-hidden");
  tip.classList.add("show");
  const rect = anchor.getBoundingClientRect();
  const w = tip.getBoundingClientRect().width;
  tip.style.left = `${Math.max(4, Math.min(rect.left + rect.width / 2 - w / 2, window.innerWidth - w - 8))}px`;
  tip.style.top = `${Math.max(4, rect.bottom + 6)}px`;
}

function hide() {
  clearTimeout(timer);
  if (tip) {
    tip.classList.remove("show");
    tip.setAttribute("aria-hidden", "true");
  }
}
