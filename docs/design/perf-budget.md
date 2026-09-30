# 性能与丝滑预算(册三 T3.5)· 口径与测量点

> 纪律:预算必须「可视、可测」。本文是预算的口径真相源;壳内 `Shift+D` 性能面板
> (js/ui/perf-panel.js + js/ui/perf.js)按本表逐行可视「当前值 + 是否达标」;
> e2e 侧由 `tools/e2e_perf_timeline.py` 等按同一口径断言。两边数据同源不同面。

## 一、预算表

| # | 预算 | 目标 | 壳内测量点(实时) | e2e 断言点 | 面板行 id |
|---|---|---|---|---|---|
| 1 | 交互响应(投影收敛) | ≤16ms 感知口径;服务端口径见行 1 备注 | 最近一次全量重投影耗时(`projector.lastProjectionTime`,含 project_get+timeline_get 双 RPC) | e2e_edit_ops / ui_smoke 的 rev 收敛等待 | `perf-budget-interact` |
| 2 | 播放帧率 | 60fps(门禁放行 ≥55) | 性能面板 500ms 滑窗 rAF 计数(`perf.js metrics.fpsNow`) | e2e_playback_survival(≈60fps) | `perf-budget-playfps` |
| 3 | 1k 片段滚动 | 60fps(P95 ≥55) | 面板标注「e2e 实测」(壳不在滚动路径上自采样,避免自证偏差) | e2e_perf_timeline `--min-fps 55`(实测 P95 60.2) | `perf-budget-scrollfps` |
| 4 | 首屏可交互 | <1s | boot → 投影到达 + 素材首览完成(`main.js` recordBoot) | —(面板可视) | `perf-budget-boot` |
| 5 | 页签切换 | <100ms | `main.switchTab` 计时(`wire-wave3.measuredSwitchTab`) | —(面板可视) | `perf-budget-tab` |
| 6 | 导出进度回调 | ≥2Hz | 导出期 onProgress 时间戳滑窗(`export.js` recordExportTick) | —(导出期面板可视) | `perf-budget-export` |

行 1 备注:「交互 ≤16ms」的严格口径是单帧内完成输入→绘制;本壳交互经服务端
单向流(rev 收敛),面板如实显示**最近投影耗时**,超出 16ms 即如实标 ✗
(投影耗时主要受服务端 bench 口径影响,属诚实呈现而非前端掉帧)。

## 二、面板读法(dev 性能面板,Shift+D;默认关)

- 上半 `perf-stats`:帧率(500ms 窗)/ rAF 间隔分布(≤17ms、≤34、≤50、>50 四桶)/
  DOM 节点数 / 未完成请求数(`api.netInflight`)/ 最近一次投影耗时 / 媒体元素池占用;
- 下半 `perf-budgets`:预算表逐行「✓ 达标 / ✗ 超标 / — 非实时项」,每行
  `data-pass="1|0|na"`(e2e 可断言,非颜色单线索);
- 采样纪律:面板打开才起采样 rAF(默认零常态开销)。

## 三、媒体元素池上限(T3.5 有界化)

- 上限常量:`POOL_MAX = 24`(`js/render/media-pool.js`,常量即预算,改动需过本文件);
- 淘汰策略:LRU(usedAt 最老者优先),播放头覆盖窗口 [t−2s, t+8s] 内的行锚定不淘;
  被淘的行若再次被播放头覆盖,`preview-loop.syncMedia` 即时补建(播放连续性优先);
- 可视:面板 `perf-stat-pool` 行显示 `占用/上限(累计淘汰 N)`;
- 4 小时长跑内存为人工项(册三台账 A3-L1 登记):以 200 轮导航模拟 + 池有界替代断言,
  结果落 `docs/bench/perf-a3.json`(池峰值 24,淘汰 6768)。

## 四、导出进度 Hz 现状(行 6 备注,册三收官)

- 预算 ≥2Hz 为目标口径;壳侧实现为纯轮询(`js/core/render-commands.js`
  setTimeout **500ms**),轮询间隔已按 A3-L3 处置收口(800→500ms 一行改),
  `e2e_perf_budget` 复测面板行「✓ 导出进度回调 2.4Hz ≥2Hz」pass=1,
  **达标闭合(A3-L3 已于 A3 期收口)**;结果落盘 `docs/bench/perf-a3.json`
  (按修复后重生成)。

## 五、册四增补:千素材媒体池(T4.1 口径,e2e 断言面)

- 预算:1000 素材目录滚动 **P95 ≥55fps**(册四收官实测 63.3);素材清单
  `BROWSE_CAP=500` 如实截断(不静默吞);缩略图**懒加载**——视口外零请求;
- 测量入口:`tools/e2e_media_perf.py --min-fps 55`(负载敏感,本机安静时段跑,
  不进 CI;同 §三 长跑替代口径纪律);
- 面板无关:该预算走 e2e 断言,不入 Shift+D 预算表(面板行面维持上文六行口径,
  不为一次性预算加常驻行)。
