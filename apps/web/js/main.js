/* CutForge 编辑器壳 · 引导与装配(T2.1;≤100 行纪律)。
 * 单向流:手势 → commands → api → projector → store → 增量渲染。
 * 旧 id 兼容红线见 TESTIDS.md;壳纯度:投影只读 + ephemeral.*(ADR-0013)。 */
import { setToken, setSessionRoot, onAuthFail, dataGet } from "./core/api.js";
import { projectStore, timelineStore, selectionStore, uiStore, ephemeralStore } from "./core/store.js";
import { reproject, refreshConflicts, refreshUiFields, applySession, selfTestRebuild, invalidateWorkspace } from "./core/projector.js";
import * as commands from "./core/commands.js";
import { startEvents, subscribe } from "./core/event-bus.js";
import { toast } from "./ui/toast.js";
import { mountBanner } from "./ui/banner.js";
import { mountTooltip } from "./ui/tooltip.js";
import { installEditorShortcuts } from "./ui/keymap.js";
import { ensureIcons } from "../assets/icons.js";
import { mountTimeline, renderTimelineView, updateSelectionView } from "./render/timeline-view.js";
import { mountRuler } from "./render/ruler.js";
import { mountPlayhead } from "./render/playhead.js";
import { mountMediaPool } from "./render/media-pool.js";
import { mountPreviewLoop, wake } from "./render/preview-loop.js";
import { mountRulerGestures, mountTimelineZoom } from "./render/gestures.js";
import * as preview from "./panels/preview.js";
import * as mediaPanel from "./panels/media-panel.js";
import * as inspector from "./panels/inspector.js";
import * as bgm from "./panels/bgm.js";
import * as expanel from "./panels/export.js";
import * as history from "./panels/history.js";
import * as notes from "./panels/notes.js";
import * as diff from "./panels/diff.js";
import * as conflicts from "./panels/conflicts.js";
import * as transitions from "./panels/transitions.js";
import * as fxlib from "./panels/fxlib.js";
import * as subtitles from "./panels/subtitles.js";
import * as textool from "./panels/textool.js";
import * as mixer from "./panels/mixer.js";
import * as multicam from "./panels/multicam.js";
import * as scenetool from "./panels/scenetool.js";
import * as queue from "./panels/queue.js";
import { mountPreviewTransform } from "./render/preview-transform.js";
import { mountScopes } from "./panels/scopes.js";
import { mountCompare } from "./panels/compare.js";
import { ensureCatalogs } from "./core/catalogs.js";
import { openWizard } from "./panels/wizard.js";
import { mountOnboarding } from "./ui/onboarding.js";
import { mountWave3Wiring, measuredSwitchTab } from "./ui/wire-wave3.js";
import { recordBoot } from "./ui/perf.js";
import { $ } from "./ui/dom.js";

