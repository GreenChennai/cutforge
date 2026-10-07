/* 审片元素拾取器(MV 审片台资产包 20261007 → cutforge 移植;铁律⑨/⑪)。
 * F12 式检查:Ctrl+Shift+C 进出,鼠标移动实时高亮 + 尺寸/坐标 tooltip,
 * 点击 = 把命中元素锚定成审片意见(落 notes_add,零新真相源)。
 *
 * 与原包的差异:拾取目标不是影片 canvas 的合成元素,而是壳 DOM(时间线片段/
 * 媒体卡/预览容器等)——cutforge 的画面是内核渲染帧(img/canvas),壳侧没有
 * 逐元素合成树;拾取器给意见提供 **DOM 锚 + atMs 快照**,意见正文仍由人写。
 * adapted from MV审片台资产包 review.js 检查模式, Apache-2.0/MIT 学习性移植。
 */
import { h } from "../ui/dom.js";
import { toast } from "../ui/toast.js";
import { playback } from "../render/preview-loop.js";

let active = false;
let overlay = null;
let tip = null;
let onPick = null; // (info) => void
let cleanupFns = [];

/** 当前播放头(审片意见的 atMs 锚;预览循环的时钟单一来源)。 */
function nowMs() {
  return Math.round(playback.clockMs());
}

/** 元素的可读锚描述(id/testid/类名链,取最稳定者)。 */
function describe(el) {
  if (el.dataset && el.dataset.testid) return `[data-testid="${el.dataset.testid}"]`;
  if (el.id) return `#${el.id}`;
  if (el.dataset && el.dataset.id) return `[data-id="${el.dataset.id}"]`;
  const cls = (el.className || "").toString().split(/\s+/).filter(Boolean).slice(0, 2);
  return cls.length ? `${el.tagName.toLowerCase()}.${cls.join(".")}` : el.tagName.toLowerCase();
}

function ensureOverlay() {
  if (overlay) return;
  overlay = h("div", { id: "review-inspect-overlay" });
  tip = h("div", { id: "review-inspect-tip" });
  document.body.appendChild(overlay);
  document.body.appendChild(tip);
}

function highlight(ev) {
  const el = document.elementFromPoint(ev.clientX, ev.clientY);
  if (!el || !overlay) return;
  const r = el.getBoundingClientRect();
  overlay.style.left = `${r.left}px`;
  overlay.style.top = `${r.top}px`;
  overlay.style.width = `${r.width}px`;
  overlay.style.height = `${r.height}px`;
  overlay.style.display = "block";
  tip.textContent = `${describe(el)}  ${Math.round(r.width)}×${Math.round(r.height)} @ ${Math.round(r.left)},${Math.round(r.top)}  · t=${nowMs()}ms`;
  tip.style.left = `${Math.min(ev.clientX + 14, window.innerWidth - 260)}px`;
  tip.style.top = `${ev.clientY + 16}px`;
  tip.style.display = "block";
  overlay.__target = el;
}

function stop() {
  active = false;
  for (const fn of cleanupFns) fn();
  cleanupFns = [];
  if (overlay) overlay.style.display = "none";
  if (tip) tip.style.display = "none";
  document.body.classList.remove("review-inspecting");
  if (onPick && overlay && overlay.__target) {
    // 最后一次点击的目标已由 click handler 消费;此处仅复位
    overlay.__target = null;
  }
}

/** 进入拾取模式。onPick({selector, rect, atMs}) 由调用方落成意见。 */
export function startInspect(onPickFn) {
  if (active) return stop();
  onPick = onPickFn;
  ensureOverlay();
  active = true;
  document.body.classList.add("review-inspecting");
  toast("元素拾取:点击画面元素 = 锚定审片意见;Esc 退出");

  const move = (ev) => highlight(ev);
  const click = (ev) => {
    const el = document.elementFromPoint(ev.clientX, ev.clientY);
    if (!el || el === overlay || el === tip) return;
    ev.preventDefault();
    ev.stopPropagation();
    const r = el.getBoundingClientRect();
    const info = {
      selector: describe(el),
      rect: { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) },
      atMs: nowMs(),
    };
    const cb = onPick;
    stop();
    if (cb) cb(info);
  };
  const key = (ev) => {
    if (ev.key === "Escape") {
      ev.preventDefault();
      stop();
    }
  };
  document.addEventListener("mousemove", move, true);
  document.addEventListener("click", click, true);
  document.addEventListener("keydown", key, true);
  cleanupFns.push(() => document.removeEventListener("mousemove", move, true));
  cleanupFns.push(() => document.removeEventListener("click", click, true));
  cleanupFns.push(() => document.removeEventListener("keydown", key, true));
}

export function isInspecting() {
  return active;
}
