/* 特效库面板 + 特效栈编辑器(册四 T4.6 FE2)。
 *
 * - 面板(tab):11 项特效分类网格(缩略 /assets/transitions/fx.*.jpg 懒加载),
 *   选中片段后点击挂载 → clip_update patch.fx(combo 追加,≤3 顺序即应用,单 Op);
 * - 检查器「特效」组附加面(buildFxStackHost):已挂 combo 列表——参数滑杆(schema
 *   从 /catalogs 渲染,松手一次 commit 整对象)、上移/下移/移除(各一笔 Op);
 * - 越界前端钳制 + 提示;combo 上限 3 由前端拦截(后端 schema maxItems 同界)。
 * 诚实口径:预览代理不含特效最终效果(canvas 代理,面板 hint 标注);未注册 fxId
 * 渲染端逐项降级 WARN。 */
import { h, clear } from "../ui/dom.js";
import { timelineStore, selectionStore } from "../core/store.js";
import { updateClip } from "../core/commands.js";
import {
  ensureCatalogs, fxList, fxDefaultParams, fxClampParam, thumbUrlFor,
} from "../core/catalogs.js";
import { toast } from "../ui/toast.js";
import { sliderField } from "../ui/controls.js";

const COMBO_MAX = 3;

/** 当前选中片段行。 */
function selectedRow() {
  const id = selectionStore.get().clipId;
  return id ? timelineStore.get().clips.find((c) => c.id === id) || null : null;
}

/** 读选中片段 combo(投影只读)。 */
function comboOf(row) {
  return (row && row.fx && Array.isArray(row.fx.combo)) ? row.fx.combo : [];
}

/** 挂载一项特效到选中片段(combo 追加;≤3;单 Op)。 */
function attachFx(fxId) {
  const row = selectedRow();
  if (!row) { toast("先选中片段再挂特效(点时间线片段)", false); return; }
  const combo = comboOf(row).map((e) => ({ fx: e.fx, params: { ...(e.params || {}) } }));
  if (combo.length >= COMBO_MAX) {
    toast(`特效栈已满(${COMBO_MAX}/3):先在检查器移除一项再挂`, false);
    return;
  }
  combo.push({ fx: fxId, params: fxDefaultParams(fxId) });
  updateClip(row.id, { fx: { combo } }, `已挂特效 ${fxId}(${combo.length}/${COMBO_MAX},可撤销)`);
}

/** 提交整组 combo(整对象替换单 Op)。 */
function commitCombo(row, combo, msg) {
  const patch = combo.length ? { fx: { combo } } : { fx: { combo: [] } };
  updateClip(row.id, patch, msg || "特效栈已更新(可撤销)");
}

/* ---------------- 特效库面板(tab) ---------------- */

export function mountFxPanel(container) {
  container.appendChild(h("h3", null, ["特效库(挂到选中片段;combo ≤3 顺序即应用)"]));
  const status = h("div", { class: "hint", testid: "fx-status" }, ["目录加载中…"]);
  container.appendChild(status);
  container.appendChild(h("div", { class: "hint" }, [
    "点击卡片 = 挂到当前选中片段(默认参数,可在检查器·特效组调参/排序/移除)。",
    "预览代理不含特效最终效果;成片效果以导出/精确预览为准。",
  ]));
  const grid = h("div", { class: "lib-grid", testid: "fx-grid" });
  container.appendChild(grid);
  const io = new IntersectionObserver((entries) => {
    for (const en of entries) {
      if (!en.isIntersecting) continue;
      const img = en.target.querySelector("img");
      if (img && !img.src) img.src = img.dataset.src;
      io.unobserve(en.target);
    }
  }, { rootMargin: "120px" });
  ensureCatalogs().then((cat) => {
    clear(grid);
    if (!cat) {
      status.textContent = "目录下发失败(/catalogs):特效库不可用(刷新页面重试)";
      return;
    }
    status.textContent = "";
    let lastCat = "";
    for (const fx of fxList()) {
      if (fx.category !== lastCat) {
        lastCat = fx.category;
        grid.appendChild(h("div", { class: "lib-cat" }, [fx.category]));
      }
      grid.appendChild(fxCard(fx, io));
    }
  });
}

function fxCard(fx, io) {
  const card = h("div", {
    class: "lib-card", testid: "fx-card", dataset: { fx: fx.id },
    title: `${fx.id} · ${fx.desc || ""}`, tabindex: "0", role: "button",
    "aria-label": `挂载特效 ${fx.name}`,
  }, [
    h("span", { class: "lib-thumb" }, [
      h("img", { alt: fx.name, "data-src": thumbUrlFor(fx.id), loading: "lazy" }),
    ]),
    h("span", { class: "lib-name" }, [fx.name]),
    h("span", { class: "lib-desc" }, [fx.desc || ""]),
  ]);
  const act = () => attachFx(fx.id);
  card.addEventListener("click", act);
  card.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); act(); } });
  io.observe(card);
  return card;
}

