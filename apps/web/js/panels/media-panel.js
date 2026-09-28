/* 素材面板(T2.5,E3-4 行为对拍):目录浏览/双击插入/拖放到轨道/一键设 BGM。
 * W12 补课:媒体缩略卡——当前为「图标卡 + 文件名 + 时长」(真缩略图候册四 T4.1,
 * 依赖代理转码产帧,诚实降级不画假缩略图)。 */
import { h, clear } from "../ui/dom.js";
import { mediaStore } from "../core/store.js";
import { browseMedia, insertMediaAuto, setBgm } from "../core/commands.js";
import { textField } from "../ui/controls.js";
import { toast } from "../ui/toast.js";
import { svgUse } from "../../assets/icons.js";

let dirField = null;
let listEl = null;

const KIND_ICON = { video: "icon-video", audio: "icon-audio", image: "icon-image" };

export function mount(container) {
  container.appendChild(h("h3", null, ["素材面板"]));
  dirField = textField({ id: "media-dir", testid: "media-dir", onEnter: () => refresh() });
  dirField.root.value = mediaStore.get().dir;
  container.appendChild(h("div", { class: "media-dir-row" }, [
    dirField.root,
    h("button", { id: "media-refresh", testid: "media-refresh", title: "刷新列表", "data-tip": "刷新列表", onclick: () => refresh() }, ["⟳"]),
  ]));
  listEl = h("div", { id: "media-list", testid: "media-list" }, [
    h("div", { class: "empty-hint" }, ["加载中…"]),
  ]);
  container.appendChild(listEl);
  container.appendChild(h("div", { class: "hint" }, ["双击 = 插到匹配轨道的播放头(帧磁吸);或拖到下方轨道任意落点"]));

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

async function refresh() {
  const dir = dirField.get().trim();
  await browseMedia(dir);
}

function renderList() {
  const st = mediaStore.get();
  clear(listEl);
  if (st.error) {
    listEl.appendChild(h("div", { class: "empty-hint" }, [st.error]));
    return;
  }
  if (!st.files.length) {
    listEl.appendChild(h("div", { class: "empty-hint" }, [
      `(${st.dir || "工程根"}) 无可导入媒体;把 mp4/mp3/png 等放进该目录后点 ⟳`,
    ]));
    return;
  }
  for (const f of st.files) {
    const row = h("div", {
      class: "media-item", draggable: true, testid: "media-item",
      title: `${f.path}${f.durationMs ? ` · ${f.durationMs}ms` : ""}`,
    }, [
      h("span", { class: "thumb", "data-tip": "图标卡(真缩略图候册四 T4.1)" }, [svgUse(KIND_ICON[f.kind] || "icon-video")]),
      h("span", { class: "bd" }, [
        h("span", { class: `badge ${f.kind}` }, [f.kind]), " ",
        f.name,
        f.durationMs ? h("span", { class: "dim" }, [` ${(f.durationMs / 1000).toFixed(1)}s`]) : null,
      ]),
      f.kind === "audio"
        ? h("button", { class: "set-bgm", dataset: { src: f.path }, testid: "set-bgm", title: "设为工程背景乐" }, ["BGM"])
        : null,
    ]);
    row.addEventListener("dblclick", () => insertMediaAuto(f));
    row.addEventListener("dragstart", (e) => {
      e.dataTransfer.setData("text/cutforge-media", f.path);
    });
    listEl.appendChild(row);
  }
}

/** boot 后首次浏览(供 main;目录默认值由 mediaStore 初始化)。 */
export function initialMediaBrowse() {
  return refresh();
}
