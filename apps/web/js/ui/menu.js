/* 右键菜单(T2.5 三件套之一;册三 T3.6 扩为四上下文 + 键盘可达)。
 * 四上下文:片段 / 轨道 / 素材卡 / 时间线空白。
 * 每项可带 keys(快捷键提示)与 why(禁用原因 → title/aria-label,非颜色单线索)。
 * 键盘可达:打开即聚焦,↑↓ 移动、Enter/Space 激活、Home/End 首尾、Esc 关闭。 */
import { h } from "./dom.js";
import { selectionStore, timelineStore } from "../core/store.js";
import {
  splitSelected, splitAt, duplicateSelectedToPlayhead, deleteSelected,
  addTrack, browseMedia, setBgm, insertMediaAuto, playheadMs,
} from "../core/commands.js";
import { selectAllClips } from "../core/nav.js";
import { toggleTrackVisible } from "../render/timeline-view.js";
import { displayCombo, comboOf } from "./keymap-registry.js";
import { clearInOut, toggleMarker } from "./markers.js";
import { fitTimeline } from "./view-ops.js";
import { PX_PER_MS } from "../core/model.js";

let active = null;

/**
 * @param {number} x
 * @param {number} y
 * @param {Array<{ label: string, keys?: string, fn?: () => void, disabled?: boolean, why?: string, sep?: boolean }>} items
 */
