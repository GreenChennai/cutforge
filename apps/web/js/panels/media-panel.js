/* 素材面板(T2.5 基础 + T4.1 升级 + 册六 T6.2 导入升级):缩略卡(懒加载)/类型过滤/
 * 搜索/最近使用/代理生成/双页签(工程素材 | 素材库)/拷贝导入。
 * 行为对拍红线:media-item 双击插入、set-bgm、右键菜单、HTML5 DnD 拖放 MIME
 * (旧壳契约)不变。册六:外部拖放/文件选择器 → media_import 拷贝入工程
 * (浏览器拿不到绝对路径时如实指引素材库/路径输入通道,见 media-lib.js)。 */
import { h, clear } from "../ui/dom.js";
import { mediaStore } from "../core/store.js";
import { browseMedia, insertMediaAuto, setBgm } from "../core/commands.js";
import { ephemeralStore } from "../core/store.js";
import { textField } from "../ui/controls.js";
import { openMediaContextMenu } from "../ui/menu.js";
import { toast } from "../ui/toast.js";
import { createMediaCard, createCardObserver, setTokenProvider } from "./media-card.js";
import { projectStore } from "../core/store.js";
import { mountMediaLib, refreshMediaLib, importPickedFiles } from "./media-lib.js";
import { mediaImport } from "../core/library-commands.js";

const FILTERS = [
  ["all", "全部"], ["video", "视频"], ["audio", "音频"], ["image", "图片"],
];

let dirField = null;
let listEl = null;
let searchInput = null;
let chipsEl = null;
let cardIO = null;
let filter = "all";

export function mount(container) {
  setTokenProvider(() => projectStore.get().token || "");
  container.appendChild(h("h3", null, ["素材面板"]));
  // 册六 T6.2 双页签:工程素材(原有面,红线不动)| 素材库(media_library)
  const projPane = h("div", { class: "media-pane", testid: "media-pane-project" });
  const libPane = h("div", { class: "media-pane", testid: "media-pane-library", hidden: true });
  const tabProj = h("button", {
    class: "chip on", testid: "media-tab-project", "aria-pressed": "true",
    title: "工程内素材(media_browse)",
    onclick: () => switchPane(true, tabProj, tabLib, projPane, libPane),
  }, ["工程素材"]);
  const tabLib = h("button", {
    class: "chip", testid: "media-tab-library", "aria-pressed": "false",
    title: "素材库(media_library;一键拷贝导入)",
    onclick: () => switchPane(false, tabProj, tabLib, projPane, libPane),
  }, ["素材库"]);
  container.appendChild(h("div", { class: "media-filters", testid: "media-tabs" }, [tabProj, tabLib]));
  container.appendChild(projPane);
  container.appendChild(libPane);
  mountMediaLib(libPane);
  buildProjectPane(projPane);
  // 外部文件拖入:media_import 拷贝导入(能拿绝对路径就导,拿不到如实指引)
  container.addEventListener("dragover", (e) => {
    if (e.dataTransfer && [...e.dataTransfer.types].includes("Files")) e.preventDefault();
  });
  container.addEventListener("drop", (e) => {
    if (!e.dataTransfer || !e.dataTransfer.files || !e.dataTransfer.files.length) return;
    e.preventDefault();
    importPickedFiles(e.dataTransfer.files);
  });
}

function switchPane(proj, tabProj, tabLib, projPane, libPane) {
  projPane.hidden = !proj;
  libPane.hidden = proj;
  tabProj.classList.toggle("on", proj);
  tabProj.setAttribute("aria-pressed", proj ? "true" : "false");
  tabLib.classList.toggle("on", !proj);
  tabLib.setAttribute("aria-pressed", proj ? "false" : "true");
  if (!proj) refreshMediaLib();
}

