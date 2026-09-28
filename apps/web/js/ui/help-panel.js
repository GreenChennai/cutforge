/* 快捷键帮助面板(T3.4):? 唤起;全表 + 分组 + 搜索,条目与键位注册表同源
 * (重绑定后即时反映当前生效组合)。只读面板:重绑定入口在设置(Ctrl+,)。 */
import { h, clear } from "./dom.js";
import { openDialog } from "./dialog.js";
import { exportTable, displayCombo } from "./keymap-registry.js";

/**
 * 打开帮助(testid=help-dialog;e2e 可断言行数/搜索过滤)。
 */
export function openHelpPanel() {
  openDialog({
    id: "help-dialog",
    title: "快捷键(当前生效组合;到设置 Ctrl+, 可重绑定)",
    build: (body) => {
      const search = h("input", {
        type: "text", testid: "help-search",
        placeholder: "搜索:名称 / 按键 / 分组(如:分割、Ctrl、播放)",
      });
      const list = h("div", { class: "help-list", testid: "help-rows" });
      body.appendChild(h("div", { class: "help-search-row" }, [search]));
      body.appendChild(list);
      const empty = h("div", { class: "dim", testid: "help-empty", hidden: true }, ["没有匹配的快捷键"]);
      body.appendChild(empty);
      search.addEventListener("input", () => render(list, empty, search.value.trim().toLowerCase()));
      render(list, empty, "");
      search.focus();
    },
  });
}

function render(list, empty, q) {
  clear(list);
  const rows = exportTable().filter((r) => !q
    || r.label.toLowerCase().includes(q)
    || r.group.toLowerCase().includes(q)
    || displayCombo(r.combo).toLowerCase().includes(q)
    || r.id.toLowerCase().includes(q));
  let lastGroup = "";
  for (const r of rows) {
    if (r.group !== lastGroup) {
      lastGroup = r.group;
      list.appendChild(h("div", { class: "help-group" }, [r.group]));
    }
    list.appendChild(h("div", { class: "help-row", testid: "help-row", "data-keybind-id": r.id }, [
      h("kbd", { class: "help-keys", testid: "help-keys" }, [displayCombo(r.combo)]),
      h("span", { class: "help-label" }, [r.label]),
    ]));
  }
  empty.hidden = rows.length > 0;
}
