/* 首启引导条(T3.7):可关,localStorage 记忆(不再自动出现)。
 * 三步文案与新手全流程一一对应:①插入素材 ②剪切 ③导出;顺带指出 ? 帮助。
 * 结构性提示(条 + 步骤编号),不依赖颜色单线索;a11y:role=note + 关闭按钮可聚焦。 */
import { h } from "./dom.js";
import { pref, setPref } from "./prefs.js";

export function mountOnboarding() {
  if (pref("onboardDismissed", false)) return;
  const bar = h("div", {
    class: "onboard-bar", testid: "onboard-bar", role: "note", "aria-label": "新手引导",
  }, [
    h("span", { class: "onboard-step" }, ["① 左侧素材面板「双击」插入时间线"]),
    h("span", { class: "onboard-step" }, ["② 选中片段:S 分割 / Del 删除 / 拖动移动"]),
    h("span", { class: "onboard-step" }, ["③ 右下「导出成片」出片"]),
    h("span", { class: "onboard-tip" }, ["按 ? 看全部快捷键"]),
    h("button", {
      class: "onboard-dismiss", testid: "onboard-dismiss",
      title: "不再显示(存本机浏览器)",
      onclick: () => {
        setPref("onboardDismissed", true);
        bar.remove();
      },
    }, ["知道了"]),
  ]);
  const header = document.querySelector('[data-testid="topbar"]');
  if (header && header.parentElement) {
    header.parentElement.insertBefore(bar, header.nextSibling);
  } else {
    document.body.prepend(bar);
  }
}
