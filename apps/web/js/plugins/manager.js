/* 插件管理面板(册七 T7.2):本地文件安装(.js/.mjs + manifest.json)→
 * plugin_validate 校验展示(valid/errors/warnings + 权限五面)→ 首启确认对话框
 * (权限列表明示,Worker/process 同规)→ 启用/禁用/卸载(localStorage 注册表)。
 * 诚实口径:浏览器无文件系统权限,「目录即安装」的服务器目录面归 CUTFORGE_PLUGINS
 * (process 形态/CLI);壳侧卸载 = 移出插件注册表;入口文件须自包含(Worker 单文件)。 */
import { h, clear } from "../ui/dom.js";
import { openDialog } from "../ui/dialog.js";
import { toast } from "../ui/toast.js";
import {
  listInstalled, putRecord, removeRecord, setEnabled, setMenuSurfaced,
  validateManifest, permissionRows, ensureKinds,
} from "./manifest.js";
import { startPlugin, stopPlugin, pluginState, onPluginStateChange } from "./host.js";
import { contribsOf, setMenuSurfacedLive } from "./contributes.js";

let listBox = null;
let candidate = null; // { manifest, code, validation }
let candHost = null;
let fileInput = null;

export function mountPlugins(container) {
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["插件(JS Worker 形态 · ADR-0024)"]),
    h("div", { class: "hint" }, [
      "安装 = 选 manifest.json + 入口 .js/.mjs(自包含单文件)→ plugin_validate 校验 → 首启确认(权限明示)。",
      "启用后代码在 Web Worker 运行:无 DOM/token,一切 /api/v1 调用经宿主按 manifest 权限裁决。",
    ]),
    h("div", { class: "filter-row" }, [
      fileInput = h("input", {
        type: "file", multiple: true, accept: ".json,.js,.mjs",
        testid: "plugin-files", "aria-label": "选择插件文件(manifest.json 与入口)",
        style: "max-width:280px",
      }),
      h("button", { testid: "plugin-install", onclick: () => installFromFiles(), title: "读取所选文件并校验 manifest" }, ["校验并安装"]),
      h("button", { testid: "plugin-refresh", onclick: () => renderList(), title: "重扫注册表与运行态" }, ["刷新"]),
    ]),
    candHost = h("div", { testid: "plugin-candidate" }),
  ]));
  listBox = h("div", { class: "panel tab-page", testid: "plugin-list" });
  container.appendChild(listBox);
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, ["权限裁决(双层)"]),
    h("div", { class: "hint" }, [
      "前端:宿主按 manifest 权限面逐调用拦截(查询→read / 写→write / 编排→exec;目录白名单强校验);",
      "后端:plugin-call 通道(CLI)同口径二次裁决——前端拦漏的,后端 GUARD_FAILED/FORBIDDEN 兜底。",
    ]),
  ]));
  fileInput.addEventListener("change", () => { candidate = null; clear(candHost); });
  onPluginStateChange(() => renderList()); // 运行态徽标(启动完成/崩溃/停止)即时重渲
  renderList();
}

/* ---------------- 安装流 ---------------- */

async function installFromFiles() {
  const files = [...(fileInput.files || [])];
  if (!files.length) {
    toast("先选插件文件:manifest.json + 入口 .js/.mjs", false);
    return;
  }
  const texts = {};
  for (const f of files) texts[f.name] = await f.text();
  const mfName = Object.keys(texts).find((n) => n === "manifest.json" || n.endsWith("/manifest.json"));
  if (!mfName) {
    toast("缺 manifest.json(必须与入口一起选择)", false);
    return;
  }
  let manifest;
  try {
    manifest = JSON.parse(texts[mfName]);
  } catch (e) {
    toast(`manifest.json 非合法 JSON:${String(e && e.message || e)}`, false);
    return;
  }
  const entryRel = String(manifest.entry || "");
  const entryName = Object.keys(texts).find((n) => n === entryRel || n.endsWith(`/${entryRel}`));
  if (!entryName) {
    toast(`入口文件缺失:manifest.entry=${entryRel || "(空)"}(请一并选择)`, false);
    return;
  }
  const validation = await validateManifest(manifest);
  candidate = { manifest, code: texts[entryName], validation };
  renderCandidate();
  ensureKinds(); // 预热工具分类缓存(权限裁决镜像的真相源)
}

