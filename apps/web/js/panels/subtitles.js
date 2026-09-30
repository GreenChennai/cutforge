/* 字幕编辑器(册四 T4.7 FE2):文本轨片段投影逐条编辑 + 批量替换 + SRT/ASS 导入导出
 * + 卡拉OK开关 + 样式卡(localStorage)。
 *
 * - 列表 = timelineStore 投影的文本轨片段(壳零手算,行数据全部投影只读);
 * - 文本改 = subtitle_set;时间改 = subtitle_retime(startMs/durationMs);均为单 Op;
 * - 批量替换 = subtitle_replace(字面量;trackId 可选);导入 = subtitle_import
 *   (SRT/ASS 自动识别,整批单 Op);导出 = subtitle_export(返回工程内相对路径);
 * - 导入路径诚实口径:浏览器壳无文件系统通道,路径手输或从资源管理器复制
 *   (字幕文件须已在工程目录内;素材面板不列 .srt/.ass,拷入后手输路径);
 * - 样式卡存 prefs(localStorage,命名空间收口);应用 = patch.textStyle 整对象替换;
 *   「应用到全部字幕」逐笔 Op(后端无批量工具,toast 明示)。
 */
import { h, clear } from "../ui/dom.js";
import { projectStore, timelineStore, selectionStore } from "../core/store.js";
import { clipKindOf, frameMsOf } from "../core/model.js";
import {
  subtitleSetText, subtitleRetime, subtitleReplace, subtitleImport, subtitleExport,
} from "../core/edit-commands.js";
import { updateClip, selectClip } from "../core/commands.js";
import { textField, numberField, selectField, toggleField } from "../ui/controls.js";
import { pref, setPref } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";

let rowsEl = null;
let countEl = null;
let trackSel = null;
let cardsEl = null;

function textTracks() {
  return (projectStore.get().project?.tracks || []).filter((t) => (t.kind || "") === "text");
}
function textClips() {
  const kinds = new Set(textTracks().map((t) => t.id));
  return timelineStore.get().clips
    .filter((c) => kinds.has(c.track) || clipKindOf(c) === "text")
    .sort((a, b) => (a.track === b.track ? a.startMs - b.startMs : a.track < b.track ? -1 : 1));
}
function styleCards() { return pref("styleCards", []); }

