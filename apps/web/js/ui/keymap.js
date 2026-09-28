/* 编辑器默认键位表(T2.5;册三 T3.4 完整体系在此扩展)。
 * 语义与旧壳逐项对拍:空格播放、←→逐帧、Home/End、S 分割、Del 删除(波纹开关)、
 * Ctrl+Z/Y 撤销重做、Ctrl+C/V 复制粘贴(会话内剪贴板)。 */
import { selectionStore, uiStore } from "../core/store.js";
import * as commands from "../core/commands.js";
import { registerShortcut } from "./shortcuts.js";

export function installEditorShortcuts() {
  const s = registerShortcut;
  s("space", () => commands.togglePlay());
  s("ArrowLeft", () => commands.seekFrame(-1));
  s("ArrowRight", () => commands.seekFrame(1));
  s("Home", () => commands.toStart());
  s("End", () => commands.toEnd());
  s("s", () => commands.splitSelected());
  s("Delete", () => commands.deleteSelected(uiStore.get().ripple));
  s("shift+Delete", () => commands.deleteSelected(true));
  s("ctrl+z", () => commands.undo());
  s("ctrl+y", () => commands.redo());
  s("ctrl+shift+z", () => commands.redo());
  s("ctrl+c", () => { window.__cfClipboard = selectionStore.get().clipId; });
  s("ctrl+v", () => {
    const id = window.__cfClipboard;
    if (id) commands.duplicateClip(id, commands.playheadMs());
  });
}
