/* 会话标记与入出点(T3.4):M 标记 / I·O 入出点 / ↑·↓ 标记跳转。
 * 纪律(ADR-0013 口径):标记与入出点都是纯会话态——不进 IR / 不落盘 / 不参与撤销;
 * 标记存 ephemeral.markers(ephemeral.* 前缀强制),入出点存 selectionStore(既有字段)。
 * 持久化判定:标记属剪辑辅助线,工程语义应由剪辑本身表达——不落盘;若未来需要
 * 随工程走,登记遗留(markers 持久化),本波不实现。 */
import { ephemeralStore, selectionStore, projectStore } from "../core/store.js";
import { frameMsOf } from "../core/model.js";
import { playheadMs, seekTo, timelineEndMs } from "../core/commands.js";
import { toast } from "./toast.js";

/** M 键:播放头处添加/移除标记(±半帧内视为同一点,toggle 语义)。 */
export function toggleMarker() {
  const t = Math.round(playheadMs());
  const cur = ephemeralStore.get().markers || [];
  const half = frameMsOf(projectStore.get().project) / 2;
  const hit = cur.find((m) => Math.abs(m - t) <= half);
  const next = hit ? cur.filter((m) => m !== hit) : [...cur, t].sort((a, b) => a - b);
  ephemeralStore.set({ markers: next });
  toast(hit ? `已移除标记 ${(t / 1000).toFixed(2)}s` : `已加标记 ${(t / 1000).toFixed(2)}s(会话级,不落盘)`);
}

/** ↑:上一个标记(无更早标记则回开头);↓:下一个(无更晚则到结尾)。 */
export function prevMarker() { jumpMarker(-1); }
export function nextMarker() { jumpMarker(1); }

function jumpMarker(dir) {
  const t = playheadMs();
  const list = (ephemeralStore.get().markers || []).filter((m) => (dir < 0 ? m < t - 1 : m > t + 1));
  const target = dir < 0
    ? (list.length ? list[list.length - 1] : 0)
    : (list.length ? list[0] : timelineEndMs());
  seekTo(target);
}

/** I:设入点;O:设出点(播放头;入 > 出时自动交换,保证 in≤out)。 */
export function setInOut(which) {
  const t = Math.round(playheadMs());
  const sel = selectionStore.get();
  let inMs = which === "in" ? t : (sel.inMs ?? 0);
  let outMs = which === "out" ? t : (sel.outMs ?? timelineEndMs());
  if (inMs > outMs) [inMs, outMs] = [outMs, inMs];
  selectionStore.set({ inMs, outMs });
  toast(`入出点:${(inMs / 1000).toFixed(2)}s – ${(outMs / 1000).toFixed(2)}s(会话级)`);
}

/** 清除入出点(时间线空白右键菜单用)。 */
export function clearInOut() {
  selectionStore.set({ inMs: null, outMs: null });
  toast("已清除入出点");
}

/** 当前标记列表(副本;overlay 绘制用)。 */
export function markersOf() {
  return (ephemeralStore.get().markers || []).slice();
}
