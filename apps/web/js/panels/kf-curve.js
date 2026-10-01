/* 可编辑曲线画布(册五 T5.1/T5.2 共用组件;curve.js 速度曲线保持独立不迁移)。
 *
 * 两种模式:
 * - "kf":关键帧锚点(timeMs/value/interp/bezier)+ 投影 keyframeSamples 采样点云。
 *   ADR-0018 求值单源:曲线形状唯一来源 = 服务端采样;锚点拖拽期画「直线示意」并
 *   显式标注(松手提交后投影刷新为真曲线),壳零插值公式;
 * - "points":归一 0..1 点集(grade.curves;点间连线为显示连线,数据本身即点集)。
 *
 * 交互:锚点/点拖拽(零 Op,松手 onCommit 一笔)、bezier 双柄拖拽(interp=bezier)、
 * 双击空白 = 打点、双击锚点 = 删点、Esc = 取消(onCancel)。全部色值经 theme token。
 */
import { cssVar } from "../render/theme.js";
import { runGesture } from "../render/gesture-kit.js";

const PAD = 10;
const HIT_PX = 9;

/**
 * @param {HTMLElement} host 挂载容器
 * @param {Object} ctl 控制器(回调面,见模块注释)
 * @returns {{ canvas: HTMLCanvasElement, draw: () => void, redraw: () => void }}
 */
