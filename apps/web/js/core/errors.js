/* 错误面收口(T2.3):envelope 分型 + 人话消息 + 怎么修(T3.6:错误消息含修复指引)。
 * 服务端 envelope 形状:{ ok, code, message, data?, ns? }(三面同码,见 docs/FLOW.md §5.5)。 */

/** INTERNAL/SCHEMA_INVALID 属契约错误 → 折叠横幅计数(旧壳 v0.6 行为对齐)。 */
export function isInternalCode(code) {
  return code === "INTERNAL" || code === "SCHEMA_INVALID";
}

/** code → 「怎么修」提示(T3.6:错误不只报错,还告诉用户下一步)。 */
const FIX_HINTS = {
  TIMEOUT: "(怎么修:确认服务窗口是否存活,稍后重试即可;命令未落账)",
  NETWORK: "(怎么修:检查服务进程与端口;连接恢复后本页自动继续,无需刷新)",
  UNAUTHORIZED: "(怎么修:回到服务窗口复制最新链接重新打开本页)",
  FORBIDDEN_PATH: "(怎么修:属壳内部错误,请反馈日志)",
  HTTP_500: "(怎么修:服务端内部错误,请看服务窗口日志后重试)",
  HTTP_502: "(怎么修:服务端内部错误,请看服务窗口日志后重试)",
  HTTP_503: "(怎么修:服务忙或重启中,稍候重试)",
  SCHEMA_INVALID: "(怎么修:工程可能是旧版字段,用 CutFlow rs_ir validate 检查)",
  INTERNAL: "(怎么修:服务端契约错误,请看服务窗口日志;重试无效请反馈)",
};

/** code → 修复提示(无登记返回空串)。 */
export function fixHintOf(code) {
  return FIX_HINTS[code] || "(怎么修:按提示修正后重试)";
}

/** envelope → 面向用户的单行消息(数据面错误走 toast,契约错误走横幅)。
 * 一律追加「怎么修」提示;消息主干与旧口径逐字兼容(e2e 断言子串仍命中)。 */
export function messageOf(env, toolName) {
  if (!env) return `${toolName || "rpc"}: 空响应`;
  if (env.ok) return "";
  const base = `${toolName || "rpc"}: ${env.code} ${env.message || ""}`.trim();
  return `${base} ${fixHintOf(env.code)}`;
}

/** 网络层失败(HTTP 异常/超时)的统一 envelope 化,调用方无需区分异常与业务失败。 */
export function netFail(code, message) {
  return { ok: false, code, message, net: true };
}
