/* 工程库卡片操作对话框 + 恢复清单(册六 T6.1 壳侧,F3;自 library.js 纯移动拆分,
 * 行数红线 ≤400)。root=库根(libraryManage/libraryRecover 免锁面,目录级操作)。 */
import { h } from "../ui/dom.js";
import { openDialog } from "../ui/dialog.js";
import { textField, selectField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";
import { projectStore } from "../core/store.js";
import { libraryManage, libraryRecover } from "../core/library-commands.js";
import { currentLibRoot, reloadLibraryGrid } from "./library.js";

const CANVAS_PRESETS = [
  ["1080x1920", "9:16 竖屏"], ["1920x1080", "16:9 横屏"], ["1080x1080", "1:1 方形"],
  ["1080x1440", "3:4"], ["2160x3840", "4K 竖屏"], ["3840x2160", "4K 横屏"],
];
const TRACK_PRESETS = [
  ["video,audio", "视频+音频(口播)"],
  ["video,video,audio", "双视频+音频(双机位)"],
  ["video,audio,text", "视频+音频+文本"],
];

function baseName(p) {
  return String(p || "").replace(/[\\/]+$/, "").split(/[\\/]/).pop();
}

function closeDlg(id) {
  document.getElementById(id)?.remove();
}

function nameError(v) {
  return !v || /[\\/:*?"<>|]/.test(v) ? "名称必填且不能含路径非法字符" : "";
}

/* ---------------- 打开(会话绑定诚实口径) ---------------- */

export function openCard(c) {
  const root = String(projectStore.get().root || "");
  if (String(c.path) === String(root)) {
    toast(`「${c.name}」就是当前已打开的工程`);
    return;
  }
  const cmd = `cutforge-cli serve "${c.path}" --open`;
  openDialog({
    id: "lib-open-dialog",
    title: `打开「${c.name}」(编辑器会话在 serve 启动时绑定工程根)`,
    build: (body) => {
      body.appendChild(h("p", { class: "hint" }, [
        "当前服务窗口绑定的工程是:", h("br"),
        h("code", {}, [root || "(未知)"]),
      ]));
      body.appendChild(h("p", {}, [h("code", { class: "lib-cmd", testid: "lib-open-cmd" }, [cmd])]));
      const copy = h("button", {
        testid: "lib-open-copy", onclick: () => {
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(cmd).then(() => toast("命令已复制,到终端粘贴回车"),
              () => toast("复制失败(权限受限),请手选命令复制", false));
          } else toast("当前环境不支持剪贴板 API,请手选命令复制", false);
        },
      }, ["复制命令"]);
      body.appendChild(h("div", { class: "wizard-actions" }, [
        copy,
        h("button", { testid: "lib-open-close", onclick: () => closeDlg("lib-open-dialog") }, ["关闭"]),
      ]));
    },
  });
}

/* ---------------- 重命名 / 复制 / 归档 / 删除 ---------------- */

export function openRename(c) {
  openDialog({
    id: "lib-rename-dialog", title: `重命名「${c.name}」(目录级;slug/OpLog 不动)`,
    build: (body) => {
      const to = textField({ testid: "lib-rename-to", ariaLabel: "新名称", placeholder: "新名称(不含路径分隔符)" });
      body.appendChild(h("label", null, ["新名称 ", to.root]));
      body.appendChild(h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "lib-rename-run", onclick: async () => {
            const v = to.get().trim();
            const err = nameError(v);
            if (err) { toast(err, false); return; }
            const env = await libraryManage("rename", c.name, { to: v, root: currentLibRoot() });
            if (env.ok) {
              toast(`已重命名:${c.name} → ${v}`);
              closeDlg("lib-rename-dialog");
              reloadLibraryGrid();
            }
          },
        }, ["重命名"]),
        h("button", { testid: "lib-rename-cancel", onclick: () => closeDlg("lib-rename-dialog") }, ["取消"]),
      ]));
    },
  });
}

export function openCopy(c) {
  openDialog({
    id: "lib-copy-dialog", title: `复制「${c.name}」(整目录复制,含素材/产物/OpLog)`,
    build: (body) => {
      const to = textField({ testid: "lib-copy-to", ariaLabel: "副本名", placeholder: "副本名" });
      to.set(`${c.name}-copy`);
      body.appendChild(h("label", null, ["副本名 ", to.root]));
      body.appendChild(h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "lib-copy-run", onclick: async () => {
            const v = to.get().trim();
            const err = nameError(v);
            if (err) { toast(err, false); return; }
            const env = await libraryManage("copy", c.name, { to: v, root: currentLibRoot() });
            if (env.ok) {
              toast(`已复制为「${v}」`);
              closeDlg("lib-copy-dialog");
              reloadLibraryGrid();
            }
          },
        }, ["复制"]),
        h("button", { testid: "lib-copy-cancel", onclick: () => closeDlg("lib-copy-dialog") }, ["取消"]),
      ]));
    },
  });
}

export async function archiveToggle(c) {
  const env = await libraryManage(c.archived ? "unarchive" : "archive", c.name, { root: currentLibRoot() });
  if (env.ok) {
    toast(c.archived ? `已从 .archives 恢复「${c.name}」` : `已归档「${c.name}」(.archives/)`);
    reloadLibraryGrid();
  }
}