function buildProjectPane(pane) {
  const container = pane;
  dirField = textField({
    id: "media-dir", testid: "media-dir", ariaLabel: "素材目录", onEnter: () => refresh(),
  });
  dirField.root.value = mediaStore.get().dir;
  container.appendChild(h("div", { class: "media-dir-row" }, [
    dirField.root,
    h("button", { id: "media-refresh", testid: "media-refresh", title: "刷新列表", "data-tip": "刷新列表", onclick: () => refresh() }, ["⟳"]),
  ]));
  // 册六 T6.2 拷贝导入行:绝对路径输入 + 文件选择器(拿不到绝对路径时诚实指引)
  const importSrc = textField({
    testid: "media-import-src", ariaLabel: "素材绝对路径", placeholder: "绝对路径导入(media_import 拷贝入工程)",
    onEnter: () => runPathImport(importSrc),
  });
  const pick = h("input", {
    type: "file", testid: "media-import-pick", class: "sr-file", title: "选择文件(media_import)",
    "aria-label": "选择文件导入(浏览器拿不到绝对路径时会给指引)",
  });
  pick.addEventListener("change", () => {
    const files = pick.files;
    importPickedFiles(files);
    pick.value = "";
  });
  container.appendChild(h("div", { class: "media-dir-row", testid: "media-import-row" }, [
    importSrc.root,
    h("button", { class: "mini", testid: "media-import-run", title: "media_import:拷贝入工程(不产 Op)", onclick: () => runPathImport(importSrc) }, ["导入"]),
    pick,
  ]));
  container.appendChild(h("label", { class: "toggle", testid: "media-import-auto-row" }, [
    h("input", { type: "checkbox", testid: "media-import-auto", title: "导入成功后自动插到播放头(素材库一键导入同样生效)" }),
    " 导入后自动插到播放头",
  ]));
  // T4.1:类型过滤 chips + 搜索(会话态,不入投影)
  chipsEl = h("div", { class: "media-filters", role: "group", "aria-label": "类型过滤", testid: "media-filters" });
  for (const [v, label] of FILTERS) {
    chipsEl.appendChild(h("button", {
      class: `chip${v === filter ? " on" : ""}`, dataset: { filter: v },
      "aria-pressed": v === filter ? "true" : "false",
      title: `只看${label}`, onclick: () => setFilter(v),
    }, [label]));
  }
  chipsEl.appendChild(h("button", {
    class: "chip chip-recent", dataset: { filter: "recent" }, "aria-pressed": "false",
    title: "最近使用(会话内插入过的素材)", onclick: () => setFilter("recent"),
  }, ["最近"]));
  container.appendChild(chipsEl);
  searchInput = textField({
    id: "media-search", testid: "media-search", ariaLabel: "按名称搜索素材",
    placeholder: "搜索名称…", onInput: () => renderList(),
  });
  container.appendChild(searchInput.root);
  listEl = h("div", { id: "media-list", testid: "media-list" }, [
    h("div", { class: "empty-hint" }, ["加载中…"]),
  ]);
  container.appendChild(listEl);
  container.appendChild(h("div", { class: "hint" }, [
    "双击 = 插到匹配轨道的播放头;或拖到下方轨道任意落点。外部文件:上方「导入」= media_import 拷贝入工程;或拖文件进面板。",
  ]));
  cardIO = createCardObserver();

  // 素材行「设为 BGM」(事件代理,旧壳口径)
  listEl.addEventListener("click", (e) => {
    const btn = /** @type {HTMLElement} */ (e.target);
    if (!btn.classList || !btn.classList.contains("set-bgm")) return;
    const path = btn.dataset.src;
    if (!path) return;
    setBgm({ src: path });
  });

  mediaStore.subscribe(() => renderList());
}

/** 路径导入(绝对路径;media_import 解析失败会如实回报素材不可达)。 */
async function runPathImport(srcField) {
  const v = String(srcField.get() || "").trim();
  if (!v) {
    toast("先填素材绝对路径(或从「素材库」页签一键导入)", false);
    return;
  }
  const env = await mediaImport(v);
  if (env.ok) {
    toast(`已导入工程:${env.data.src}(原文件不动)`);
    srcField.set("");
    const auto = document.querySelector('[data-testid="media-import-auto"]');
    if (auto && auto.checked) {
      insertMediaAuto({ path: env.data.src, kind: env.data.kind, durationMs: env.data.durationMs });
    }
    await refresh();
  }
}

async function refresh() {
  const dir = dirField.get().trim();
  await browseMedia(dir);
}

function setFilter(v) {
  filter = v;
  for (const btn of chipsEl.querySelectorAll(".chip")) {
    const on = btn.dataset.filter === v;
    btn.classList.toggle("on", on);
    btn.setAttribute("aria-pressed", on ? "true" : "false");
  }
  renderList();
}

/** 过滤 + 搜索 + 最近使用的可见集(排序保持浏览序;最近按使用序)。 */
function visibleFiles() {
  const st = mediaStore.get();
  const q = (searchInput.get() || "").trim().toLowerCase();
  if (filter === "recent") {
    const recent = ephemeralStore.get().recentMedia || [];
    return recent
      .map((p) => st.files.find((f) => f.path === p))
      .filter((f) => f && (!q || f.name.toLowerCase().includes(q)));
  }
  return st.files.filter((f) => (filter === "all" || f.kind === filter)
    && (!q || f.name.toLowerCase().includes(q)));
}

function renderList() {
  const st = mediaStore.get();
  clear(listEl);
  if (st.error) {
    listEl.appendChild(h("div", { class: "empty-hint" }, [st.error]));
    return;
  }
  const files = visibleFiles();
  if (!files.length) {
    listEl.appendChild(h("div", { class: "empty-hint" }, [
      filter === "recent" ? "还没有使用记录:双击素材插入后这里会记住"
        : `(${st.dir || "工程根"}) ${qText()}无可导入媒体;把 mp4/mp3/png 等放进该目录后点 ⟳`,
    ]));
    return;
  }
  if (filter === "recent") {
    listEl.appendChild(h("div", { class: "empty-hint dim" }, ["最近使用(会话内;从上到下由新到旧)"]));
  }
  for (const f of files) {
    const row = createMediaCard(f, { onInsert: insertMediaAuto });
    listEl.appendChild(row);
    cardIO.observe(row);
  }
}

function qText() {
  const q = (searchInput.get() || "").trim();
  return q ? `搜索「${q}」` : "";
}

/** boot 后首次浏览(供 main;目录默认值由 mediaStore 初始化)。 */
export function initialMediaBrowse() {
  return refresh();
}