/* ---------------- 检查器「特效」组附加面(combo 栈编辑) ---------------- */

/**
 * 构建特效栈编辑器宿主(inspector 附加;__cfRefresh(row) 随选中刷新)。
 * @param {() => Object|null} rowOf
 */
export function buildFxStackHost(rowOf) {
  const host = h("div", { class: "fxstack-host", testid: "fx-stack" });
  function refresh(row) {
    clear(host);
    if (!row) { host.appendChild(h("div", { class: "hint" }, ["未选中片段"])); return; }
    const combo = comboOf(row);
    if (!combo.length) {
      host.appendChild(h("div", { class: "hint", testid: "fx-stack-empty" }, [
        "未挂特效:到「特效」页签点卡片挂载(≤3)。",
      ]));
      return;
    }
    host.appendChild(h("div", { class: "hint" }, [
      `特效栈 ${combo.length}/${COMBO_MAX}(顺序即应用顺序;调参松手即一笔 Op)`,
    ]));
    combo.forEach((entry, i) => {
      const schema = fxList().find((f) => f.id === entry.fx);
      const box = h("div", { class: "fxstack-item", testid: "fx-stack-item" });
      const head = h("div", { class: "fxstack-head" }, [
        h("b", null, [`${i + 1}. ${(schema && schema.name) || entry.fx}`]),
        h("button", {
          class: "mini", title: "上移(与前一项交换,单 Op)", "aria-label": `上移 ${entry.fx}`,
          disabled: i === 0 ? true : null,
          onclick: () => {
            const cur = comboOf(rowOf() || row).map((e) => ({ fx: e.fx, params: { ...(e.params || {}) } }));
            if (i === 0) return;
            [cur[i - 1], cur[i]] = [cur[i], cur[i - 1]];
            commitCombo(row, cur);
          },
        }, ["↑"]),
        h("button", {
          class: "mini", title: "下移(与后一项交换,单 Op)", "aria-label": `下移 ${entry.fx}`,
          onclick: () => {
            const cur = comboOf(rowOf() || row).map((e) => ({ fx: e.fx, params: { ...(e.params || {}) } }));
            if (i >= cur.length - 1) return;
            [cur[i], cur[i + 1]] = [cur[i + 1], cur[i]];
            commitCombo(row, cur);
          },
        }, ["↓"]),
        h("button", {
          class: "mini", title: "从栈中移除(整对象替换单 Op)", "aria-label": `移除 ${entry.fx}`,
          onclick: () => {
            const cur = comboOf(rowOf() || row).filter((_, j) => j !== i);
            commitCombo(row, cur, `已移除 ${entry.fx}(可撤销)`);
          },
        }, ["✕"]),
      ]);
      box.appendChild(head);
      if (!schema) {
        box.appendChild(h("div", { class: "hint" }, ["未注册 fxId:渲染端将降级并 WARN(移除可清理)"]));
      }
      for (const p of (schema && schema.params) || []) {
        const cur = (entry.params || {})[p.name];
        const def = p.default !== undefined ? p.default : p.min;
        const field = sliderField({
          min: p.min, max: p.max, step: p.max - p.min <= 2 ? 0.05 : (p.max - p.min <= 20 ? 0.5 : 1),
          testid: `fx-param-${String(p.name).replace(/[^a-z0-9]/gi, "-")}`,
          onChange: (v) => {
            const rowNow = rowOf();
            if (!rowNow) return;
            const { value, clamped } = fxClampParam(entry.fx, p.name, Number(v));
            if (clamped) toast(`${p.name} 越界,已钳制到 ${value}(后端 schema 同界)`, false);
            const cNow = comboOf(rowNow).map((e) => ({ fx: e.fx, params: { ...(e.params || {}) } }));
            if (!cNow[i] || cNow[i].fx !== entry.fx) return; // 栈已变,丢弃过期编辑
            cNow[i].params = { ...cNow[i].params, [p.name]: value };
            commitCombo(rowNow, cNow);
          },
        });
        field.set(cur !== undefined ? cur : def);
        box.appendChild(h("label", { class: "v2-field" }, [`${p.name}`, field.root]));
      }
      host.appendChild(box);
    });
  }
  host.__cfRefresh = refresh;
  // Op 落盘后随投影刷新(读回桥生效口;选中不变时特效栈也要跟上)。
  // 本编辑器自持草稿、松手才 commit:拖拽中(input)不产 Op 不重建,松手后重建无感。
  // 订阅随宿主建(检查器仅 ui-fields 重建时重建宿主,频次≈会话级,无累积风险)。
  timelineStore.subscribe(() => refresh(rowOf()));
  return host;
}
