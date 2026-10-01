/* 工程库 / 布局迁移 / 导出矩阵编排 / 素材库命令面(册六 T6.1/T6.3/T6.2 壳侧,F3)。
 *
 * 与 commands.js 同反馈口径(envelope 原样返回 + toast);刻意不进串行编辑队列:
 * 库七操作/迁移为目录级操作、批量导出为编排入队,与时间线 Op 时序无关,
 * 入队反而会在长操作期阻塞 undo/redo(同 render-commands 的裁决)。
 * root 语义(与 dispatch 一致):library 系列/media_library = 库根目录(调用方显式给,
 * rpcArgs 注入的会话工程根会被同键覆盖);migrate_layout/media_import = 工程目录。
 */
import { call } from "./api.js";
import { uiStore } from "./store.js";
import { messageOf, isInternalCode } from "./errors.js";
import { toast } from "../ui/toast.js";

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

/* ---------------- T6.1 工程库 ---------------- */

/** library_list(查询;root=库根)。opts: {root, query, includeArchived} */
export function libraryList(opts = {}) {
  const args = { root: opts.root };
  if (opts.query) args.query = opts.query;
  if (opts.includeArchived) args.includeArchived = true;
  return call("library_list", args);
}

/** library_manage(写):action = new|rename|copy|archive|unarchive|delete。
 * new 的 extra 可带 {to, layout, slug, fps, canvasW, canvasH, tracks}。 */
export function libraryManage(action, name, extra = {}) {
  const args = { root: extra.root, action, name };
  for (const k of ["to", "layout", "slug", "fps", "canvasW", "canvasH", "tracks"]) {
    const v = extra[k];
    if (v !== undefined && v !== null && v !== "" && !(Array.isArray(v) && !v.length)) args[k] = v;
  }
  return call("library_manage", args);
}

/** library_recover(写):action=list(缺省)|recover(+name)。root=库根。 */
export function libraryRecover(action = "list", name, root) {
  const args = { root, action };
  if (name) args.name = name;
  return call("library_recover", args);
}

/** migrate_layout(写):V1/V2 → v3 一次性迁移;v3 工程幂等 NOOP。 */
export function migrateLayout(root) {
  return call("migrate_layout", { root, to: "v3" });
}

/* ---------------- T6.3 导出矩阵编排 ---------------- */

/** export_preflight(查询):导出前轻探测检查清单。opts: {inMs, outMs, target} */
export function exportPreflight(opts = {}) {
  const args = {};
  if (opts.inMs > 0) args.inMs = Math.round(opts.inMs);
  if (opts.outMs > 0) args.outMs = Math.round(opts.outMs);
  if (opts.target !== undefined && opts.target !== null) args.target = opts.target;
  // 黑帧抽样/静音段 RMS 逐段解码,大工程可能超默认 10s:放宽到 60s(豁免渲染级)
  return call("export_preflight", args, { timeoutMs: 60000 });
}

/** export_all_variants(编排):action=run 入队全变体(逐变体独立 runId 走渲染队列);
 * action=status 按 parentRunId 聚合。args 透传 format/qualityTier/bitrateTier/
 * inMs/outMs/videoOnly/ratios(缺省 = 工程 outputs,再缺省 9x16/16x9/1x1)。 */
export function exportAllVariants(args = {}) {
  return call("export_all_variants", { action: "run", ...args });
}

/** 批量父任务聚合状态(action=status;拉取式)。 */
export function variantStatus(parentRunId) {
  return call("export_all_variants", { action: "status", parentRunId });
}

/* ---------------- T6.2 素材库 ---------------- */

/** media_import(写):src = 在位绝对路径或素材库相对引用 → 拷入工程
 * (布局感知落点;不产 Op 不改 IR)。返回 data.src = 工程内相对路径。
 * libraryRoot = 素材库根(相对引用解析面;缺省走服务端 env/用户目录缺省)。 */
export function mediaImport(src, libraryRoot) {
  const args = { src };
  if (libraryRoot) args.libraryRoot = libraryRoot;
  return call("media_import", args);
}

/** media_library(查询;root=素材库根):list + kind/tag/query 过滤。 */
export function mediaLibraryList(opts = {}) {
  const args = { root: opts.root, action: "list" };
  if (opts.kind) args.kind = opts.kind;
  if (opts.tag) args.tag = opts.tag;
  if (opts.query) args.query = opts.query;
  return call("media_library", args);
}

/** media_library action=tag:条目标签整组替换(entry = 素材库相对引用)。 */
export function mediaLibraryTag(entry, tags, root) {
  return call("media_library", { root, action: "tag", entry, tags });
}

export { report };
