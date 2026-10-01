/* 专业工具命令簇(册五 A5-FE2,T5.3/T5.4/T5.6 壳侧):
 * 复合片段 / 多机位 / 场景检测 / 响度单 / 编码探测 / OTIO·EDL 互操作 / 渲染队列。
 * 独立成模块原因:commands.js 与 edit-commands.js 行数红线(≤400)已顶格。
 * 口径:产 Op 的写命令(compound 系与 multicam_cut、scene_detect+autoSplit)走 commands
 * 的串行队列(与 undo/redo 不交错,完成后重投影);纯计算/查询(multicam_sync、
 * audio_loudness、encode_probe、render_queue、otio_export)直连 api 不入队——其中
 * ffmpeg 解码/测量类耗时以秒计,显式放宽 RPC 超时(api.RENDER_CLASS 不含本簇)。
 * 会话结果(同步集/切换点/测量/探测/OTIO 产物)写 ephemeral(ADR-0013,刷新即失)。 */
import { call } from "./api.js";
import { ephemeralStore, uiStore, timelineStore } from "./store.js";
import { reproject } from "./projector.js";
import { messageOf, isInternalCode } from "./errors.js";
import { toast } from "../ui/toast.js";
import { enqueue, selectClip } from "./commands.js";
import { newRequestId } from "./model.js";

/** 耗时查询类统一超时(与渲染类同量级;loudnorm 全片测量/整片抽帧以秒~分钟计)。 */
const SLOW_MS = 300000;

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

/* ---------------- 复合片段(T5.4;单 Op 原子,可撤销) ---------------- */

/**
 * 打包为复合片段(compound_create):选中 ≥2 片段 → 单壳片段落 toTrack@startMs。
 * 前置校验(与后端 GUARD 同向的前端拦截):≥2 / 同轨 / 全视频 / 不含复合。
 * @param {string[]} clipIds
 * @returns {Promise<Object>} envelope(data.clipId = 新壳 id)
 */
export function compoundCreate(clipIds) {
  return enqueue(async () => {
    const pre = compoundPrecheck(clipIds);
    if (pre.err) {
      toast(pre.err, false);
      return { ok: false, code: "GUARD_FAILED", message: pre.err };
    }
    const env = await call("compound_create", {
      clipIds, toTrack: pre.track, startMs: Math.max(0, Math.round(pre.startMs)),
      requestId: newRequestId(),
    });
    if (env.ok) {
      await reproject();
      const cid = env.data && env.data.clipId;
      if (cid) selectClip(cid);
      toast(`已打包为复合片段(${env.data.innerClips ?? clipIds.length} 个子片段,单 Op 可撤销)`);
    }
    return report(env, "compound_create");
  });
}

/** 打包前置校验:返回 {err} 或 {track, startMs}(时间线几何只读投影,壳零手算语义)。 */
export function compoundPrecheck(clipIds) {
  if (!clipIds || clipIds.length < 2) return { err: "框选 ≥2 个片段后可用(当前选中不足)" };
  const rows = timelineStore.get().clips.filter((c) => clipIds.includes(c.id));
  if (rows.length !== clipIds.length) return { err: "选中片段有已删除项,请重新框选" };
  const track = rows[0].track;
  if (!rows.every((r) => r.track === track)) return { err: "打包要求同一轨道上的片段(跨轨请先移动)" };
  if (!rows.every((r) => (r.trackKind || "video") === "video")) return { err: "仅视频片段可打包(音频/文本/复合不支持)" };
  if (rows.some((r) => r.compound)) return { err: "已含复合片段:复合不可嵌套(深度上限两级,内核 GUARD)" };
  return { track, startMs: Math.min(...rows.map((r) => r.startMs)) };
}

/** 解包复合片段(compound_unbind):局部时间域平移回主时间线,id 重分配,单 Op。 */
export function compoundUnbind(clipId) {
  return enqueue(async () => {
    const env = await call("compound_unbind", { clipId, requestId: newRequestId() });
    if (env.ok) {
      await reproject();
      selectClip(null);
      toast(`已解包(${env.data.unbound ?? 0} 个子片段还原,可撤销)`);
    }
    return report(env, "compound_unbind");
  });
}

/* ---------------- 多机位(T5.4;ADR-0019 展开方案) ---------------- */

