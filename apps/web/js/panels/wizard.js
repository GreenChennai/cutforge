/* 新建工程向导(T2.5,B11-2 对拍;W8 修复:模态语义经 ui/dialog——焦点陷阱 + Esc)。 */
import { createProject } from "../core/commands.js";
import { openDialog } from "../ui/dialog.js";
import { h } from "../ui/dom.js";
import { toast } from "../ui/toast.js";

/** 打开向导(保持旧 id/#wizard 锚点;遮罩 id=wizard 由 dialog 承担,
 * 打开前先移除 index.html 的静态 hidden 占位,避免同 id 双元素)。 */
export function openWizard() {
  document.getElementById("wizard")?.remove();
  openDialog({
    id: "wizard",
    title: "新建空工程(CutForge 可独立起步,不依赖 CutFlow)",
    build: (body) => {
      const name = h("input", { type: "text", id: "wiz-name", testid: "wiz-name", placeholder: "my-first-cut" });
      const ratio = h("select", { id: "wiz-ratio", testid: "wiz-ratio" }, [
        h("option", { value: "1080x1920" }, ["9:16 竖屏(1080×1920)"]),
        h("option", { value: "1080x1440" }, ["3:4(1080×1440)"]),
        h("option", { value: "1920x1080" }, ["16:9 横屏(1920×1080)"]),
      ]);
      const fps = h("select", { id: "wiz-fps", testid: "wiz-fps" });
      for (const f of ["24", "25", "30", "50", "60"]) {
        const opt = h("option", { value: f }, [f]);
        if (f === "30") opt.selected = true;
        fps.appendChild(opt);
      }
      const trV = h("input", { type: "checkbox", id: "wiz-tr-v", testid: "wiz-tr-v", checked: true });
      const trA = h("input", { type: "checkbox", id: "wiz-tr-a", testid: "wiz-tr-a", checked: true });
      const trT = h("input", { type: "checkbox", id: "wiz-tr-t", testid: "wiz-tr-t" });
      body.appendChild(h("label", null, ["工程名 ", name]));
      body.appendChild(h("label", null, ["画幅 ", ratio]));
      body.appendChild(h("label", null, ["帧率 ", fps]));
      body.appendChild(h("label", null, [
        "初始轨道 ",
        h("span", { class: "toggle" }, [trV, " 视频"]),
        h("span", { class: "toggle" }, [trA, " 音频"]),
        h("span", { class: "toggle" }, [trT, " 文本"]),
      ]));
      body.appendChild(h("div", { class: "hint", id: "wiz-hint", testid: "wiz-hint" }, [
        "将在当前工程旁创建同名目录并生成 05_时间线工程/project.json;创建后用打印的命令打开它。",
      ]));
      const actions = h("div", { class: "wizard-actions" }, [
        h("button", { id: "wiz-create", testid: "wiz-create", onclick: () => submit() }, ["创建"]),
        h("button", { id: "wiz-cancel", testid: "wiz-cancel" }, ["取消"]),
      ]);
      body.appendChild(actions);
      actions.querySelector("#wiz-cancel").addEventListener("click", () => {
        document.getElementById("wizard")?.remove();
      });
      name.focus();

      async function submit() {
        const nameV = name.value.trim();
        if (!nameV || /[\\/:*?"<>|]/.test(nameV)) {
          toast("工程名必填,且不能含路径非法字符", false);
          return;
        }
        const [w, hgt] = ratio.value.split("x").map(Number);
        const tracks = [];
        if (trV.checked) tracks.push("video");
        if (trA.checked) tracks.push("audio");
        if (trT.checked) tracks.push("text");
        if (!tracks.length) {
          toast("至少勾选一条初始轨道", false);
          return;
        }
        const env = await createProject({
          name: nameV, fps: Number(fps.value), canvasW: w, canvasH: hgt, tracks,
        });
        if (env && env.ok) document.getElementById("wizard")?.remove();
      }
    },
  });
}