export function mount(container) {
  container.appendChild(h("h3", null, ["字幕编辑器(文本轨片段投影逐条;单 Op 可撤销)"]));
  countEl = h("div", { class: "hint", testid: "sub-count" }, ["加载中…"]);
  container.appendChild(countEl);
  rowsEl = h("div", { class: "sub-rows", testid: "sub-rows" });
  container.appendChild(rowsEl);

  /* ---- 批量替换 ---- */
  const findF = textField({ testid: "sub-find", ariaLabel: "查找串", placeholder: "查找(字面量,非正则)" });
  const replF = textField({ testid: "sub-replace", ariaLabel: "替换串", placeholder: "替换为(清空用空串)" });
  trackSel = h("select", { testid: "sub-track", "aria-label": "限定文本轨" });
  const box1 = h("fieldset", { class: "insp-group", testid: "sub-replace-box" }, [
    h("legend", null, ["批量替换(subtitle_replace)"]),
    h("div", { class: "sub-row" }, [findF.root, replF.root, trackSel,
      h("button", {
        testid: "sub-replace-run", title: "全部文本轨逐条替换(单 Op)",
        onclick: () => {
          const f = findF.get();
          if (!f) { toast("查找串不能为空", false); return; }
          subtitleReplace(f, replF.get(), trackSel.value || undefined);
        },
      }, ["替换"])]),
  ]);
  container.appendChild(box1);

  /* ---- 导入/导出 ---- */
  const srcF = textField({ testid: "sub-import-src", ariaLabel: "字幕文件路径", placeholder: "工程内相对路径,如 01_原始素材/subs.srt" });
  const fmtSel = selectField({ testid: "sub-export-format", options: [["srt", "SRT"], ["ass", "ASS"]] });
  const outF = textField({ testid: "sub-export-out", ariaLabel: "导出路径", placeholder: "缺省 06_成片输出/subtitles_<轨>.<格式>" });
  const box2 = h("fieldset", { class: "insp-group", testid: "sub-io-box" }, [
    h("legend", null, ["导入 / 导出(SRT·ASS 往返)"]),
    h("div", { class: "sub-row" }, [
      srcF.root,
      h("button", {
        testid: "sub-import-run", title: "subtitle_import(整批单 Op)",
        onclick: () => {
          const src = srcF.get().trim();
          if (!src) { toast("先填字幕文件路径(工程内相对路径)", false); return; }
          subtitleImport(src, trackSel.value || undefined);
        },
      }, ["导入"]),
    ]),
    h("div", { class: "hint" }, [
      "浏览器壳无文件系统通道:把 .srt/.ass 拷进工程目录后手输路径(素材面板不列字幕文件,拷入候 BE)。",
    ]),
    h("div", { class: "sub-row" }, [
      fmtSel.root, outF.root,
      h("button", {
        testid: "sub-export-run", title: "subtitle_export(返回工程内相对路径)",
        onclick: () => subtitleExport(fmtSel.get(), outF.get().trim() || undefined, trackSel.value || undefined),
      }, ["导出"]),
    ]),
  ]);
  container.appendChild(box2);

  /* ---- 样式卡 ---- */
  const cardName = textField({ testid: "sub-card-name", ariaLabel: "样式卡名", placeholder: "样式卡名(存本机浏览器)" });
  cardsEl = h("div", { class: "sub-cards", testid: "sub-cards" });
  const box3 = h("fieldset", { class: "insp-group", testid: "sub-style-box" }, [
    h("legend", null, ["样式卡(整套 textStyle 存 localStorage,一键套用)"]),
    h("div", { class: "sub-row" }, [
      cardName.root,
      h("button", {
        testid: "sub-card-save", title: "把选中字幕片段的整套 textStyle 存为样式卡",
        onclick: () => saveCard(cardName.get().trim()),
      }, ["存选中为样式卡"]),
    ]),
    cardsEl,
  ]);
  container.appendChild(box3);

  timelineStore.subscribe(() => refresh());
  projectStore.subscribe((patch) => { if (patch.project !== undefined) fillTrackOptions(); });
  refresh();
  renderCards();
}

/** 页签激活时刷新(main.switchTab 调用)。 */
export function refresh() {
  fillTrackOptions();
  renderRows();
  renderCards();
}

function fillTrackOptions() {
  if (!trackSel) return;
  const cur = trackSel.value;
  clear(trackSel);
  trackSel.appendChild(h("option", { value: "" }, ["全部文本轨"]));
  for (const t of textTracks()) {
    trackSel.appendChild(h("option", { value: t.id }, [t.id]));
  }
  trackSel.value = cur;
  if (trackSel.selectedIndex < 0) trackSel.value = "";
}

function renderRows() {
  if (!rowsEl) return;
  clear(rowsEl);
  const clips = textClips();
  countEl.textContent = clips.length
    ? `共 ${clips.length} 条字幕(${textTracks().map((t) => t.id).join("/") || "无文本轨"});双击文本编辑,时间框改后即提交`
    : "没有字幕:检查器加「+文本轨」后,按 T 添加文本或用下方导入";
  if (!clips.length) return;
  const fms = frameMsOf(projectStore.get().project);
  for (const c of clips) {
    rowsEl.appendChild(subRow(c, fms));
  }
}