export function createCurveCanvas(host, ctl) {
  const W = ctl.width || 340;
  const H = ctl.height || 170;
  const canvas = /** @type {HTMLCanvasElement} */ (document.createElement("canvas"));
  canvas.width = W;
  canvas.height = H;
  canvas.className = "kfc-canvas";
  canvas.setAttribute("data-testid", ctl.testid || "kf-curve-canvas");
  canvas.setAttribute("aria-label", ctl.ariaLabel || "曲线编辑画布");
  host.appendChild(canvas);
  let drag = null; // {kind:"anchor"|"handle"|"point", idx, which?}
  let dragging = false;

  const frame = () => ctl.frameOf();
  const xOf = (t, f) => PAD + (t / Math.max(1, f.dur)) * (W - PAD * 2);
  const yOf = (v, f) => H - PAD - ((v - f.lo) / Math.max(1e-9, f.hi - f.lo)) * (H - PAD * 2);
  const tAt = (px, f) => Math.max(0, Math.min(f.dur, ((px - PAD) / (W - PAD * 2)) * f.dur));
  const vAt = (py, f) => {
    const n = (H - PAD - py) / (H - PAD * 2);
    return f.lo + n * (f.hi - f.lo);
  };

  /** 命中检测:柄 > 锚/点 > 空。 */
  function hit(px, py) {
    const f = frame();
    // 1) bezier 柄(选中锚且 interp=bezier)
    if (f.sel >= 0 && f.anchors[f.sel] && f.anchors[f.sel].interp === "bezier" && f.anchors[f.sel + 1]) {
      const box = segBox(f, f.sel);
      for (const which of [0, 1]) {
        const cp = f.anchors[f.sel].bezier || [0.42, 0, 0.58, 1];
        const hx = box.x + cp[which * 2] * box.w;
        const hy = box.y + (1 - cp[which * 2 + 1]) * box.h;
        if (Math.hypot(px - hx, py - hy) <= HIT_PX) return { kind: "handle", idx: f.sel, which };
      }
    }
    // 2) 锚点/点(就近)
    let best = -1;
    let bestD = HIT_PX;
    f.anchors.forEach((a, i) => {
      const d = Math.hypot(px - xOf(a.t, f), py - yOf(a.v, f));
      if (d <= bestD) { bestD = d; best = i; }
    });
    if (best >= 0) return { kind: ctl.mode === "points" ? "point" : "anchor", idx: best };
    return null;
  }

  function segBox(f, i) {
    const a = f.anchors[i];
    const b = f.anchors[i + 1];
    const x = xOf(a.t, f);
    const y = yOf(a.v, f);
    return { x, y, w: xOf(b.t, f) - x, h: yOf(b.v, f) - y };
  }

  function draw() {
    const ctx = canvas.getContext("2d");
    const f = frame();
    ctx.fillStyle = cssVar("--cf-panel-deep") || "";
    ctx.fillRect(0, 0, W, H);
    // 网格(四分)
    ctx.strokeStyle = cssVar("--cf-line") || "";
    ctx.lineWidth = 1;
    for (const fy of [0.25, 0.5, 0.75]) {
      ctx.beginPath();
      ctx.moveTo(PAD, PAD + (H - PAD * 2) * fy);
      ctx.lineTo(W - PAD, PAD + (H - PAD * 2) * fy);
      ctx.stroke();
    }
    for (const fx of [0.25, 0.5, 0.75]) {
      ctx.beginPath();
      ctx.moveTo(PAD + (W - PAD * 2) * fx, PAD);
      ctx.lineTo(PAD + (W - PAD * 2) * fx, H - PAD);
      ctx.stroke();
    }
    // 采样点云(服务端求值投影;拖拽中 = 过期,降淡)
    ctx.fillStyle = cssVar("--cf-accent-soft") || "";
    const sampleAlpha = dragging ? 0.28 : 0.85;
    ctx.globalAlpha = sampleAlpha;
    for (const [t, v] of f.samples || []) {
      ctx.fillRect(xOf(t, f) - 1, yOf(v, f) - 1, 2, 2);
    }
    ctx.globalAlpha = 1;
    // 直线示意(拖拽期):锚点间连线(非插值公式,示意线,提交后以采样为准)
    if (dragging && f.anchors.length > 1) {
      ctx.strokeStyle = cssVar("--cf-line-strong") || "";
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      f.anchors.forEach((a, i) => {
        const x = xOf(a.t, f);
        const y = yOf(a.v, f);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
      ctx.setLineDash([]);
    }
    // 播放头(kf 模式)
    if (ctl.mode !== "points" && f.playheadT !== undefined && f.playheadT !== null) {
      const x = Math.round(xOf(f.playheadT, f)) + 0.5;
      if (x >= PAD && x <= W - PAD) {
        ctx.strokeStyle = cssVar("--cf-playhead") || "";
        ctx.beginPath();
        ctx.moveTo(x, PAD);
        ctx.lineTo(x, H - PAD);
        ctx.stroke();
      }
    }
    // 点模式:显示连线(数据即点集)
    if (ctl.mode === "points" && f.anchors.length > 1) {
      ctx.strokeStyle = cssVar("--cf-accent") || "";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      f.anchors.forEach((a, i) => {
        const x = xOf(a.t, f);
        const y = yOf(a.v, f);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
      ctx.lineWidth = 1;
    }
    // 锚点(菱形)
    f.anchors.forEach((a, i) => {
      drawDiamond(ctx, xOf(a.t, f), yOf(a.v, f), i === f.sel);
    });
    // bezier 柄(选中锚)
    if (f.sel >= 0 && f.anchors[f.sel] && f.anchors[f.sel].interp === "bezier" && f.anchors[f.sel + 1]) {
      const cp = f.anchors[f.sel].bezier || [0.42, 0, 0.58, 1];
      const box = segBox(f, f.sel);
      ctx.strokeStyle = cssVar("--cf-line-strong") || "";
      for (const which of [0, 1]) {
        const hx = box.x + cp[which * 2] * box.w;
        const hy = box.y + (1 - cp[which * 2 + 1]) * box.h;
        ctx.beginPath();
        ctx.moveTo(which === 0 ? box.x : box.x + box.w, which === 0 ? box.y : box.y + box.h);
        ctx.lineTo(hx, hy);
        ctx.stroke();
        drawDiamond(ctx, hx, hy, true, 4);
      }
    }
  }

  function drawDiamond(ctx, x, y, selected, r = 5) {
    ctx.fillStyle = selected ? (cssVar("--cf-select") || "") : (cssVar("--cf-accent") || "");
    ctx.beginPath();
    ctx.moveTo(x, y - r);
    ctx.lineTo(x + r, y);
    ctx.lineTo(x, y + r);
    ctx.lineTo(x - r, y);
    ctx.closePath();
    ctx.fill();
    if (selected) {
      ctx.strokeStyle = cssVar("--cf-bg") || "";
      ctx.stroke();
    }
  }

  function snapTime(t, f) {
    const snapped = ctl.snapTimeOf ? ctl.snapTimeOf(t) : t;
    return Math.max(0, Math.min(f.dur, Math.round(snapped)));
  }

  canvas.addEventListener("pointerdown", (e) => {
    const rect = canvas.getBoundingClientRect();
    const px = ((e.clientX - rect.left) / rect.width) * W;
    const py = ((e.clientY - rect.top) / rect.height) * H;
    const h = hit(px, py);
    if (h) {
      drag = h;
      if (h.kind === "anchor" || h.kind === "point") {
        if (ctl.onSelect) ctl.onSelect(h.idx);
        if (ctl.mode !== "points") dragging = true;
      } else {
        dragging = true;
      }
      /** 指针事件 → 草稿更新(move 与 end 共用;end 补最后一帧防 rAF 丢帧)。 */
      const applyPointer = (ev) => {
        if (!drag) return;
        const r2 = canvas.getBoundingClientRect();
        const mx = ((ev.clientX - r2.left) / r2.width) * W;
        const my = ((ev.clientY - r2.top) / r2.height) * H;
        const f2 = frame();
        if (drag.kind === "anchor") {
          ctl.onAnchorDrag(drag.idx, snapTime(tAt(mx, f2), f2), vAt(my, f2));
        } else if (drag.kind === "point") {
          ctl.onAnchorDrag(drag.idx,
            Math.max(0, Math.min(1, (mx - PAD) / (W - PAD * 2))),
            Math.max(0, Math.min(1, (H - PAD - my) / (H - PAD * 2))));
        } else if (drag.kind === "handle") {
          const box = segBox(f2, drag.idx);
          const cp = f2.anchors[drag.idx].bezier || [0.42, 0, 0.58, 1];
          const nx = Math.max(0, Math.min(1, (mx - box.x) / Math.max(1, box.w)));
          const ny = Math.max(0, Math.min(1, 1 - (my - box.y) / Math.max(1e-9, box.h)));
          const next = cp.slice();
          next[drag.which * 2] = Math.round(nx * 1000) / 1000;
          next[drag.which * 2 + 1] = Math.round(ny * 1000) / 1000;
          ctl.onHandleDrag(drag.idx, next);
        }
        draw();
      };
      runGesture(canvas, e, {
        move: applyPointer,
        end: (ev) => {
          const wasDrag = dragging;
          if (ev && drag) applyPointer(ev); // 补最后一帧(rAF 合帧可能吞掉收尾 move)
          drag = null;
          dragging = false;
          draw();
          if (wasDrag && ctl.onCommit) ctl.onCommit();
        },
        cancel: () => {
          drag = null;
          dragging = false;
          draw();
          if (ctl.onCancel) ctl.onCancel();
        },
      });
    } else if (ctl.onSelect) {
      ctl.onSelect(-1);
    }
  });

  canvas.addEventListener("dblclick", (e) => {
    const rect = canvas.getBoundingClientRect();
    const px = ((e.clientX - rect.left) / rect.width) * W;
    const py = ((e.clientY - rect.top) / rect.height) * H;
    const f = frame();
    const h = hit(px, py);
    if (h && (h.kind === "anchor" || h.kind === "point")) {
      if (ctl.onAnchorDel) ctl.onAnchorDel(h.idx);
    } else if (ctl.onCanvasDbl) {
      if (ctl.mode === "points") {
        ctl.onCanvasDbl(
          Math.max(0, Math.min(1, (px - PAD) / (W - PAD * 2))),
          Math.max(0, Math.min(1, (H - PAD - py) / (H - PAD * 2))));
      } else {
        ctl.onCanvasDbl(snapTime(tAt(px, f), f), vAt(py, f));
      }
    }
  });

  draw();
  return { canvas, draw, redraw: draw };
}
