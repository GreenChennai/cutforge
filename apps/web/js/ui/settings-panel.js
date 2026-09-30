/* 设置面板(T3.4):Ctrl+, 唤起。
 * - 快捷键重绑定:键位捕获控件(按下新组合即捕获)→ 冲突检测(提示冲突项,
 *   可「仍要绑定」强制——被占组合的既有绑定自动停用并提示,或取消);
 * - 恢复默认:清空覆盖表,立即回默认键位;
 * - 通用开关:批量操作前确认(T3.7,缺省开)。
 * 键位表存 localStorage 用户偏好(prefs.js),非工程数据。 */
import { h, clear } from "./dom.js";
import { openDialog } from "./dialog.js";
import { exportTable, comboOf, findConflicts, rebind, resetAll, displayCombo } from "./keymap-registry.js";
import { keyOf } from "./shortcuts.js";
import { confirmBatch, setConfirmBatch, pref, setPref } from "./prefs.js";
import { toast } from "./toast.js";

let listHost = null;
let capturing = null; // { id, onKey, off }

/** 打开设置(testid=settings-dialog)。 */
export function openSettings() {
  openDialog({
    id: "settings-dialog",
    title: "设置(键位表存本机浏览器,不进工程)",
    build: (body) => {
      body.appendChild(h("h4", { class: "set-sub" }, ["快捷键(点击「改」后按新组合;Esc 取消)"]));
      listHost = h("div", { class: "keybind-list", testid: "keybind-list" });
      body.appendChild(listHost);
      body.appendChild(h("div", { class: "set-actions" }, [
        h("button", {
          testid: "keybind-reset",
          onclick: () => {
            stopCapture();
            resetAll();
            renderList();
            toast("键位已恢复默认");
          },
        }, ["恢复默认键位"]),
      ]));
      body.appendChild(h("h4", { class: "set-sub" }, ["通用"]));
      const batch = h("input", {
        type: "checkbox", testid: "setting-confirm-batch",
        onchange: (e) => setConfirmBatch(/** @type {HTMLInputElement} */ (e.target).checked),
      });
      /** @type {HTMLInputElement} */ (batch).checked = confirmBatch();
      body.appendChild(h("label", { class: "toggle" }, [batch, " 批量操作前先确认(如 OpLog 一键撤销)"]));
      // 吸附强度档(T4.2:主开关 = 顶栏「磁吸」;档位决定候选源与半径)
      const strength = h("select", { testid: "setting-snap-strength", "aria-label": "吸附强度" }, [
        h("option", { value: "loose" }, ["松(仅帧网格)"]),
        h("option", { value: "standard" }, ["标准(帧网格 + 片段边缘 + 播放头)"]),
        h("option", { value: "strong" }, ["强(再加会话标记,半径 12px)"]),
      ]);
      strength.value = pref("snapStrength", "standard");
      strength.addEventListener("change", () => setPref("snapStrength", strength.value));
      body.appendChild(h("label", { class: "toggle" }, ["吸附强度 ", strength]));
      body.appendChild(h("div", { class: "dim" }, ["提示:按 ? 随时查看当前生效的全部快捷键。"]));
      renderList();
    },
    onClose: () => stopCapture(),
  });
}

function renderList() {
  clear(listHost);
  for (const r of exportTable()) {
    listHost.appendChild(rowOf(r));
  }
}

function rowOf(r) {
  const row = h("div", { class: "keybind-row", testid: `keybind-row-${r.id}` }, [
    h("kbd", { class: "help-keys", testid: `keybind-combo-${r.id}` }, [displayCombo(comboOf(r.id))]),
    h("span", { class: "keybind-label" }, [`${r.group} · ${r.label}`]),
    h("button", {
      testid: `keybind-capture-${r.id}`,
      onclick: () => startCapture(r.id, row),
    }, ["改"]),
  ]);
  return row;
}

/** 进入捕获态:窗口级 capture 监听先于对话框 Esc 处理,捕获期间接管全部按键。 */
function startCapture(id, row) {
  stopCapture();
  const zone = h("div", { class: "keybind-capture", testid: `keybind-capture-zone-${id}` }, [
    h("span", { class: "dim" }, ["按新组合…(Esc 取消)"]),
  ]);
  row.appendChild(zone);
  const onKey = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      stopCapture();
      renderList();
      return;
    }
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return; // 裸修饰键不成组合
    const combo = keyOf(e);
    if (!combo) return;
    const conflicts = findConflicts(combo, id);
    if (!conflicts.length) {
      apply(id, combo);
      return;
    }
    showConflict(zone, id, combo, conflicts);
  };
  window.addEventListener("keydown", onKey, true);
  capturing = { id, onKey, zone, row };
}

/** 捕获态行内冲突提示:可强制(既有绑定停用)或取消。 */
function showConflict(zone, id, combo, conflicts) {
  clear(zone);
  const names = conflicts.map((cid) => exportTable().find((r) => r.id === cid)?.label || cid).join("、");
  zone.appendChild(h("span", { class: "keybind-conflict", testid: `keybind-conflict-${id}` }, [
    `⚠ 与「${names}」冲突`,
  ]));
  zone.appendChild(h("button", {
    testid: `keybind-force-${id}`,
    onclick: () => apply(id, combo, true),
  }, ["仍要绑定"]));
  zone.appendChild(h("button", { onclick: () => { stopCapture(); renderList(); } }, ["取消"]));
}

function apply(id, combo, force = false) {
  const res = rebind(id, combo, force);
  stopCapture();
  renderList();
  if (res.disabled.length) {
    toast(`已绑定;原属「${res.disabled.join("、")}」的组合已停用`);
  } else {
    toast("键位已更新");
  }
}

function stopCapture() {
  if (!capturing) return;
  window.removeEventListener("keydown", capturing.onKey, true);
  capturing = null;
}
