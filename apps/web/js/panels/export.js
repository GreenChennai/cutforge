/* 导出面板(T2.5,E5 对拍 + 新增):双后端(cutforge 异步 / ffmpeg 经 CutFlow)
 * + 剪映草稿(export_jianying)+ 产物清单(render_probe)。
 * 进度面:轮询为主(确定性),render.progress SSE 事件作为提速信号触发即查。 */
import { h, clear } from "../ui/dom.js";
import { projectStore } from "../core/store.js";
import { runExportCutforge, runExportFfmpeg, exportJianying, refreshRenderOutputs } from "../core/render-commands.js";
import { selectField } from "../ui/controls.js";
import { openDialog } from "../ui/dialog.js";
import { toast } from "../ui/toast.js";
import { subscribe } from "../core/event-bus.js";

let backendSel = null;
let ratioSel = null;
let progressEl = null;
let filesEl = null;

export function mount(container) {
  container.appendChild(h("h3", null, ["导出(编辑器内出片,E5)"]));
  backendSel = selectField({
    id: "exp-backend", testid: "export-backend",
    options: [["cutforge", "cutforge 内核(不依赖 CutFlow)"], ["ffmpeg", "ffmpeg(CutFlow rs_render)"]],
  });
  container.appendChild(h("label", null, ["后端 ", backendSel.root]));
  ratioSel = selectField({
    id: "exp-ratio", testid: "export-ratio",
    options: [["9x16", "9x16"], ["3x4", "3x4"], ["16x9", "16x9"]],
  });
  container.appendChild(h("label", null, ["画幅(仅 ffmpeg 后端) ", ratioSel.root]));
  container.appendChild(h("div", { class: "exp-actions" }, [
    h("button", { id: "exp-run", testid: "export-run", onclick: () => runExport() }, ["导出成片"]),
    h("button", { id: "exp-jy", testid: "export-jianying", "data-tip": "生成剪映草稿(export_jianying;不落盘成片)", onclick: () => runJianying() }, ["导出剪映草稿"]),
  ]));
  progressEl = h("div", { id: "exp-progress", class: "exp-progress", testid: "export-progress" });
  container.appendChild(progressEl);
  container.appendChild(h("h3", null, ["产物(06_成片输出)"]));
  filesEl = h("div", { id: "exp-files", testid: "export-files" });
  container.appendChild(filesEl);

  // render.progress SSE:渲染状态变化 → 立即拉产物清单(提速信号;轮询仍是主路径)
  subscribe("render.progress", (data) => {
    if (data && (data.state === "ok" || data.state === "fail")) refreshFiles();
  });
  refreshFiles();
}

function setProgress(text) {
  progressEl.textContent = text;
}

/** render-commands 的 onProgress(state, text) → 面板只展示人话文本。 */
function onProgress(_state, text) {
  setProgress(text);
}

function runExport() {
  if (backendSel.get() === "cutforge") {
    runExportCutforge(onProgress);
  } else {
    runExportFfmpeg(ratioSel.get(), onProgress);
  }
}

function runJianying() {
  const slug = projectStore.get().project?.slug || "cutforge";
  openDialog({
    id: "jy-dialog",
    title: "导出剪映草稿(export_jianying)",
    build: (body) => {
      const nameField = h("input", { type: "text", testid: "jy-name", value: `${slug}-draft` });
      const out = h("div", { class: "exp-progress", testid: "jy-progress" });
      const run = h("button", {
        testid: "jy-run",
        onclick: async () => {
          const name = /** @type {HTMLInputElement} */ (nameField).value.trim();
          if (!name) {
            toast("草稿名必填", false);
            return;
          }
          run.disabled = true;
          await exportJianying(name, (state, text) => {
            out.textContent = text;
            if (state === "done" || state === "fail") run.disabled = false;
          });
        },
      }, ["生成"]);
      const close = h("button", { testid: "jy-cancel" }, ["关闭"]);
      close.addEventListener("click", () => document.getElementById("jy-dialog")?.remove());
      body.appendChild(h("label", null, ["草稿名 ", nameField]));
      body.appendChild(out);
      body.appendChild(h("div", { class: "wizard-actions" }, [run, close]));
    },
  });
}

async function refreshFiles() {
  const files = await refreshRenderOutputs();
  clear(filesEl);
  if (!files.length) {
    filesEl.appendChild(h("div", { class: "dim" }, ["(暂无产物)"]));
    return;
  }
  for (const f of files) {
    filesEl.appendChild(h("div", { class: "row", testid: "export-file" }, [
      h("span", { class: "oid" }, [String(f.file)]),
      h("span", { class: "bd" }, [`${f.bytes} B`]),
    ]));
  }
}
