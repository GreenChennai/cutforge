/* DOM 工具(T2.5):h() 元素工厂 + 查询/清理助手。全部面板建 DOM 走这里,
 * 杜绝 innerHTML 字符串拼接(注入安全 + 便于 keyed 复用)。 */

/** @returns {HTMLElement} */
export function $(id) {
  return /** @type {HTMLElement} */ (document.getElementById(id));
}

/**
 * 元素工厂。attrs 支持:class, id, testid(data-testid), dataset, on{事件表},
 * onclick/oninput/onchange/onkeydown 等驼峰事件键(转 addEventListener),
 * html(仅静态字面量,禁插值), 其余按属性赋值。
 * @param {string} tag
 * @param {Object|null} attrs
 * @param {Array<Node|string>|Node|string} children
 */
export function h(tag, attrs = null, children = null) {
  const el = document.createElement(tag);
  if (attrs) {
    for (const [k, v] of Object.entries(attrs)) {
      if (v === undefined || v === null || v === false) continue;
      if (k === "class") el.className = v;
      else if (k === "testid") el.setAttribute("data-testid", v);
      else if (k === "dataset") for (const [dk, dv] of Object.entries(v)) el.dataset[dk] = dv;
      else if (k === "on") for (const [ev, fn] of Object.entries(v)) el.addEventListener(ev, fn);
      else if (/^on[a-z]+$/.test(k)) el.addEventListener(k.slice(2), v); // onclick → click
      else if (k === "html") el.innerHTML = /** @type {string} */ (v); // 仅限静态字面量调用点
      else if (v === true) el.setAttribute(k, "");
      else el.setAttribute(k, String(v));
    }
    // T3.6:有 data-tip 而无 title 的元素补原生 title(读屏/悬停兜底,提示双通道)
    const tip = el.getAttribute("data-tip");
    if (tip && !el.getAttribute("title")) el.setAttribute("title", tip);
  }
  if (children !== null && children !== undefined) {
    appendChildren(el, children);
  }
  return el;
}

function appendChildren(el, children) {
  const list = Array.isArray(children) ? children : [children];
  for (const c of list) {
    if (c === null || c === undefined || c === false) continue;
    el.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
  }
}

/** 清空子节点(不在时间线主渲染路径上;时间线走 keyed reconciliation)。 */
export function clear(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
}

/** 圆整数字显示。 */
export function fmtMs(ms) {
  return `${Math.round(ms)}ms`;
}
