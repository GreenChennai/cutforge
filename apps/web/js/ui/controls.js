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
 * @param {{ id?: string, testid?: string, placeholder?: string, onInput?: (v: string) => void,
 *          onEnter?: () => void }} opts
 * @returns {Field}
 */
export function textField(opts = {}) {
  const input = h("input", {
    type: "text", id: opts.id || null, testid: opts.testid || null,
    placeholder: opts.placeholder || null,
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
 * @param {string} legend
 * @param {HTMLElement[]} fields
 * @param {{ open?: boolean, testid?: string }} [opts]
 * @returns {HTMLElement}
 */
export function collapseGroup(legend, fields, opts = {}) {
  const wrap = h("div", { class: "insp-fields-wrap" }, fields);
  const box = h("fieldset", { class: "insp-group", testid: opts.testid || null }, [
    h("legend", {
      onclick: () => box.classList.toggle("collapsed"),
      "data-tip": "点击折叠/展开",
    }, [legend]),
    wrap,
  ]);
  if (opts.open === false) box.classList.add("collapsed");
  return box;
}
