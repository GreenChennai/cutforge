/* 图标表(T2.1:assets/ 内联 SVG symbol;单例注入,组件经 svgUse() 引用)。
 * 图标全部为自绘基础几何形;语义见各 symbol 的 <title>。 */
const SPRITE_ID = "cf-icon-sprite";

const ICONS = {
  "icon-video": `<title>视频</title><path d="M2 4h8a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zm10 3.2 3-2.2v6l-3-2.2z"/>`,
  "icon-audio": `<title>音频</title><path d="M8 1v9.2a2.6 2.6 0 1 1-1.4-2.3V3.5L13 2v6.3a2.6 2.6 0 1 1-1.4-2.3V2.4z"/>`,
  "icon-image": `<title>图片</title><path d="M2 3h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm1 8h10l-3.2-4.3-2.3 3-1.7-2z"/>`,
  "icon-eye": `<title>显示轨道</title><path d="M8 3C4.5 3 1.7 5.3.5 8c1.2 2.7 4 5 7.5 5s6.3-2.3 7.5-5C14.3 5.3 11.5 3 8 3zm0 8a3 3 0 1 1 0-6 3 3 0 0 1 0 6zm0-1.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3z"/>`,
  "icon-eye-off": `<title>隐藏轨道</title><path d="M1.4.6 15.4 14.6l-1 1-2.7-2.7A8.6 8.6 0 0 1 8 13C4.5 13 1.7 10.7.5 8a9.9 9.9 0 0 1 3.3-3.9L.4 1.6zM8 5a3 3 0 0 0-2.8 4L8.9 7.2 9.9 8z"/>`,
  "icon-refresh": `<title>刷新</title><path d="M8 2a6 6 0 0 1 5.7 4.1l-1.9.6A4 4 0 0 0 4.3 6H6v2H1V3h2v1.5A6 6 0 0 1 8 2zm5 9.7V13h2v3h-5v-2h1.6A6 6 0 0 1 2.3 9.9l1.9-.6A4 4 0 0 0 13 11.7z" transform="translate(0 -1)"/>`,
  "icon-scissors": `<title>分割</title><path d="M9.4 8 14 3.4A2 2 0 1 0 12.6 2L8 6.6 3.4 2A2 2 0 1 0 2 3.4L6.6 8 2 12.6A2 2 0 1 0 3.4 14L8 9.4l4.6 4.6A2 2 0 1 0 14 12.6z"/>`,
  "icon-plus": `<title>新增</title><path d="M7 2h2v5h5v2H9v5H7V9H2V7h5z"/>`,
  "icon-trash": `<title>删除</title><path d="M6 2h4v1h4v2H2V3h4zM3 6h10l-.8 8.1a1 1 0 0 1-1 .9H4.8a1 1 0 0 1-1-.9z"/>`,
  "icon-copy": `<title>复制</title><path d="M4 1h8a1 1 0 0 1 1 1v8h-2V3H4zm-2 3h8a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z"/>`,
  "icon-play": `<title>播放</title><path d="M4 2l10 6-10 6z"/>`,
  "icon-pause": `<title>暂停</title><path d="M3 2h4v12H3zM9 2h4v12H9z"/>`,
  "icon-film": `<title>成片</title><path d="M1 3h14v10H1zm2 2v2h2V5zm8 0v2h2V5zM3 9v2h2V9zm8 0v2h2V9zM7 5v6h2V5z"/>`,
  "icon-note": `<title>标注</title><path d="M2 2h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H8l-4 4v-4H2a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z"/>`,
  "icon-wave": `<title>波形占位纹理</title><path d="M1 7h1v2H1zm2-2h1v6H3zm2-2h1v10H5zm2 1h1v8H7zm2-3h1v14H9zm2 4h1v6h-1zm2 2h1v2h-1z"/>`,
};

/** 幂等注入 sprite(单例;重复调用无害)。 */
export function ensureIcons() {
  if (document.getElementById(SPRITE_ID)) return;
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.id = SPRITE_ID;
  svg.setAttribute("style", "display:none");
  svg.setAttribute("aria-hidden", "true");
  for (const [id, body] of Object.entries(ICONS)) {
    const sym = document.createElementNS("http://www.w3.org/2000/svg", "symbol");
    sym.id = id;
    sym.setAttribute("viewBox", "0 0 16 16");
    sym.innerHTML = body;
    svg.appendChild(sym);
  }
  document.body.appendChild(svg);
}

/** 生成引用 symbol 的 <svg><use> 节点。 */
export function svgUse(name, cls = "icon") {
  const el = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  el.setAttribute("class", cls);
  el.setAttribute("aria-hidden", "true");
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#${name}`);
  el.appendChild(use);
  return el;
}
