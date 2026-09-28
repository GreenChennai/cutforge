/* 投影器(T2.2 单向流):内核投影 → store 派生。
 *
 * 单向数据流:用户手势 → commands(意图)→ api(内核调用)→ reproject() →
 * store 更新 → 视图增量渲染。本模块是 store 的唯一合法写入方(除 ui 会话开关)。
 *
 * 精准失效:workspace.changed 只重投影「工程/时间线」两个投影面 + 冲突计数,
 * 不触碰 notes/diff/export-files(各面板按自己的事件面自刷新),且媒体池走增量 diff
 * (绝不销毁重建,修「播放中被销毁→currentTime 归零」)。
 *
 * 重建铁律(T2.2):四个投影派生面清空后必须可由本模块完全重建,
 * selfTestRebuild() 提供逐字段 diff=0 的自测入口(挂 window.__cutforgeSelfTest)。
 */
import { call, dataGet } from "./api.js";
import { projectStore, timelineStore, mediaStore, uiStore, projectStore as ps } from "./store.js";
import { messageOf } from "./errors.js";

let inFlight = false;
let dirtyAgain = false;

/** 合并式重投影:并发调用合并为「跑完后若又脏则再跑一轮」,不排队堆积。 */
export async function reproject() {
  if (inFlight) {
    dirtyAgain = true;
    return;
  }
  inFlight = true;
  try {
    do {
      dirtyAgain = false;
      await reprojectOnce();
    } while (dirtyAgain);
  } finally {
    inFlight = false;
  }
}

async function reprojectOnce() {
  const [envP, envT] = await Promise.all([call("project_get"), call("timeline_get")]);
  if (envP.ok) {
    projectStore.set({ project: envP.data.project, rev: envP.data.rev, loaded: true });
  } else if (!envP.net) {
    console.warn("[projector] project_get:", messageOf(envP, "project_get"));
  }
  if (envT.ok) {
    timelineStore.set({ clips: envT.data.clips, rev: envT.data.rev });
  }
}

/** 冲突计数(E8 停写横幅的数据源)。 */
export async function refreshConflicts() {
  const env = await call("conflict_list");
  uiStore.set({ conflicts: env.ok ? (env.data.conflicts || []).length : 0 });
  return env;
}

/** 检查器字段真相源(/ui-fields 数据面下发;壳不读文件)。 */
export async function refreshUiFields() {
  const doc = await dataGet("/ui-fields");
  if (doc && doc.editable) {
    uiStore.set({ uiFields: doc });
    return doc;
  }
  return null;
}

/** workspace.changed → 精准失效(禁止全量 refresh)。 */
export function invalidateWorkspace() {
  reproject();
  refreshConflicts();
}

/** 素材浏览(mediaStore 派生;dir 为工程内相对路径)。 */
export async function refreshMedia(dir) {
  const wantDir = dir !== undefined ? dir : mediaStore.get().dir;
  const env = await call("media_browse", wantDir ? { dir: wantDir } : {});
  if (env.ok) {
    mediaStore.set({
      dir: env.data.dir || wantDir || "",
      files: env.data.files || [],
      total: env.data.total || 0,
      truncated: Boolean(env.data.truncated),
      error: "",
    });
  } else {
    mediaStore.set({ files: [], error: env.message || "浏览失败" });
  }
  return env;
}

/* ---------------- 重建铁律自测(T2.2/AC-2.5)---------------- */

/** 内核派生面快照(逐字段;playhead/selection 等会话态不在重建承诺内)。 */
export function snapshotDerived() {
  return JSON.stringify({
    project: projectStore.get(),
    timeline: timelineStore.get(),
    media: mediaStore.get(),
    uiFields: uiStore.get().uiFields,
    conflicts: uiStore.get().conflicts,
  });
}

export function resetDerived() {
  // 会话锚点(root/token/projectRel)不属派生面:清派生值前先留存,重建注入。
  const keep = {
    root: projectStore.get().root,
    token: projectStore.get().token,
    projectRel: projectStore.get().projectRel,
  };
  projectStore.reset();
  timelineStore.reset();
  mediaStore.reset();
  uiStore.set({ uiFields: null, conflicts: 0 });
  projectStore.set(keep);
}

/**
 * 清空 → 全量重投影 → 逐字段比对。
 * @returns {Promise<{ok: boolean, before: string, after: string}>}
 */
export async function selfTestRebuild() {
  const before = snapshotDerived();
  resetDerived();
  await reproject();
  await refreshUiFields();
  await refreshConflicts();
  await refreshMedia();
  const after = snapshotDerived();
  return { ok: before === after, before, after };
}

/** 导出会话根(api.rpcArgs 注入用)。 */
export function applySession(sess) {
  ps.set({
    root: sess.root,
    token: sess.token || "",
    projectRel: sess.projectRel || "05_时间线工程/project.json",
  });
}
