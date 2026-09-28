/* 册三接线面(T3.4/T3.5/A2 遗留):装配期一次性挂接的横幅/徽标/度量,不进 main 主流程。
 * - A2 遗留①:api 层 TIMEOUT/NETWORK 失败 → uiStore.connBanner(断连横幅已就位),
 *   传输恢复(onNetOk)自动收起;不动既有重试/幂等逻辑;
 * - A2 遗留②:事件通道连接态 → conn-badge 徽标(已连/重连中/降级轮询/退避重试),
 *   文本即状态(非颜色单线索),testid=conn-badge;
 * - T3.5:页签切换耗时记录(switchTab 度量挂勾)+ 侧面板开合(css class 订阅)。
 */
import { onNetDown, onNetOk } from "../core/api.js";
import { onConnState } from "../core/event-bus.js";
import { uiStore } from "../core/store.js";
import { recordTabSwitch } from "./perf.js";
import { $ } from "./dom.js";

const BADGE_TEXT = {
  ok: "已连(SSE)",
  reconnecting: "重连中…",
  polling: "降级轮询",
  retry: "轮询重试…",
};

export function mountWave3Wiring() {
  // A2 遗留①:net 错误横幅(组件/渲染路径 T3.2 已就位,此处只写 store)
  let netDownAt = 0;
  onNetDown((msg) => {
    netDownAt = Date.now();
    uiStore.set({ connBanner: `⛔ 服务连接异常:${msg} —— 恢复后自动收起` });
  });
  onNetOk(() => {
    if (uiStore.get().connBanner) uiStore.set({ connBanner: "" });
  });

  // A2 遗留②:连接态徽标(SSE 30min 流寿命/断连可见)
  onConnState((state) => {
    uiStore.set({
      connBadge: BADGE_TEXT[state] || state,
      connBadgeState: state,
    });
  });

  // 侧面板开合(Tab):workbench class 订阅(视图态,非颜色单线索:面板在/不在)
  uiStore.subscribe((patch, st) => {
    if (patch.panelsOpen !== undefined) {
      $("workbench")?.classList.toggle("panels-collapsed", !st.panelsOpen);
    }
  });
}

/** 页签切换度量挂勾(T3.5 预算「tab 切换 <100ms」):包装 main.switchTab。 */
export function measuredSwitchTab(switchTabFn) {
  return (tab) => {
    const t0 = performance.now();
    switchTabFn(tab);
    recordTabSwitch(performance.now() - t0);
  };
}
