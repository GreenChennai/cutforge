/* 新建工程向导(T2.5;册四 T4.9 升级为画布设置面板):预设五档 + 自定义宽高
 * (ADR-0015:64–7680 且偶数,前端拦截 + 提示)+ 帧率契约枚举。
 * 勘察口径(诚实):画幅/帧率仅 project_new 工具承接(创建时定死),已建工程无
 * canvas 修改工具——本面板即新建时的画布设置面,运行中改画幅登记「canvas_update 候 BE」。
 * W8 修复保持:模态语义经 ui/dialog(焦点陷阱 + Esc);旧 id 锚点全保留。 */
import { createProject } from "../core/commands.js";
import { openDialog } from "../ui/dialog.js";
import { h } from "../ui/dom.js";
import { toast } from "../ui/toast.js";

/** 预设五档(任务口径;3:4 为旧档保留)。 */
const CANVAS_PRESETS = [
  ["1080x1920", "9:16 竖屏(1080×1920)"],
  ["1920x1080", "16:9 横屏(1920×1080)"],
  ["1080x1080", "1:1 方形(1080×1080)"],
  ["3840x2160", "4K 横屏(3840×2160)"],
  ["2160x3840", "4K 竖屏(2160×3840)"],
  ["1080x1440", "3:4(1080×1440,旧档)"],
  ["custom", "自定义…"],
];
const CANVAS_MIN = 64;
const CANVAS_MAX = 7680;

/** 画布边长校验(ADR-0015:整数、64–7680、偶数;返回错误文案或空串)。 */
function canvasDimError(v) {
  const n = Number(v);
  if (!Number.isInteger(n)) return "须为整数(像素)";
  if (n < CANVAS_MIN || n > CANVAS_MAX) return `范围 ${CANVAS_MIN}–${CANVAS_MAX}`;
  if (n % 2 !== 0) return "须为偶数(编码器 yuv420p 约束)";
  return "";
}

/** 打开向导(保持旧 id/#wizard 锚点;遮罩 id=wizard 由 dialog 承担,
 * 打开前先移除 index.html 的静态 hidden 占位,避免同 id 双元素)。 */
export function openWizard() {
  document.getElementById("wizard")?.remove();
  openDialog({
    id: "wizard",
    title: "新建空工程 · 画布设置(画幅在创建时定死;已建工程暂无修改工具,候 BE)",
    build: (body) => {
      const name = h("input", { type: "text", id: "wiz-name", testid: "wiz-name", placeholder: "my-first-cut" });
      const ratio = h("select", { id: "wiz-ratio", testid: "wiz-ratio" });
      for (const [v, label] of CANVAS_PRESETS) ratio.appendChild(h("option", { value: v }, [label]));
      const custW = h("input", { type: "number", id: "wiz-cust-w", testid: "wiz-cust-w", min: CANVAS_MIN, max: CANVAS_MAX, step: 2, placeholder: "宽(64–7680 偶数)" });
      const custH = h("input", { type: "number", id: "wiz-cust-h", testid: "wiz-cust-h", min: CANVAS_MIN, max: CANVAS_MAX, step: 2, placeholder: "高(64–7680 偶数)" });
      const custRow = h("div", { class: "wiz-cust-row", testid: "wiz-cust", hidden: true }, [
        h("label", null, ["宽 ", custW]), h("label", null, ["高 ", custH]),
        h("span", { class: "hint", id: "wiz-cust-hint", testid: "wiz-cust-hint" }, ["校验:整数 · 64–7680 · 偶数(前端拦截)"]),
      ]);
      ratio.addEventListener("change", () => {
        custRow.hidden = ratio.value !== "custom";
      });
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
      body.appendChild(h("label", null, ["画幅预设 ", ratio]));
      body.appendChild(custRow);
      body.appendChild(h("label", null, ["帧率 ", fps, h("span", { class: "hint" }, ["(契约枚举 24/25/30/50/60;任意帧率候 BE)"])]));
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
        let w = 0;
        let hgt = 0;
        if (ratio.value === "custom") {
          const ew = canvasDimError(custW.value);
          const eh = canvasDimError(custH.value);
          if (ew || eh) {
            toast(`自定义画幅不可用:${ew ? `宽 ${ew}` : ""}${ew && eh ? ";" : ""}${eh ? `高 ${eh}` : ""}`, false);
            return;
          }
          w = Number(custW.value);
          hgt = Number(custH.value);
        } else {
          [w, hgt] = ratio.value.split("x").map(Number);
        }
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
