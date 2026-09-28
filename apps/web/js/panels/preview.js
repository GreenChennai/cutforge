/* 预览面板(T2.5):canvas 画质代理 + 传输控制 + 「精确预览」(render_frame)。
 * 诚实标注:canvas 代理不含转场/特效/字幕烧录的最终效果(E2-6 口径);
 * 精确预览调 render_frame 工具显示服务端最终帧——工具由并行任务落库,
 * 未就绪时把服务端错误如实呈现,不做假图。 */
import { $, h } from "../ui/dom.js";
import { projectStore } from "../core/store.js";
import { playback } from "../render/preview-loop.js";
import { mediaUrlFor } from "../core/model.js";
import { seekFrame } from "../core/commands.js";
import { precisePreview } from "../core/render-commands.js";
import { toast } from "../ui/toast.js";
import { cssVar } from "../render/theme.js";

let canvas = null;
let preciseBtn = null;
let preciseImg = null;
let preciseHint = null;

export function mount(container) {
  canvas = h("canvas", { id: "pv-canvas", testid: "preview-canvas", width: 1080, height: 1920 });
  container.appendChild(h("div", { id: "pv-stage", testid: "preview-stage" }, [canvas]));
  container.appendChild(h("div", { class: "pv-hint" }, [
    "预览为画质代理:不含转场 / 特效 / 字幕烧录的最终效果;成片请用右侧导出(E2-6 诚实标注)。",
    "「精确预览」调内核逐帧渲染当前帧。",
  ]));
  container.appendChild(h("div", { id: "pv-transport", testid: "preview-transport" }, [
    h("button", { id: "pv-frame-prev", testid: "preview-frame-prev", title: "←", onclick: () => seekFrame(-1) }, ["← 一帧"]),
    h("button", { id: "pv-frame-next", testid: "preview-frame-next", title: "→", onclick: () => seekFrame(1) }, ["一帧 →"]),
    h("span", { id: "pv-time", testid: "preview-time" }, ["0.000s"]),
    h("span", { class: "tb-sep" }),
    preciseBtn = h("button", {
      id: "pv-precise", testid: "preview-precise",
      title: "内核逐帧渲染播放头所在帧(render_frame)",
      onclick: () => runPrecise(),
      "data-tip": "服务端按最终合成口径渲染当前帧(慢于代理,结果为准)",
    }, ["◎ 精确预览"]),
  ]));
  preciseImg = h("img", { id: "pv-precise-frame", testid: "preview-precise-frame", alt: "精确预览帧", hidden: true });
  preciseHint = h("div", { id: "pv-precise-hint", testid: "preview-precise-hint", class: "dim" });
  container.appendChild(preciseImg);
  container.appendChild(preciseHint);
  container.appendChild(h("div", { id: "pv-media", hidden: true, testid: "pv-media" }));

  // 画布尺寸跟随工程画幅;注册 canvas 代理绘制(播放循环回调)
  projectStore.subscribe((patch, st) => {
    if ((patch.project !== undefined || patch.__reset__) && st.project) {
      const cv = st.project.canvas;
      if (cv && (canvas.width !== cv.width || canvas.height !== cv.height)) {
        canvas.width = cv.width;
        canvas.height = cv.height;
      }
    }
  });
  playback.registerRenderer(drawProxy);
}

/** canvas 画质代理(旧壳 drawPreview 口径:可见行按轨序自下而上,overlay/position/scale)。 */
function drawProxy(rows) {
  const ctx = canvas.getContext("2d");
  const p = projectStore.get().project;
  if (p && (canvas.width !== p.canvas.width || canvas.height !== p.canvas.height)) {
    canvas.width = p.canvas.width;
    canvas.height = p.canvas.height;
  }
  ctx.fillStyle = cssVar("--cf-stage"); // 舞台底(经 token,零硬编码)
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  if (!p) return;
  const t = playback.clockMs();
  const trackOrder = (tid) => p.tracks.findIndex((x) => x.id === tid);
  const vis = rows
    .filter(({ row }) => rowCoversLocal(row, t))
    .filter(({ row }) => row.track.charAt(0) !== "A")
    .sort((a, b) => trackOrder(a.row.track) - trackOrder(b.row.track));
  for (const { row, el } of vis) {
    ctx.save();
    const ov = row.overlay;
    const pos = row.position;
    const scale = row.scale || 1;
    if (ov) {
      ctx.globalAlpha = ov.opacity ?? 1;
      ctx.drawImage(el, ov.x, ov.y, ov.w, ov.h);
    } else if (pos || scale !== 1) {
      const w = canvas.width * scale;
      const hgt = canvas.height * scale;
      const dx = pos ? (pos.x / 100) * (canvas.width - w) : (canvas.width - w) / 2;
      const dy = pos ? (pos.y / 100) * (canvas.height - hgt) : (canvas.height - hgt) / 2;
      ctx.drawImage(el, dx, dy, w, hgt);
    } else {
      ctx.drawImage(el, 0, 0, canvas.width, canvas.height);
    }
    ctx.restore();
  }
}

function rowCoversLocal(row, t) {
  return Boolean(row.src) && t >= row.startMs && t < row.endMs;
}

/** 精确预览:render_frame(工具未落库/失败 → 服务端错误如实呈现)。 */
async function runPrecise() {
  const t = playback.clockMs();
  preciseHint.textContent = "内核渲染中…";
  preciseImg.hidden = true;
  const env = await precisePreview(Math.round(t));
  if (!env.ok) {
    preciseHint.textContent = `精确预览不可用:${env.code} ${env.message || ""}`.trim()
      + (env.code === "INTERNAL" ? "(render_frame 工具可能尚未在服务端落库)" : "");
    toast(`精确预览:${env.code}`, false);
    return;
  }
  // render_frame 契约:data.media = 工程内相对路径,壳经 /media 加载
  const framePath = env.data && (env.data.media || env.data.frame || env.data.path || env.data.output);
  if (framePath && !/^[a-zA-Z]:[\\/]/.test(framePath)) {
    preciseImg.src = mediaUrlFor(framePath, projectStore.get().token);
    preciseImg.hidden = false;
    preciseHint.textContent = `精确帧 @${Math.round(t)}ms(内核 render_frame)`;
  } else if (framePath) {
    preciseHint.textContent = `render_frame 返回了非工程内相对路径,壳拒绝加载(壳纯度)`;
  } else {
    preciseHint.textContent = `render_frame 已响应但未返回可显示帧:${JSON.stringify(env.data || {}).slice(0, 120)}`;
  }
}
