/* 素材库页签(册六 T6.2 壳侧,F3):media_library 浏览/kind·标签·名称过滤/
 * 一键导入工程(media_import 拷贝导入,FE1「拷贝导入候 BE」欠账的壳侧收口)。
 *
 * 诚实口径:浏览器沙箱不给拖入/ picked 文件的磁盘绝对路径(File.path 仅部分
 * webview 暴露)——可用导入通道 = ①素材库引用一键导入(本页签)/ ②绝对路径输入;
 * 拖放与文件选择器检测到文件时:能拿绝对路径就直接 media_import,拿不到就如实
 * 指引两条通道(不假报成功)。库根缺省面 = env CUTFORGE_MEDIA /
 * %USERPROFILE%\CutForge\Media(壳读不到 env,首次需手填,偏好记忆)。
 */
import { h, clear } from "../ui/dom.js";
import { openDialog } from "../ui/dialog.js";
import { textField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";
import { pref, setPref } from "../ui/prefs.js";
import { fmtDur } from "./library.js";
import { mediaImport, mediaLibraryList, mediaLibraryTag } from "../core/library-commands.js";
import { browseMedia, insertMediaAuto } from "../core/commands.js";

const KINDS = [["", "全部"], ["video", "视频"], ["audio", "音频"], ["image", "图片"], ["lut", "LUT"], ["text", "字幕"]];

let kind = "";
let listEl = null;
let emptyEl = null;

function libRoot() {
  return String(pref("mediaLibRoot", "") || "");
}

export function mountMediaLib(container) {
  const rootIn = textField({
    testid: "media-lib-root", ariaLabel: "素材库根目录", placeholder: "素材库根目录(env CUTFORGE_MEDIA 缺省)",
    onEnter: () => refreshMediaLib(),
  });
  rootIn.set(libRoot());
  const chips = h("div", { class: "media-filters", testid: "media-lib-kinds", role: "group", "aria-label": "素材库类型过滤" });
  for (const [v, label] of KINDS) {
    chips.appendChild(h("button", {
      class: `chip${v === kind ? " on" : ""}`, dataset: { filter: v },
      "aria-pressed": v === kind ? "true" : "false", title: `只看${label}`,
      onclick: () => {
        kind = v;
        for (const b of chips.querySelectorAll(".chip")) {
          const on = b.dataset.filter === kind;
          b.classList.toggle("on", on);
          b.setAttribute("aria-pressed", on ? "true" : "false");
        }
        refreshMediaLib();
      },
    }, [label]));
  }
  const tagIn = textField({
    testid: "media-lib-tag", ariaLabel: "按标签过滤", placeholder: "标签过滤…",
    onInput: () => refreshMediaLib(),
  });
  const queryIn = textField({
    testid: "media-lib-query", ariaLabel: "按名称搜索素材库", placeholder: "搜索名称…",
    onInput: () => refreshMediaLib(),
  });
  container.appendChild(h("div", { class: "media-dir-row" }, [
    rootIn.root,
    h("button", { class: "mini", testid: "media-lib-refresh", title: "media_library 重新扫描(幂等合并 manifest)", onclick: () => refreshMediaLib() }, ["⟳"]),
  ]));
  container.appendChild(chips);
  container.appendChild(h("div", { class: "media-dir-row" }, [tagIn.root, queryIn.root]));
  listEl = h("div", { class: "media-lib-list", testid: "media-lib-list" });
  emptyEl = h("div", { class: "empty-hint", testid: "media-lib-empty", hidden: true });
  container.appendChild(listEl);
  container.appendChild(emptyEl);
  container.appendChild(h("div", { class: "hint" }, [
    "「导入」= 拷贝入工程(原文件不动,同名同内容幂等);勾「自动插入」后导入即插到播放头。",
  ]));
  refreshMediaLib();
}

export async function refreshMediaLib() {
  const rootIn = document.querySelector('[data-testid="media-lib-root"]');
  if (rootIn) setPref("mediaLibRoot", String(rootIn.value || "").trim());
  if (!listEl) return;
  const root = libRoot();
  if (!root) {
    clear(listEl);
    emptyEl.hidden = false;
    emptyEl.textContent = "先填素材库根目录(env CUTFORGE_MEDIA 缺省面,壳读不到 env;填一次即记忆)";
    return;
  }
  const tagIn = document.querySelector('[data-testid="media-lib-tag"]');
  const queryIn = document.querySelector('[data-testid="media-lib-query"]');
  const env = await mediaLibraryList({
    root, kind: kind || undefined,
    tag: tagIn && tagIn.value ? tagIn.value.trim() : undefined,
    query: queryIn && queryIn.value ? queryIn.value.trim() : undefined,
  });
  clear(listEl);
  const entries = env.ok ? (env.data.entries || []) : [];
  if (!env.ok) {
    emptyEl.hidden = false;
    emptyEl.textContent = `素材库不可读:${env.message || env.code}`;
    return;
  }
  emptyEl.hidden = entries.length > 0;
  if (!entries.length) {
    emptyEl.textContent = "(素材库为空或无命中:把文件放进库根后点 ⟳ 扫描;或清空过滤)";
    return;
  }
  for (const e of entries) listEl.appendChild(rowEl(e));
}

function rowEl(e) {
  return h("div", { class: "media-lib-item", testid: "media-lib-item", dataset: { ref: e.ref, kind: e.kind } }, [
    h("span", { class: `badge ${e.kind === "lut" || e.kind === "text" ? "image" : e.kind}` }, [e.kind]),
    h("span", { class: "media-name", title: e.ref }, [e.name]),
    h("span", { class: "dim" }, [
      e.durationMs ? ` ${fmtDur(e.durationMs)}` : "",
      e.tags && e.tags.length ? ` #${e.tags.join(" #")}` : "",
    ]),
    h("button", {
      class: "mini", testid: "media-lib-import", title: "media_import:拷贝入工程(布局感知落点)",
      onclick: () => importRef(e.ref),
    }, ["导入"]),
    h("button", {
      class: "mini", testid: "media-lib-tag-edit", title: "media_library tag:改条目标签(整组替换)",
      onclick: () => editTags(e),
    }, ["标签"]),
  ]);
}

/** 一键导入:media_import(素材库相对引用;库根 = 页签输入值,与展示同源)
 * → 可选自动插入播放头。 */
async function importRef(ref) {
  const auto = document.querySelector('[data-testid="media-import-auto"]');
  const env = await mediaImport(ref, libRoot() || undefined);
  if (!env.ok) return;
  const src = env.data.src;
  toast(`已导入工程:${src}(原文件不动)`);
  if (auto && auto.checked) {
    insertMediaAuto({ path: src, kind: env.data.kind, durationMs: env.data.durationMs });
  } else {
    browseMedia(); // 素材面板刷新(media_browse 重新浏览当前目录)
  }
  refreshMediaLib();
}

function editTags(e) {
  openDialog({
    id: "media-lib-tags-dialog", title: `标签:${e.name}(整组替换;空 = 清空)`,
    build: (body) => {
      const tags = textField({
        testid: "media-lib-tags-input", ariaLabel: "标签(逗号分隔)", placeholder: "tag1,tag2",
      });
      tags.set((e.tags || []).join(","));
      body.appendChild(h("label", null, ["标签(逗号分隔) ", tags.root]));
      body.appendChild(h("div", { class: "wizard-actions" }, [
        h("button", {
          testid: "media-lib-tags-run", onclick: async () => {
            const list = tags.get().split(/[,，]/).map((s) => s.trim()).filter(Boolean);
            const env = await mediaLibraryTag(e.ref, list, libRoot());
            if (env.ok) {
              toast("标签已更新");
              document.getElementById("media-lib-tags-dialog")?.remove();
              refreshMediaLib();
            }
          },
        }, ["保存"]),
        h("button", { testid: "media-lib-tags-cancel", onclick: () => document.getElementById("media-lib-tags-dialog")?.remove() }, ["取消"]),
      ]));
    },
  });
}

/**
 * 拖放/文件选择器入口(供 media-panel 调用):逐文件尝试绝对路径导入。
 * 浏览器拿不到绝对路径时如实指引(不假报成功);返回已发起的导入数。
 */
export async function importPickedFiles(files) {
  let started = 0;
  for (const f of Array.from(files || [])) {
    const abs = f.path || ""; // 仅部分 webview 暴露;浏览器拖入/选择一律拿不到
    if (!abs) continue;
    started += 1;
    const env = await mediaImport(abs);
    if (env.ok) toast(`已导入工程:${env.data.src}`);
  }
  if (!started) {
    toast("浏览器拿不到拖入文件的磁盘绝对路径(沙箱口径):请把文件放进素材库根后到「素材库」页签一键导入,或在导入框直接输入绝对路径", false);
  } else {
    browseMedia();
  }
  return started;
}
