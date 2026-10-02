/* 插件贡献点(册七 T7.2;首批三类,per PLUGIN-SPEC contributes):
 *
 * - 命令(contributes.commands):keymap 注册(group=插件,缺省不绑组合,设置面板
 *   可自由绑定;帮助面板全表可见),亦可作为时间线右键菜单项出现(见下);
 * - 菜单:契约面 commands.id 是唯一贡献点载体(manifest schema additionalProperties
 *   收口,无独立 menus 键)——壳侧把启用的插件命令默认呈现为片段/时间线右键菜单项,
 *   管理面板可逐条关掉(menuOff,localStorage);标签带「插件」前缀与插件名,可辨源;
 * - 面板(contributes.panels):iframe-free 自定义页签——插件推送 HTML 片段,宿主
 *   受控渲染(标签白名单 + 剥属性 + 禁 script/事件属性;交互控件不进面板),可切
 *   文本视图对照原文。
 */
import { h } from "../ui/dom.js";
import { defineBinding, undefineByPrefix } from "../ui/keymap-registry.js";
import { invokeContribution } from "./host.js";

/** pid → { manifest, panels: [{san, title, tabBtn, container, raw}] } */
const contribs = new Map();

/** dom id 安全化(贡献点 id 允许 `.`;dom id/CSS 选择器只留字母数字连字符)。 */
function sanId(s) {
  return String(s).replace(/[^a-zA-Z0-9-]/g, "-");
}

/* ---------------- 注册 / 注销(宿主在插件 ready/stop 时调用)---------------- */

export function registerContribs(pid, manifest, { menuOff = [] } = {}) {
  unregisterContribs(pid);
  const entry = { manifest, panels: [], menuOff: new Set(menuOff) };
  contribs.set(pid, entry);
  const c = manifest.contributes || {};
  for (const cmd of c.commands || []) {
    defineBinding(
      `plugin.${pid}.${cmd.id}`, "插件",
      `${manifest.name || pid} · ${cmd.title}`, "",
      () => invokeContribution(pid, "command", cmd.id, { via: "keymap" }),
    );
  }
  for (const p of c.panels || []) entry.panels.push(mountPluginPanel(pid, manifest, p));
}

export function unregisterContribs(pid) {
  const entry = contribs.get(pid);
  if (!entry) return;
  undefineByPrefix(`plugin.${pid}.`);
  for (const p of entry.panels) {
    p.tabBtn.remove();
    p.container.remove();
  }
  contribs.delete(pid);
}

/** 插件菜单项(右键菜单消费;启用插件的全部命令,menuOff 逐条关)。 */
export function pluginMenuItems(ctx = {}) {
  const items = [];
  for (const [pid, entry] of contribs) {
    const cmds = entry.manifest.contributes?.commands || [];
    for (const cmd of cmds) {
      if (entry.menuOff.has(cmd.id)) continue;
      items.push({
        label: `插件:${cmd.title}(${entry.manifest.name || pid})`,
        fn: () => invokeContribution(pid, "command", cmd.id, { via: "menu", ...ctx }),
        why: "插件贡献点命令(管理面板可从菜单隐藏)",
      });
    }
  }
  return items;
}

/** 菜单呈现开关(管理面板;未注册的插件直接落 menuOff 注册表)。 */
export function setMenuSurfacedLive(pid, cmdId, on) {
  const entry = contribs.get(pid);
  if (entry) {
    if (on) entry.menuOff.delete(cmdId);
    else entry.menuOff.add(cmdId);
  }
}

/** 已注册贡献点观察面(管理面板渲染)。 */
export function contribsOf(pid) {
  return contribs.get(pid) || null;
}

/* ---------------- 面板贡献点(iframe-free 受控渲染)---------------- */

function mountPluginPanel(pid, manifest, p) {
  const tabKey = `plugin-${sanId(pid)}-${sanId(p.id)}`;
  const tabBtn = h("button", { "data-tab": tabKey, testid: `tab-plugin-${sanId(`${pid}-${p.id}`)}` },
    [`插件:${p.title}`]);
  const container = h("div", { class: "tab", id: `tab-${tabKey}`, testid: `panel-plugin-${sanId(`${pid}-${p.id}`)}` });
  container.appendChild(h("div", { class: "panel tab-page" }, [
    h("h3", null, [`插件面板:${p.title}(${manifest.name || pid})`]),
    h("div", { class: "filter-row" }, [
      h("button", {
        testid: `plugin-panel-text-${sanId(`${pid}-${p.id}`)}`,
        onclick: (e) => toggleTextView(e.currentTarget, container),
      }, ["文本视图"]),
      h("span", { class: "hint" }, ["受控渲染:标签白名单+剥属性,禁脚本/交互控件;交互请走插件命令"]),
    ]),
    h("div", { class: "plugin-panel-body", testid: `plugin-panel-body-${sanId(`${pid}-${p.id}`)}` }, [
      h("div", { class: "empty-hint" }, ["(等待插件推送内容:插件经桥 setPanel 更新此区)"]),
    ]),
  ]));
  document.getElementById("tabs").appendChild(tabBtn);
  document.querySelector("main").appendChild(container);
  return { id: p.id, san: sanId(`${pid}-${p.id}`), title: p.title, tabBtn, container };
}

