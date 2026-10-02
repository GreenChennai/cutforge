/* 脚本页签(册七 T7.3):cutforge-script-v1 编辑/运行/片段库 + AI plan 粘贴区。
 *
 * 运行通道勘察口径(诚实):后端没有「直接跑脚本」的 RPC 工具——cutforge-script
 * 宿主只挂在 CLI run-script 子命令(沙箱在派发前拦截,浏览器壳不可达);壳侧脚本
 * 以 **plan 形态运行**:steps 逐项 {tool,args} → preview_plan 副本 dry-run(真工程
 * 零写入,输出=逐步工具回执)→「以 plan 提交」进差异面板人审批准 → apply_plan 落地。
 * 脚本 = plan JSON 生成器,与批准流共用同一会合点(core/plan.js);script_run RPC 登记候 BE。
 */
import { h, clear } from "../ui/dom.js";
import { timelineStore, projectStore } from "../core/store.js";
import { markersOf } from "../ui/markers.js";
import { pref, setPref } from "../ui/prefs.js";
import { toast } from "../ui/toast.js";
import * as plan from "../core/plan.js";

const SCRIPT_MAX_STEPS = 200; // 与 crates/cutforge-script Policy::new 同额(客户端镜像)

let editor = null;
let gutter = null;
let outputHost = null;
let statusLine = null;
let libSelect = null;
let libName = null;
let linenumToggle = null;
let runBtn = null;
let submitBtn = null;
let aiBox = null;
let runToken = 0;

/* ---------------- 内置片段(生成器:从当前会话态生成步骤;载入时定格)---------------- */

const BUILTINS = [
  {
    name: "内置:按标记切割",
    why: "标记是会话态(不落盘),生成器在载入时读取当前标记,逐个命中片段生成 clip_split。",
    gen() {
      const ms = markersOf();
      if (!ms.length) return { error: "没有标记:先在播放头按 M 加几个标记(会话级),再载入本片段" };
      const steps = [];
      // 逐片段内按标记降序生成:静态脚本无变量,顺序切割会让后继 clipId 失位
      // (降序则前一次分割不影响更早的 tMs);片段间相互独立,依赖冲突由预演兜底。
      for (const c of timelineStore.get().clips) {
        const hits = ms.filter((m) => m > c.startMs && m < c.endMs).sort((a, b) => b - a);
        for (const m of hits) {
          steps.push({ tool: "clip_split", args: { clipId: c.id, tMs: Math.round(m) } });
        }
      }
      return { note: `${ms.length} 个标记 × 命中片段 → ${steps.length} 步 clip_split(片段内降序)`, script: wrap(steps) };
    },
  },
  {
    name: "内置:批量变色(调色)",
    why: "给全部视频片段统一挂同一 grade(temperature+saturation;整对象替换,已有调色会被覆盖——可改步骤后运行)。",
    gen() {
      const videoTracks = new Set((projectStore.get().project?.tracks || [])
        .filter((t) => (t.kind || "video") === "video").map((t) => t.id));
      const rows = timelineStore.get().clips.filter((c) => videoTracks.has(c.track));
      if (!rows.length) return { error: "时间线没有视频片段:先从素材面板双击插入一段" };
      const steps = rows.map((c) => ({
        tool: "clip_update",
        args: { clipId: c.id, patch: { grade: { temperature: 20, saturation: 1.15 } }, summary: "脚本:统一暖调" },
      }));
      return { note: `${rows.length} 个视频片段 → ${steps.length} 步 clip_update(grade 整对象)`, script: wrap(steps) };
    },
  },
  {
    name: "内置:批量转场",
    why: "每条视频轨相邻片段之间挂 fade(xfade tr.fade,500ms;首片段无前邻不挂)。",
    gen() {
      const videoTracks = (projectStore.get().project?.tracks || [])
        .filter((t) => (t.kind || "video") === "video").map((t) => t.id);
      const steps = [];
      for (const tid of videoTracks) {
        const rows = timelineStore.get().clips.filter((c) => c.track === tid)
          .sort((a, b) => a.startMs - b.startMs);
        for (let i = 1; i < rows.length; i += 1) {
          steps.push({ tool: "transition_set", args: { clipId: rows[i].id, type: "fade", fx: "tr.fade", durMs: 500 } });
        }
      }
      if (!steps.length) return { error: "视频轨不足两个片段:批量转场挂在与前一片段之间" };
      return { note: `${steps.length} 步 transition_set(fade 500ms)`, script: wrap(steps) };
    },
  },
];

