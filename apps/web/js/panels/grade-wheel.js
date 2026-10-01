/* 调色三色轮(Lift/Gamma/Gain;册五 T5.2 FE)。
 *
 * 简化实现(计划书口径):角度 = 色相,半径 = 强度,写回 RGB 三值。
 * 色相向量基(屏幕系,等距 120°):R=(1,0),G=(−.5,−.866),B=(−.5,+.866);
 * 正解(s=0 保中性轴):r=2px/3,b=(−r+py/.866)/2,g=(−r−py/.866)/2;
 * 逆解:px=1.5·Δr,py=.866·(2Δb+Δr)。Δ = 值 − 中性(0 或 1)。
 * 色值纪律:盘面光谱为逐像素数学生成(取色器的数据面,同 <input type=color>),
 * 非主题色;轮圈/指针/面板 chrome 全部经 theme token(cssVar),零硬编码。
 */
import { h } from "../ui/dom.js";
import { cssVar } from "../render/theme.js";
import { runGesture } from "../render/gesture-kit.js";

const DIR_R = [1, 0];
const DIR_G = [-0.5, -0.8660254037844386];
const DIR_B = [-0.5, 0.8660254037844386];

/** 轮位置(θ 顺屏时针自 +x,ρ∈[0,1])→ RGB 分量(中性轴保持,s=0;未夹取)。 */
export function wheelToRgb(theta, rho) {
  const px = rho * Math.cos(theta);
  const py = rho * Math.sin(theta);
  const r = (2 * px) / 3;
  const b = (-r + py / 0.8660254037844386) / 2;
  const g = (-r - py / 0.8660254037844386) / 2;
  return [r, g, b];
}

/** 分量 → 通道值(中性 + 强度×分量,域夹取)。 */
export function compToChannel(comp, neutral, strength, lo, hi) {
  return Math.min(hi, Math.max(lo, neutral + strength * comp));
}

/** RGB 三值 → 轮位置 {theta, rho}(rho 夹到 0..1 显示域)。 */
export function rgbToWheel(rgb, neutral) {
  const d = rgb.map((v) => v - neutral);
  const px = 1.5 * d[0];
  const py = 0.8660254037844386 * (2 * d[2] + d[0]);
  const rho = Math.min(1, Math.hypot(px, py));
  const theta = Math.atan2(py, px);
  return { theta, rho };
}

/**
 * 构建一枚色轮组件。
 * @param {{
 *   label: string, testid: string, neutral: number, strength: number, step?: number,
 *   lo: number, hi: number,
 *   get: () => [number, number, number]|null, set: (rgb: [number, number, number]) => void,
 *   onCommit: () => void, onCancel?: () => void, reset: () => void,
 * }} opts
 */
