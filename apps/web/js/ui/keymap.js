/* 编辑器键位(T3.4 完整体系;对齐达芬奇/剪映心智):
 * - 绑定即数据:全部经 keymap-registry 注册(可导出全表 ≥40 条,供帮助面板/设置/下波 e2e);
 * - 全部可重绑定:设置(Ctrl+,)捕获新键 → 冲突检测 → 恢复默认;覆盖存 localStorage;
 * - J/K/L 倒放/正放按倍速链(J×2=2x 倒放);I/O 入出点接 selectionStore 既有字段;
 *   M 标记为会话级 ephemeral(不落盘不进 IR;持久化判定见 ui/markers.js 头注);
 * - 输入态/模态屏蔽在 shortcuts.js 调度器;每个按键的裁决可经 testid=shortcut-gate 断言。 */
import { selectionStore, timelineStore, projectStore, uiStore, playbackStore } from "../core/store.js";
import { clipKindOf, targetTrackForKind } from "../core/model.js";
import * as commands from "../core/commands.js";
import * as nav from "../core/nav.js";
import * as edit from "../core/edit-commands.js";
import { playback } from "../render/preview-loop.js";
import { registerShortcut, clearRoutes, routeCount, keyOf } from "./shortcuts.js";
import { defineBinding, comboOf, exportTable, onOverridesChange } from "./keymap-registry.js";
import { toggleMarker, prevMarker, nextMarker, setInOut } from "./markers.js";
import { zoomBy, fitTimeline, togglePreviewFullscreen, togglePanels, focusExportPanel } from "./view-ops.js";
import { openHelpPanel } from "./help-panel.js";
import { openSettings } from "./settings-panel.js";
import { togglePerfPanel } from "./perf-panel.js";
import { openClipContextMenu } from "./menu.js";
import * as textool from "../panels/textool.js";

/* ---- 播放链(J/L 倍速 ×2,上限 8x;K/空格复位 1x)---- */
const SPEED_MAX = 8;

function playForwardChain() {
  const { playing, speed } = playbackStore.get();
  if (!playing || speed <= 0) {
    playback.setRate(1);
    playback.setPlaying(true);
  } else {
    playback.setRate(Math.min(SPEED_MAX, speed * 2));
  }
}

function playReverseChain() {
  const { playing, speed } = playbackStore.get();
  if (!playing || speed >= 0) {
    playback.setRate(-1);
    playback.setPlaying(true);
  } else {
    playback.setRate(-Math.min(SPEED_MAX, Math.abs(speed) * 2));
  }
}

/** K:播放中 → 暂停(并复位倍速);已暂停 → 复位倍速。 */
function pauseAndReset() {
  if (playbackStore.get().playing) playback.setPlaying(false);
  else playback.setRate(1);
}

/** 键盘打开片段上下文菜单(Enter;锚到选中片段可视位置)。 */
function openClipMenuKeyboard() {
  const id = selectionStore.get().clipId;
  const el = id ? document.querySelector(`.clip[data-id="${id}"]`) : null;
  if (!el) {
    nav.selectSibling(1); // 无选中:选中第一个片段(下次 Enter 开菜单)
    return;
  }
  const r = /** @type {HTMLElement} */ (el).getBoundingClientRect();
  openClipContextMenu(r.left + 8, r.bottom + 4);
}

/** 服务端剪贴板(T4.2):Ctrl+C = clip_copy(不产 Op);观察面 __cfClipboard 兼容保留。 */
function serverCopy() {
  const id = selectionStore.get().clipId;
  if (!id) return;
  window.__cfClipboard = id;
  edit.copyClip(id);
}

/** Ctrl+V = clip_paste_at(带属性;单 Op)。源片段行用于定位轨型(被删后如实提示)。 */
function serverPaste() {
  const id = window.__cfClipboard;
  if (!id) return;
  const row = timelineStore.get().clips.find((c) => c.id === id);
  if (!row) {
    import("../ui/toast.js").then((m) => m.toast("剪贴板来源已不在时间线,请重新复制", false));
    return;
  }
  const tracks = projectStore.get().project?.tracks || [];
  const trackId = targetTrackForKind(tracks, clipKindOf(row));
  edit.pasteClipAt(trackId, Math.round(commands.playheadMs()));
}

