/* 右键菜单(T2.5 三件套之一;册三 T3.6 扩为四上下文 + 键盘可达;T4.2 轨道全功能)。
 * 四上下文:片段 / 轨道 / 素材卡 / 时间线空白。
 * 每项可带 keys(快捷键提示)与 why(禁用原因 → title/aria-label,非颜色单线索)。
 * 键盘可达:打开即聚焦,↑↓ 移动、Enter/Space 激活、Home/End 首尾、Esc 关闭。 */
import { h } from "./dom.js";
import { selectionStore, timelineStore, projectStore } from "../core/store.js";
import {
  splitSelected, splitAt, duplicateSelectedToPlayhead, deleteSelected,
  addTrack, browseMedia, setBgm, insertMediaAuto, playheadMs,
} from "../core/commands.js";
import {
  updateTrack, gapDelete, splitAllAt, copyClip, pasteClipAt, markHistory,
} from "../core/edit-commands.js";
import { selectAllClips } from "../core/nav.js";
import { toggleTrackVisible } from "../render/timeline-view.js";
import { trackColorChoices } from "../render/track-head.js";
import { proxyFor } from "../core/media-cache.js";
import { displayCombo, comboOf } from "./keymap-registry.js";
import { clearInOut, toggleMarker } from "./markers.js";
import { fitTimeline } from "./view-ops.js";
import { toast } from "./toast.js";
import { clipKindOf, targetTrackForKind, PX_PER_MS } from "../core/model.js";

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
 * T4.2 增:服务端剪贴板复制(clip_copy;粘贴 = clip_paste_at 带属性单 Op)。 */
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
    { label: "全轨分割(播放头)", keys: keys("clip.splitAll", "Shift+S"), fn: () => splitAllAt(t),
      why: "播放头处所有轨命中片段一次全分割(单 Op)" },
    { sep: true },
    { label: "复制(服务端剪贴板)", keys: keys("edit.copy", "Ctrl+C"), fn: () => copyClip(clipId),
      disabled: !has, why: !has ? "未选中片段" : "带属性复制;粘贴到任意同型轨(会话级剪贴板)" },
    { label: "粘贴到播放头", keys: keys("edit.paste", "Ctrl+V"), fn: () => pasteTo(row),
      disabled: !window.__cfClipboard, why: !window.__cfClipboard ? "剪贴板为空:先选中片段 Ctrl+C" : null },
    { label: "复制到播放头", keys: keys("clip.dup", "Ctrl+V"), fn: () => duplicateSelectedToPlayhead(), disabled: !has, why: !has ? "未选中片段" : null },
    { sep: true },
    { label: "删除", keys: keys("edit.delete", "Del"), fn: () => deleteSelected(false), disabled: !has, why: !has ? "未选中片段" : null },
    { label: "波纹删除", keys: keys("edit.rippleDelete", "Shift+Del"), fn: () => deleteSelected(true), disabled: !has, why: !has ? "未选中片段" : null },
  ]);
}

/** 粘贴(服务端剪贴板):目标轨 = 源片段轨型匹配;源已删则如实提示重拷。 */
function pasteTo(srcRow) {
  const id = window.__cfClipboard;
  if (!id) return;
  const row = srcRow && srcRow.id === id
    ? srcRow
    : timelineStore.get().clips.find((c) => c.id === id);
  if (!row) return;
  const trackId = targetTrackForKind(projectStore.get().project?.tracks || [], clipKindOf(row));
  pasteClipAt(trackId, Math.round(playheadMs()));
}

/** 视口客户坐标 → 时间线内容时刻 ms(与拖拽/框选同一显示映射)。 */
function msAtClientX(clientX) {
  const wrap = document.getElementById("timeline-wrap");
  if (!wrap) return 0;
  return Math.max(0, (clientX - wrap.getBoundingClientRect().left + wrap.scrollLeft) / PX_PER_MS);
}

/** ② 轨道右键菜单(T4.2 全功能:锁定/静音/独奏/隐藏(视图+合成)/重命名提示/
 * 标识色/清间隙;上移下移与删除 = 后端无工具,诚实禁用并登记遗留)。 */
