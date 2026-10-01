/* 关键帧纯数据助手(册五 T5.1 FE;ADR-0018 求值单源纪律的壳侧对齐面)。
 *
 * - 壳零插值:曲线形状唯一来源 = 投影 keyframeSamples(服务端 Rust 求值采样),
 *   本模块不内置任何缓动公式,只做「关键帧数组的增删改查 + 整组 patch 构造」;
 * - 写通道唯一:patch.keyframes 整组替换(读投影数组 → 本地编辑 → 整组写回,单 Op);
 * - 属性白名单/校验真相在内核(crates/cutforge-core/src/keyframes.rs),壳只镜像
 *   展示域(min/max 用于画布归一化与打点缺省值),越界由服务端 SCHEMA_INVALID 裁决;
 * - 语义时刻:timeMs 相对片段起点(播放头 ms − startMs 夹取),同属性严格递增。
 */

/** 可打点固定属性(白名单的固定段;fx.<fxId>.<param> 动态键见 fxParamMeta)。
 * domain = 画布 Y 归一化展示域(显示映射,非内核校验);def = 打点缺省值。 */
export const PROP_META = {
  "position.x": { label: "位置 X", domain: [0, 100], def: 50, step: 1 },
  "position.y": { label: "位置 Y", domain: [0, 100], def: 50, step: 1 },
  scale: { label: "缩放", domain: [0, 4], def: 1, step: 0.05 },
  rotation: { label: "旋转", domain: [-180, 180], def: 0, step: 5 },
  opacity: { label: "不透明度", domain: [0, 1], def: 1, step: 0.05 },
  volume: { label: "音量", domain: [0, 2], def: 1, step: 0.05 },
  speed: { label: "速度", domain: [0.25, 4], def: 1, step: 0.05 },
};

/** 检查器秒表挂载面(本册范围;speed 不挂秒表——与 speedCurve 互斥,编辑器内显式打点)。 */
export const WATCHABLE = ["position.x", "position.y", "scale", "rotation", "opacity", "volume"];

/** interp 枚举(schema 封闭枚举;回弹不可表达——bezier y 钳 0..1 无过冲,诚实不设)。 */
export const INTERP_PRESETS = [
  ["linear", "线性"],
  ["easeIn", "缓入"],
  ["easeOut", "缓出"],
  ["easeInOut", "缓入缓出"],
  ["hold", "保持(hold)"],
  ["bezier", "贝塞尔"],
];

/** CSS 标准缓动四参数(与内核 EASE_* 常量同源,仅作 bezier 预设初值)。 */
export const EASE_CP = {
  easeIn: [0.42, 0, 1, 1],
  easeOut: [0, 0, 0.58, 1],
  easeInOut: [0.42, 0, 0.58, 1],
};

/** 读投影 keyframes(可能为 null)。 */
export function keyframesOf(row) {
  return row && Array.isArray(row.keyframes) ? row.keyframes : [];
}

/** 某属性的关键帧子序列(按 timeMs 升序;投影书写序已过严格递增闸,防御性再排)。 */
export function kfsOfProp(row, prop) {
  return keyframesOf(row)
    .filter((k) => k.property === prop)
    .slice()
    .sort((a, b) => a.timeMs - b.timeMs);
}

/** 属性是否已启用动画(≥1 关键帧)。 */
export function propAnimated(row, prop) {
  return keyframesOf(row).some((k) => k.property === prop);
}

/** fx 键参数元(fx-catalog 目录动态域;未注册回落采样域)。 */
export function fxParamMeta(prop) {
  const parts = String(prop).split(".");
  if (parts.length < 3 || parts[0] !== "fx") return null;
  return { fxId: parts[1], param: parts.slice(2).join(".") };
}

/** 展示域:固定属性查表;fx 键按目录参数 min/max;再回落采样值域外扩 10%。 */
export function domainOf(row, prop) {
  const meta = PROP_META[prop];
  if (meta) return meta.domain.slice();
  const fx = fxParamMeta(prop);
  if (fx) {
    const s = samplesOf(row, prop);
    if (s.length) return sampleDomain(s);
    return [0, 1];
  }
  const s = samplesOf(row, prop);
  return s.length ? sampleDomain(s) : [0, 1];
}

function sampleDomain(samples) {
  let lo = Infinity;
  let hi = -Infinity;
  for (const [, v] of samples) {
    lo = Math.min(lo, v);
    hi = Math.max(hi, v);
  }
  if (!Number.isFinite(lo)) return [0, 1];
  if (lo === hi) return [lo - 1, hi + 1];
  const pad = (hi - lo) * 0.1;
  return [lo - pad, hi + pad];
}

