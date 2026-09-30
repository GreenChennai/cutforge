/* 表单控件工厂(T2.5):数字(带拖拽调节)/文本/下拉/开关/折叠分组。
 * 受控约定:值由 store/投影经 set() 下发;用户输入经 onInput 上抛(commands 层决定
 * 何时发命令——检查器为「暂存草稿、应用才提交」模式,与 e2e fill→apply 流程对齐)。
 */
import { h } from "./dom.js";

/** @typedef {{ root: HTMLElement, input: HTMLInputElement|HTMLSelectElement, set: (v: *|null) => void, get: () => string }} Field */

/**
 * 数字字段(带 ◂▸ 拖拽调节手柄;Playwright fill 直达 input,手柄不干扰自动化)。
 * @param {{ id?: string, testid?: string, label?: string, step?: number, min?: number, max?: number,
 *          onInput?: (v: string) => void }} opts
 * @returns {Field}
 */
export function numberField(opts = {}) {
  const input = h("input", {
    type: "number",
    id: opts.id || null,
    testid: opts.testid || null,
    step: opts.step !== undefined ? opts.step : null,
    min: opts.min !== undefined ? opts.min : null,
    max: opts.max !== undefined ? opts.max : null,
  });
  input.addEventListener("input", () => opts.onInput && opts.onInput(input.value));

  const step = opts.step || 1;
  let dragging = false;
  const drag = h("span", { class: "num-drag", "data-tip": "拖动调节(±step)", "aria-hidden": "true" }, ["◂▸"]);
  drag.addEventListener("pointerdown", (e) => {
    dragging = true;
    drag.setPointerCapture(e.pointerId);
    let lastX = e.clientX;
    const onMove = (ev) => {
      if (!dragging) return;
      const delta = Math.round((ev.clientX - lastX) / 4) * step;
      if (delta !== 0) {
        lastX = ev.clientX;
        const base = Number(input.value) || 0;
        input.value = String(base + delta);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }
    };
    const onUp = () => {
      dragging = false;
      drag.removeEventListener("pointermove", onMove);
      drag.removeEventListener("pointerup", onUp);
    };
    drag.addEventListener("pointermove", onMove);
    drag.addEventListener("pointerup", onUp);
    e.preventDefault();
  });

  const wrap = h("span", { class: "num-wrap" }, [input, drag]);
  return {
    root: wrap,
    input,
    get: () => input.value,
    set(v) {
      input.value = (v === undefined || v === null) ? "" : String(v);
    },
  };
}

/**
 * 文本字段。
 * @param {{ id?: string, testid?: string, placeholder?: string, ariaLabel?: string,
 *          onInput?: (v: string) => void, onEnter?: () => void }} opts
 * @returns {Field}
 */
export function textField(opts = {}) {
  const input = h("input", {
    type: "text", id: opts.id || null, testid: opts.testid || null,
    placeholder: opts.placeholder || null, "aria-label": opts.ariaLabel || null,
  });
  input.addEventListener("input", () => opts.onInput && opts.onInput(input.value));
  if (opts.onEnter) {
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") opts.onEnter();
    });
  }
  return {
    root: input, input,
    get: () => input.value,
    set(v) { input.value = (v === undefined || v === null) ? "" : String(v); },
  };
}

/**
 * 下拉字段。options: [value, label][]
 * @param {{ id?: string, testid?: string, options: Array<[string, string]>, onChange?: (v: string) => void }} opts
 * @returns {Field}
 */
export function selectField(opts = {}) {
  const input = h("select", { id: opts.id || null, testid: opts.testid || null });
  for (const [v, label] of opts.options) {
    const opt = h("option", { value: v }, [label]);
    input.appendChild(opt);
  }
  input.addEventListener("change", () => opts.onChange && opts.onChange(input.value));
  return {
    root: input, input,
    get: () => input.value,
    set(v) { input.value = (v === undefined || v === null) ? "" : String(v); },
  };
}

/**
 * 滑杆字段(册四 FE2):range + number 双联(拖杆粗调,数字框精调;两端同值同步)。
 * 受控约定同 numberField:set() 下发投影值,input/change 上抛(检查器草稿模式仍
 * 「应用才提交」;fx/动效等即时面板可监听 change=松手一次)。
 * @param {{ id?: string, testid?: string, min?: number, max?: number, step?: number,
 *          onInput?: (v: string) => void, onChange?: (v: string) => void }} opts
 * @returns {Field}
 */
