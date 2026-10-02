/* 插件 manifest 注册表 + 权限裁决镜像(册七 T7.2/ADR-0024/PLUGIN-SPEC)。
 *
 * - 注册表即安装:壳侧把 manifest+入口代码存 localStorage(cutforge.plugins.v1);
 *   浏览器无文件系统权限,「目录即安装」的服务器目录面归 CUTFORGE_PLUGINS(process
 *   形态/CLI),壳侧卸载 = 移出注册表(诚实口径,面板有标注);
 * - 权限裁决镜像:与 crates/cutforge-mcp/src/plugin.rs::authorize 同口径前端复刻
 *   (查询→read / 写→write / 编排→exec;越权 GUARD_FAILED,FORBIDDEN 语义前缀),
 *   工具分类真相源 = GET /api/v1/tools(与 tools/list 同一注册表);服务端仍是
 *   第二道保险(plugin-call CLI 面逐次裁决,本镜像只做前置拦截);
 * - 目录白名单(filesystem 声明面)强校验:携带路径类参数(src/out/path/dir/file)
 *   的调用,绝对路径/../未声明白名单一律拒(PLUGIN-SPEC §三"目录白名单的强校验
 *   随 G2 壳侧 Worker 宿主落地")。
 */
import { call, toolsCatalog } from "../core/api.js";

const NS = "cutforge.plugins.v1";

/** @typedef {{ id:string, manifest:Object, code:string, enabled:boolean, menuOff:Array<string> }} PluginRecord */

function loadAll() {
  try {
    const v = JSON.parse(localStorage.getItem(NS) || "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}
function saveAll(map) {
  try {
    localStorage.setItem(NS, JSON.stringify(map));
  } catch { /* 隐私模式:注册表退化为会话内 */ }
}

export function listInstalled() {
  return Object.values(loadAll());
}
export function getRecord(id) {
  return loadAll()[id] || null;
}
export function putRecord(rec) {
  const map = loadAll();
  map[rec.id] = rec;
  saveAll(map);
}
export function removeRecord(id) {
  const map = loadAll();
  delete map[id];
  saveAll(map);
}
export function setEnabled(id, on) {
  const m = loadAll();
  if (!m[id]) return;
  m[id].enabled = Boolean(on);
  saveAll(m);
}
export function setMenuSurfaced(id, cmdId, on) {
  const m = loadAll();
  const rec = m[id];
  if (!rec) return;
  const off = new Set(rec.menuOff || []);
  if (on) off.delete(cmdId);
  else off.add(cmdId);
  rec.menuOff = [...off];
  m[id] = rec;
  saveAll(m);
}

/* ---------------- 校验(plugin_validate 服务端面)---------------- */

/**
 * manifest 校验:schema 契约 + 语义面(服务端 plugin_validate 工具)。
 * @returns {Promise<{valid:boolean, errors:string[], warnings:string[]}>}
 */
export async function validateManifest(manifest) {
  const env = await call("plugin_validate", { manifest });
  if (env.ok) return { valid: true, errors: [], warnings: env.data?.warnings || [] };
  return {
    valid: Boolean(env.data?.valid),
    errors: env.data?.errors || [env.message || env.code],
    warnings: env.data?.warnings || [],
  };
}

/* ---------------- 工具分类真相源 + 裁决镜像 ---------------- */

/** @type {Array<{name:string,kind:string}>|null} */
let kindsCache = null;

/** 工具 kind 查询(缓存;/api/v1/tools 失败返回 null → 裁决面诚实降级为放行+警示,
 * 第二道保险仍是服务端)。 */
export async function ensureKinds() {
  if (kindsCache) return kindsCache;
  kindsCache = await toolsCatalog();
  return kindsCache;
}
export function kindOf(tool) {
  const hit = kindsCache?.find((t) => t.name === tool);
  return hit ? hit.kind : null;
}

const PATH_ARGS = ["src", "out", "path", "dir", "file"];

/**
 * 权限裁决(前端镜像;与 plugin.rs::authorize 同口径)。
 * @returns {{ ok: true } | { ok: false, code: string, message: string }}
 */
export function authorize(manifest, tool, args = {}) {
  const kind = kindOf(tool);
  if (!kind) {
    return { ok: false, code: "INTERNAL", message: `FORBIDDEN: 未知工具 ${tool}(不在 /api/v1/tools 注册表)` };
  }
  const perms = manifest.permissions || {};
  const perm = kind === "query" ? "read" : kind === "write" ? "write" : "exec";
  if (!perms[perm]) {
    return {
      ok: false,
      code: "GUARD_FAILED",
      message: `FORBIDDEN: 插件 ${manifest.id} 未声明 permissions.${perm},拒绝调用 ${kind} 类工具 ${tool}`,
    };
  }
  // 目录白名单强校验(声明面):路径类参数必须工程内相对路径且首段在白名单
  const hasPath = PATH_ARGS.some((k) => typeof args[k] === "string" && args[k] !== "");
  if (hasPath) {
    const allow = Array.isArray(perms.filesystem) ? perms.filesystem : [];
    for (const k of PATH_ARGS) {
      const v = args[k];
      if (typeof v !== "string" || !v) continue;
      if (/^([a-zA-Z]:[\\/]|\/|\\\\)/.test(v) || v.split(/[\\/]/).includes("..")) {
        return { ok: false, code: "GUARD_FAILED", message: `FORBIDDEN: 参数 ${k}=${v} 是绝对路径或含 ..(只允许工程内相对路径)` };
      }
      const head = v.split(/[\\/]/)[0];
      if (!allow.includes(head)) {
        return {
          ok: false,
          code: "GUARD_FAILED",
          message: `FORBIDDEN: 目录 "${head}" 不在 permissions.filesystem 白名单([${allow.join(", ") || "空"}]);插件 ${manifest.id} 请在 manifest 声明`,
        };
      }
    }
  }
  return { ok: true };
}

/** 权限五面展示行(首启确认对话框/管理面板共用;true 才列出,未声明=无权限)。 */
export function permissionRows(manifest) {
  const p = manifest.permissions || {};
  const rows = [
    ["read(查询)", p.read],
    ["write(工程写)", p.write],
    ["network(网络)", p.network],
    ["exec(编排/执行)", p.exec],
  ];
  const fs = Array.isArray(p.filesystem) ? p.filesystem : [];
  rows.push(["filesystem(目录白名单)", fs.length ? fs.join("、") : false]);
  return rows;
}