async function boot() {
  const tBoot = performance.now();
  const urlToken = new URLSearchParams(location.search).get("token") || "";
  setToken(urlToken);
  ensureIcons(); mountTooltip(); mountBanner();
  mountRuler(); mountPlayhead(); mountTimeline();
  mountRulerGestures((ms) => commands.seekTo(ms));
  mountTimelineZoom();
  preview.mount($("preview")); mediaPanel.mount($("media-panel")); inspector.mount($("inspector"));
  bgm.mount($("bgm-panel")); expanel.mount($("export"));
  history.mount($("tab-history")); notes.mount($("tab-notes")); diff.mount($("tab-diff")); conflicts.mount($("tab-conflicts"));
  transitions.mount($("tab-transitions")); fxlib.mountFxPanel($("tab-fx")); subtitles.mount($("tab-subtitles"));
  // 册五 A5-FE2 三页签:混音台(T5.3)/ 多机位+场景检测(T5.4)/ 渲染队列(T5.6)
  mixer.mount($("tab-mixer"));
  multicam.mount($("tab-multicam")); scenetool.mount($("tab-multicam"));
  queue.mount($("tab-queue"));
  mountPreviewTransform(); // 画布变换把手层(缩放/旋转/文本拖位置;T4.9)
  mountScopes(); // 示波器面板(T5.2:入口按钮挂预览传输行;开启才采样)
  mountCompare(); // A/B 分屏对比(T5.2 登记项:基准帧快照 vs 当前帧,拖割线)
  textool.mountToolbarButton(); // 工具栏「T 文本」按钮(键位 T 见 keymap)
  mountMediaPool(); // 宿主 #pv-media 由 preview 面板提供,此处只做绑定校验
  mountPreviewLoop(); // 媒体池对齐 + 预览循环(播放解耦核心)
  bindChrome(); installEditorShortcuts(); wireEvents();
  mountWave3Wiring(); // A2 遗留接线(net 横幅/conn-badge)+ 面板开合订阅
  mountOnboarding();  // 首启引导条(T3.7;localStorage 记忆)

  // 会话握手(/session 数据面,api 层白名单收口):失败 → token 横幅(旧壳口径,壳停摆)
  const sess = await dataGet("/session");
  if (!sess || !sess.root) {
    uiStore.set({ tokenBanner: "⚠ token 已变更或缺失(服务重启会换新 token):请回到服务窗口复制最新链接重新打开本页。" });
    toast("token 鉴权失败", false);
    return;
  }
  if (sess.token && sess.token !== urlToken) {
    uiStore.set({ tokenBanner: "⚠ 本页 token 与服务当前 token 不一致,请改用服务窗口打印的最新链接。" });
  }
  setSessionRoot(sess.root);
  applySession(sess);
  $("session-info").textContent = `root=${sess.root}`;
  await refreshUiFields();   // 检查器真相源先行(旧壳顺序)
  await reproject();         // 投影 → store → 增量渲染(#rev 翻牌,e2e 就绪锚点)
  await refreshConflicts();
  await mediaPanel.initialMediaBrowse();
  ensureCatalogs();          // 转场/特效/花字目录预热(面板各自也会 ensure,幂等)
  startEvents(sess.token);
  window.__cutforgeSelfTest = selfTestRebuild; // T2.2 重建铁律自测入口(TESTIDS.md §五)
  recordBoot(performance.now() - tBoot); // T3.5 首屏可交互预算(投影+素材首览完成)
}
function wireEvents() {
  subscribe("workspace.changed", () => { invalidateWorkspace(); wake(); });
  subscribe("notes.changed", () => { if (uiStore.get().tab === "notes") notes.refresh(); });
  subscribe("cutlist.changed", () => toast("cutlist.json 已被外部更新"));
  subscribe("resync", () => invalidateWorkspace());
  onAuthFail(() => toast("数据面鉴权失败:token 已变更,请用最新链接重开", false));
  // #rev 翻牌(e2e 就绪锚点:初始 "-",投影到达后与服务端 rev 一致)+ 落盘脉冲点
  let revPulseTimer = 0;
  projectStore.subscribe((patch, st) => {
    if (patch.rev !== undefined) {
      $("rev").textContent = String(st.rev ?? "-");
      const dot = $("rev-dot"); // 保存脉冲(T3.2):绿点一次性放大淡出 = 已保存
      if (dot) {
        dot.classList.remove("pulse");
        void dot.offsetWidth;
        dot.classList.add("pulse");
        clearTimeout(revPulseTimer);
        revPulseTimer = setTimeout(() => dot.classList.remove("pulse"), 300);
      }
    }
  });
  timelineStore.subscribe(() => renderTimelineView());
  selectionStore.subscribe(updateSelectionView);
  selectionStore.subscribe(() => renderTimelineView());
  ephemeralStore.subscribe(() => renderTimelineView());
  uiStore.subscribe((patch) => { if (patch.tab !== undefined) switchTab(patch.tab); });
}

function bindChrome() {
  for (const b of document.querySelectorAll("#tabs button")) {
    b.addEventListener("click", () => uiStore.set({ tab: b.dataset.tab }));
  }
  $("btn-project-new").addEventListener("click", openWizard);
  $("btn-undo").addEventListener("click", () => commands.undo());
  $("btn-redo").addEventListener("click", () => commands.redo());
  $("pv-to-start").addEventListener("click", () => commands.toStart());
  $("pv-to-end").addEventListener("click", () => commands.toEnd());
  $("pv-play").addEventListener("click", () => commands.togglePlay());
  $("magnet").addEventListener("change", (e) => uiStore.set({ magnet: e.target.checked }));
  $("ripple").addEventListener("change", (e) => uiStore.set({ ripple: e.target.checked }));
}

/** 页签切换(T3.5:计时进性能面板;T3.2 既有入场动效不变)。 */
const switchTab = measuredSwitchTab(switchTabNow);

function switchTabNow(tab) {
  document.querySelectorAll("#tabs button").forEach((x) => x.classList.toggle("active", x.dataset.tab === tab));
  document.querySelectorAll(".tab").forEach((x) => x.classList.toggle("active", x.id === `tab-${tab}`));
  // 回时间线页签必重渲一次:隐藏期(clientWidth=0)的投影变化没有 scroll/resize 事件
  // 可消费,不重渲会呈现过期片段面(增量渲染只碰变化节点,开销可忽略)。
  if (tab === "timeline") renderTimelineView();
  if (tab === "history") history.refresh();
  if (tab === "notes") notes.refresh();
  if (tab === "diff") diff.refresh();
  if (tab === "conflicts") conflicts.refresh();
  if (tab === "subtitles") subtitles.refresh();
  if (tab === "mixer") mixer.renderTracks();
  if (tab === "queue") queue.refresh();
}
boot();