function renderCandidate() {
  clear(candHost);
  if (!candidate) return;
  const { manifest, validation } = candidate;
  const card = h("div", { class: "plugin-card", testid: "plugin-candidate-card" });
  card.appendChild(h("div", { class: "filter-row" }, [
    h("b", null, [manifest.name || "(无名)"]),
    h("span", { class: "badge" }, [`v${manifest.version || "?"}`]),
    h("span", { class: "badge" }, [manifest.form || "?" ]),
    h("code", null, [manifest.id || "(缺 id)"]),
    validation.valid
      ? h("span", { class: "badge ok", testid: "plugin-valid" }, ["校验通过"])
      : h("span", { class: "badge err", testid: "plugin-invalid" }, ["校验不通过"]),
  ]));
  if (manifest.description) card.appendChild(h("div", { class: "hint" }, [manifest.description]));
  for (const e of validation.errors || []) {
    card.appendChild(h("div", { class: "plan-err", testid: "plugin-error" }, [`错误:${e}`]));
  }
  for (const w of validation.warnings || []) {
    card.appendChild(h("div", { class: "plan-warn", testid: "plugin-warning" }, [`建议:${w}`]));
  }
  card.appendChild(permTable(manifest));
  if (validation.valid) {
    card.appendChild(h("div", { class: "filter-row" }, [
      h("button", {
        class: "primary", testid: "plugin-install-confirm",
        onclick: () => confirmInstall(),
      }, ["写入注册表(默认停用)"]),
    ]));
  }
  candHost.appendChild(card);
}

function permTable(manifest) {
  const rows = permissionRows(manifest)
    .map(([label, v]) => h("div", { class: "plugin-perm-row", testid: "plugin-perm" }, [
      h("span", { class: `badge ${v ? "ok" : ""}` }, [v ? "已声明" : "未声明"]),
      h("span", null, [label + (typeof v === "string" ? `: ${v}` : "")]),
    ]));
  return h("div", { class: "plugin-perms", testid: "plugin-perms" }, rows);
}

function confirmInstall() {
  if (!candidate) return;
  const rec = {
    id: candidate.manifest.id,
    manifest: candidate.manifest,
    code: candidate.code,
    enabled: false,
    menuOff: [],
  };
  putRecord(rec);
  candidate = null;
  clear(candHost);
  fileInput.value = "";
  toast(`已安装 ${rec.id}(停用态;启用需首启确认)`);
  renderList();
}

/* ---------------- 列表与生命周期 ---------------- */

function renderList() {
  clear(listBox);
  const all = listInstalled();
  listBox.appendChild(h("h3", null, [`已安装(${all.length})`]));
  if (!all.length) {
    listBox.appendChild(h("div", { class: "empty-hint", testid: "plugin-empty" }, [
      "(空:上方选择 manifest.json + 入口文件安装;三件套示例在 apps/web/examples/plugins/)",
    ]));
    return;
  }
  for (const rec of all) listBox.appendChild(pluginRow(rec));
}

function pluginRow(rec) {
  const m = rec.manifest || {};
  const state = rec.enabled ? (pluginState(rec.id) || "starting") : "disabled";
  const stateBadge = h("span", {
    class: `badge ${state === "running" ? "ok" : state === "disabled" ? "" : "err"}`,
    testid: "plugin-state", "data-state": state,
  }, [state === "running" ? "运行中" : state === "starting" ? "启动中" : state === "disabled" ? "已停用" : "已崩溃/停止"]);
  const row = h("div", { class: "plugin-card", testid: "plugin-row", dataset: { pid: rec.id } });
  row.appendChild(h("div", { class: "filter-row" }, [
    h("b", null, [m.name || rec.id]),
    h("span", { class: "badge" }, [`v${m.version || "?"}`]),
    h("span", { class: "badge" }, [m.form || "?"]),
    h("code", null, [rec.id]),
    stateBadge,
  ]));
  if (m.description) row.appendChild(h("div", { class: "hint" }, [m.description]));
  row.appendChild(permTable(m));
  row.appendChild(contribRow(rec));
  row.appendChild(h("div", { class: "filter-row" }, [
    rec.enabled
      ? h("button", {
          testid: "plugin-disable", title: "停用:终止 Worker 并摘除贡献点(注册表保留)",
          onclick: () => lifecycle(rec, false),
        }, ["停用"])
      : h("button", {
          testid: "plugin-enable", title: "启用(首启弹权限确认)",
          onclick: () => enableFlow(rec),
        }, ["启用"]),
    h("button", {
      testid: "plugin-uninstall", title: "卸载:移出注册表(目录删除属文件系统操作,需手动)",
      onclick: () => uninstall(rec),
    }, ["卸载"]),
  ]));
  return row;
}

