/* 编辑命令扩展簇(T4.2/T4.3):clip_trim 四件套 / clip_split_all / track_update /
 * clip_gap_delete / clip_copy+clip_paste_at / 批量循环 / 历史快照标记。
 * 独立成模块原因:commands.js 行数红线(≤400);复用其串行队列(enqueue 导出),
 * 保证与 undo/redo 连点不交错。单向流不变:意图 → api → reproject。
 * 批量原子性诚实口径:后端无批量 patch 工具(56 工具逐笔),多选批量 = 逐笔 Op,
 * toast 明示「每片段一笔,撤销需逐笔」;已登记「批量候 BE」遗留。 */
import { call } from "./api.js";
import { ephemeralStore, timelineStore, projectStore } from "./store.js";
import { reproject } from "./projector.js";
import { messageOf } from "./errors.js";
import { toast } from "../ui/toast.js";
import { enqueue } from "./commands.js";

function report(env, name, okMsg) {
  if (!env.ok) toast(messageOf(env, name), false);
  else if (okMsg) toast(okMsg);
  return env;
}

/* ---------------- clip_trim 四件套(每手势恰好一笔 Op) ---------------- */

/**
 * @param {string} clipId
 * @param {"trim"|"roll"|"slip"|"slide"} mode
 * @param {"in"|"out"} edge trim/roll 必给;slip/slide 省略
 * @param {number} deltaMs 有符号变化量
 */
export function trimClip(clipId, mode, edge, deltaMs) {
  return enqueue(async () => {
    const args = { clipId, mode, deltaMs };
    if (mode === "trim" || mode === "roll") args.edge = edge;
    const env = await call("clip_trim", args);
    if (env.ok) await reproject();
    return report(env, "clip_trim");
  });
}

/* ---------------- 播放头全分割 / 间隙删除 ---------------- */

/** clip_split_all:播放头处所有轨命中片段一次全分割(单 Op,undo 一刀还原)。 */
export function splitAllAt(tMs) {
  return enqueue(async () => {
    const env = await call("clip_split_all", { tMs: Math.max(0, Math.round(tMs)) });
    if (env.ok) {
      await reproject();
      toast(`已在 ${(tMs / 1000).toFixed(2)}s 全轨分割(单 Op,可撤销)`);
    }
    report(env, "clip_split_all");
  });
}

/** clip_gap_delete:删除指定轨上包含 tMs 的间隙(单 Op 原子)。 */
export function gapDelete(trackId, tMs) {
  return enqueue(async () => {
    const env = await call("clip_gap_delete", { trackId, tMs: Math.max(0, Math.round(tMs)) });
    if (env.ok) {
      await reproject();
      toast(`已删除 ${trackId} 间隙并闭合(可撤销)`);
    }
    report(env, "clip_gap_delete");
  });
}

/* ---------------- 服务端剪贴板(clip_copy + clip_paste_at) ---------------- */

/** 复制片段到服务端会话剪贴板(不产 Op、不升 rev、不可撤销——工具语义如此,面板如实提示)。 */
export function copyClip(clipId) {
  return enqueue(async () => {
    const env = await call("clip_copy", { clipId });
    report(env, "clip_copy", "已复制到剪贴板(会话级;粘贴走右键/Ctrl+V)");
    return env;
  });
}

/** 带属性粘贴到指定轨+时间点(单 Op;重叠/跨轨型由服务端 GUARD 拒绝)。 */
export function pasteClipAt(trackId, startMs) {
  return enqueue(async () => {
    const env = await call("clip_paste_at", { trackId, startMs: Math.max(0, Math.round(startMs)) });
    if (env.ok) await reproject();
    report(env, "clip_paste_at", "已粘贴(带属性,可撤销)");
    return env;
  });
}

/* ---------------- 轨道属性(track_update 七字段) ---------------- */

/**
 * @param {string} trackId
 * @param {{ name?: string, locked?: boolean, mute?: boolean, solo?: boolean,
 *          hidden?: boolean, heightPx?: number, color?: string }} patch
 */
export function updateTrack(trackId, patch) {
  return enqueue(async () => {
    const env = await call("track_update", { trackId, patch });
    if (env.ok) await reproject(); // 轨道头状态随 projectStore patch 由 timeline-view 重绘
    return report(env, "track_update");
  });
}

/* ---------------- 批量循环(诚实:逐笔 Op) ---------------- */

/**
 * 多选批量 patch:逐 clip_update 循环(每片段一笔 Op)。后端无批量工具(已登记遗留),
 * toast 明示撤销口径;操作前打快照标记(历史面板分隔)。resolve 为成功笔数。
 * @param {string[]} clipIds
 * @param {(row: Object) => Object} patchOf 由投影行算 patch(逐笔最新态)
 */
export function batchUpdateClips(clipIds, patchOf) {
  return enqueue(async () => {
    if (!clipIds.length) return 0;
    markHistory(`批量修改前(${clipIds.length} 段)`);
    let done = 0;
    for (const id of clipIds) {
      const row = timelineStore.get().clips.find((c) => c.id === id);
      if (!row) continue;
      const patch = patchOf(row);
      if (!patch || !Object.keys(patch).length) continue;
      const env = await call("clip_update", { clipId: id, patch });
      if (env.ok) done += 1;
      else report(env, "clip_update");
    }
    await reproject();
    if (done > 1) {
      toast(`已批量应用至 ${done} 个片段(每片段一笔 Op,撤销需逐笔)`);
    } else if (done === 1) {
      toast("已应用 1 个片段(可撤销)");
    }
    return done;
  });
}

/* ---------------- 历史快照标记(T4.3;会话态不落盘) ---------------- */

/** 打快照标记:锚在当前 rev(投影只读),历史列表中显示分隔线。 */
export function markHistory(label) {
  const rev = Number(projectStore.get().rev) || 0;
  const marks = (ephemeralStore.get().historyMarks || []).slice();
  marks.push({ rev, label: label || "快照", t: Date.now() });
  ephemeralStore.set({ historyMarks: marks.slice(-50) });
}

/* ---------------- 最近使用素材(T4.1;会话态) ---------------- */

/** 插入成功后前插最近使用(去重,上限 12)。 */
export function noteRecentMedia(path) {
  if (!path) return;
  const cur = (ephemeralStore.get().recentMedia || []).filter((p) => p !== path);
  ephemeralStore.set({ recentMedia: [path, ...cur].slice(0, 12) });
}
