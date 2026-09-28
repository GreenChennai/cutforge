/* 用户手势 → 意图 → api → 重投影(T2.2 单向流的命令面)。
 *
 * - 全部写命令经串行队列排队:快速连点(e2e undo/redo 循环)不产生交错 RPC;
 * - 每条命令完成即重投影,视图以服务端状态收敛(C6 断言纪律的壳侧对齐);
 * - 非幂等写显式带 requestId 幂等键(model.newRequestId)。
 */
import { call } from "./api.js";
import {
  projectStore, timelineStore, selectionStore, playbackStore, uiStore, ephemeralStore,
} from "./store.js";
import { reproject, refreshConflicts, refreshMedia } from "./projector.js";
import {
  frameMsOf, snapMs, msAdd, clampMs, timelineEndMsOf, targetTrackForKind,
  trackKindForMedia, newRequestId,
} from "./model.js";
import { messageOf, isInternalCode } from "./errors.js";
import { toast } from "../ui/toast.js";
import { playback } from "../render/preview-loop.js";

/** @type {Array<() => Promise<void>>} */
let queue = [];
let draining = false;

/** 串行执行:命令排队,逐个 drain(避免 undo/redo 连点交错);透传 fn 返回值。 */
function enqueue(fn) {
  return new Promise((resolve) => {
    queue.push(async () => {
      try {
        resolve(await fn());
      } catch (e) {
        console.error("[commands] 队列任务异常", e);
        resolve(undefined);
      }
    });
    drain();
  });
}

async function drain() {
  if (draining) return;
  draining = true;
  try {
    while (queue.length) {
      const job = queue.shift();
      await job();
    }
  } finally {
    draining = false;
  }
}

/* ---------------- 反馈 ---------------- */

function report(env, name, okMsg) {
  if (!env.ok) {
    if (isInternalCode(env.code)) {
      uiStore.set({ internalErrors: uiStore.get().internalErrors + 1 });
    } else if (!env.net) {
      toast(messageOf(env, name), false);
    }
  } else if (okMsg) {
    toast(okMsg);
  }
  return env;
}

/** 破坏性操作反馈(T3.7):5s 内可点「撤销」真撤销(调既有 undo 队列)。 */
function toastUndoable(msg) {
  toast(`${msg}(可撤销)`, true, { action: { label: "撤销", fn: () => undo() } });
}

/* ---------------- 选择 / 播放头(纯会话态) ---------------- */

export function selectClip(id) {
  const clips = timelineStore.get().clips;
  if (id && !clips.some((c) => c.id === id)) id = null;
  // 单选即清框选集(clipIds 只在框选手势中维护;T3.3)
  selectionStore.set({ clipId: id, clipIds: [] });
}

export function playheadMs() {
  return playback.clockMs();
}

export function timelineEndMs() {
  return timelineEndMsOf(timelineStore.get().clips);
}

function magnetOn() {
  return uiStore.get().magnet;
}

function snapAt(ms) {
  return snapMs(ms, magnetOn(), frameMsOf(projectStore.get().project));
}

export function seekTo(ms) {
  playback.seek(clampMs(ms, 0, timelineEndMs()));
}

export function seekFrame(dir) {
  seekTo(playheadMs() + dir * frameMsOf(projectStore.get().project));
}

export function toStart() { seekTo(0); }
export function toEnd() { seekTo(timelineEndMs()); }
export function togglePlay() { playback.setPlaying(!playbackStore.get().playing); }

/* ---------------- 命令 ---------------- */

/** 双击/拖放插入素材(durationMs 优先用浏览元信息,缺省由服务端探测)。 */
export function insertMedia(src, trackId, startMs, durationMs) {
  return enqueue(async () => {
    if (!trackId) {
      toast("工程没有轨道:先点「+视频轨」再插入素材", false);
      return;
    }
    const args = { trackId, src, startMs, requestId: newRequestId() };
    if (durationMs) args.durationMs = durationMs;
    const env = await call("clip_add", args);
    if (env.ok) {
      toast(`已插入 ${src.split("/").pop()} → ${trackId}@${Math.round(startMs)}ms`);
      await reproject();
      // 选中刚插入的片段(同轨末尾:旧壳口径)
      const mine = timelineStore.get().clips.filter((c) => c.track === trackId).pop();
      if (mine) selectClip(mine.id);
    }
    report(env, "clip_add");
  });
}