function wrap(steps) {
  return { format: "cutforge-script-v1", steps };
}

/* ---------------- 装配 ---------------- */

export function mount(container) {
  const editorPanel = h("div", { class: "panel tab-page" });
  editorPanel.appendChild(h("h3", null, ["脚本(cutforge-script-v1 · 以 plan 形态运行)"]));
  editorPanel.appendChild(h("div", { class: "hint" }, [
    "后端无「直接跑脚本」RPC(脚本宿主在 CLI run-script 侧,浏览器不可达):这里把脚本 steps 逐项经 ",
    "preview_plan 副本预演(真工程零写入),「以 plan 提交」进差异面板人审批准后 apply_plan 落地。",
    "script_run RPC 已登记候 BE。",
  ]));
  editorPanel.appendChild(h("div", { class: "filter-row" }, [
    libSelect = h("select", { testid: "script-lib-select", "aria-label": "片段库" }),
    h("button", { testid: "script-lib-load", onclick: () => loadSnippet(), title: "载入选中片段(内置项按当前会话态生成)" }, ["载入"]),
    libName = h("input", { type: "text", placeholder: "库脚本名…", testid: "script-lib-name", "aria-label": "库脚本名", style: "width:140px" }),
    h("button", { testid: "script-lib-save", onclick: () => saveToLib(), title: "把编辑器当前内容存入库(localStorage)" }, ["存入库"]),
    h("button", { testid: "script-lib-del", onclick: () => delFromLib(), title: "删除选中的库脚本" }, ["删除库项"]),
    h("button", { testid: "script-export", onclick: () => exportPlan(), title: "把当前脚本导出为 plan JSON({plan:[{tool,args}]})" }, ["导出 plan JSON"]),
    linenumToggle = h("label", { class: "toggle" }, [
      h("input", { type: "checkbox", checked: true, testid: "script-linenum", onchange: (e) => gutter.hidden = !e.target.checked }),
      " 行号",
    ]),
  ]));
  const wrap0 = h("div", { class: "script-editor-wrap" });
  gutter = h("div", { class: "script-gutter", testid: "script-gutter", "aria-hidden": "true" });
  editor = h("textarea", {
    class: "script-editor", testid: "script-editor", spellcheck: "false",
    "aria-label": "脚本编辑区(cutforge-script-v1 JSON)",
    placeholder: "{\n  \"format\": \"cutforge-script-v1\",\n  \"steps\": [\n    { \"tool\": \"…\", \"args\": { … } }\n  ]\n}",
  });
  editor.addEventListener("input", () => renderGutter());
  editor.addEventListener("scroll", syncScroll);
  wrap0.appendChild(gutter);
  wrap0.appendChild(editor);
  editorPanel.appendChild(wrap0);
  runBtn = h("button", {
    class: "primary", testid: "script-run", onclick: () => runPreview(),
    title: "steps → preview_plan(副本 dry-run,真工程零写入;输出=逐步工具回执)",
  }, ["运行(预演 preview_plan)"]);
  submitBtn = h("button", {
    testid: "script-submit", onclick: () => submitToApproval(),
    title: "把脚本转成 plan 送差异面板批准流(人审逐项批准后 apply_plan 落地)",
  }, ["以 plan 提交(进批准流)"]);
  editorPanel.appendChild(h("div", { class: "filter-row" }, [
    runBtn, submitBtn,
    h("button", { testid: "script-stop", onclick: () => stopRun(), title: "丢弃进行中的运行结果(在途请求由超时兜底,不可中断服务端)" }, ["停止"]),
  ]));
  container.appendChild(editorPanel);

  const outPanel = h("div", { class: "panel tab-page" });
  outPanel.appendChild(h("h3", null, ["输出(结构化逐步回执;脚本无 stdout 概念)"]));
  statusLine = h("div", { class: "hint", testid: "script-status" }, ["(尚未运行)"]);
  outPanel.appendChild(statusLine);
  outputHost = h("div", { testid: "script-output" });
  outPanel.appendChild(outputHost);
  container.appendChild(outPanel);

  const aiPanel = h("div", { class: "panel tab-page" });
  aiPanel.appendChild(h("h3", null, ["AI 生成落点(人在环)"]));
  aiPanel.appendChild(h("div", { class: "hint" }, [
    "把 AI 产出的 plan JSON({plan:[{tool,args}]} 或裸数组)粘到这里:先进 preview_plan 预览,",
    "再由差异面板逐项人审批准——未经批准零落地。",
  ]));
  aiBox = h("textarea", {
    class: "script-editor ai", testid: "script-ai-src", spellcheck: "false",
    "aria-label": "AI 产出的 plan JSON 粘贴区", placeholder: "{\"plan\":[{\"tool\":\"clip_update\",\"args\":{…}}]}",
  });
  aiPanel.appendChild(aiBox);
  aiPanel.appendChild(h("div", { class: "filter-row" }, [
    h("button", { class: "primary", testid: "script-ai-preview", onclick: () => previewAiPlan(), title: "粘贴内容 → 预演 → 差异面板人审" }, ["预览此 plan(preview_plan)"]),
  ]));
  container.appendChild(aiPanel);

  fillLibSelect();
  renderGutter();
}

