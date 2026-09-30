/* 编辑命令扩展簇(T4.2/T4.3):clip_trim 四件套 / clip_split_all / track_update /
 * clip_gap_delete / clip_copy+clip_paste_at / 批量循环 / 历史快照标记。
 * 独立成模块原因:commands.js 行数红线(≤400);复用其串行队列(enqueue 导出),
 * 保证与 undo/redo 连点不交错。单向流不变:意图 → api → reproject。
 * 批量原子性诚实口径:后端无批量 patch 工具(56 工具逐笔),多选批量 = 逐笔 Op,
 * toast 明示「每片段一笔,撤销需逐笔」;已登记「批量候 BE」遗留。
 * 册四 FE2 增(T4.4~T4.9):transition_set / motion(clip_update)/ text_add /
 * subtitle_* / audio_beats。 */
import { call } from "./api.js";
import { ephemeralStore, timelineStore, projectStore } from "./store.js";
import { reproject } from "./projector.js";
import { messageOf } from "./errors.js";
import { toast } from "../ui/toast.js";
import { selectClip, enqueue } from "./commands.js";

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

/* ---------------- 转场(T4.5;transition_set) ---------------- */

/**
 * 应用目录转场:全量 58 项目录走 fx="tr.<id>" 直通(type="fade" 与之并存,以 fx 为准);
 * durMs 缺省沿用现值。移除转场用 clearTransition。
 * @param {string} clipId
 * @param {{ id: string, durMs?: number }} t
 */
export function applyTransition(clipId, t) {
  return enqueue(async () => {
    const args = { clipId, type: "fade", fx: `tr.${t.id}` };
    if (t.durMs !== undefined && !Number.isNaN(t.durMs)) args.durMs = Math.max(0, Math.round(t.durMs));
    const env = await call("transition_set", args);
    if (env.ok) {
      await reproject();
      toast(`转场 ${t.id} 已应用(可撤销)`);
    }
    return report(env, "transition_set");
  });
}

/** 关闭转场(type="none";显式关闭语义,可撤销)。 */
export function clearTransition(clipId) {
  return enqueue(async () => {
    const env = await call("transition_set", { clipId, type: "none" });
    if (env.ok) {
      await reproject();
      toast("转场已关闭(可撤销)");
    }
    return report(env, "transition_set");
  });
}

/* ---------------- 动效(T4.6;clip_update.motion 按字段合并) ---------------- */

/** @param {string} clipId @param {{in?:string,inMs?:number,out?:string,outMs?:number}} patch */
export function applyMotion(clipId, patch) {
  return enqueue(async () => {
    const env = await call("clip_update", { clipId, patch: { motion: patch } });
    if (env.ok) await reproject();
    return report(env, "clip_update");
  });
}

/**
 * mo.<id> 直通别名(motion_set inFx/outFx;优先于枚举,未注册渲染降级 WARN)。
 * @param {string} clipId @param {"in"|"out"} side
 */
export function applyMotionAlias(clipId, side, alias) {
  return enqueue(async () => {
    const arg = side === "in" ? "inFx" : "outFx";
    const env = await call("motion_set", { clipId, [arg]: alias });
    if (env.ok) await reproject();
    return report(env, "motion_set");
  });
}

/* ---------------- 文本与字幕(T4.7;text_add / subtitle_*) ---------------- */

/**
 * 播放头处加文本(text_add;响应带 clipId → 壳自动选中)。
 * @param {{ text: string, atMs: number, durationMs?: number, trackId?: string,
 *          textStyle?: Object, huazi?: Object }} a
 */