export function openTrackContextMenu(trackId, x, y) {
  const track = (projectStore.get().project?.tracks || []).find((t) => t.id === trackId) || {};
  const isAudio = (track.kind || "video") === "audio";
  const laneCls = document
    .querySelector(`[data-testid="track-lane-${trackId}"]`)?.classList;
  const viewHidden = Boolean(laneCls?.contains("hidden-by-user"));
  const locked = Boolean(track.locked);
  const items = [
    {
      label: locked ? `解锁此轨` : "锁定此轨",
      fn: () => updateTrack(trackId, { locked: !locked }),
      why: "锁定后拒绝拖拽/裁剪/插入等编辑手势(track_update.locked)",
    },
    {
      label: viewHidden ? "显示此轨(视图)" : "隐藏此轨(仅视图)",
      keys: "眼睛", fn: () => toggleTrackVisible(trackId),
      why: "ephemeral 视图隐藏:不落盘/不参与撤销",
    },
    {
      label: track.hidden ? "取消合成隐藏" : "合成隐藏(不参与渲染)",
      fn: () => updateTrack(trackId, { hidden: !track.hidden }),
      why: "track_update.hidden(渲染联动候 BE3,先落字段)",
    },
    { sep: true },
    { label: "重命名(双击轨头名)", disabled: true, why: "双击轨头名称即可重命名(Enter 确认)" },
    ...(!isAudio ? [] : [
      {
        label: track.mute ? "取消静音" : "静音此轨",
        fn: () => updateTrack(trackId, { mute: !track.mute }),
        why: "track_update.mute(渲染混音联动候 BE3)",
      },
      {
        label: track.solo ? "取消独奏" : "独奏此轨",
        fn: () => updateTrack(trackId, { solo: !track.solo }),
        why: "track_update.solo:仅独奏轨出声(联动候 BE3)",
      },
    ]),
    {
      label: "删除此轨播放头处间隙",
      fn: () => gapDelete(trackId, Math.round(playheadMs())),
      why: "clip_gap_delete:删间隙并闭合后继(单 Op)",
    },
    { sep: true },
    {
      label: "标识色", disabled: true,
      why: "选择下方色项写入 track_update.color(可撤销)",
    },
    ...trackColorChoices().map(([value, name]) => ({
      label: `  ${name}`,
      fn: () => updateTrack(trackId, { color: value }),
    })),
    { sep: true },
    { label: "上移此轨", disabled: true, why: "内核暂未提供 track_reorder 工具(已登记遗留)" },
    { label: "下移此轨", disabled: true, why: "内核暂未提供 track_reorder 工具(已登记遗留)" },
    { label: "删除此轨", disabled: true, why: "内核暂未提供 track_delete 工具(已登记遗留;可先隐藏此轨)" },
    { sep: true },
    { label: "新增视频轨", keys: "+", fn: () => addTrack("video") },
    { label: "新增音频轨", fn: () => addTrack("audio") },
    { label: "新增文本轨", fn: () => addTrack("text") },
  ];
  openContextMenu(x, y, items);
}

/** ③ 时间线空白右键菜单(T4.2 增全轨分割/服务端剪贴板粘贴)。 */
export function openTimelineContextMenu(x, y) {
  const t = Math.round(playheadMs());
  const hasClip = Boolean(window.__cfClipboard);
  openContextMenu(x, y, [
    { label: "粘贴到播放头(带属性)", keys: keys("edit.paste", "Ctrl+V"), fn: () => pasteTo(null), disabled: !hasClip, why: !hasClip ? "剪贴板为空:先选中片段按 Ctrl+C" : "clip_paste_at:带属性粘贴(单 Op)" },
    { label: "全选片段", keys: keys("edit.selectAll", "Ctrl+A"), fn: () => selectAllClips() },
    { label: "全轨分割(播放头)", keys: keys("clip.splitAll", "Shift+S"), fn: () => splitAllAt(t),
      why: "播放头处所有轨命中片段一次全分割(单 Op)" },
    { sep: true },
    { label: "标记播放头位置", keys: keys("mark.marker", "M"), fn: () => toggleMarker() },
    { label: "清除入出点", fn: () => clearInOut() },
    { label: "打历史快照标记", fn: () => markHistory("手动快照"),
      why: "历史面板分隔线(会话态,不落盘)" },
    { sep: true },
    { label: "适应窗口", keys: keys("view.fit", "\\"), fn: () => fitTimeline() },
    { label: "新增视频轨", fn: () => addTrack("video") },
    { label: "新增音频轨", fn: () => addTrack("audio") },
  ]);
}

/** ④ 素材卡右键菜单(T4.1 增:复制路径/生成代理;「在资源管理器打开」诚实降级——
 * 浏览器沙箱无此能力,不假实现)。 */
export function openMediaContextMenu(item, x, y) {
  openContextMenu(x, y, [
    { label: "插入到播放头", keys: "双击", fn: () => insertMediaAuto(item), why: "插到匹配轨型的播放头处(统一吸附)" },
    item.kind === "audio"
      ? { label: "设为工程 BGM", fn: () => setBgm({ src: item.path }), why: "工程级背景乐(bgm_set,可撤销)" }
      : { label: "设为工程 BGM", disabled: true, why: "仅音频素材可设为 BGM" },
    { sep: true },
    { label: "复制完整路径", fn: () => copyPath(item),
      why: "写系统剪贴板(需浏览器权限;失败时路径入 toast 可手选复制)" },
    { label: "生成代理(1/2 分辨率)", fn: () => generateProxy(item),
      why: "media_proxy:供导出「用代理预览」消费(导出默认原片)" },
    { label: "在资源管理器打开", disabled: true,
      why: "浏览器沙箱无此能力(诚实降级);可用「复制完整路径」后手动打开" },
    { sep: true },
    { label: "刷新素材列表", fn: () => browseMedia(), why: "重新浏览当前目录" },
  ]);
}

/** 复制路径:clipboard API 优先,失败降级 toast 展示路径(不假报成功)。 */
function copyPath(item) {
  const path = item.path;
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(path).then(
      () => toast(`已复制路径:${path}`),
      () => toast(`复制失败(权限受限),路径:${path}`, false),
    );
  } else {
    toast(`当前环境不支持剪贴板 API,路径:${path}`, false);
  }
}

/** 生成代理:media_proxy generate(ffmpeg 子进程,可能数秒;完成后回报状态)。 */
function generateProxy(item) {
  if (item.kind === "audio" || item.kind === "image") {
    toast("代理仅对视频素材有意义(音频/图片无需代理)", false);
    return;
  }
  toast("代理生成中(ffmpeg,数秒至数分钟,取决于素材大小)…");
  proxyFor(item.path, true).then((r) => {
    if (r.ok && r.state === "ready") toast("代理已就绪(导出面板勾选「用代理预览」生效)");
    else toast(`代理不可用:${r.message || r.state}`, false);
  });
}
