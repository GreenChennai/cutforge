/* 错误面收口(T2.3):envelope 分型 + 人话消息。
 * 服务端 envelope 形状:{ ok, code, message, data?, ns? }(三面同码,见 docs/FLOW.md §5.5)。 */

/** INTERNAL/SCHEMA_INVALID 属契约错误 → 折叠横幅计数(旧壳 v0.6 行为对齐)。 */
export function isInternalCode(code) {
  return code === "INTERNAL" || code === "SCHEMA_INVALID";
}

/** envelope → 面向用户的单行消息(数据面错误走 toast,契约错误走横幅)。 */
export function messageOf(env, toolName) {
  if (!env) return `${toolName || "rpc"}: 空响应`;
  if (env.ok) return "";
  return `${toolName || "rpc"}: ${env.code} ${env.message || ""}`.trim();
}

/** 网络层失败(HTTP 异常/超时)的统一 envelope 化,调用方无需区分异常与业务失败。 */
export function netFail(code, message) {
  return { ok: false, code, message, net: true };
}
