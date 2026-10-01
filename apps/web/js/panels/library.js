/* 工程库视图(册六 T6.1 壳侧,F3):启动器的 Web 面。
 *
 * 勘察诚实口径:一个 serve 进程 = 一个工程根(/session 绑定,壳无热切换通道),
 * 故本视图 =「库管理 + 启动指引」双职责:七操作(rename/copy/archive/unarchive/
 * delete/new)跨工程真实可用(免锁面工具,root=库根);「打开」= 当前工程就地
 * 确认 / 其他工程给出 serve 命令一键复制(终端选号器 pick_project_interactive
 * 保留数字键选号,本视图同习惯支持 1–9 数字键开卡)。缩略图经 /media 数据面,
 * 只达当前会话工程(越工程拒绝,诚实回落画幅占位块)。
 * 迁移入口:打开 v2/v1 工程时顶栏提示条(不打扰,可关);常驻入口在「工程」菜单。
 * 卡片操作对话框/恢复清单在 panels/library-ops.js(行数红线拆分,纯移动)。
 */
import { h, clear, $ } from "../ui/dom.js";
import { openDialog } from "../ui/dialog.js";
import { openContextMenu } from "../ui/menu.js";
import { textField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";
import { projectStore } from "../core/store.js";
import { pref, setPref } from "../ui/prefs.js";
import { mediaUrlFor } from "../core/model.js";
import { libraryList, libraryRecover } from "../core/library-commands.js";
import {
  openCard, openRename, openCopy, archiveToggle, openDelete, openNewInLibrary,
  refreshRecoverStrip,
} from "./library-ops.js";

export const MIGRATE_HINT = "v3 扁平布局:媒体→media/、成片→exports/、真相源到工程根;"
  + "OpLog/rev/历史不动,一次性迁移且幂等(ADR-0021)。";

let libRoot = "";
let cards = [];
let showArchived = false;

/** 会话工程布局探测(projectRel 由 /session 下发,壳不读盘):v3=根/project.json。 */
export function sessionLayout() {
  const rel = String(projectStore.get().projectRel || "");
  if (!rel || rel === "project.json") return "v3";
  if (rel.includes("05_ir")) return "v1";
  return "v2";
}

/** 库根缺省:用户偏好 > 当前工程父目录(与 new/向导的「工程旁」约定同源)。 */
export function defaultLibraryRoot() {
  return String(pref("libraryRoot", "") || parentDir(projectStore.get().root) || "");
}

export function currentLibRoot() {
  return libRoot || defaultLibraryRoot();
}

function parentDir(p) {
  const s = String(p || "").replace(/[\\/]+$/, "");
  const i = Math.max(s.lastIndexOf("\\"), s.lastIndexOf("/"));
  return i > 0 ? s.slice(0, i) : "";
}

export function fmtDur(ms) {
  if (ms === undefined || ms === null) return "—";
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function fmtDate(ms) {
  if (!ms) return "—";
  const d = new Date(Number(ms));
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getMonth() + 1}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/* ---------------- 工程库视图(启动器) ---------------- */

export function openLibrary() {
  document.getElementById("library-dialog")?.remove(); // 同 id 双开防御(wizard 同口径)
  libRoot = currentLibRoot();
  let recoverStrip = null;
  openDialog({
    id: "library-dialog",
    title: "工程库(一个服务窗口开一个工程;打开其他工程用卡片给出的命令)",
    build: (body) => {
      body.classList.add("lib-body");
      const rootIn = textField({
        testid: "lib-root", ariaLabel: "库根目录", placeholder: "库根目录(缺省 = 当前工程旁)",
        onEnter: () => reloadLibraryGrid(),
      });
      rootIn.set(libRoot);
      const grid = h("div", { class: "lib-grid", testid: "lib-grid" });
      const emptyEl = h("div", { class: "empty-hint", testid: "lib-empty", hidden: true });
      const search = textField({
        testid: "lib-search", ariaLabel: "按名称搜索工程", placeholder: "搜索名称/slug…",
        onInput: () => reloadLibraryGrid(),
      });
      const archChip = h("button", {
        class: "chip", testid: "lib-archived", title: "并入归档区(.archives)",
        "aria-pressed": "false",
        onclick: () => {
          showArchived = !showArchived;
          archChip.classList.toggle("on", showArchived);
          archChip.setAttribute("aria-pressed", showArchived ? "true" : "false");
          reloadLibraryGrid();
        },
      }, ["归档"]);
      const recoverStripEl = h("div", { class: "lib-recover", testid: "lib-recover-box", hidden: true });
      recoverStrip = recoverStripEl;
      body.appendChild(h("div", { class: "lib-toolbar" }, [
        h("label", { class: "lib-root-row" }, ["库根 ", rootIn.root]),
        h("button", { class: "mini", testid: "lib-refresh", title: "library_list 重新扫描", onclick: () => reloadLibraryGrid() }, ["⟳ 刷新"]),
        h("button", { class: "mini", testid: "lib-new", title: "在库内新建工程(名称+画幅+布局+轨道模板)", onclick: () => openNewInLibrary() }, ["＋ 新建"]),
      ]));
      body.appendChild(h("div", { class: "lib-toolbar" }, [
        search.root, archChip,
        h("span", { class: "hint" }, ["数字键 1–9 打开对应卡片(与终端选号器同习惯)"]),
      ]));
      body.appendChild(recoverStripEl);
      body.appendChild(grid);
      body.appendChild(emptyEl);
      body.appendChild(h("div", { class: "hint" }, [
        "删除进 .trash/<时间戳>-<名>/,可从文件管理器捞回;归档进 .archives/。锁定 = 工程正被别的服务窗口打开。",
      ]));
      // 数字键选号(输入框内不劫持;启停随对话框生死,无需解绑)
      body.addEventListener("keydown", (e) => {
        const tag = (e.target && e.target.tagName) || "";
        if (/^(INPUT|SELECT|TEXTAREA)$/.test(tag)) return;
        const n = Number(e.key);
        if (n >= 1 && n <= 9 && cards[n - 1]) openCard(cards[n - 1]);
      });
    },
  });
  // openDialog 的 build 在 append 前执行:首次拉数据/恢复清单放挂载后
  // (对话框此时已在 DOM,querySelector 才找得到 lib-grid)。
  reloadLibraryGrid();
  refreshRecoverStrip(recoverStrip);
}

/** library_list → 卡片栅格(搜索/归档筛选同参下发)。 */
export async function reloadLibraryGrid() {
  libRoot = String(libRoot || defaultLibraryRoot());
  setPref("libraryRoot", libRoot);
  const grid = document.querySelector('[data-testid="lib-grid"]');
  const emptyEl = document.querySelector('[data-testid="lib-empty"]');
  if (!grid || !emptyEl) return;
  const search = document.querySelector('[data-testid="lib-search"]');
  const env = await libraryList({
    root: libRoot,
    query: search && search.value ? search.value : undefined,
    includeArchived: showArchived,
  });
  clear(grid);
  cards = env.ok ? (env.data.projects || []) : [];
  if (!env.ok) {
    emptyEl.hidden = false;
    emptyEl.textContent = `库不可读:${env.message || env.code}(检查库根目录)`;
    return;
  }
  emptyEl.hidden = cards.length > 0;
  if (!cards.length) emptyEl.textContent = "(库内暂无工程:点「＋ 新建」,或检查库根目录)";
  for (const c of cards) grid.appendChild(cardEl(c));
}

function cardEl(c) {
  const locked = Boolean(c.locked);
  const moveWhy = locked ? "工程正被别的服务窗口打开(活进程持锁),移动类操作被拒" : null;
  const invalid = !c.valid;
  const canvas = c.canvas || {};
  const cur = String(c.path) === String(projectStore.get().root);
  const ops = [
    ["lib-open", "打开", () => openCard(c), invalid ? "project.json 缺失或不可解析" : null],
    ["lib-rename", "重命名", () => openRename(c), moveWhy],
    ["lib-copy", "复制", () => openCopy(c), moveWhy],
    [c.archived ? "lib-unarchive" : "lib-archive", c.archived ? "恢复" : "归档",
      () => archiveToggle(c), moveWhy],
    ["lib-delete", "删除", () => openDelete(c), moveWhy],
  ];
  return h("div", {
    class: "lib-card", testid: "lib-card",
    dataset: { name: c.name, path: c.path, archived: String(Boolean(c.archived)), locked: String(locked) },
  }, [
    thumbEl(c),
    h("div", { class: "lib-name", title: c.path }, [
      h("span", { testid: "lib-card-name" }, [c.name]),
      cur ? h("span", { class: "badge", testid: "lib-badge-current" }, ["当前"]) : null,
      c.archived ? h("span", { class: "badge warn", testid: "lib-badge-archived" }, ["归档"]) : null,
      locked ? h("span", { class: "badge", testid: "lib-badge-locked" }, ["锁定"]) : null,
      invalid ? h("span", { class: "badge warn", testid: "lib-badge-invalid" }, ["无效"]) : null,
    ]),
    h("div", { class: "lib-meta", testid: "lib-meta" }, [
      `${canvas.width || "?"}×${canvas.height || "?"} · ${c.fps || "?"}fps · ${fmtDur(c.durationMs)}`
        + ` · ${c.clipCount ?? "—"} 段 · ${fmtDate(c.modifiedAtMs)}`,
    ]),
    h("div", { class: "lib-ops" }, ops.map(([tid, label, fn, why]) => h("button", {
      class: "mini", testid: tid, disabled: why ? true : null,
      "aria-disabled": why ? "true" : "false", title: why || label, onclick: fn,
    }, [label]))),
  ]);
}

/** 缩略位:仅当前会话工程可经 /media 直载(其余拒绝,画幅占位块诚实回落)。 */
function thumbEl(c) {
  const box = h("div", { class: "lib-thumb", testid: "lib-thumb" });
  const canvas = c.canvas || {};
  if (canvas.width && canvas.height) box.style.aspectRatio = `${canvas.width} / ${canvas.height}`;
  const root = String(projectStore.get().root || "").replace(/[\\/]+$/, "");
  const tp = String(c.thumbnailPath || "");
  if (tp && String(c.path) === root && tp.startsWith(root)) {
    const rel = tp.slice(root.length).replace(/^[\\/]+/, "");
    const img = h("img", { src: mediaUrlFor(rel, projectStore.get().token), alt: `${c.name} 缩略图`, loading: "lazy" });
    img.addEventListener("error", () => { img.remove(); box.classList.add("empty"); });
    box.appendChild(img);
  } else {
    box.classList.add("empty");
    box.title = "缩略图仅当前工程可直载(服务端 /media 限会话工程内);块面比例 = 工程画幅";
  }
  return box;
}

/* ---------------- 迁移入口(提示条 + 常驻菜单项) ---------------- */

/** 迁移提示按 root 记忆(诚实口径:/session 的 projectRel 在 serve 启动时缓存,
 * 迁移成功后本会话重载依旧读到旧值——迁移过的工程不再重弹,登记候 BE 动态派生)。 */
function dismissedMap() {
  const m = pref("migrateDismissed", {});
  return m && typeof m === "object" ? m : {};
}

function dismissMigrate(root) {
  const m = dismissedMap();
  m[String(root)] = true;
  setPref("migrateDismissed", m);
}

/** 顶栏迁移提示条(打开 v2/v1 工程时;知道了该工程不再打扰)。 */
export function mountMigrateNotice() {
  const root = String(projectStore.get().root || "");
  if (sessionLayout() === "v3" || dismissedMap()[root]) return;
  const banner = $("banner");
  if (!banner || document.querySelector('[data-testid="banner-migrate"]')) return;
  const kind = sessionLayout().toUpperCase();
  const row = h("div", { class: "banner-row warn", testid: "banner-migrate", hidden: true }, [
    h("span", null, [`当前工程为 ${kind} 布局,可迁移到 v3 扁平布局(差异见「工程」菜单)`]),
    h("button", { class: "mini", testid: "migrate-banner-run", title: "migrate_layout to=v3(迁移后页面重载)", onclick: () => runMigrate() }, ["立即迁移"]),
    h("button", {
      class: "mini", testid: "migrate-banner-dismiss", title: "本工程不再提示",
      onclick: () => { dismissMigrate(root); row.remove(); },
    }, ["知道了"]),
  ]);
  banner.appendChild(row);
  row.hidden = false;
}

/** 迁移确认 → 执行 → 重载(服务端常驻缓存/投影路径随重载重建)。 */
export function runMigrate() {
  openDialog({
    id: "migrate-dialog", title: "迁移到 v3 扁平布局?",
    build: (body) => {
      body.appendChild(h("p", { class: "hint" }, [MIGRATE_HINT]));
      body.appendChild(h("p", { class: "hint" }, [
        "预检冲突(映射目标已存在)整体拒绝;迁移完成后本页将自动重载。",
      ]));
      body.appendChild(h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "migrate-run", onclick: async () => {
            const { migrateLayout } = await import("../core/library-commands.js");
            const env = await migrateLayout(projectStore.get().root);
            if (env.ok) {
              dismissMigrate(projectStore.get().root); // 已是 v3(含幂等 NOOP);session projectRel 缓存旧值,不再重弹
              document.getElementById("migrate-dialog")?.remove();
              toast(env.data.idempotent ? "已是 v3 布局(幂等 NOOP),即将重载" : "迁移完成(v3),即将重载");
              setTimeout(() => location.reload(), 600);
            }
          },
        }, ["迁移并重载"]),
        h("button", { testid: "migrate-cancel", onclick: () => document.getElementById("migrate-dialog")?.remove() }, ["取消"]),
      ]));
    },
  });
}

/* ---------------- 「工程」菜单(顶栏常驻入口) ---------------- */

export function openProjectMenu(x, y) {
  const v3 = sessionLayout() === "v3";
  openContextMenu(x, y, [
    { label: "新建工程(向导)", fn: () => import("./wizard.js").then((m) => m.openWizard()), why: "project_new 向导(模板/画幅/布局)" },
    { label: "工程库…", fn: () => openLibrary(), why: "库卡片/七操作/恢复入口(启动器)" },
    { sep: true },
    {
      label: "迁移到 v3 扁平布局…", fn: () => runMigrate(), disabled: v3,
      why: v3 ? "当前工程已是 v3 扁平布局" : `migrate_layout to=v3(工程级,一次性;${MIGRATE_HINT})`,
    },
    {
      label: `当前布局:${sessionLayout()}`, disabled: true,
      why: v3 ? "真相源在工程根;媒体 media/、成片 exports/" : "迁移入口在上一项(工程菜单常驻,不弹窗打扰)",
    },
  ]);
}