export function createColorWheel(opts) {
  const SIZE = 116;
  const R = SIZE / 2 - 8;
  const canvas = /** @type {HTMLCanvasElement} */ (h("canvas", {
    width: String(SIZE), height: String(SIZE),
    class: "grade-wheel", testid: opts.testid,
    "aria-label": `${opts.label} 色轮(角度=色相,半径=强度;拖动调值)`,
  }));
  const nums = [0, 1, 2].map(() => /** @type {HTMLInputElement} */ (h("input", {
    type: "number", class: "grade-wheel-num", step: opts.step || 0.01,
    "aria-label": `${opts.label} 分量`,
  })));
  nums.forEach((n, i) => {
    n.addEventListener("change", () => {
      const v = Number(n.value);
      if (Number.isNaN(v)) return;
      const cur = opts.get() || [opts.neutral, opts.neutral, opts.neutral];
      const next = cur.slice();
      next[i] = v;
      opts.set(/** @type {[number,number,number]} */ (next));
      opts.onCommit();
    });
  });
  const root = h("div", { class: "grade-wheel-box" }, [
    h("div", { class: "grade-wheel-head" }, [
      h("b", null, [opts.label]),
      h("button", {
        class: "mini", type: "button", testid: `${opts.testid}-reset`,
        title: `回中性(每通道 ${opts.neutral})`, "aria-label": `${opts.label} 回中性`,
        onclick: () => { opts.reset(); draw(); opts.onCommit(); },
      }, ["归零"]),
    ]),
    canvas,
    h("div", { class: "grade-wheel-nums" }, nums),
  ]);

  function drawDisc() {
    const ctx = canvas.getContext("2d");
    const img = ctx.createImageData(SIZE, SIZE);
    const ring = parseHex(cssVar("--cf-gray-600"));
    for (let y = 0; y < SIZE; y += 1) {
      for (let x = 0; x < SIZE; x += 1) {
        const dx = x - SIZE / 2;
        const dy = y - SIZE / 2;
        const rho = Math.hypot(dx, dy) / R;
        const i = (y * SIZE + x) * 4;
        if (rho > 1.06) {
          img.data[i + 3] = 0;
          continue;
        }
        if (rho > 0.94) {
          // 轮圈(token 色;超出光谱域的 chrome)
          img.data[i] = ring[0]; img.data[i + 1] = ring[1]; img.data[i + 2] = ring[2]; img.data[i + 3] = 255;
          continue;
        }
        const rr = Math.min(1, rho);
        const theta = Math.atan2(dy, dx);
        const comps = wheelToRgb(theta, rr);
        // 显示域:中性灰心(0.5)→ 边缘全强度(0.5+0.5·分量)
        img.data[i] = Math.round((0.5 + 0.5 * comps[0]) * 255);
        img.data[i + 1] = Math.round((0.5 + 0.5 * comps[1]) * 255);
        img.data[i + 2] = Math.round((0.5 + 0.5 * comps[2]) * 255);
        img.data[i + 3] = 255;
      }
    }
    ctx.putImageData(img, 0, 0);
  }

  function parseHex(s) {
    const v = parseInt(String(s || "").replace("#", ""), 16);
    return Number.isNaN(v) ? [128, 128, 128] : [(v >> 16) & 255, (v >> 8) & 255, v & 255];
  }

  function drawPointer(ctx, forced) {
    const cur = forced || opts.get();
    if (!cur) return;
    const { theta, rho } = rgbToWheel(cur, opts.neutral);
    const px = SIZE / 2 + Math.cos(theta) * rho * R;
    const py = SIZE / 2 + Math.sin(theta) * rho * R;
    ctx.strokeStyle = cssVar("--cf-fg") || "";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.arc(px, py, 5, 0, Math.PI * 2);
    ctx.stroke();
    ctx.lineWidth = 1;
  }

  function draw() {
    drawDisc();
    drawPointer(canvas.getContext("2d"), null);
    const cur = opts.get();
    nums.forEach((n, i) => {
      const v = cur ? cur[i] : opts.neutral;
      if (document.activeElement !== n) n.value = String(round2(v));
    });
  }

  function round2(v) {
    return Math.round(v * 100) / 100;
  }

  canvas.addEventListener("pointerdown", (e) => {
    const start = opts.get() ? opts.get().slice() : null;
    const apply = (ev) => {
      const rect = canvas.getBoundingClientRect();
      const px = ((ev.clientX - rect.left) / rect.width) * SIZE - SIZE / 2;
      const py = ((ev.clientY - rect.top) / rect.height) * SIZE - SIZE / 2;
      const rho = Math.min(1, Math.hypot(px, py) / R);
      const theta = Math.atan2(py, px);
      const comps = wheelToRgb(theta, rho);
      opts.set(/** @type {[number,number,number]} */ ([
        compToChannel(comps[0], opts.neutral, opts.strength, opts.lo, opts.hi),
        compToChannel(comps[1], opts.neutral, opts.strength, opts.lo, opts.hi),
        compToChannel(comps[2], opts.neutral, opts.strength, opts.lo, opts.hi),
      ]));
      draw();
    };
    apply(e);
    runGesture(canvas, e, {
      move: apply,
      end: (ev) => {
        if (ev) apply(ev); // 补最后一帧(rAF 合帧可能吞掉收尾 move;跟手纪律)
        opts.onCommit();
      },
      cancel: () => {
        if (start) opts.set(/** @type {[number,number,number]} */ (start));
        else opts.reset();
        draw();
        if (opts.onCancel) opts.onCancel();
      },
    });
  });

  draw();
  return { root, draw };
}
