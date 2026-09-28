/* 顶部横幅(E6-2 token / E8 冲突停写 / v0.6 契约错误折叠 / 断连横幅)。
 * uiStore 驱动,单一渲染路径;入场滑入动画由 css(cf-slide-down-in)承担。
 * connBanner:断连/网络错误横幅——组件与渲染路径就位(T3.2),接线(事件层写
 * uiStore.connBanner)由后续波次完成。 */
import { uiStore } from "../core/store.js";
import { h, $ } from "./dom.js";

let connDotInited = false;

export function mountBanner() {
  uiStore.subscribeNow((patch, st) => {
    if (patch.tokenBanner !== undefined || patch.__reset__) {
      const b = $("token-banner");
      b.hidden = !st.tokenBanner;
      b.textContent = st.tokenBanner || "";
    }
    if (patch.conflicts !== undefined || patch.__reset__) {
      const b = $("conflict-banner");
      b.hidden = st.conflicts === 0;
      if (st.conflicts) {
        b.textContent = `⛔ 存在 ${st.conflicts} 项未裁决冲突,已停写(到「冲突」页查看;裁决后删除 .cutforge/conflicts/*.json)`;
      }
    }
    if (patch.internalErrors !== undefined || patch.__reset__) {
      const b = $("internal-banner");
      const n = st.internalErrors;
      b.hidden = n === 0;
      if (n) {
        b.textContent = `⚠ ${n} 次契约错误(最近见服务端日志)。工程可能是旧版 id/字段,请用 CutFlow rs_ir validate 检查。`;
      }
    }
    if (patch.connBanner !== undefined || patch.__reset__) {
      const b = $("conn-banner");
      b.hidden = !st.connBanner;
      if (st.connBanner && !connDotInited) {
        // 呼吸点只注入一次(文本可反复更新)
        b.appendChild(h("span", { class: "conn-dot", "aria-hidden": "true" }));
        connDotInited = true;
      }
      if (st.connBanner) {
        const dot = b.querySelector(".conn-dot");
        b.textContent = "";
        if (dot) b.appendChild(dot);
        b.appendChild(document.createTextNode(st.connBanner));
      }
    }
    if (patch.connBadge !== undefined || patch.connBadgeState !== undefined || patch.__reset__) {
      // A2 遗留②:连接态徽标(文本即状态,非颜色单线索;testid=conn-badge)
      const badge = $("conn-badge");
      if (badge) {
        badge.textContent = st.connBadge || "";
        badge.dataset.state = st.connBadgeState || "init";
      }
    }
  });
  $("banner").hidden = false;
}
