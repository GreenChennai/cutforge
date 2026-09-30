/* 文本工具面(册四 T4.7 FE2):T 键/工具栏「文本」→ 播放头处 text_add(响应 clipId
 * 自动选中);花字库(12 模板,CSS 近似预览 + 参数覆盖,挂 inspector 文本组页签)。
 *
 * - 文本位置拖拽(画布所见即所得)在 render/preview-transform.js(与画中画变换同层);
 * - 花字消费:clip_update patch.huazi = {template, params}(整对象替换单 Op);
 *   huazi 无法经 clip_update 置空(Option None = 不改,内核口径)——「移除花字」以
 *   template 置空串实现渲染端诚实降级(纯文本 + WARN),登记「huazi 清除候 BE」;
 * - ASS 色 &HAABBGGRR ↔ 网页色转换仅用于预览/取色控件,提交回传 ASS 原形。
 */
import { h, clear } from "../ui/dom.js";
import { timelineStore, selectionStore, projectStore } from "../core/store.js";
import { textAdd } from "../core/edit-commands.js";
import { updateClip } from "../core/commands.js";
import { ensureCatalogs, huaziList } from "../core/catalogs.js";
import { toast } from "../ui/toast.js";
import { numberField } from "../ui/controls.js";
import { cssVar } from "../render/theme.js";

/** ASS 色(&HAABBGGRR / &HBBGGRR)→ "#RRGGBB"(仅预览/取色用;非法输入返回 null)。 */
export function assToHex(ass) {
  const m = /^&H([0-9a-fA-F]{6}|[0-9a-fA-F]{8})&?$/.exec(String(ass || "").trim());
  if (!m) return null;
  const hex = m[1];
  const bgr = hex.length === 8 ? hex.slice(2) : hex;
  const b = bgr.slice(0, 2);
  const g = bgr.slice(2, 4);
  const r = bgr.slice(4, 6);
  return `#${r}${g}${b}`.toLowerCase();
}

/** "#RRGGBB" → ASS &H00BBGGRR(不透明)。 */
export function hexToAss(v) {
  const m = /^#([0-9a-fA-F]{6})$/.exec(String(v || "").trim());
  if (!m) return null;
  const r = m[1].slice(0, 2);
  const g = m[1].slice(2, 4);
  const b = m[1].slice(4, 6);
  return `&H00${b}${g}${r}`.toUpperCase();
}

/* ---------------- 文本工具(T 键 / 工具栏) ---------------- */

/** 播放头处加文本:目标轨缺省走服务端「第一文本轨」;壳侧先探测,无文本轨给引导。 */
export function addTextAtPlayhead() {
  const tracks = projectStore.get().project?.tracks || [];
  const textTrack = tracks.find((t) => (t.kind || "") === "text");
  const ph = Math.round(selectionStore.get().playheadMs || 0);
  if (!textTrack) {
    toast("没有文本轨:先在检查器底部点「+文本轨」,再按 T 添加文本", false);
    return;
  }
  textAdd({
    text: "双击片段编辑文本",
    atMs: ph,
    durationMs: 3000,
    trackId: textTrack.id,
  });
}

/** 工具栏「文本」按钮(main 装配期调用;快捷键 T 见 keymap)。 */
export function mountToolbarButton() {
  const toolbar = document.getElementById("toolbar");
  if (!toolbar) return;
  toolbar.appendChild(h("button", {
    id: "btn-text-add", testid: "text-add", title: "在播放头处添加文本(T;text_add,可撤销)",
    "data-tip": "文本轨片段:画布内直接拖位置,双击片段进检查器改样式",
    onclick: () => addTextAtPlayhead(),
  }, ["T 文本"]));
}

/* ---------------- 花字库(inspector 文本组页签) ---------------- */

