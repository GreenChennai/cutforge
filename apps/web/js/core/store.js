/* 状态层(T2.2):发布-订阅 store 工厂 + 六 store 单例 + ephemeral store(ADR-0013)。
 *
 * - 每次写入产生 patch 记录(供调试与测试断言;patches() 可取最近 200 条);
 * - transact(fn) 事务批次:fn 内多次 set 聚合为一次通知(patch 合并下发);
 * - 一致性铁律:project/timeline/media/ui 四个"内核派生面"必须可由投影完全重建
 *   (reset() 清空 → projector.rebuildFromProjection() 重建;自测入口挂 window);
 * - ephemeral store 与投影 store 物理分离,patch 键强制 `ephemeral.` 前缀,
 *   满足「不进 IR / 不落盘 / 不参与撤销」三原则(ADR-0013)。
 *
 * @typedef {{ store: string, t: number, patch: Object<string, *> }} StorePatch
 */

/** @type {StorePatch[]} */
const GLOBAL_LOG = [];

/**
 * @param {string} name
 * @param {Object<string, *>} initState
 * @param {{ ephemeral?: boolean }} [opts]
 */
export function createStore(name, initState, opts = {}) {
  /** @type {Object<string, *>} */
  let state = { ...initState };
  /** @type {Set<(patch: Object) => void>} */
  const subs = new Set();
  let txDepth = 0;
  /** @type {Object<string, *>} */
  let txMerged = null;

  const record = (patch) => {
    const stamped = opts.ephemeral
      ? Object.fromEntries(Object.keys(patch).map((k) => [`ephemeral.${k}`, patch[k]]))
      : patch;
    GLOBAL_LOG.push({ store: name, t: Date.now(), patch: stamped });
    if (GLOBAL_LOG.length > 200) GLOBAL_LOG.shift();
  };

  const notify = (patch) => { for (const fn of subs) fn(patch, state); };

  return {
    name,
    /** 当前快照(只读约定:调用方不得改写返回对象)。 */
    get: () => state,
    /** 浅合并写入;逐键比对,无变化不通知。返回实际生效的 patch。 */
    set(patch) {
      const eff = {};
      for (const k of Object.keys(patch)) {
        if (!Object.is(state[k], patch[k])) eff[k] = patch[k];
      }
      if (!Object.keys(eff).length) return eff;
      state = { ...state, ...eff };
      record(eff);
      if (txDepth > 0) {
        txMerged = { ...(txMerged || {}), ...eff };
      } else {
        notify(eff);
      }
      return eff;
    },
    /** 事务批次:fn 内的全部 set 聚合为一次通知(视图只渲染一轮)。 */
    transact(fn) {
      txDepth += 1;
      try {
        fn();
      } finally {
        txDepth -= 1;
        if (txDepth === 0 && txMerged) {
          const merged = txMerged;
          txMerged = null;
          notify(merged);
        }
      }
    },
    subscribe(fn) {
      subs.add(fn);
      return () => subs.delete(fn);
    },
    /** 订阅并立即收到一次回调(装配期初始化用)。 */
    subscribeNow(fn) {
      const off = this.subscribe(fn);
      fn({}, state);
      return off;
    },
    /** 清空回初始态(重建自测用;会以全量 patch 通知订阅者)。 */
    reset(next) {
      state = { ...initState, ...(next || {}) };
      record({ __reset__: true, ...state });
      notify({ __reset__: true });
    },
    /** 最近 patch 记录(调试/断言)。 */
    patches: () => GLOBAL_LOG.filter((p) => p.store === name).slice(),
  };
}

/* ---------------- 六 store(T2.2)---------------- */

/** 工程:会话根/token/工程投影/rev(内核投影派生面,可重建)。 */
export const projectStore = createStore("project", {
  root: "", token: "", projectRel: "", project: null, rev: null, loaded: false,
});

/** 时间线:轨道/片段投影(内核投影派生面,可重建;壳只读,几何零手算)。 */
export const timelineStore = createStore("timeline", {
  clips: [], rev: null,
});

/** 选择:选中集/播放头/入出点(纯客户端会话态;播放头不是内核状态)。
 * clipId 是主选中(锚点,兼容单选语义);clipIds 是框选/加选的扩展集(T3.3),
 * 两者都只影响视图高亮与作用域命令,不进投影。 */
export const selectionStore = createStore("selection", {
  clipId: null, clipIds: [], playheadMs: 0, inMs: null, outMs: null,
});

/** 播放:播放态/速度(rAF 时钟的权威持有者是 render/preview-loop)。 */
export const playbackStore = createStore("playback", {
  playing: false, speed: 1,
});

/** 素材:素材池列表/浏览路径(投影派生面,可重建)。 */
export const mediaStore = createStore("media", {
  dir: "01_原始素材", files: [], total: 0, truncated: false, error: "",
});

/** UI:页签/开关/横幅计数/检查器字段真相源(派生面可重建;开关为会话态)。
 * connBanner:断连/网络错误横幅文本(T3.4 接线:api 层 net 事件写入,恢复自动收起)。
 * connBadge:事件通道连接态徽标(A2 遗留接线:testid=conn-badge)。
 * blade:切割模式(B 键;会话开关,光标变刀,点击片段即分割)。
 * tool:工具模式(T4.2:A 选择/B 刀/T 裁剪;blade 字段由 tool 派生兼容旧读点)。
 * panelsOpen:侧面板开合(Tab 键;视图态)。 */
export const uiStore = createStore("ui", {
  tab: "timeline",
  magnet: true,
  ripple: false,
  tokenBanner: "",
  connBanner: "",
  connBadge: "连接中…",
  connBadgeState: "init",
  blade: false,
  tool: "select",
  panelsOpen: true,
  conflicts: 0,
  internalErrors: 0,
  uiFields: null,
});

/** 临时投影(ADR-0013):拖拽 ghost/吸附线/轨头视图隐藏/会话标记。物理隔离 + 强制前缀。
 * T4.1/T4.3 增:recentMedia(素材最近使用,会话序)/ historyMarks(历史快照标记)。
 * T4.2 增:trimLineMs(roll/slide 边界拖拽指示线,ephemeral 不落盘)。 */
export const ephemeralStore = createStore("ephemeral", {
  dragGhost: null,   // {clipId, trackId, startMs, durationMs, trim:"l"|"r"|null}
  snapMs: null,      // 吸附指示线位置(ms)
  trimLineMs: null,  // roll/slide 共享边界拖拽指示线(ms)
  hiddenTracks: [],  // 眼睛开关(视图隐藏;不落盘,刷新即回默认)
  markers: [],       // 会话标记 ms 列表(T3.4:M 键;ephemeral 不落盘不进 IR)
  recentMedia: [],   // 最近使用素材 path 序(插入即前插;会话态)
  historyMarks: [],  // 历史面板快照标记 [{rev,label,t}](导出/批量前自动打标)
}, { ephemeral: true });

/** 调试面:全局 patch 记录(测试断言 ephemeral.* 前缀纪律用)。 */
export function allPatches() {
  return GLOBAL_LOG.slice();
}
