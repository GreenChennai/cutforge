/* 纯模型助手(T2.2 单向流的"显示映射"面)。
 * 壳纯度红线(ADR-0013 + check-shell-purity):
 * - 一切 startMs/endMs/durationMs 语义值只读自内核投影,壳只做「像素↔毫秒显示映射」
 *   与「播放头→媒体 currentTime 的播放映射」;
 * - 时间算术统一走 msAdd()(避免壳内散落的时间线手算,门禁模式串见 cli check-shell-purity);
 * - PX_PER_MS 是显示映射会话态:默认 0.06(e2e 兼容红线,e2e_edit_ops/e2e_preview 以
 *   0.06 换算点击坐标,任何路径不得在装配时改值);缩放(T3.3 滚轮 ±)经 setPxPerMs()
 *   改显示映射,不触碰投影;临时投影只允许 ephemeral.*(ADR-0013)。 */

/** 时间线缩放:像素/毫秒(ESM live binding,导入方读到最新值)。 */
export let PX_PER_MS = 0.06;

/** 缩放边界(显示映射;0.02=远看全貌,0.30=近看帧级)。 */
export const PX_PER_MS_MIN = 0.02;
export const PX_PER_MS_MAX = 0.3;

/** 设置缩放(滚轮/手势专用;越界夹取。返回是否生效)。 */
export function setPxPerMs(v) {
  const next = Math.min(PX_PER_MS_MAX, Math.max(PX_PER_MS_MIN, v));
  if (next === PX_PER_MS) return false;
  PX_PER_MS = next;
  return true;
}

export function frameMsOf(project) {
  return 1000 / ((project && project.fps) || 30);
}

/** 帧磁吸(magnet 开):四舍五入到帧网格;壳不做任何别的取整语义。 */
export function snapMs(ms, magnetOn, fms) {
  const snapped = magnetOn ? Math.round(ms / fms) * fms : ms;
  return Math.round(snapped);
}

/** 时间算术统一入口(基准 + 位移)。 */
export function msAdd(baseMs, deltaMs) {
  return baseMs + deltaMs;
}

export function clampMs(ms, lo, hiMs) {
  return Math.max(lo, Math.min(ms, hiMs));
}

/** 时间线显示端点(空工程兜底 10s;来源全部为投影行 endMs)。 */
export function timelineEndMsOf(clips) {
  if (!clips || !clips.length) return 10000;
  return Math.max(10000, ...clips.map((c) => c.endMs));
}

/** 轨型判定:优先投影行 trackKind,回退稳定 id 前缀(V/A/T)。 */
export function clipKindOf(row) {
  if (row.trackKind) return row.trackKind;
  return ({ V: "video", A: "audio", T: "text" })[String(row.track || "").charAt(0)] || "video";
}

/** clip 块显示名(src 文件名 / 文本 / id)。 */
export function clipLabelOf(row) {
  const base = row.src || row.text || row.id || "";
  return `${String(base).split(/[\\/]/).pop()} ${((row.endMs - row.startMs) / 1000).toFixed(1)}s`;
}

/** 素材 kind → 优先落点轨型(image 视同 video 落视频轨)。 */
export function trackKindForMedia(kind) {
  return kind === "audio" ? "audio" : (kind === "image" ? "video" : kind);
}

/** 素材插入的目标轨道:匹配轨型优先,视频轨兜底,最后任意轨。 */
export function targetTrackForKind(tracks, want) {
  return (tracks || []).find((t) => t.kind === want)?.id
    || (tracks || []).find((t) => t.kind === "video")?.id
    || (tracks || [])[0]?.id || null;
}

/** /media 数据面 URL(token 走查询参数:数据面鉴权两通道之一)。 */
export function mediaUrlFor(src, token) {
  return `/media?path=${encodeURIComponent(src)}&token=${encodeURIComponent(token)}`;
}

/** 播放映射:时间线时刻 t → 该投影行的素材源时刻(秒)。壳纯度约定:
 * 只读投影字段(sourceInMs/startMs/speed),是播放映射而非时间线运算(旧壳口径)。 */
export function sourceSecAt(row, tMs) {
  const spd = row.speed || 1;
  return ((row.sourceInMs || 0) + (tMs - row.startMs) * spd) / 1000;
}

/** 播放映射:时刻 t 是否落在该行可见区间。 */
export function rowCovers(row, tMs) {
  return Boolean(row.src) && tMs >= row.startMs && tMs < row.endMs;
}

/** 带点字段读("transition.type" → clip.transition?.type)。 */
export function nestedGet(obj, field) {
  const dot = field.indexOf(".");
  if (dot < 0) return obj ? obj[field] : undefined;
  const inner = obj ? obj[field.slice(0, dot)] : undefined;
  return inner ? inner[field.slice(dot + 1)] : undefined;
}

/** request_id 幂等键(T2.3:非幂等写去重)。 */
export function newRequestId() {
  return `ui-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}