/** RUN 表:id → 执行(重绑定只改组合,不改语义)。 */
const RUN = {
  "play.toggle": () => commands.togglePlay(),
  "play.j": () => playReverseChain(),
  "play.k": () => pauseAndReset(),
  "play.l": () => playForwardChain(),
  "play.frameBack": () => commands.seekFrame(-1),
  "play.frameFwd": () => commands.seekFrame(1),
  "play.secBack": () => commands.seekTo(commands.playheadMs() - 1000),
  "play.secFwd": () => commands.seekTo(commands.playheadMs() + 1000),
  "play.home": () => commands.toStart(),
  "play.end": () => commands.toEnd(),
  "mark.in": () => setInOut("in"),
  "mark.out": () => setInOut("out"),
  "mark.marker": () => toggleMarker(),
  "mark.prev": () => prevMarker(),
  "mark.next": () => nextMarker(),
  "mode.select": () => nav.setTool("select"),
  "mode.blade": () => nav.setTool(uiStore.get().tool === "blade" ? "select" : "blade"),
  "mode.trim": () => nav.setTool(uiStore.get().tool === "trim" ? "select" : "trim"),
  "text.add": () => textool.addTextAtPlayhead(),
  "clip.split": () => commands.splitSelected(),
  "clip.splitAll": () => edit.splitAllAt(commands.playheadMs()),
  "edit.delete": () => commands.deleteSelected(uiStore.get().ripple),
  "edit.deleteBksp": () => commands.deleteSelected(uiStore.get().ripple),
  "edit.rippleDelete": () => commands.deleteSelected(true),
  "edit.copy": () => serverCopy(),
  "edit.cut": () => { serverCopy(); commands.deleteSelected(false); },
  "edit.paste": () => serverPaste(),
  "edit.undo": () => commands.undo(),
  "edit.redo": () => commands.redo(),
  "edit.selectAll": () => nav.selectAllClips(),
  "clip.nudgeBack": () => nav.nudgeSelected(-1),
  "clip.nudgeFwd": () => nav.nudgeSelected(1),
  "clip.nudgeBack10": () => nav.nudgeSelected(-10),
  "clip.nudgeFwd10": () => nav.nudgeSelected(10),
  "clip.prev": () => nav.selectSibling(-1),
  "clip.next": () => nav.selectSibling(1),
  "clip.trackUp": () => nav.moveSelectedTrack(-1),
  "clip.trackDown": () => nav.moveSelectedTrack(1),
  "clip.menu": () => openClipMenuKeyboard(),
  "view.zoomIn": () => zoomBy(1.25),
  "view.zoomOut": () => zoomBy(1 / 1.25),
  "view.fit": () => fitTimeline(),
  "view.panels": () => togglePanels(),
  "view.fullscreen": () => togglePreviewFullscreen(),
  "global.export": () => focusExportPanel(),
  "global.settings": () => openSettings(),
  "global.help": () => openHelpPanel(),
  "view.perf": () => togglePerfPanel(),
};

