/* 键盘导航/选择命令面(T3.6):±1 clip 选中、全选、帧微调、跨轨移动、切割模式开关。
 * 独立成模块的原因:commands.js 已到行数红线(≤400 纪律),这一簇是纯会话态导航
 * (选择/视图移动),与「投影写命令」语境分离也便于下波键位 e2e 定点驱动。
 * 与 commands.js 同一单向流:全部经既有 moveClip 命令入队,不绕过投影。 */
import { projectStore, timelineStore, selectionStore, uiStore } from "./store.js";
import { frameMsOf, clipKindOf, msAdd } from "./model.js";
import { selectClip, moveClip } from "./commands.js";
import { toast } from "../ui/toast.js";

/** 工具模式开关(T4.2:A 选择 / B 刀 / T 裁剪;会话开关,不进 IR)。
 * blade 字段为旧读点兼容(=tool==="blade");光标/命中区/徽标随 tool 切换。 */
export function setTool(tool) {
  const t = tool === "blade" || tool === "trim" ? tool : "select";
  uiStore.set({ tool: t, blade: t === "blade" });
}

/** 兼容旧读点(T3.4 e2e 口径):B/A 开关走统一 setTool。 */
export function setBlade(on) {
  setTool(on ? "blade" : "select");
}

/** 按「轨序 + startMs」给全片段排序(键盘 ±1 clip 导航的稳定序)。 */
function clipsOrdered() {
  const order = (projectStore.get().project?.tracks || []).map((t) => t.id);
  return [...timelineStore.get().clips].sort((a, b) => {
    const d = order.indexOf(a.track) - order.indexOf(b.track);
    return d !== 0 ? d : a.startMs - b.startMs;
  });
}

/** ±1 clip 选中(Alt+←/→;越界夹取,空工程静默)。 */
export function selectSibling(dir) {
  const list = clipsOrdered();
  if (!list.length) return;
  const cur = selectionStore.get().clipId;
  const idx = list.findIndex((c) => c.id === cur);
  const next = list[Math.min(list.length - 1, Math.max(0, (idx < 0 ? 0 : idx) + dir))];
  selectClip(next.id);
}

/** 全选片段(Ctrl+A):主选中 = 首个,框选集 = 全部。 */
export function selectAllClips() {
  const list = clipsOrdered();
  if (!list.length) return;
  selectionStore.set({ clipId: list[0].id, clipIds: list.map((c) => c.id) });
}

/** 键盘微调选中片段(±帧;Ctrl+←/→ 1 帧,Ctrl+Shift ±10 帧;直移不磁吸)。
 * 时间算术统一走 msAdd()(壳纯度 R1:壳不自算时间线语义)。 */
export function nudgeSelected(deltaFrames) {
  const id = selectionStore.get().clipId;
  const row = timelineStore.get().clips.find((c) => c.id === id);
  if (!row) {
    toast("先选中片段(Alt+←/→ 或点片段)", false);
    return;
  }
  moveClip(id, Math.max(0, Math.round(msAdd(row.startMs, frameMsOf(projectStore.get().project) * deltaFrames))));
}

/** 键盘跨轨(Alt+↑/↓):移到上/下一条同轨型轨道(startMs 不变)。 */
export function moveSelectedTrack(dir) {
  const id = selectionStore.get().clipId;
  const row = timelineStore.get().clips.find((c) => c.id === id);
  if (!row) {
    toast("先选中片段(Alt+←/→ 或点片段)", false);
    return;
  }
  const kind = clipKindOf(row);
  const compat = (projectStore.get().project?.tracks || [])
    .filter((t) => (t.kind || clipKindOf({ track: t.id })) === kind)
    .map((t) => t.id);
  if (compat.length < 2) {
    toast(`只有一条 ${kind} 轨,无处可移(先在检查器加轨)`, false);
    return;
  }
  const idx = compat.indexOf(row.track);
  const target = compat[Math.min(compat.length - 1, Math.max(0, idx + dir))];
  if (target === row.track) return;
  moveClip(id, row.startMs, target);
}
