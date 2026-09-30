/* 目录面(册四 T4.5~T4.7):GET /catalogs 单一数据源(转场 58 / fx 11 / motion 19 / 花字 12)。
 * 后端编译期嵌入、GET 下发(不经 MCP 工具,工具数口径不变);壳侧一次加载缓存进
 * uiStore(会话态;失败如实置空,面板各自给空态)。缩略图静态资产位于
 * /assets/transitions/<id>.jpg(240x135 jpeg;id 即文件名主干,转场与 fx 同表)。 */
import { dataGet } from "./api.js";
import { uiStore } from "./store.js";

/** 缩略图 URL(id 直映文件;资产缺失时 <img> onerror 回落图标位)。 */
export function thumbUrlFor(id) {
  return `/assets/transitions/${encodeURIComponent(id)}.jpg`;
}

/** 保证目录已加载(幂等;并发调用共享同一 in-flight promise)。 */
let inflight = null;
export function ensureCatalogs() {
  if (uiStore.get().catalogs) return Promise.resolve(uiStore.get().catalogs);
  if (inflight) return inflight;
  inflight = (async () => {
    const doc = await dataGet("/catalogs");
    const catalogs = doc && doc.transition && doc.fx ? doc : null;
    uiStore.set({ catalogs });
    inflight = null;
    return catalogs;
  })();
  return inflight;
}

/** 同步读取(未加载返回 null;面板渲染前先 ensureCatalogs)。 */
export function catalogsNow() {
  return uiStore.get().catalogs || null;
}

/** 转场目录行(id/name/category/directional);未加载返回 []。 */
export function transitionList() {
  const c = catalogsNow();
  return (c && c.transition && c.transition.transitions) || [];
}

/** 转场缺省时长(catalog.defaultDurMs,schema 缺省同源 500)。 */
export function transitionDefaultDurMs() {
  const c = catalogsNow();
  return (c && c.transition && c.transition.defaultDurMs) || 500;
}

/** fx 目录行(id/name/category/desc/params[]);未加载返回 []。 */
export function fxList() {
  const c = catalogsNow();
  return (c && c.fx && c.fx.fx) || [];
}

/** motion 目录:{in:[{id,name,kind}], out:[…]}(真实渲染项;未加载返回空组)。 */
export function motionLists() {
  const c = catalogsNow();
  const m = (c && c.fx && c.fx.motion) || {};
  return { in: m.in || [], out: m.out || [] };
}

/** 花字目录行(id/name/category/desc/params[]/style?/karaoke?);未加载返回 []。 */
export function huaziList() {
  const c = catalogsNow();
  return (c && c.huazi && c.huazi.huazi) || [];
}

/** 按 id 查 fx 参数 schema(挂载时算默认/钳制用)。 */
export function fxSchemaOf(fxId) {
  return fxList().find((f) => f.id === fxId) || null;
}

/** 按 id 查花字模板。 */
export function huaziSchemaOf(id) {
  return huaziList().find((x) => x.id === id) || null;
}

/**
 * fx 参数默认值(schema default 优先,数值型缺省取 min 可用则 min,再退 0)。
 * @returns {Object<string, number>}
 */
export function fxDefaultParams(fxId) {
  const schema = fxSchemaOf(fxId);
  const out = {};
  if (!schema) return out;
  for (const p of schema.params || []) {
    out[p.name] = p.default !== undefined ? p.default : (p.min !== undefined ? p.min : 0);
  }
  return out;
}

/** fx 参数钳制(前端拦截:越界夹取并返回是否发生过夹取)。 */
export function fxClampParam(fxId, name, v) {
  const schema = fxSchemaOf(fxId);
  const p = schema && (schema.params || []).find((x) => x.name === name);
  if (!p || typeof v !== "number" || Number.isNaN(v)) return { value: v, clamped: false };
  let value = v;
  if (p.min !== undefined) value = Math.max(p.min, value);
  if (p.max !== undefined) value = Math.min(p.max, value);
  return { value, clamped: value !== v };
}