/** 素材面板双击:自动挑目标轨 + 磁吸到播放头;播放头位置被占时自动接到该轨
 * 最尾空位(T3.7 盲测卡点修复:新手「双击 N 个素材依次排开」预期,不报重叠错)。 */
export function insertMediaAuto(item) {
  const tracks = projectStore.get().project?.tracks || [];
  const trackId = targetTrackForKind(tracks, trackKindForMedia(item.kind));
  const want = snapAt(playheadMs());
  return insertMedia(item.path, trackId, freeStartAt(trackId, want, item.durationMs), item.durationMs);
}

/** want 位置可容纳时长 dur → 原样;被占 → 该轨最尾端(追加语义)。 */
function freeStartAt(trackId, want, durationMs) {
  const dur = durationMs || 1;
  const hit = timelineStore.get().clips.some((c) => c.track === trackId && want < c.endMs && want + dur > c.startMs);
  if (!hit) return want;
  const end = timelineStore.get().clips
    .filter((c) => c.track === trackId)
    .reduce((acc, c) => Math.max(acc, c.endMs), 0);
  return snapAt(end);
}

/** clip_move(拖拽落点;幂等键防重)。 */
export function moveClip(clipId, startMs, toTrack) {
  return enqueue(async () => {
    const args = { clipId, startMs, requestId: newRequestId() };
    if (toTrack) args.toTrack = toTrack;
    const env = await call("clip_move", args);
    if (env.ok) await reproject();
    report(env, "clip_move");
  });
}

/** clip_update(trim/检查器 patch)。 */
export function updateClip(clipId, patch, okMsg) {
  return enqueue(async () => {
    const env = await call("clip_update", { clipId, patch });
    if (env.ok) {
      await reproject();
      if (okMsg) toast(okMsg);
    }
    report(env, "clip_update");
  });
}

export function splitSelected() {
  return enqueue(async () => {
    const id = selectionStore.get().clipId;
    if (!id) {
      toast("先选中片段再分割(点时间线上的片段)", false);
      return;
    }
    const row = timelineStore.get().clips.find((c) => c.id === id);
    if (!row) return;
    const t = playheadMs();
    if (t <= row.startMs || t >= row.endMs) {
      toast("播放头不在选中片段内部", false);
      return;
    }
    await splitNow(id, snapAt(t));
  });
}

/** 切割模式(B 键)点击分割:分割点 = 点击位置(帧磁吸),不要求播放头在片段内。 */
export function splitAt(clipId, ms) {
  return enqueue(async () => {
    await splitNow(clipId, ms);
  });
}

async function splitNow(clipId, tMs) {
  const env = await call("clip_split", { clipId, tMs });
  if (env.ok) {
    await reproject();
    toast(`已在 ${(tMs / 1000).toFixed(2)}s 分割(可撤销)`);
  }
  report(env, "clip_split");
  return env;
}

export function deleteSelected(ripple) {
  return enqueue(async () => {
    const id = selectionStore.get().clipId;
    if (!id) return;
    // 波纹删已在队列内:走免入队内层,嵌套 enqueue 会自死锁
    if (ripple) return rippleDeleteNow(id);
    const env = await call("clip_delete", { clipId: id });
    if (env.ok) {
      selectClip(null);
      await reproject();
      toastUndoable("已删除");
    }
    report(env, "clip_delete");
  });
}

/** 波纹删:删后同轨后继片段整体左移被删时长(每步都是独立 Op,旧壳口径)。 */
export function rippleDelete(clipId) {
  return enqueue(() => rippleDeleteNow(clipId));
}

function rippleDeleteNow(clipId) {
  return (async () => {
    const clips = timelineStore.get().clips;
    const row = clips.find((c) => c.id === clipId);
    if (!row) return;
    const dur = row.endMs - row.startMs;
    const followers = clips
      .filter((c) => c.track === row.track && c.startMs >= row.endMs)
      .map((c) => c.id);
    const env = await call("clip_delete", { clipId });
    if (!env.ok) {
      report(env, "clip_delete");
      return;
    }
    for (const fid of followers) {
      const f = (await call("timeline_get")).data.clips.find((c) => c.id === fid);
      if (f) await call("clip_move", { clipId: fid, startMs: msAdd(f.startMs, -dur) });
    }
    selectClip(null);
    await reproject();
    toastUndoable(followers.length
      ? `已波纹删除(后移 ${followers.length} 个片段)`
      : "已波纹删除");
  })();
}

