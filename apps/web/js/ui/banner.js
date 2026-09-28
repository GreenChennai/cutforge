/* 顶部横幅(E6-2 token / E8 冲突停写 / 契约错误折叠):uiStore 驱动,单一渲染路径。 */
import { uiStore } from "../core/store.js";
import { $ } from "./dom.js";

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
  });
  $("banner").hidden = false;
}