/** 同步集维护(会话态;素材右键「加入/移出多机位集」与面板手输共用)。 */
export function mcAddAngle(path) {
  if (!path) return;
  const cur = ephemeralStore.get().mcAngles || [];
  if (cur.includes(path)) { toast("该素材已在同步集内", false); return; }
  ephemeralStore.set({ mcAngles: [...cur, path] });
}
export function mcRemoveAngle(path) {
  ephemeralStore.set({ mcAngles: (ephemeralStore.get().mcAngles || []).filter((p) => p !== path) });
}
export function mcClearAngles() { ephemeralStore.set({ mcAngles: [] }); }

/**
 * 同步分析(multicam_sync;纯计算不落盘不产 Op):波形互相关,结果入 ephemeral.multicam。
 * @param {string[]} angles angles[0] = 基准
 * @param {number} [windowMs] 搜索窗 ±毫秒
 */
export async function multicamSync(angles, windowMs) {
  const args = { angles: angles.slice() };
  if (windowMs !== undefined && !Number.isNaN(windowMs)) args.windowMs = windowMs;
  const env = await call("multicam_sync", args, { timeoutMs: SLOW_MS });
  if (env.ok && env.data) {
    ephemeralStore.set({
      multicam: {
        angles: env.data.angles || [], confidence: env.data.confidence,
        degraded: env.data.degraded !== false, reference: env.data.reference,
        windowMs: env.data.windowMs, at: Date.now(),
      },
      // 新结果 → 旧切换点/旧序列引用失效,一并清空(诚实:防陈旧 offsets 落序列)
      mcSwitches: [],
    });
    toast(`同步分析完成:最小置信度 ${Math.round((env.data.confidence ?? 0) * 100)}%(启发式 pcm-xcorr)`);
  }
  return report(env, "multicam_sync");
}

/** 切换点收集(播放头逐点;存播放头绝对 ms,生成时归一到相对序列起点)。 */
export function mcAddSwitch(tAbsMs, angle) {
  const cur = ephemeralStore.get().mcSwitches || [];
  ephemeralStore.set({ mcSwitches: [...cur, { tAbs: Math.max(0, Math.round(tAbsMs)), angle }] });
}
export function mcRemoveSwitch(i) {
  const cur = ephemeralStore.get().mcSwitches || [];
  ephemeralStore.set({ mcSwitches: cur.filter((_, k) => k !== i) });
}
export function mcClearSwitches() { ephemeralStore.set({ mcSwitches: [] }); }

/**
 * 生成序列(multicam_cut 单 Op):切换点归一(首点=0/严格递增由归一保证)→ 展开
 * 普通片段序列落视频轨。angles 来自最近一次 multicam_sync(无则拒绝,诚实提示)。
 * @param {string} trackId 目标视频轨
 * @param {number} durationMs 序列总时长
 */
export function multicamCut(trackId, durationMs) {
  return enqueue(async () => {
    const mc = ephemeralStore.get().multicam;
    if (!mc || !mc.angles || mc.angles.length < 2) {
      toast("先做「同步分析」取得角度偏移,再生成序列", false);
      return { ok: false, code: "GUARD_FAILED", message: "缺 multicam_sync 结果" };
    }
    const raw = (ephemeralStore.get().mcSwitches || []).slice()
      .sort((a, b) => a.tAbs - b.tAbs);
    if (!raw.length) { toast("先在播放头处打至少一个切换点", false); return { ok: false, code: "GUARD_FAILED", message: "缺切换点" }; }
    const base = raw[0].tAbs;
    const switches = raw.map((s) => ({ tMs: s.tAbs - base, angle: s.angle }));
    const env = await call("multicam_cut", {
      trackId, startMs: base, durationMs: Math.max(1, Math.round(durationMs)),
      angles: mc.angles.map((a) => ({ src: a.src, offsetMs: a.offsetMs })),
      switches, requestId: newRequestId(),
    }, { timeoutMs: SLOW_MS });
    if (env.ok) {
      await reproject();
      mcClearSwitches();
      toast(`多机位序列已生成:${switches.length} 切换点 → ${trackId}@${base}ms(单 Op 可撤销)`);
    }
    return report(env, "multicam_cut");
  });
}

/* ---------------- 场景检测(T5.4;frame-diff 启发式) ---------------- */