export function textAdd(a) {
  return enqueue(async () => {
    const env = await call("text_add", {
      text: a.text, atMs: Math.max(0, Math.round(a.atMs)),
      ...(a.durationMs ? { durationMs: Math.round(a.durationMs) } : {}),
      ...(a.trackId ? { trackId: a.trackId } : {}),
      ...(a.textStyle ? { textStyle: a.textStyle } : {}),
      ...(a.huazi ? { huazi: a.huazi } : {}),
      requestId: `ui-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    });
    if (env.ok) {
      await reproject();
      const cid = env.data && env.data.clipId;
      if (cid) selectClip(cid);
      toast(`文本已添加${cid ? `(${cid})` : ""}(可撤销)`);
    }
    return report(env, "text_add");
  });
}

/** 字幕文本修改(subtitle_set;单 Op)。 */
export function subtitleSetText(clipId, text) {
  return enqueue(async () => {
    const env = await call("subtitle_set", { clipId, text });
    if (env.ok) await reproject();
    return report(env, "subtitle_set");
  });
}

/** 字幕时间微调(subtitle_retime;单 Op;startMs/durationMs 至少给一)。 */
export function subtitleRetime(clipId, startMs, durationMs) {
  return enqueue(async () => {
    const args = { clipId };
    if (startMs !== undefined && startMs !== null) args.startMs = Math.max(0, Math.round(startMs));
    if (durationMs !== undefined && durationMs !== null) args.durationMs = Math.max(1, Math.round(durationMs));
    const env = await call("subtitle_retime", args);
    if (env.ok) await reproject();
    return report(env, "subtitle_retime");
  });
}

/** 批量替换(subtitle_replace;find 为字面量;trackId 缺省=全部文本轨)。 */
export function subtitleReplace(find, replace, trackId) {
  return enqueue(async () => {
    const args = { find, replace };
    if (trackId) args.trackId = trackId;
    const env = await call("subtitle_replace", args);
    if (env.ok) {
      await reproject();
      const n = env.data && (env.data.changed ?? env.data.count);
      toast(`批量替换完成${typeof n === "number" ? `:${n} 条` : ""}(可撤销)`);
    }
    return report(env, "subtitle_replace");
  });
}

/** 字幕导入(subtitle_import;SRT/ASS 自动识别;src 工程内相对路径)。 */
export function subtitleImport(src, trackId) {
  return enqueue(async () => {
    const args = { src, requestId: `ui-${Date.now()}-${Math.random().toString(36).slice(2, 8)}` };
    if (trackId) args.trackId = trackId;
    const env = await call("subtitle_import", args);
    if (env.ok) {
      await reproject();
      const n = env.data && (env.data.count ?? (env.data.clips || []).length);
      toast(`字幕已导入${typeof n === "number" ? ` ${n} 条` : ""}(整批单 Op,可撤销)`);
    }
    return report(env, "subtitle_import");
  });
}

/** 字幕导出(subtitle_export;format srt/ass;out 缺省走服务端缺省路径)。 */
export function subtitleExport(format, out, trackId) {
  return enqueue(async () => {
    const args = { format };
    if (out) args.out = out;
    if (trackId) args.trackId = trackId;
    const env = await call("subtitle_export", args);
    if (env.ok) {
      const f = env.data && (env.data.file || env.data.out || env.data.path);
      toast(`字幕已导出${f ? `:${f}` : ""}`);
    }
    return report(env, "subtitle_export");
  });
}

/* ---------------- 卡点(T4.8;audio_beats 纯计算 + 会话节拍) ---------------- */

/**
 * 节拍检测(audio_beats;纯计算不落盘不产 Op):结果写 ephemeral.beats(会话态),
 * 供吸附候选(standard 档)与标尺 scrub 吸附。启发式 engine=onset-energy,诚实标注。
 * @param {string} src 工程内相对路径
 * @param {number} [sensitivity] 0..1
 */
export function detectBeats(src, sensitivity) {
  return enqueue(async () => {
    const args = { src };
    if (sensitivity !== undefined && !Number.isNaN(sensitivity)) args.sensitivity = sensitivity;
    const env = await call("audio_beats", args);
    if (env.ok && env.data) {
      ephemeralStore.set({
        beats: {
          src, bpm: env.data.bpm, beats: env.data.beats || [],
          confidence: env.data.confidence, degraded: env.data.degraded !== false,
          at: Date.now(),
        },
      });
      toast(`节拍检测完成:BPM ${env.data.bpm} · ${env.data.beatCount ?? (env.data.beats || []).length} 拍`
        + `(启发式,置信度 ${(env.data.confidence ?? 0) * 100 | 0}%)`);
    }
    return report(env, "audio_beats");
  });
}

/** 清除会话节拍(不落盘,刷新即失;按钮显式清除用)。 */
export function clearBeats() {
  ephemeralStore.set({ beats: null });
}