export function openDelete(c) {
  openDialog({
    id: "lib-delete-dialog", title: `删除「${c.name}」?`,
    build: (body) => {
      body.appendChild(h("p", { class: "hint" }, [
        "删除 = 整目录移入 .trash/<时间戳>-名/(不真删,可捞回);活进程持锁时会被拒绝。",
      ]));
      body.appendChild(h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "lib-delete-run", onclick: async () => {
            const env = await libraryManage("delete", c.name, { root: currentLibRoot() });
            if (env.ok) {
              toast(`已移入 .trash:「${c.name}」(可从文件管理器捞回)`);
              closeDlg("lib-delete-dialog");
              reloadLibraryGrid();
            }
          },
        }, ["删除(入 .trash)"]),
        h("button", { testid: "lib-delete-cancel", onclick: () => closeDlg("lib-delete-dialog") }, ["取消"]),
      ]));
    },
  });
}

/* ---------------- 库内新建(名称+画幅+布局+轨道模板) ---------------- */

export function openNewInLibrary() {
  openDialog({
    id: "lib-new-dialog", title: "库内新建(library_manage new;ADR-0021:过渡期缺省 v2)",
    build: (body) => {
      const name = textField({ testid: "lib-new-name", ariaLabel: "工程名", placeholder: "my-next-cut" });
      const canvas = selectField({ testid: "lib-new-canvas", options: CANVAS_PRESETS });
      const layout = selectField({
        testid: "lib-new-layout", options: [["v2", "v2(缺省;目录契约 0.5)"], ["v3", "v3 扁平(media/ + exports/)"]],
      });
      const tracks = selectField({ testid: "lib-new-tracks", options: TRACK_PRESETS });
      body.appendChild(h("label", null, ["工程名 ", name]));
      body.appendChild(h("label", null, ["画幅 ", canvas.root]));
      body.appendChild(h("label", null, ["布局 ", layout.root]));
      body.appendChild(h("label", null, ["轨道模板 ", tracks.root]));
      body.appendChild(h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "lib-new-run", onclick: async () => {
            const v = name.get().trim();
            const err = nameError(v);
            if (err) { toast(err, false); return; }
            const [w, hh] = canvas.get().split("x").map(Number);
            const env = await libraryManage("new", v, {
              root: currentLibRoot(), layout: layout.get(), canvasW: w, canvasH: hh,
              fps: 30, tracks: tracks.get().split(","),
            });
            if (env.ok) {
              toast(`已创建「${v}」(${env.data.layout});打开它:在工程库卡片点「打开」拿命令`);
              closeDlg("lib-new-dialog");
              reloadLibraryGrid();
            }
          },
        }, ["创建"]),
        h("button", { testid: "lib-new-cancel", onclick: () => closeDlg("lib-new-dialog") }, ["取消"]),
      ]));
    },
  });
}

/* ---------------- 恢复清单(library_recover list → recover) ---------------- */

export async function refreshRecoverStrip(strip) {
  const env = await libraryRecover("list", undefined, currentLibRoot());
  const items = env.ok ? (env.data.stale || []) : [];
  while (strip.firstChild) strip.removeChild(strip.firstChild);
  strip.hidden = !items.length;
  if (!items.length) return;
  strip.appendChild(h("div", { class: "lib-recover-title" }, [
    `⚠ ${items.length} 个工程有残留锁(崩溃/强关遗留),可恢复:`,
  ]));
  for (const it of items) {
    const name = baseName(it.root);
    strip.appendChild(h("div", { class: "lib-recover-row", testid: "lib-recover-item" }, [
      h("span", { class: "lib-recover-name" }, [name]),
      h("span", { class: "hint" }, [
        `pid ${it.pid}${it.pidAlive ? "(存活)" : "(已退出)"} · 锁龄 ${Math.round((it.ageMs || 0) / 1000)}s`
          + (it.session ? ` · 会话 rev ${it.session[0]}→${it.session[1]}` : ""),
      ]),
      h("button", {
        class: "mini", testid: "lib-recover-run", title: "library_recover recover:清残留锁 + OpLog 一致性校验",
        onclick: async () => {
          const r = await libraryRecover("recover", name, currentLibRoot());
          if (r.ok) {
            toast(`「${name}」已恢复(rev ${r.data.rev},${r.data.opCount} Op 对账通过)`);
            refreshRecoverStrip(strip);
            reloadLibraryGrid();
          }
        },
      }, ["恢复"]),
    ]));
  }
}

/** 顶栏恢复提示条检查(启动后静默一次;有可恢复项才显示)。 */
export async function checkRecoverBanner() {
  const env = await libraryRecover("list", undefined, currentLibRoot());
  const items = env.ok ? (env.data.stale || []) : [];
  if (!items.length) return;
  const { openLibrary } = await import("./library.js");
  const banner = document.getElementById("banner");
  if (!banner || document.querySelector('[data-testid="banner-recover"]')) return;
  const row = h("div", { class: "banner-row warn", testid: "banner-recover", hidden: true }, [
    h("span", null, [`检测到 ${items.length} 个工程有崩溃残留锁,可一键恢复(清锁 + OpLog 对账)`]),
    h("button", { class: "mini", testid: "recover-banner-open", title: "打开工程库的恢复清单", onclick: () => openLibrary() }, ["去恢复"]),
  ]);
  banner.appendChild(row);
  row.hidden = false;
}
