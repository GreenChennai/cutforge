/* 素材缩略卡(T4.1):真缩略图(懒加载)/音频迷你波形/图片直显/代理状态徽标。
 * 懒加载:IntersectionObserver 视口内才发 media_thumbnail(RPC 排队限流在 media-cache);
 * 图片卡直显 /media(浏览器 loading=lazy 双保险);音频卡画 coarse 波形。
 * 缩略不可用(无 ffmpeg 等)时回落图标卡,不假图。 */
import { h } from "../ui/dom.js";
import { mediaUrlFor } from "../core/model.js";
import { thumbnailFor, proxyFor, proxyStateOf } from "../core/media-cache.js";
import { drawMiniWave } from "../render/waveform.js";
import { svgUse } from "../../assets/icons.js";

const KIND_ICON = { video: "icon-video", audio: "icon-audio", image: "icon-image" };

/** 令牌来源(mediaUrlFor 需要;由 panel 装配时注入,避免 import 环)。 */
let tokenOf = () => "";

/** @param {() => string} fn */
export function setTokenProvider(fn) { tokenOf = fn; }

/**
 * @param {Object} item media_browse 行 {name, path, bytes, kind, durationMs}
 * @param {{ onInsert: (item: Object) => void }} handlers
 * @returns {HTMLElement} 卡片根(testid=media-item;可见时调用方触发 loadMedia)
 */
export function createMediaCard(item, handlers) {
  const thumb = h("span", { class: `thumb ${item.kind}`, "data-tip": "缩略位(视口内才加载)" });
  const card = h("div", {
    class: "media-item", draggable: true, testid: "media-item",
    dataset: { path: item.path, kind: item.kind },
    title: `${item.path}${item.durationMs ? ` · ${(item.durationMs / 1000).toFixed(1)}s` : ""}`,
  }, [
    thumb,
    h("span", { class: "bd" }, [
      h("span", { class: `badge ${item.kind}` }, [item.kind]), " ",
      h("span", { class: "media-name" }, [item.name]),
      item.durationMs
        ? h("span", { class: "dim" }, [` ${(item.durationMs / 1000).toFixed(1)}s`])
        : null,
    ]),
    item.kind === "video" ? proxyBadge(item.path) : null,
    item.kind === "audio"
      ? h("button", { class: "set-bgm", dataset: { src: item.path }, testid: "set-bgm", title: "设为工程背景乐" }, ["BGM"])
      : null,
  ]);
  card.addEventListener("dblclick", () => handlers.onInsert(item));
  card.addEventListener("dragstart", (e) => {
    e.dataTransfer.setData("text/cutforge-media", item.path);
  });
  card.__cfLoad = () => loadThumb(thumb, item);
  return card;
}

/** 视口进入:按类型加载缩略位(每卡至多一次)。 */
function loadThumb(thumb, item) {
  if (thumb.dataset.loaded === "1") return;
  thumb.dataset.loaded = "1";
  if (item.kind === "image") {
    const img = h("img", {
      src: mediaUrlFor(item.path, tokenOf()), alt: item.name,
      loading: "lazy", "data-tip": "图片直显(/media 数据面)",
    });
    img.addEventListener("error", () => {
      thumb.textContent = "";
      thumb.appendChild(svgUse(KIND_ICON.image));
    });
    thumb.textContent = "";
    thumb.appendChild(img);
    return;
  }
  if (item.kind === "audio") {
    const cv = document.createElement("canvas");
    cv.className = "thumb-wave";
    cv.width = 64;
    cv.height = 20;
    cv.setAttribute("aria-hidden", "true");
    thumb.textContent = "";
    thumb.appendChild(cv);
    drawMiniWave(cv, item.path);
    return;
  }
  // video:占位图标 → media_thumbnail 生成后换真图(缓存命中即回)
  thumbnailFor(item.path).then((r) => {
    if (!r.ok || !thumb.isConnected) return; // 失败保持图标(诚实回落)
    const img = h("img", {
      src: mediaUrlFor(r.file, tokenOf()), alt: item.name,
      "data-tip": r.cached ? "缩略图(缓存命中)" : "缩略图(新生成)",
    });
    thumb.textContent = "";
    thumb.appendChild(img);
  });
}

/** 代理徽标:状态懒查询(fs 存在性检查,零 ffmpeg);生成按钮由右键菜单/点击触发。 */
function proxyBadge(src) {
  const el = h("button", {
    class: "proxy-badge", testid: "media-proxy", title: "代理状态:点击生成(media_proxy)",
    "data-tip": "1/2 分辨率代理;生成后导出面板「用代理预览」生效(导出默认原片)",
    onclick: (e) => {
      e.stopPropagation();
      el.textContent = "…";
      proxyFor(src, true).then((r) => {
        el.textContent = r.ok && r.state === "ready" ? "代理✓" : "代理✗";
        el.classList.toggle("ready", r.ok && r.state === "ready");
      });
    },
  }, ["代理"]);
  const known = proxyStateOf(src);
  if (known) {
    el.textContent = known === "ready" ? "代理✓" : "代理";
    el.classList.toggle("ready", known === "ready");
  }
  return el;
}

/** 视口观察器(面板列表共用一个 IO;回调触发卡片懒加载)。 */
export function createCardObserver() {
  const io = new IntersectionObserver((entries) => {
    for (const en of entries) {
      if (en.isIntersecting) {
        const card = en.target;
        if (card.__cfLoad) card.__cfLoad();
        io.unobserve(card);
      }
    }
  }, { rootMargin: "120px" });
  return io;
}
