/* 素材面板(T2.5 基础 + T4.1 升级):缩略卡(懒加载)/类型过滤/搜索/最近使用/
 * 代理生成/导入降级。行为对拍红线:media-item 双击插入、set-bgm、右键菜单、
 * HTML5 DnD 拖放 MIME(旧壳契约)不变。
 * 导入通道实况(诚实口径):后端 56 工具与数据面均无「外部文件拷入工程」通道
 * (/media 只读、/rpc 无上传、resolve_within_root 拒工程根外路径)——外部拖放
 * 如实提示「把文件放进工程目录后刷新」,登记「拷贝导入候 BE 补」。 */
import { h, clear } from "../ui/dom.js";
import { mediaStore } from "../core/store.js";
import { browseMedia, insertMediaAuto, setBgm } from "../core/commands.js";
import { ephemeralStore } from "../core/store.js";
import { textField } from "../ui/controls.js";
import { openMediaContextMenu } from "../ui/menu.js";
import { toast } from "../ui/toast.js";
import { createMediaCard, createCardObserver, setTokenProvider } from "./media-card.js";
import { projectStore } from "../core/store.js";

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
  dirField = textField({
    id: "media-dir", testid: "media-dir", ariaLabel: "素材目录", onEnter: () => refresh(),
  });
  dirField.root.value = mediaStore.get().dir;
  container.appendChild(h("div", { class: "media-dir-row" }, [
    dirField.root,
    h("button", { id: "media-refresh", testid: "media-refresh", title: "刷新列表", "data-tip": "刷新列表", onclick: () => refresh() }, ["⟳"]),
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
    "双击 = 插到匹配轨道的播放头;或拖到下方轨道任意落点。外部文件:先拷进工程目录再点 ⟳(浏览器无拷入通道)",
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
  // 外部文件拖入:诚实降级(无导入拷贝通道,登记候 BE 补)
  container.addEventListener("dragover", (e) => {
    if (e.dataTransfer && [...e.dataTransfer.types].includes("Files")) e.preventDefault();
  });
  container.addEventListener("drop", onExternalDrop);

  mediaStore.subscribe(() => renderList());
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

/** 外部文件/文件夹拖入:浏览器壳无「拷贝进工程」通道(见文件头),如实指引。 */
function onExternalDrop(e) {
  if (!e.dataTransfer || !e.dataTransfer.files || !e.dataTransfer.files.length) return;
  e.preventDefault();
  const n = e.dataTransfer.files.length;
  toast(`检测到 ${n} 个外部文件:浏览器壳无「拷入工程」通道(拷贝导入候 BE 补)。`
    + `请把文件复制进 ${mediaStore.get().dir || "工程目录"} 后点 ⟳ 刷新`, false);
}

/** boot 后首次浏览(供 main;目录默认值由 mediaStore 初始化)。 */
export function initialMediaBrowse() {
  return refresh();
}
