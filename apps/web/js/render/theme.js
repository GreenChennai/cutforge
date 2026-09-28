/* 主题取色(T3.1/ADR-0014):canvas 绘制面的唯一取色口。
 *
 * 纪律:js 零硬编码色值(check-shell-purity R5 扫描)——canvas 的 fillStyle/strokeStyle
 * 一律经本模块读 css 语义 token(--cf-*,定义点 css/tokens.css),与 DOM 面同源:
 * 改 token 两面同步,未来浅色主题(册七可选)零 js 改动。
 * 缓存一次(mount 后首取);tokens.css 是构建期内联样式表,会话内不热切主题。
 */

/** @type {Map<string, string>} */
const cache = new Map();

/** 读一个 --cf-* token(返回计算值字符串,如六位十六进制色或带透明度函数色);未命中返回空串。 */
export function cssVar(name) {
  if (cache.has(name)) return /** @type {string} */ (cache.get(name));
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  cache.set(name, v);
  return v;
}

/** 主题热替换入口(预留;当前仅深色,重置缓存即可)。 */
export function resetThemeCache() {
  cache.clear();
}