function contribRow(rec) {
  const c = rec.manifest?.contributes || {};
  const live = contribsOf(rec.id);
  const parts = [];
  for (const cmd of c.commands || []) {
    const surfaced = live ? !live.menuOff.has(cmd.id) : !(rec.menuOff || []).includes(cmd.id);
    parts.push(h("label", { class: "toggle", title: "作为时间线右键菜单项呈现" }, [
      h("input", {
        type: "checkbox", checked: surfaced ? true : null, testid: "plugin-menu-toggle",
        onchange: (e) => {
          setMenuSurfaced(rec.id, cmd.id, e.target.checked);
          setMenuSurfacedLive(rec.id, cmd.id, e.target.checked);
        },
      }),
      ` 命令 ${cmd.id}(${cmd.title} → 右键菜单)`,
    ]));
  }
  for (const p of c.panels || []) {
    parts.push(h("span", { class: "badge", title: "面板贡献点(启用后出现专属页签)" }, [`面板 ${p.id} · ${p.title}`]));
  }
  if (!parts.length) parts.push(h("span", { class: "hint" }, ["(无贡献点:纯能力插件)"]));
  return h("div", { class: "plugin-contribs", testid: "plugin-contribs" }, parts);
}

function lifecycle(rec, on) {
  setEnabled(rec.id, on);
  if (on) {
    const r = { ...rec, enabled: true };
    const res = startPlugin(r);
    if (!res.ok) toast(`启用失败:${res.message}`, false);
  } else {
    stopPlugin(rec.id);
    toast(`已停用 ${rec.id}`);
  }
  renderList();
}

/** 启用流:首启(无确认记录)弹权限确认对话框;确认后写偏好再启动。 */
function enableFlow(rec) {
  const m = rec.manifest || {};
  const confirmedKey = `pluginConfirmed.${rec.id}.${m.version || ""}`;
  let confirmed = false;
  try { confirmed = localStorage.getItem(confirmedKey) === "1"; } catch { /* 隐私模式:每次都确认 */ }
  if (confirmed) {
    lifecycle(rec, true);
    return;
  }
  const dlg = openDialog({
    id: "plugin-confirm-dialog",
    title: `启用插件「${m.name || rec.id}」?`,
    onClose: () => {},
    build: (body) => {
      body.appendChild(h("p", null, [
        `${m.name || rec.id} v${m.version || "?"}(${m.form || "worker"} 形态)`,
        m.description ? ` — ${m.description}` : "",
      ]));
      body.appendChild(h("p", { class: "hint" }, [
        "插件代码将在 Web Worker 沙箱运行:无 DOM、无 token;一切 API 调用经宿主按下列声明权限裁决,",
        "写操作走 Op 通道(actor 归因可审计可撤销)。请确认它申请的权限:",
      ]));
      body.appendChild(permTable(m));
      const yes = h("button", { class: "primary", testid: "plugin-confirm-ok" }, ["确认启用"]);
      const no = h("button", { testid: "plugin-confirm-cancel" }, ["取消"]);
      yes.addEventListener("click", () => {
        try { localStorage.setItem(confirmedKey, "1"); } catch { /* 会话内生效 */ }
        dlg.close();
        lifecycle(rec, true);
      });
      no.addEventListener("click", () => dlg.close());
      body.appendChild(h("div", { class: "wizard-actions" }, [yes, no]));
      yes.focus();
    },
  });
}

function uninstall(rec) {
  const wasEnabled = rec.enabled;
  if (wasEnabled) stopPlugin(rec.id);
  removeRecord(rec.id);
  toast(`已卸载 ${rec.id}(壳侧注册表移除;若为目录安装,删除目录请到文件系统手动完成)`);
  renderList();
}