export function duplicateClip(clipId, startMs) {
  return enqueue(async () => {
    const env = await call("clip_duplicate", { clipId, startMs, requestId: newRequestId() });
    if (env.ok) await reproject();
    report(env, "clip_duplicate", "已复制到播放头(可撤销)");
  });
}

export function duplicateSelectedToPlayhead() {
  const id = selectionStore.get().clipId;
  if (!id) return Promise.resolve();
  return duplicateClip(id, snapAt(playheadMs()));
}

/* 键盘可达命令(选择/微调/跨轨/切割模式开关)在 core/nav.js(T3.6;
 * 本文件行数红线 ≤400,该簇为纯会话态导航,独立成模块便于键位 e2e 驱动)。 */

export function addTrack(kind) {
  return enqueue(async () => {
    const env = await call("track_add", { kind, requestId: newRequestId() });
    if (env.ok) await reproject();
    report(env, "track_add", `已新增 ${kind} 轨(可撤销)`);
  });
}

export function undo(batch = 1) {
  return enqueue(async () => {
    const env = await call("undo", batch > 1 ? { batch } : {});
    if (env.ok) await reproject();
    report(env, "undo");
  });
}

export function redo(batch = 1) {
  return enqueue(async () => {
    const env = await call("redo", batch > 1 ? { batch } : {});
    if (env.ok) await reproject();
    report(env, "redo");
  });
}

/* ---------------- BGM(工程级,bgm_set/bgm_clear) ---------------- */

export function setBgm({ src, gainDb, ducking, loop }) {
  return enqueue(async () => {
    if (!src) {
      toast("先填 BGM 音频路径(或从素材面板选)", false);
      return;
    }
    const args = { src };
    if (gainDb !== undefined && !Number.isNaN(gainDb)) args.gainDb = gainDb;
    if (ducking !== undefined) args.ducking = ducking;
    if (loop !== undefined) args.loop = loop;
    const env = await call("bgm_set", args);
    if (env.ok) {
      await reproject();
      toast("BGM 已更新(可撤销)");
    }
    report(env, "bgm_set");
  });
}

export function clearBgm() {
  return enqueue(async () => {
    const env = await call("bgm_set", { src: null });
    if (env.ok) {
      await reproject();
      toast("已清除 BGM(可撤销)");
    }
    report(env, "bgm_set");
  });
}

/* ---------------- 导出/渲染类命令见 render-commands.js(刻意不入串行队列:
 * 渲染以分钟计,入队会在渲染期阻塞 undo/redo,违背 E6-3 壳侧语义)---------------- */

/* ---------------- 标注 ---------------- */

export function addNote(body, anchor) {
  return enqueue(async () => {
    if (!body) {
      toast("标注正文必填", false);
      return;
    }
    const env = await call("notes_add", { anchor, body, author: "user" });
    report(env, "notes_add", "标注已创建");
    return env;
  });
}

export function resolveNote(noteId, reply, opIds) {
  return enqueue(() => call("notes_resolve", { noteId, reply, opIds }));
}

export function undoBatch(batch) {
  return enqueue(async () => {
    if (!batch) {
      toast("先勾选要撤销的 Op 行", false);
      return;
    }
    const env = await call("undo", { batch });
    if (env.ok) {
      await reproject();
      toast(`已撤销 ${batch} 笔(可重做)`);
    }
    report(env, "undo");
  });
}

/* ---------------- 新建工程向导 ---------------- */

export function createProject({ name, fps, canvasW, canvasH, tracks }) {
  return enqueue(async () => {
    const root = projectStore.get().root;
    // 新工程目录 = 当前工程根的同名父目录下(壳只做字符串拼接,创建由服务端完成)
    const sep = root.includes("\\") ? "\\" : "/";
    const parts = root.replace(/[\\/]+$/, "").split(/[\\/]/);
    parts.pop();
    const target = parts.join(sep) + sep + name;
    const env = await call("project_new", { root: target, slug: name, fps, canvasW, canvasH, tracks });
    if (env.ok) {
      const hint = `已创建 ${env.data.project}。新工程需单独启动服务:cutforge-cli serve "${target}" --open`;
      uiStore.set({ tokenBanner: `✅ ${hint}` });
      toast(hint);
    }
    report(env, "project_new");
    return env;
  });
}

/* ---------------- 素材刷新 ---------------- */

export function browseMedia(dir) {
  return refreshMedia(dir);
}

export { ephemeralStore, selectionStore, playbackStore, uiStore };