/** 投影采样点集([[tMs,v],…];ADR-0018:壳唯一曲线形状来源)。 */
export function samplesOf(row, prop) {
  const rows = (row && Array.isArray(row.keyframeSamples)) ? row.keyframeSamples : [];
  const hit = rows.find((e) => e.property === prop);
  return hit && Array.isArray(hit.samples) ? hit.samples : [];
}

/** 当前值(检查器秒表打点用):draftOf() 优先(检查器草稿),回落投影,再回落缺省。 */
export function currentValueOf(row, prop, draftOf) {
  const d = draftOf ? draftOf() : undefined;
  if (d !== undefined && d !== null && d !== "" && !Number.isNaN(Number(d))) return Number(d);
  if (row) {
    if (prop === "position.x") return row.position && row.position.x !== undefined ? Number(row.position.x) : PROP_META[prop].def;
    if (prop === "position.y") return row.position && row.position.y !== undefined ? Number(row.position.y) : PROP_META[prop].def;
    const v = row[prop];
    if (v !== undefined && v !== null) return Number(v);
  }
  return PROP_META[prop] ? PROP_META[prop].def : 0;
}

/** 打点时刻:播放头 → 片段内相对 ms(帧磁吸由调用方做;此处只夹取 + 圆整)。 */
export function kfTimeAt(row, playheadMs) {
  const dur = row ? Math.max(1, Math.round(row.endMs - row.startMs)) : 1;
  const rel = row ? Math.round(playheadMs - row.startMs) : 0;
  return Math.min(dur, Math.max(0, rel));
}

/* ---------------- 整组 patch 构造(唯一写通道:数组整替) ---------------- */

/** 克隆投影关键帧行为可写草稿(数值域归一)。 */
export function draftOf(row) {
  return keyframesOf(row).map((k) => ({
    property: String(k.property),
    timeMs: Math.max(0, Math.round(Number(k.timeMs) || 0)),
    value: Number(k.value) || 0,
    interp: k.interp || "linear",
    bezier: Array.isArray(k.bezier) ? k.bezier.slice() : undefined,
  }));
}

/** 在草稿中 upsert(同属性同刻 = 改值;否则插入并保持同属性严格递增)。 */
export function upsertKf(draft, prop, timeMs, value) {
  const t = Math.max(0, Math.round(timeMs));
  const list = draft.filter((k) => k.property === prop);
  const hit = list.find((k) => k.timeMs === t);
  if (hit) {
    hit.value = value;
    return;
  }
  const kf = { property: prop, timeMs: t, value, interp: "linear" };
  // 插到同属性第一条 >t 之前(保持书写序严格递增;整组其余属性顺序不动)
  let idx = draft.length;
  for (let i = 0; i < draft.length; i += 1) {
    if (draft[i].property === prop && draft[i].timeMs > t) { idx = i; break; }
  }
  draft.splice(idx, 0, kf);
}

/** 移除某属性全部关键帧;返回是否发生变化。 */
export function removePropKfs(draft, prop) {
  const next = draft.filter((k) => k.property !== prop);
  const changed = next.length !== draft.length;
  draft.length = 0;
  for (const k of next) draft.push(k);
  return changed;
}

/** 删除单条(按引用)。 */
export function dropKf(draft, kf) {
  const i = draft.indexOf(kf);
  if (i >= 0) draft.splice(i, 1);
}

/** 移动某条到新时刻(同属性邻居间夹取,保严格递增;返回夹取后的值)。 */
export function moveKf(draft, kf, wantMs, durMs) {
  const same = draft.filter((k) => k.property === kf.property);
  const pos = same.indexOf(kf);
  const lo = pos > 0 ? same[pos - 1].timeMs + 1 : 0;
  const hi = pos >= 0 && pos < same.length - 1 ? same[pos + 1].timeMs - 1 : Math.max(0, Math.round(durMs));
  kf.timeMs = Math.min(hi, Math.max(lo, Math.round(wantMs)));
  return kf.timeMs;
}

/** patch 载荷(整组替换;册五收口起空数组 [] = 清除全部关键帧,内核已接受)。 */
export function kfPatch(draft) {
  const clean = draft.map((k) => {
    const out = { property: k.property, timeMs: k.timeMs, value: k.value };
    if (k.interp && k.interp !== "linear") out.interp = k.interp;
    if (k.interp === "bezier" && Array.isArray(k.bezier) && k.bezier.length === 4) out.bezier = k.bezier;
    return out;
  });
  return { keyframes: clean };
}