/* ---------------- 编辑器辅助 ---------------- */

function renderGutter() {
  const n = editor.value.split("\n").length;
  const lines = [];
  for (let i = 1; i <= n; i += 1) lines.push(String(i));
  clear(gutter);
  gutter.appendChild(document.createTextNode(lines.join("\n")));
  syncScroll();
}
function syncScroll() {
  gutter.scrollTop = editor.scrollTop;
}

/** 解析 cutforge-script-v1 → plan items(格式/步数/逐项 tool 校验;诚实报错)。 */
function parseScript() {
  let v;
  try {
    v = JSON.parse(editor.value);
  } catch (e) {
    return { error: `脚本不是合法 JSON:${String(e && e.message || e)}` };
  }
  if (!v || v.format !== "cutforge-script-v1") {
    return { error: "缺 format: \"cutforge-script-v1\"(脚本宿主契约;示例片段可一键载入)" };
  }
  if (!Array.isArray(v.steps)) return { error: "缺 steps 数组" };
  if (v.steps.length > SCRIPT_MAX_STEPS) {
    return { error: `步数超限:${v.steps.length} > ${SCRIPT_MAX_STEPS}(与后端沙箱同额)` };
  }
  const planItems = [];
  for (let i = 0; i < v.steps.length; i += 1) {
    const s = v.steps[i];
    if (!s || typeof s.tool !== "string" || !s.tool) {
      return { error: `steps[${i}] 缺 tool(字符串)` };
    }
    planItems.push({ tool: s.tool, args: s.args && typeof s.args === "object" ? s.args : {} });
  }
  if (!planItems.length) return { error: "steps 为空(至少一步)" };
  return { plan: planItems };
}

/* ---------------- 运行 / 提交 / 停止 ---------------- */

async function runPreview() {
  const parsed = parseScript();
  if (parsed.error) {
    renderOutput({ error: parsed.error });
    return;
  }
  const token = ++runToken;
  setBusy(true, "预演中…(副本 dry-run)");
  plan.submitDraft(parsed.plan, "脚本");
  const pv = await plan.previewDraft();
  if (token !== runToken) return; // 已停止:丢弃过期结果
  setBusy(false);
  renderOutput(pv);
}

function stopRun() {
  if (!plan.isBusy()) {
    toast("当前没有进行中的运行", false);
    return;
  }
  runToken += 1;
  setBusy(false, "已停止:丢弃在途结果(请求由超时兜底)");
}

async function submitToApproval() {
  const parsed = parseScript();
  if (parsed.error) {
    toast(parsed.error, false);
    return;
  }
  plan.submitDraft(parsed.plan, "脚本");
  plan.clearApprovals();
  toast(`已送批准流(${parsed.plan.length} 项):到「差异面板」预演并逐项人审`);
  renderOutput(null, `已提交批准流(${parsed.plan.length} 项;等价 plan 面与预演同源)`);
}

function setBusy(busy, msg) {
  runBtn.disabled = busy;
  submitBtn.disabled = busy;
  statusLine.textContent = msg || "就绪";
}

/* ---------------- 输出渲染(结构化逐步回执)---------------- */