/** 模板 CSS 近似预览类(hz.<id> → css 类;动画类以静态示意 + 角标标注)。 */
const HUAZI_PREVIEW_CLASS = {
  "hz.outline": "hzp-outline", "hz.neon": "hzp-neon", "hz.glow": "hzp-glow",
  "hz.emboss": "hzp-emboss", "hz.extrude": "hzp-extrude", "hz.box": "hzp-box",
  "hz.brush": "hzp-brush", "hz.gradient": "hzp-gradient", "hz.pop": "hzp-anim",
  "hz.typewriter": "hzp-anim", "hz.wave": "hzp-anim", "hz.karaoke": "hzp-karaoke",
};

/**
 * 花字选择器宿主(inspector 文本组附加;__cfRefresh(row) 随选中刷新)。
 * @param {() => Object|null} rowOf
 */
export function buildHuaziHost(rowOf) {
  const host = h("div", { class: "huazi-host", testid: "huazi-picker" });
  host.appendChild(h("div", { class: "hint" }, [
    "预览为 CSS 近似(以内核渲染为准);未注册模板渲染降级纯文本。",
  ]));
  const grid = h("div", { class: "huazi-grid" });
  host.appendChild(grid);
  const paramsBox = h("div", { class: "huazi-params" });
  host.appendChild(paramsBox);
  /** @type {string|null} */ let picked = null;
  /** @type {Object<string, *>} */ let draftParams = {};

  ensureCatalogs().then((cat) => {
    clear(grid);
    if (!cat) {
      grid.appendChild(h("div", { class: "hint" }, ["花字目录不可用(/catalogs 下发失败)"]));
      return;
    }
    for (const hz of huaziList()) {
      const previewCls = HUAZI_PREVIEW_CLASS[hz.id] || "";
      const isAnim = (hz.category || "") === "动画";
      const card = h("button", {
        class: `huazi-card${previewCls ? ` ${previewCls}` : ""}`,
        testid: "huazi-card", dataset: { huazi: hz.id }, title: `${hz.id} · ${hz.desc || ""}`,
        type: "button",
        onclick: () => { picked = hz.id; draftParams = defaultsOf(hz); renderStates(); renderParams(hz); },
      }, [
        h("span", { class: "huazi-sample" }, [isAnim ? `${hz.name}(动)` : hz.name]),
      ]);
      applyPreviewColors(card, hz); // 模板色:目录 ASS 值运行时换算内联(壳源零裸色值)
      grid.appendChild(card);
    }
    renderStates();
    applyBar();
  });

  function defaultsOf(hz) {
    const out = {};
    for (const p of hz.params || []) {
      out[p.name] = p.type === "color" ? (p.default || "") : (p.default !== undefined ? p.default : 0);
    }
    return out;
  }

  /** CSS 近似预览的模板色:从目录参数默认值(ASS)运行时换算内联(以渲染为准)。
   * 对比度护栏(axe color-contrast):按模板主色亮度选深/浅背板 + 可读文字色,
   * 色板全部经 token(theme.cssVar),壳源零裸色值。 */
  function applyPreviewColors(card, hz) {
    const sample = card.querySelector(".huazi-sample");
    if (!sample) return;
    const colorOf = (name) => assToHex(((hz.params || []).find((p) => p.name === name) || {}).default);
    const lum = (hexv) => {
      const n = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/.exec(hexv || "");
      if (!n) return 0;
      const ch = (i) => parseInt(n[i], 16) / 255;
      return 0.2126 * ch(1) + 0.7152 * ch(2) + 0.0722 * ch(3);
    };
    const dark = cssVar("--cf-gray-950");
    const light = cssVar("--cf-gray-100");
    const main = colorOf("color") || colorOf("halo") || colorOf("highlight");
    const back = colorOf("backColor");
    if (back) { // box/brush:色块底衬型——底衬原色上按亮度选可读文字色
      sample.style.backgroundColor = back;
      sample.style.color = lum(back) > 0.35 ? dark : light;
      return;
    }
    if (hz.id === "hz.gradient") {
      const from = colorOf("from");
      const to = colorOf("to");
      sample.style.background = `linear-gradient(135deg, ${from || main}, ${to || main})`;
      sample.style.webkitBackgroundClip = "text";
      sample.style.backgroundClip = "text";
      sample.style.color = "transparent";
      return;
    }
    if (hz.id === "hz.outline" || hz.id === "hz.emboss" || hz.id === "hz.extrude") {
      // 描边/立体型:模板主色偏深(黑描边/深影)→ 浅背板 + 深字,描边原色保留
      sample.style.backgroundColor = light;
      sample.style.color = dark;
      const stroke = colorOf("outlineColor");
      if (stroke) sample.style.webkitTextStroke = `2px ${stroke}`;
      const sh = colorOf("dark") || colorOf("shadowColor");
      if (sh) sample.style.textShadow = `2px 2px 0 ${sh}`;
      return;
    }
    // 发光/卡拉OK类:主色直接做文字色,背板按主色亮度取深/浅(无主色的动画类不动,默认即达标)
    if (!main) return;
    sample.style.backgroundColor = lum(main) > 0.35 ? dark : light;
    sample.style.color = main;
  }

  function renderStates() {
    const row = rowOf();
    const cur = row && row.huazi && row.huazi.template;
    for (const el of grid.querySelectorAll(".huazi-card")) {
      el.classList.toggle("on", el.dataset.huazi === (picked || cur || ""));
    }
  }

  function renderParams(hz) {
    clear(paramsBox);
    if (!hz) return;
    for (const p of hz.params || []) {
      if (p.type === "color") {
        const wrap = h("label", { class: "v2-field" }, [p.name]);
        const picker = h("input", { type: "color", "aria-label": `${hz.id} ${p.name}` });
        const hexv = assToHex(p.default);
        if (hexv) picker.value = hexv;
        picker.addEventListener("input", () => { draftParams[p.name] = hexToAss(picker.value) || p.default; });
        wrap.appendChild(picker);
        paramsBox.appendChild(wrap);
      } else {
        const f = numberField({ min: p.min, max: p.max, step: p.max - p.min <= 2 ? 0.1 : 1 });
        f.set(p.default);
        f.input.addEventListener("input", () => {
          const v = Number(f.get());
          if (Number.isNaN(v)) return;
          let val = v;
          if (p.min !== undefined) val = Math.max(p.min, val);
          if (p.max !== undefined) val = Math.min(p.max, val);
          if (val !== v) { f.set(val); toast(`${p.name} 越界,已钳制到 ${val}`, false); }
          draftParams[p.name] = val;
        });
        paramsBox.appendChild(h("label", { class: "v2-field" }, [p.name, f.root]));
      }
    }
  }

  function applyBar() {
    const bar = h("div", { class: "huazi-bar" });
    bar.appendChild(h("button", {
      class: "mini", testid: "huazi-apply", title: "挂载花字(patch.huazi 整对象替换单 Op)",
      onclick: () => {
        const row = rowOf();
        if (!row) { toast("先选中文本片段", false); return; }
        if (!picked) { toast("先点选一个花字模板", false); return; }
        const params = Object.keys(draftParams).length ? draftParams : undefined;
        updateClip(row.id, { huazi: params ? { template: picked, params } : { template: picked } },
          `花字 ${picked} 已挂载(可撤销)`);
      },
    }, ["应用花字"]));
    bar.appendChild(h("button", {
      class: "mini", testid: "huazi-clear", title: "渲染降级为纯文本(huazi 无法经 clip_update 置空,"
        + "以空模板诚实降级;清除字段候 BE)",
      onclick: () => {
        const row = rowOf();
        if (!row) return;
        updateClip(row.id, { huazi: { template: "" } }, "花字已降级为纯文本(可撤销)");
      },
    }, ["移除花字"]));
    host.appendChild(bar);
  }

  host.__cfRefresh = () => {
    picked = null;
    draftParams = {};
    renderStates();
    clear(paramsBox);
  };
  return host;
}