/**
 * 剪切点检测(scene_detect):结果入 ephemeral.scene。autoSplitTrackId 给出时服务端
 * 单 Op 多点切段(TrackSplitAt,切点严格包含才切)——该路径产 Op,入串行队列。
 * @param {string} src @param {number} sensitivity @param {string|null} [autoSplitTrackId]
 */
export function sceneDetect(src, sensitivity, autoSplitTrackId) {
  const run = async () => {
    const args = { src };
    if (sensitivity !== undefined && !Number.isNaN(sensitivity)) args.sensitivity = sensitivity;
    if (autoSplitTrackId) args.autoSplit = { trackId: autoSplitTrackId };
    const env = await call("scene_detect", args, { timeoutMs: SLOW_MS });
    if (env.ok && env.data) {
      ephemeralStore.set({ scene: { ...env.data, at: Date.now() } });
      const n = env.data.cutCount ?? (env.data.cuts || []).length;
      toast(autoSplitTrackId
        ? `检测完成:${n} 剪切点,已自动切段(${autoSplitTrackId},单 Op)`
        : `检测完成:${n} 剪切点(frame-diff 启发式;未自动切段)`);
    }
    return report(env, "scene_detect");
  };
  return autoSplitTrackId ? enqueue(run) : run();
}

/* ---------------- 响度单 / 编码探测(T5.3/T5.6 查询面) ---------------- */

/**
 * 响度测量(audio_loudness;与导出 loudnorm 双 pass 同源参数):结果入
 * ephemeral.loudness(电平表静态呈现的数据口;实时 RMS/Peak 候播放链采样口,登记)。
 * @param {string} src 音频/成片(工程内相对路径) @param {number} [target] LUFS
 */
export async function measureLoudness(src, target) {
  const args = { src };
  if (target !== undefined && !Number.isNaN(target)) args.target = target;
  const env = await call("audio_loudness", args, { timeoutMs: SLOW_MS });
  if (env.ok && env.data) {
    ephemeralStore.set({ loudness: { ...env.data, at: Date.now() } });
  }
  return report(env, "audio_loudness");
}

/** 编码探测(encode_probe;试编毫秒级):结果入 ephemeral.encodeProbe(导出面板展示)。 */
export async function probeEncoders() {
  const env = await call("encode_probe", {}, { timeoutMs: SLOW_MS });
  if (env.ok && env.data) ephemeralStore.set({ encodeProbe: env.data });
  return report(env, "encode_probe");
}

/* ---------------- OTIO / EDL 互操作(T5.5) ---------------- */

/**
 * 导出工程交换文件(otio_export;format otio|edl,派生物落盘不产 Op)。
 * @param {string} format @param {string} [out] 工程内相对路径(缺省 06_成片输出/<slug>.<ext>)
 */
export async function otioExport(format, out) {
  const args = { format };
  if (out) args.out = out;
  const env = await call("otio_export", args);
  if (env.ok && env.data) {
    ephemeralStore.set({ otioLast: { format: env.data.format, file: env.data.out } });
    toast(`已导出 ${env.data.format.toUpperCase()}:${env.data.out}(${env.data.bytes} B)`);
  }
  return report(env, "otio_export");
}

/**
 * 导入 OTIO 新建工程(otio_import;目标 root 必须不存在,project_new 同类免锁)。
 * root 显式覆盖会话注入(api.rpcArgs 展开序:调用方 args 在后)。
 * @param {string} targetRoot 目标工程目录(不存在) @param {string} src OTIO 文件路径
 */
export async function otioImport(targetRoot, src) {
  const env = await call("otio_import", { root: targetRoot, src });
  if (env.ok) {
    const n = (env.data && env.data.warnings) ? env.data.warnings.length : 0;
    toast(`OTIO 已导入新工程 ${targetRoot}(子集外警告 ${n} 条)。新工程需单独启动服务`);
  }
  return report(env, "otio_import");
}

/* ---------------- 渲染队列(T5.6;render_queue 五 action) ---------------- */

/** 队列清单(render_queue list;{runId,state,output?,error?}[])。 */
export async function queueList() {
  return call("render_queue", { action: "list" });
}

/**
 * 队列操作(pause|resume|cancel|retry;状态机由后端裁决,失败如实透传)。
 * @param {string} action @param {string} runId
 */
export async function queueAction(action, runId) {
  const env = await call("render_queue", { action, runId });
  return report(env, `render_queue ${action}`);
}