function renderOutput(pv, note) {
  clear(outputHost);
  if (note) statusLine.textContent = note;
  if (!pv) return;
  if (pv.error) {
    statusLine.textContent = "运行失败";
    outputHost.appendChild(h("div", { class: "plan-err", testid: "script-out-error" }, [`${pv.code || ""} ${pv.error}`]));
    return;
  }
  const sum = pv.summary || {};
  statusLine.textContent = `预演完成:${sum.total ?? "?"} 步,ok ${sum.ok ?? "?"} / 错 ${sum.err ?? "?"};rev ${pv.revFrom} → ${pv.revTo}(副本,真工程未动)。落地请「以 plan 提交」进差异面板人审。`;
  const list = h("div", { testid: "script-out-list" });
  (pv.items || []).forEach((it, i) => {
    list.appendChild(h("div", { class: `plan-card ${it.ok ? "" : "err"}`, testid: "script-out-item" }, [
      h("div", { class: "filter-row" }, [
        h("span", { class: "badge" }, [`#${i}`]),
        h("code", null, [it.tool]),
        it.ok ? h("span", { class: "badge ok" }, ["ok"]) : h("span", { class: "badge err", title: String(it.message || "") }, [it.code]),
        h("span", { class: "plan-rev" }, [`rev ${it.revBefore}→${it.revAfter}`]),
        h("span", { class: "hint" }, [`${(it.changes || []).length} 项字段变更`]),
      ]),
      h("div", { class: "hint" }, [String(it.message || "")]),
    ]));
  });
  outputHost.appendChild(list);
}

/* ---------------- 片段库(localStorage)与导出 ---------------- */

function lib() { return pref("scriptLib", {}); }

function fillLibSelect() {
  clear(libSelect);
  for (const b of BUILTINS) libSelect.appendChild(h("option", { value: `builtin:${b.name}` }, [b.name]));
  for (const name of Object.keys(lib()).sort()) {
    libSelect.appendChild(h("option", { value: `lib:${name}` }, [`库:${name}`]));
  }
}

function loadSnippet() {
  const v = libSelect.value || "";
  if (v.startsWith("builtin:")) {
    const b = BUILTINS.find((x) => `builtin:${x.name}` === v);
    const r = b.gen();
    if (r.error) {
      toast(r.error, false);
      statusLine.textContent = `片段「${b.name}」载入失败:${r.error}`;
      return;
    }
    editor.value = `${JSON.stringify(r.script, null, 2)}\n`;
    renderGutter();
    statusLine.textContent = `已载入「${b.name}」:${r.note}(生成自当前会话态,可手改后运行)`;
    return;
  }
  const name = v.slice(4);
  const text = lib()[name];
  if (typeof text !== "string") {
    toast("库项不存在(可能已被删除)", false);
    fillLibSelect();
    return;
  }
  editor.value = text;
  renderGutter();
  statusLine.textContent = `已载入库脚本「${name}」`;
}

function saveToLib() {
  const name = (libName.value || "").trim();
  if (!name) {
    toast("先填库脚本名", false);
    return;
  }
  const map = lib();
  map[name] = editor.value;
  setPref("scriptLib", map);
  fillLibSelect();
  libSelect.value = `lib:${name}`;
  toast(`已存入库:${name}(localStorage,重启保留)`);
}

function delFromLib() {
  const v = libSelect.value || "";
  if (!v.startsWith("lib:")) {
    toast("内置片段不可删除(选中一个「库:」项再删)", false);
    return;
  }
  const name = v.slice(4);
  const map = lib();
  delete map[name];
  setPref("scriptLib", map);
  fillLibSelect();
  toast(`已删除库脚本:${name}`);
}

/** 导出 plan JSON(脚本 → {plan:[{tool,args}]};批准流/apply_plan 直接可用)。 */
function exportPlan() {
  const parsed = parseScript();
  if (parsed.error) {
    toast(parsed.error, false);
    return;
  }
  const blob = new Blob([JSON.stringify({ plan: parsed.plan }, null, 2)], { type: "application/json" });
  const a = h("a", { href: URL.createObjectURL(blob), download: `cutforge-plan-${Date.now().toString(36)}.json` });
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 2000);
  toast("已导出 plan JSON");
}

/* ---------------- AI 粘贴区 ---------------- */

async function previewAiPlan() {
  const r = plan.parsePlanText(aiBox.value);
  if (r.error) {
    toast(r.error, false);
    return;
  }
  plan.submitDraft(r.plan, "AI 粘贴");
  setBusy(true, "AI plan 预演中…");
  const pv = await plan.previewDraft();
  setBusy(false);
  renderOutput(pv);
}