export function openContextMenu(x, y, items) {
  closeContextMenu();
  const menu = h("div", { class: "cf-menu", role: "menu", testid: "context-menu", "aria-label": "上下文菜单" });
  const buttons = [];
  for (const item of items) {
    if (item.sep) {
      menu.appendChild(h("div", { class: "cf-menu-sep", role: "separator" }));
      continue;
    }
    const kids = [h("span", { class: "cf-menu-label" }, [item.label])];
    if (item.keys) kids.push(h("kbd", { class: "cf-menu-keys" }, [item.keys]));
    const btn = h("button", {
      role: "menuitem",
      disabled: item.disabled ? true : null,
      "aria-disabled": item.disabled ? "true" : null,
      title: item.disabled ? (item.why || "当前不可用") : (item.why || null),
      onclick: () => { closeContextMenu(); if (item.fn) item.fn(); },
    }, kids);
    menu.appendChild(btn);
    buttons.push(btn);
  }
  document.body.appendChild(menu);
  // 视口内夹取
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - rect.width - 8)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - rect.height - 8)}px`;
  active = menu;
  // 键盘可达:打开即聚焦首项;↑↓/Enter/Esc 在菜单内循环
  if (buttons.length) buttons[0].focus();
  menu.addEventListener("keydown", (e) => {
    const idx = buttons.indexOf(/** @type {HTMLElement} */ (document.activeElement));
    if (e.key === "ArrowDown") {
      e.preventDefault();
      buttons[Math.min(buttons.length - 1, idx + 1 < 0 ? 0 : idx + 1)]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      buttons[Math.max(0, idx <= 0 ? 0 : idx - 1)]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      buttons[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      buttons[buttons.length - 1]?.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeContextMenu();
    }
  });
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

/** 组合展示:从注册表读当前绑定(重绑定后提示跟着变)。 */
function keys(id, fallback) {
  const c = comboOf(id) || fallback;
  return c ? displayCombo(c) : "—";
}

/** ① 片段右键菜单(选中态先行;调用方保证已 set 选中)。
 * 分割(T3.7 盲测卡点修复):播放头在片段内 → 分割在播放头(=按 S 同义);
 * 否则右键点中片段内部 → 直接分割在右键位置(新手「对着哪切哪」心智)。
 * 两处都不在 → 禁用并给 title 说明怎么修。 */
export function openClipContextMenu(x, y) {
  const clipId = selectionStore.get().clipId;
  const has = Boolean(clipId);
  const row = has ? timelineStore.get().clips.find((c) => c.id === clipId) : null;
  const t = playheadMs();
  const clickMs = msAtClientX(x);
  const atPlayhead = Boolean(row) && t > row.startMs && t < row.endMs;
  const atClick = Boolean(row) && clickMs > row.startMs && clickMs < row.endMs;
  openContextMenu(x, y, [
    {
      label: "分割", keys: keys("clip.split", "S"),
      disabled: !has || (!atPlayhead && !atClick),
      why: !has ? "未选中片段"
        : (!atPlayhead && !atClick) ? "播放头与右键位置都不在片段内:先把播放头移进片段(点标尺),或右键点片段内部"
          : null,
      fn: () => (atPlayhead ? splitSelected() : splitAt(clipId, Math.round(clickMs))),
    },
    { label: "复制到播放头", keys: keys("clip.dup", "Ctrl+V"), fn: () => duplicateSelectedToPlayhead(), disabled: !has, why: !has ? "未选中片段" : null },
    { sep: true },
    { label: "删除", keys: keys("edit.delete", "Del"), fn: () => deleteSelected(false), disabled: !has, why: !has ? "未选中片段" : null },
    { label: "波纹删除", keys: keys("edit.rippleDelete", "Shift+Del"), fn: () => deleteSelected(true), disabled: !has, why: !has ? "未选中片段" : null },
  ]);
}

/** 视口客户坐标 → 时间线内容时刻 ms(与拖拽/框选同一显示映射)。 */
function msAtClientX(clientX) {
  const wrap = document.getElementById("timeline-wrap");
  if (!wrap) return 0;
  return Math.max(0, (clientX - wrap.getBoundingClientRect().left + wrap.scrollLeft) / PX_PER_MS);
}

/** ② 轨道右键菜单(轨头/轨道行空白)。 */
export function openTrackContextMenu(trackId, x, y) {
  const hidden = document
    .querySelector(`[data-testid="track-lane-${trackId}"]`)?.classList.contains("hidden-by-user");
  openContextMenu(x, y, [
    { label: hidden ? "显示此轨" : "隐藏此轨(仅视图)", keys: "眼睛", fn: () => toggleTrackVisible(trackId),
      why: "ephemeral 视图隐藏:不落盘/不参与撤销" },
    { sep: true },
    { label: "新增视频轨", keys: "+", fn: () => addTrack("video") },
    { label: "新增音频轨", fn: () => addTrack("audio") },
    { label: "新增文本轨", fn: () => addTrack("text") },
    { sep: true },
    { label: "删除此轨", disabled: true, why: "内核暂未提供 track_delete 工具(已登记遗留;可先隐藏此轨)" },
  ]);
}

/** ③ 时间线空白右键菜单。 */
export function openTimelineContextMenu(x, y) {
  const hasClip = Boolean(window.__cfClipboard);
  openContextMenu(x, y, [
    { label: "粘贴到播放头", keys: keys("edit.paste", "Ctrl+V"), fn: pasteClipboard, disabled: !hasClip, why: !hasClip ? "剪贴板为空:先选中片段按 Ctrl+C" : null },
    { label: "全选片段", keys: keys("edit.selectAll", "Ctrl+A"), fn: () => selectAllClips() },
    { sep: true },
    { label: "标记播放头位置", keys: keys("mark.marker", "M"), fn: () => toggleMarker() },
    { label: "清除入出点", fn: () => clearInOut() },
    { sep: true },
    { label: "适应窗口", keys: keys("view.fit", "\\"), fn: () => fitTimeline() },
    { label: "新增视频轨", fn: () => addTrack("video") },
    { label: "新增音频轨", fn: () => addTrack("audio") },
  ]);
}

function pasteClipboard() {
  const id = window.__cfClipboard;
  if (!id) return;
  import("../core/commands.js").then((c) => c.duplicateClip(id, c.playheadMs()));
}

/** ④ 素材卡右键菜单。 */
export function openMediaContextMenu(item, x, y) {
  openContextMenu(x, y, [
    { label: "插入到播放头", keys: "双击", fn: () => insertMediaAuto(item), why: "插到匹配轨型的播放头处(帧磁吸)" },
    item.kind === "audio"
      ? { label: "设为工程 BGM", fn: () => setBgm({ src: item.path }), why: "工程级背景乐(bgm_set,可撤销)" }
      : { label: "设为工程 BGM", disabled: true, why: "仅音频素材可设为 BGM" },
    { sep: true },
    { label: "刷新素材列表", fn: () => browseMedia(), why: "重新浏览当前目录" },
  ]);
}
