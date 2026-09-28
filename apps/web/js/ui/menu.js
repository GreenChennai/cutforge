/* 右键菜单(T2.5 三件套之一;时间线片段菜单用,册三供基础)。 */
import { h } from "./dom.js";
import { selectionStore } from "../core/store.js";
import { splitSelected, duplicateSelectedToPlayhead, deleteSelected } from "../core/commands.js";

let active = null;

/**
 * @param {number} x
 * @param {number} y
 * @param {Array<{ label: string, fn?: () => void, disabled?: boolean, sep?: boolean }>} items
 */
export function openContextMenu(x, y, items) {
  closeContextMenu();
  const menu = h("div", { class: "cf-menu", role: "menu", testid: "context-menu" });
  for (const item of items) {
    if (item.sep) {
      menu.appendChild(h("div", { class: "cf-menu-sep" }));
      continue;
    }
    menu.appendChild(h("button", {
      role: "menuitem",
      disabled: item.disabled ? true : null,
      onclick: () => { closeContextMenu(); if (item.fn) item.fn(); },
    }, [item.label]));
  }
  document.body.appendChild(menu);
  // 视口内夹取
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - rect.width - 8)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - rect.height - 8)}px`;
  active = menu;
  setTimeout(() => {
    document.addEventListener("mousedown", onDocDown, true);
    document.addEventListener("keydown", onKey, true);
  }, 0);
}

function onDocDown(e) {
  if (active && !active.contains(/** @type {Node} */ (e.target))) closeContextMenu();
}
function onKey(e) {
  if (e.key === "Escape") closeContextMenu();
}

export function closeContextMenu() {
  if (!active) return;
  active.remove();
  active = null;
  document.removeEventListener("mousedown", onDocDown, true);
  document.removeEventListener("keydown", onKey, true);
}

/** 时间线片段右键菜单(选中态先行;调用方保证已 set 选中)。 */
export function openClipContextMenu(x, y) {
  const has = Boolean(selectionStore.get().clipId);
  openContextMenu(x, y, [
    { label: "分割(S)", fn: () => splitSelected(), disabled: !has },
    { label: "复制到播放头", fn: () => duplicateSelectedToPlayhead(), disabled: !has },
    { sep: true },
    { label: "删除(Del)", fn: () => deleteSelected(false), disabled: !has },
    { label: "波纹删除(Shift+Del)", fn: () => deleteSelected(true), disabled: !has },
  ]);
}