export function sliderField(opts = {}) {
  const min = opts.min !== undefined ? opts.min : 0;
  const max = opts.max !== undefined ? opts.max : 100;
  const step = opts.step !== undefined ? opts.step : 1;
  const range = h("input", { type: "range", min, max, step, class: "slider-range" });
  const num = h("input", { type: "number", min, max, step, class: "slider-num" });
  if (opts.id) num.id = opts.id;
  if (opts.testid) num.setAttribute("data-testid", opts.testid);
  const sync = (v, fire) => {
    const clamped = Math.min(max, Math.max(min, Number(v)));
    if (Number.isNaN(clamped)) return;
    range.value = String(clamped);
    num.value = String(clamped);
    if (fire) fire(clamped);
  };
  range.addEventListener("input", () => sync(range.value, (v) => opts.onInput && opts.onInput(String(v))));
  range.addEventListener("change", () => opts.onChange && opts.onChange(range.value));
  num.addEventListener("input", () => opts.onInput && opts.onInput(num.value));
  num.addEventListener("change", () => sync(num.value || min, (v) => opts.onChange && opts.onChange(String(v))));
  const wrap = h("span", { class: "slider-wrap" }, [range, num]);
  return {
    root: wrap, input: num,
    get: () => num.value,
    set(v) {
      if (v === undefined || v === null || v === "") { num.value = ""; return; }
      sync(v, null);
    },
  };
}

/**
 * 颜色字段(册四 T4.7 文本组):color picker + 文本框双联(#RRGGBB;schema pattern 同构)。
 * 缺省值由调用方经 set() 下发(投影现值/服务端默认),工厂不持任何硬编码色(R5 纪律)。
 * @param {{ id?: string, testid?: string, onChange?: (v: string) => void }} opts
 * @returns {Field}
 */
export function colorField(opts = {}) {
  const picker = h("input", { type: "color", class: "color-pick", "aria-label": opts.ariaLabel || "颜色" });
  const text = h("input", { type: "text", class: "color-text", placeholder: "#RRGGBB", spellcheck: "false" });
  if (opts.id) text.id = opts.id;
  if (opts.testid) text.setAttribute("data-testid", opts.testid);
  const norm = (v) => {
    const s = String(v || "").trim().toLowerCase();
    return /^#[0-9a-f]{6}$/.test(s) ? s : (/^#[0-9a-f]{3}$/.test(s)
      ? `#${s[1]}${s[1]}${s[2]}${s[2]}${s[3]}${s[3]}` : null);
  };
  picker.addEventListener("input", () => {
    text.value = picker.value;
    if (opts.onChange) opts.onChange(picker.value);
  });
  text.addEventListener("input", () => opts.onChange && opts.onChange(text.value));
  text.addEventListener("change", () => {
    const n = norm(text.value);
    if (n) { text.value = n; picker.value = n; if (opts.onChange) opts.onChange(n); }
  });
  const wrap = h("span", { class: "color-wrap" }, [picker, text]);
  return {
    root: wrap, input: text,
    get: () => text.value,
    set(v) {
      const n = norm(v);
      text.value = n || String(v || "");
      if (n) picker.value = n; // picker 只认全写色;无效值时不动(文本框保真,不持硬编码色)
    },
  };
}

/**
 * 开关(checkbox)。
 * @param {{ id?: string, testid?: string, label: string, checked?: boolean, onChange?: (v: boolean) => void }} opts
 * @returns {{ root: HTMLElement, input: HTMLInputElement, set: (v: boolean) => void, get: () => boolean }}
 */
export function toggleField(opts = {}) {
  const input = h("input", {
    type: "checkbox", id: opts.id || null, testid: opts.testid || null,
    checked: opts.checked ? true : null,
  });
  input.addEventListener("change", () => opts.onChange && opts.onChange(input.checked));
  const root = h("label", { class: "toggle" }, [input, ` ${opts.label}`]);
  return {
    root, input,
    get: () => input.checked,
    set(v) { input.checked = Boolean(v); },
  };
}

/**
 * 折叠分组(fieldset + legend 点击开合;open 缺省展开)。
 * T3.2 微交互:展开入场 160ms(cf-expand-in;折叠即时,不演 layout 动画)。
 * @param {string} legend
 * @param {HTMLElement[]} fields
 * @param {{ open?: boolean, testid?: string }} [opts]
 * @returns {HTMLElement}
 */
export function collapseGroup(legend, fields, opts = {}) {
  const wrap = h("div", { class: "insp-fields-wrap" }, fields);
  const box = h("fieldset", { class: "insp-group", testid: opts.testid || null }, [
    h("legend", {
      onclick: () => {
        const expanding = box.classList.contains("collapsed");
        box.classList.toggle("collapsed");
        if (expanding) {
          wrap.classList.add("cf-anim-expand");
          wrap.addEventListener("animationend", () => wrap.classList.remove("cf-anim-expand"), { once: true });
        }
      },
      "data-tip": "点击折叠/展开",
    }, [legend]),
    wrap,
  ]);
  if (opts.open === false) box.classList.add("collapsed");
  return box;
}