/** 绑定定义表(id / 分组 / 文案 / 默认组合)——注册表数据源,顺序即帮助面板展示序。 */
function defineAll() {
  const d = (id, group, label, combo) => defineBinding(id, group, label, combo, RUN[id]);
  // ---- 播放(10)----
  d("play.toggle", "播放", "播放/暂停", "space");
  d("play.j", "播放", "倒放(J×2=2x 倒放)", "j");
  d("play.k", "播放", "暂停/复位倍速", "k");
  d("play.l", "播放", "正放(L×2=2x,L×4=4x)", "l");
  d("play.frameBack", "播放", "后退一帧", "arrowleft");
  d("play.frameFwd", "播放", "前进一帧", "arrowright");
  d("play.secBack", "播放", "后退 1 秒", "shift+arrowleft");
  d("play.secFwd", "播放", "前进 1 秒", "shift+arrowright");
  d("play.home", "播放", "跳到开头", "home");
  d("play.end", "播放", "跳到结尾", "end");
  // ---- 标记(5)----
  d("mark.in", "标记", "设入点", "i");
  d("mark.out", "标记", "设出点", "o");
  d("mark.marker", "标记", "添加/移除标记(会话级)", "m");
  d("mark.prev", "标记", "上一个标记", "arrowup");
  d("mark.next", "标记", "下一个标记", "arrowdown");
  // ---- 编辑(24;T4.7 起 T = 文本工具,裁剪模式迁 V)----
  d("mode.select", "编辑", "选择模式(A)", "a");
  d("mode.blade", "编辑", "切割模式(B;点击片段即分割)", "b");
  d("mode.trim", "编辑", "裁剪模式(V;点片段边缘拖动修剪)", "v");
  d("text.add", "编辑", "播放头处添加文本(T;text_add)", "t");
  d("clip.split", "编辑", "分割(播放头处)", "s");
  d("clip.splitAll", "编辑", "全轨分割(播放头处所有轨,单 Op)", "shift+s");
  d("edit.delete", "编辑", "删除选中片段", "delete");
  d("edit.deleteBksp", "编辑", "删除选中片段(退格)", "backspace");
  d("edit.rippleDelete", "编辑", "波纹删除", "shift+delete");
  d("edit.copy", "编辑", "复制选中片段", "ctrl+c");
  d("edit.cut", "编辑", "剪切选中片段", "ctrl+x");
  d("edit.paste", "编辑", "粘贴到播放头", "ctrl+v");
  d("edit.undo", "编辑", "撤销", "ctrl+z");
  d("edit.redo", "编辑", "重做", "ctrl+y");
  d("edit.selectAll", "编辑", "全选片段", "ctrl+a");
  d("clip.nudgeBack", "编辑", "选中片段左移 1 帧", "ctrl+arrowleft");
  d("clip.nudgeFwd", "编辑", "选中片段右移 1 帧", "ctrl+arrowright");
  d("clip.nudgeBack10", "编辑", "选中片段左移 10 帧", "ctrl+shift+arrowleft");
  d("clip.nudgeFwd10", "编辑", "选中片段右移 10 帧", "ctrl+shift+arrowright");
  d("clip.prev", "编辑", "选中上一个片段", "alt+arrowleft");
  d("clip.next", "编辑", "选中下一个片段", "alt+arrowright");
  d("clip.trackUp", "编辑", "移到上一兼容轨", "alt+arrowup");
  d("clip.trackDown", "编辑", "移到下一兼容轨", "alt+arrowdown");
  d("clip.menu", "编辑", "打开片段菜单(上下文)", "enter");
  // ---- 视图(5)----
  d("view.zoomIn", "视图", "放大(视口中心)", "+");
  d("view.zoomOut", "视图", "缩小(视口中心)", "-");
  d("view.fit", "视图", "适应窗口", "\\");
  d("view.panels", "视图", "侧面板开合(Tab;焦点在控件上时仍走焦点移动)", "tab");
  d("view.fullscreen", "视图", "预览全屏", "f");
  // ---- 全局(4)----
  d("global.export", "全局", "打开导出面板", "ctrl+s");
  d("global.settings", "全局", "设置(重绑定键位)", "ctrl+,");
  d("global.help", "全局", "快捷键帮助", "?");
  d("view.perf", "全局", "性能面板(dev)", "shift+d");
}

/* ---- 装配 ---- */

let defined = false;

export function installEditorShortcuts() {
  if (!defined) {
    defineAll();
    defined = true;
    onOverridesChange(reinstall);
  }
  reinstall();
  // e2e/控制台观察面(下波键位 e2e 遍历口;导出全表含默认与生效组合)
  window.__cfKeymap = {
    table: exportTable,
    routeCount,
    keyOf,
    comboOf,
  };
}

/** 依当前生效组合重装调度路由(覆盖变更/恢复默认后调用)。 */
function reinstall() {
  clearRoutes();
  for (const b of exportTable()) {
    const combo = comboOf(b.id);
    if (!combo) continue; // 用户停用/默认不绑
    const fn = RUN[b.id];
    if (fn) registerShortcut(combo, fn);
  }
}
