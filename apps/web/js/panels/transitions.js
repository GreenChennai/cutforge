/* 转场库面板(册四 T4.5 FE2):58 项目录分类网格 + 搜索/收藏 + 点击/拖放应用。
 *
 * - 数据:GET /catalogs(编译期嵌入;壳不读文件);缩略图 /assets/transitions/<id>.jpg
 *   (240x135 jpeg,IntersectionObserver 懒加载);
 * - 应用:transition_set {type:"fade", fx:"tr.<id>", durMs}(fx 为准;单 Op 可撤销);
 *   点击 = 应用到选中片段;拖到片段 = 应用到该片段(HTML5 DnD,事件代理在 #tracks);
 * - 当前片段已应用转场卡片高亮(.applied);收藏存 localStorage(prefs 命名空间);
 * - 钳制提示:应用时 durMs > 片段时长 → 后端钳制 + WARN,壳 toast 诚实预告(不阻塞);
 *   检查器「转场」组的持续钳制提示见 insp-groups.js。
 */
import { h, clear } from "../ui/dom.js";
import { timelineStore, selectionStore } from "../core/store.js";
import { applyTransition } from "../core/edit-commands.js";
import {
  ensureCatalogs, transitionList, transitionDefaultDurMs, thumbUrlFor,
} from "../core/catalogs.js";
import { textField } from "../ui/controls.js";
import { pref, setPref } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";

let searchInput = null;
let favOnly = false;
let grid = null;
let status = null;
/** @type {IntersectionObserver|null} */
let io = null;

function favs() { return pref("transFavs", []); }
function isFav(id) { return favs().includes(id); }
function toggleFav(id) {
  const cur = favs();
  const next = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
  setPref("transFavs", next);
  renderGrid();
}

export function mount(container) {
  container.appendChild(h("h3", null, ["转场库(58 项 · ffmpeg xfade 实测枚举)"]));
  status = h("div", { class: "hint", testid: "trans-status" }, ["目录加载中…"]);
  container.appendChild(status);
  const bar = h("div", { class: "lib-bar" });
  searchInput = textField({
    testid: "trans-search", ariaLabel: "搜索转场", placeholder: "搜索名称/ID…", onInput: () => renderGrid(),
  });
  bar.appendChild(searchInput.root);
  const favChip = h("button", {
    class: "chip", testid: "trans-fav-chip", "aria-pressed": "false",
    title: "只看收藏(收藏存本机浏览器)", onclick: () => {
      favOnly = !favOnly;
      favChip.classList.toggle("on", favOnly);
      favChip.setAttribute("aria-pressed", favOnly ? "true" : "false");
      renderGrid();
    },
  }, ["★ 收藏"]);
  bar.appendChild(favChip);
  container.appendChild(bar);
  container.appendChild(h("div", { class: "hint" }, [
    "点击卡片 = 应用到选中片段;或拖到时间线片段上(durMs 缺省走目录默认,时长/方向在检查器·转场组调)。",
    "转场应用在与前一片段之间;预览代理不含转场最终效果。",
  ]));
  grid = h("div", { class: "lib-grid", testid: "trans-grid" });
  container.appendChild(grid);
  io = new IntersectionObserver((entries) => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      const img = en.target.querySelector("img");
      if (img && !img.src) img.src = img.dataset.src;
      io.unobserve(en.target);
    }
  }, { rootMargin: "140px" });
  ensureCatalogs().then((cat) => {
    if (!cat) { status.textContent = "目录下发失败(/catalogs):转场库不可用(刷新页面重试)"; return; }
    status.textContent = "";
    renderGrid();
  });
  // 已应用高亮:选中/投影变化时按当前片段转场刷新卡片态
  const refreshApplied = () => markApplied();
  selectionStore.subscribe(refreshApplied);
  timelineStore.subscribe(refreshApplied);
  mountDropTargets();
}

function visibleTransitions() {
  const q = (searchInput.get() || "").trim().toLowerCase();
  return transitionList().filter((t) => (!favOnly || isFav(t.id))
    && (!q || t.name.toLowerCase().includes(q) || t.id.toLowerCase().includes(q)));
}