function subRow(c, fms) {
  const startF = numberField({ step: fms, min: 0, testid: "sub-start" });
  const durF = numberField({ step: fms, min: 1, testid: "sub-dur" });
  startF.set(c.startMs);
  durF.set(c.endMs - c.startMs);
  const karaoke = toggleField({
    testid: "sub-karaoke", label: "卡拉OK",
    checked: Boolean(c.textStyle && c.textStyle.karaoke),
    onChange: (on) => {
      updateClip(c.id, { textStyle: { ...(c.textStyle || {}), karaoke: on } },
        `卡拉OK ${on ? "开" : "关"}(可撤销)`);
    },
  });
  const timeCommit = () => {
    const s = Number(startF.get());
    const d = Number(durF.get());
    if (Number.isNaN(s) && Number.isNaN(d)) return;
    subtitleRetime(c.id, Number.isNaN(s) ? null : s, Number.isNaN(d) ? null : d);
  };
  startF.input.addEventListener("change", timeCommit);
  durF.input.addEventListener("change", timeCommit);
  const textSpan = h("span", {
    class: "sub-text", testid: "sub-text", title: "双击编辑文本(subtitle_set)", tabindex: "0",
  }, [c.text || "(空)"]);
  const editInline = () => {
    const input = h("input", { type: "text", value: c.text || "", "aria-label": "编辑字幕文本" });
    let closed = false; // Enter 提交后 input 脱离 DOM,随行 blur 会再触发——关一次即哑
    const close = () => {
      if (closed) return;
      closed = true;
      input.replaceWith(textSpan);
    };
    const commit = () => {
      const v = input.value;
      close();
      if (v !== (c.text || "")) subtitleSetText(c.id, v);
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") commit();
      if (e.key === "Escape") close();
    });
    input.addEventListener("blur", commit);
    textSpan.replaceWith(input);
    input.focus();
    input.select();
  };
  textSpan.addEventListener("dblclick", editInline);
  textSpan.addEventListener("keydown", (e) => { if (e.key === "Enter") editInline(); });
  return h("div", {
    class: "sub-row sub-item", testid: "sub-row", dataset: { clipId: c.id },
    onclick: () => selectClip(c.id),
  }, [
    h("span", { class: "dim sub-id" }, [`${c.track}`]),
    h("label", null, ["起 ", startF.root]),
    h("label", null, ["长 ", durF.root]),
    textSpan,
    karaoke.root,
  ]);
}

/* ---------------- 样式卡 ---------------- */

function saveCard(name) {
  const row = textClips().find((c) => c.id === selectionStore.get().clipId)
    || timelineStore.get().clips.find((c) => c.id === selectionStore.get().clipId);
  if (!row) { toast("先选中一个片段(取其 textStyle)", false); return; }
  if (!name) { toast("先填样式卡名", false); return; }
  if (!row.textStyle || !Object.keys(row.textStyle).length) {
    toast("该片段没有 textStyle(先在检查器文本组调样式)", false);
    return;
  }
  const cards = styleCards().filter((x) => x.name !== name);
  cards.push({ name, style: { ...row.textStyle }, t: Date.now() });
  setPref("styleCards", cards);
  renderCards();
  toast(`样式卡「${name}」已存(本机浏览器)`);
}

function renderCards() {
  const host = cardsEl;
  if (!host) return;
  clear(host);
  const cards = styleCards();
  if (!cards.length) {
    host.appendChild(h("div", { class: "hint" }, ["还没有样式卡:调好样式后「存选中为样式卡」"]));
    return;
  }
  for (const card of cards) {
    host.appendChild(h("span", { class: "sub-card", testid: "sub-card" }, [
      h("button", {
        class: "mini", title: "套用到选中片段(patch.textStyle 整对象替换,单 Op)",
        onclick: () => applyCard(card, false),
      }, [`◧ ${card.name}`]),
      h("button", {
        class: "mini", title: "套用到全部字幕片段(逐笔 Op,撤销需逐笔)",
        onclick: () => applyCard(card, true),
      }, ["全部"]),
      h("button", {
        class: "mini", title: "删除样式卡(仅本机,不产 Op)",
        onclick: () => {
          setPref("styleCards", styleCards().filter((x) => x.name !== card.name));
          renderCards();
        },
      }, ["✕"]),
    ]));
  }
}

function applyCard(card, all) {
  const clips = all ? textClips() : textClips().filter((c) => c.id === selectionStore.get().clipId);
  if (!clips.length) { toast(all ? "没有字幕片段" : "先选中目标字幕片段", false); return; }
  if (clips.length === 1) {
    updateClip(clips[0].id, { textStyle: { ...card.style } }, `样式卡「${card.name}」已应用(可撤销)`);
    return;
  }
  // 逐笔诚实口径(后端无批量工具;与批量修改同一口径)
  let done = 0;
  for (const c of clips) {
    updateClip(c.id, { textStyle: { ...card.style } });
    done += 1;
  }
  toast(`样式卡已应用至 ${done} 条字幕(每条一笔 Op,撤销需逐笔)`);
}