function toggleTextView(btn, container) {
  const body = container.querySelector(".plugin-panel-body");
  const raw = body.querySelector(".plugin-panel-raw");
  const rendered = body.querySelector(".plugin-panel-html");
  if (raw && rendered) { // 当前双视图:切回渲染
    raw.remove();
    rendered.hidden = false;
    btn.textContent = "文本视图";
    return;
  }
  body.appendChild(h("pre", { class: "plugin-panel-raw", testid: "plugin-panel-raw" },
    [rawTextOf(body)]));
  if (rendered) rendered.hidden = true;
  btn.textContent = "渲染视图";
}

function rawTextOf(body) {
  const rendered = body.querySelector(".plugin-panel-html");
  return rendered ? rendered.textContent : "(尚未收到插件内容)";
}

/** 插件推送片段 → 消毒 → 渲染进面板(panelId=contributes.panels[].id;未知忽略并告警)。 */
export function renderPanelFragment(pid, panelId, html) {
  const entry = contribs.get(pid);
  const panel = entry?.panels.find((x) => x.id === panelId || x.title === panelId || x.san === sanId(panelId));
  if (!panel) {
    console.warn(`[plugins] ${pid} 推送未注册面板 ${panelId}(忽略)`);
    return;
  }
  const body = panel.container.querySelector(".plugin-panel-body");
  const old = body.querySelector(".plugin-panel-html");
  if (old) old.remove();
  const holder = h("div", { class: "plugin-panel-html", testid: `plugin-panel-html-${panel.san}` });
  holder.appendChild(sanitizeFragment(String(html || "")));
  const raw = body.querySelector(".plugin-panel-raw");
  if (raw) raw.remove(); // 新内容到达回到渲染视图(文本视图看新内容再点)
  body.appendChild(holder);
}

/* ---------------- 消毒器(白名单;零 innerHTML 路径)---------------- */

const ALLOWED_TAGS = new Set([
  "h1", "h2", "h3", "h4", "h5", "h6", "p", "div", "span", "ul", "ol", "li",
  "table", "thead", "tbody", "tr", "td", "th", "b", "strong", "i", "em",
  "code", "pre", "br", "hr", "img", "small",
]);
const DROP_TAGS = new Set([
  "script", "style", "iframe", "object", "embed", "form", "input", "button",
  "select", "textarea", "link", "meta", "base", "svg", "math", "video", "audio",
  "canvas", "template", "frame", "frameset",
]);

function sanitizeFragment(html) {
  let doc;
  try {
    doc = new DOMParser().parseFromString(html, "text/html");
  } catch {
    return document.createTextNode("(片段解析失败)");
  }
  const frag = document.createDocumentFragment();
  for (const node of [...doc.body.childNodes]) frag.appendChild(cleanNode(node));
  return frag;
}

function cleanNode(node) {
  if (node.nodeType === Node.TEXT_NODE) return document.createTextNode(node.textContent);
  if (node.nodeType !== Node.ELEMENT_NODE) return document.createTextNode("");
  const tag = node.tagName.toLowerCase();
  if (DROP_TAGS.has(tag)) return document.createTextNode(""); // 危险标签整树丢弃
  if (!ALLOWED_TAGS.has(tag)) { // 未知标签:解包保留内容
    const wrap = document.createDocumentFragment();
    for (const c of [...node.childNodes]) wrap.appendChild(cleanNode(c));
    return wrap;
  }
  const el = document.createElement(tag);
  if (tag === "img") { // 唯一放行属性:src(http/https/data:image) + alt
    const src = node.getAttribute("src") || "";
    if (/^(https?:|data:image\/)/.test(src)) el.setAttribute("src", src);
    el.setAttribute("alt", node.getAttribute("alt") || "插件图片");
  }
  for (const c of [...node.childNodes]) el.appendChild(cleanNode(c));
  return el;
}