function renderGrid() {
  if (!grid) return;
  clear(grid);
  const list = visibleTransitions();
  if (!transitionList().length) return; // 目录未就绪:保持加载态
  if (!list.length) {
    grid.appendChild(h("div", { class: "empty-hint", testid: "trans-empty" }, [
      favOnly ? "还没有收藏:点卡片右上角 ★ 收藏常用转场" : "无匹配转场,换个关键词试试",
    ]));
    return;
  }
  let lastCat = "";
  for (const t of list) {
    if (t.category !== lastCat) {
      lastCat = t.category;
      grid.appendChild(h("div", { class: "lib-cat" }, [t.category]));
    }
    grid.appendChild(transCard(t));
  }
  markApplied();
}

function transCard(t) {
  const card = h("div", {
    class: "lib-card", testid: "trans-card", dataset: { trans: t.id }, draggable: true,
    title: `${t.id}${t.directional ? " · 方向编码在名内" : ""}`, tabindex: "0", role: "button",
    "aria-label": `应用转场 ${t.name}`,
  }, [
    h("span", { class: "lib-thumb" }, [
      h("img", { alt: t.name, "data-src": thumbUrlFor(t.id), loading: "lazy" }),
    ]),
    h("span", { class: "lib-name" }, [t.name]),
    h("button", {
      class: `lib-fav${isFav(t.id) ? " on" : ""}`, testid: "trans-fav", title: "收藏/取消收藏",
      "aria-label": `收藏 ${t.name}`, "aria-pressed": isFav(t.id) ? "true" : "false",
      onclick: (e) => { e.stopPropagation(); toggleFav(t.id); },
    }, ["★"]),
  ]);
  const act = () => applyToSelected(t);
  card.addEventListener("click", act);
  card.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); act(); } });
  card.addEventListener("dragstart", (e) => {
    e.dataTransfer.setData("text/cutforge-transition", t.id);
  });
  if (io) io.observe(card);
  return card;
}

function applyToSelected(t, clipId) {
  const row = clipId
    ? timelineStore.get().clips.find((c) => c.id === clipId)
    : timelineStore.get().clips.find((c) => c.id === selectionStore.get().clipId);
  if (!row) { toast("先选中目标片段(转场应用在其与前一片段之间)", false); return; }
  const dur = transitionDefaultDurMs();
  const clipDur = row.endMs - row.startMs;
  if (dur > clipDur) {
    toast(`转场时长 ${dur}ms > 片段时长 ${clipDur}ms:渲染端将钳制到片段时长(不阻塞)`, false);
  }
  applyTransition(row.id, { id: t.id, durMs: dur });
}

/** 当前选中片段的转场卡片高亮(fx=tr.<id> 或 type 直配旧枚举)。 */
function markApplied() {
  if (!grid) return;
  const sel = timelineStore.get().clips.find((c) => c.id === selectionStore.get().clipId);
  const fx = sel && sel.transition && sel.transition.fx;
  const type = sel && sel.transition && sel.transition.type;
  const curId = fx && fx.startsWith("tr.") ? fx.slice(3) : (type || "");
  for (const el of grid.querySelectorAll(".lib-card")) {
    el.classList.toggle("applied", Boolean(curId) && el.dataset.trans === curId);
  }
}

/** 拖放落点(装配一次;事件代理在 #tracks):拖到片段 = 应用到该片段。 */
function mountDropTargets() {
  const tracks = document.getElementById("tracks");
  if (!tracks) return;
  tracks.addEventListener("dragover", (e) => {
    if (e.dataTransfer && [...e.dataTransfer.types].includes("text/cutforge-transition")
      && /** @type {HTMLElement} */ (e.target).closest?.(".clip")) {
      e.preventDefault();
    }
  });
  tracks.addEventListener("drop", (e) => {
    const id = e.dataTransfer ? e.dataTransfer.getData("text/cutforge-transition") : "";
    if (!id) return;
    const clipEl = /** @type {HTMLElement} */ (e.target).closest?.(".clip");
    if (!clipEl) return;
    e.preventDefault();
    applyToSelected({ id }, clipEl.dataset.id);
  });
}
