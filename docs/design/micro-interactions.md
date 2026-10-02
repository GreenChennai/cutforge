# 微交互清单(册三 T3.2)· 实现位置登记

> 纪律:C-R1 降噪——动效只用于**状态变化**;时长梯度 fast 80ms / base 160ms / slow 240ms
> (token:`--cf-dur-fast/base/slow`);缓动标准 `cubic-bezier(.2,0,0,1)`(`--cf-ease`),
> 入场 `--cf-ease-enter` / 出场 `--cf-ease-exit`;一律 transform/opacity 合成器路径
> (box-shadow/filter 仅用于小面积状态色,禁 width/left/top/height 动画);
> **拖拽过程零动画(直接跟手)**;同屏并发入场动画 ≤3(全部为 ≤240ms 一次性短动效,
> 循环动画仅两处豁免:导出不确定进度条、断连横幅呼吸点);
> `prefers-reduced-motion` 在 `css/motion.css` 一处总控全量降级(动画/过渡压到 0.01ms)。
> 录屏存档(AC-3.2,record-captures.py 可重录):docs/design/recordings/ 共 45 条 webm(册三 19 条 960×600;册五 15 支/册六 6 支/册七 5 支 720×450 低分辨率,单条 ≤2MB,合计 ≈7.77MB ≤ 8MB 红线);27 项逐行回链见下表「录屏」列,册五/册六/册七回链见第八~十一节;关键项覆盖:拖拽 ghost 跟手+吸附脉冲(02)、非法落点红态(03)、trim 时长气泡(04)、框选多选(07)、右键菜单(14)、模态缩放入场(13)、toast 撤销(15)、断连横幅滑入(18)、首启引导(01)、性能面板(19-perf-panel.webm)。

## 一、拖拽

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 1 | ghost 半透明跟手(复用节点,零动画) | `js/render/timeline-view.js` renderGhost + `js/render/gestures.js` move 路径 + `css/components/timeline.css` `.clip.ghost` | `drag-ghost` | 已实现 | [录屏:recordings/02-drag-ghost-snap.webm](recordings/02-drag-ghost-snap.webm) |
| 2 | 落点阴影 | `.clip.ghost` box-shadow `--cf-shadow-drop` | 同上 | 已实现 | [录屏:recordings/02-drag-ghost-snap.webm](recordings/02-drag-ghost-snap.webm) |
| 3 | 目标轨高亮 / 非法落点红态 | `js/render/gesture-kit.js` highlightLane + `.track.drop-target` / `.track.drop-invalid`;ghost 红态 `.clip.ghost.invalid` | `track-lane-<id>` | 已实现 | [录屏:recordings/03-drop-invalid-red.webm](recordings/03-drop-invalid-red.webm) |
| 4 | 吸附瞬间 80ms 对齐脉冲 | `js/render/playhead.js` pulseSnap + `#snap-pulse` + `css/motion.css` cf-snap-pulse(限频 120ms 防频闪) | `snap-pulse` | 已实现 | [录屏:recordings/02-drag-ghost-snap.webm](recordings/02-drag-ghost-snap.webm) |
| 5 | trim 实时时长气泡 / 移动时间码气泡 | `js/render/gesture-kit.js` showBubble + `css/components/forms.css` `.cf-bubble` | `cf-bubble` | 已实现 | [录屏:recordings/04-trim-bubble.webm + 02-drag-ghost-snap.webm(移动时间码)](recordings/04-trim-bubble.webm) |
| 6 | 相邻片段碰撞约束(trim) | `js/render/gestures.js` onClipPointerDown(prevEnd/nextStart 夹取) | — | 已实现 | [录屏:recordings/04-trim-bubble.webm](recordings/04-trim-bubble.webm) |
| 7 | 拖拽零 Op、松手单 Op、Esc/失焦取消 | `js/render/gesture-kit.js` runGesture(cancel 路径)+ gestures.js(ADR-0013 ephemeral) | — | 已实现 | [录屏:recordings/02-drag-ghost-snap.webm(Esc 段)](recordings/02-drag-ghost-snap.webm) |

## 二、时间线

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 8 | clip hover 边缘把手浮现(160ms) | `css/components/timeline.css` `.clip .edge`(opacity 过渡;hover/selected/trim-preview 显形) | `clip-edge-l/r` | 已实现 | [录屏:recordings/05-clip-hover-select.webm](recordings/05-clip-hover-select.webm) |
| 9 | 选中光晕过渡(80ms) | `.clip.selected` outline/box-shadow 过渡(`--cf-select-glow`) | `clip.selected` | 已实现 | [录屏:recordings/05-clip-hover-select.webm](recordings/05-clip-hover-select.webm) |
| 10 | 播放头离散 seek 平滑(120ms) | `js/render/playhead.js` smoothSeek + `#playhead.smooth`;`js/render/preview-loop.js` seek 挂载(播放路径不挂) | `playhead` | 已实现 | [录屏:recordings/06-playhead-seek-ruler.webm](recordings/06-playhead-seek-ruler.webm) |
| 11 | 标尺 hover 时间气泡 | `js/render/gestures.js` mountRulerGestures(pointermove 悬停;scrub 路径复用) | `ruler` | 已实现 | [录屏:recordings/06-playhead-seek-ruler.webm](recordings/06-playhead-seek-ruler.webm) |
| 12 | 标尺按下即跳 + 连续 scrub(抽帧节流 ≤60Hz) | mountRulerGestures(pointerdown 即 seek;scrub rAF 合帧) | `ruler` | 已实现 | [录屏:recordings/06-playhead-seek-ruler.webm](recordings/06-playhead-seek-ruler.webm) |
| 13 | 框选多选(Shift/Ctrl 加选) | `js/render/gestures.js` onLanePointerDown + `#marquee-box`(timeline.css)+ selectionStore.clipIds(timeline-view 两处高亮) | `marquee-box` | 已实现 | [录屏:recordings/07-marquee-multiselect.webm](recordings/07-marquee-multiselect.webm) |
| 14 | 自动卷入(视口边缘 60px) | `js/render/gesture-kit.js` edgeScroll(clip 拖拽/框选挂载) | — | 已实现 | [录屏:recordings/02-drag-ghost-snap.webm(边缘卷入段)](recordings/02-drag-ghost-snap.webm)) |
| 15 | Ctrl/⌘/Alt+滚轮缩放(视口中心锚定) | `js/render/gestures.js` mountTimelineZoom + `js/core/model.js` setPxPerMs(live binding;缺省 0.06 为 e2e 红线不被装配触碰) | `timeline-wrap` | 已实现 | [录屏:recordings/08-zoom-wheel-fit.webm](recordings/08-zoom-wheel-fit.webm) |

## 三、面板

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 16 | 页签切换入场交叉淡入(160ms) | `css/layout.css` `.tab.active`(cf-fade-in;离场即时) | `tab-*` | 已实现 | [录屏:recordings/09-tab-crossfade.webm](recordings/09-tab-crossfade.webm) |
| 17 | 检查器分组展开 160ms | `js/ui/controls.js` collapseGroup + `.insp-fields-wrap.cf-anim-expand`(cf-expand-in;折叠即时) | `insp-group-*` | 已实现 | [录屏:recordings/10-inspector-group.webm](recordings/10-inspector-group.webm) |
| 18 | 素材卡 hover 提亮(不做位移,C-R1) | `css/components/media.css` `.media-item:hover`(filter brightness) | `media-item` | 已实现 | [录屏:recordings/11-media-hover-wave.webm](recordings/11-media-hover-wave.webm) |
| 19 | 波形入场淡入(160ms) | `css/components/timeline.css` `canvas.clip-wave`(cf-fade-in)+ `js/render/waveform.js`(纹理色经 `--cf-wave-line`) | `clip canvas` | 已实现 | [录屏:recordings/11-media-hover-wave.webm](recordings/11-media-hover-wave.webm) |

## 四、全局

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 20 | 按钮 hover/press 双态 | `css/base.css`(hover 换面+描边;active translateY 1px) | 全部 button | 已实现 | [录屏:recordings/12-button-tooltip.webm](recordings/12-button-tooltip.webm) |
| 21 | toast 滑入 + 自动淡出 | `js/ui/toast.js`(TTL 前 180ms 挂 `.out`)+ `css/components/forms.css` `.toast`(rise-in/out) | `toast` / `toast-err` | 已实现 | [录屏:recordings/15-toast-undo.webm](recordings/15-toast-undo.webm) |
| 22 | 模态缩放入场 + 遮罩渐显 | `css/components/forms.css` `.cf-mask`(cf-fade-in 160ms)/ `.cf-dialog`(cf-dialog-in 240ms) | `dialog` / `wizard` / `jy-dialog` | 已实现 | [录屏:recordings/13-dialog-zoom.webm](recordings/13-dialog-zoom.webm) |
| 23 | 导出进度条平滑(不跳变) | `js/panels/export.js` setProgress + `css/components/export.css` `.exp-bar`(active=不确定进度循环/ok/err;内核暂无百分比,诚实口径) | `export-progress` | 已实现 | [录屏:recordings/16-export-progress.webm](recordings/16-export-progress.webm) |
| 24 | 右键菜单入场(160ms)+ hover 态 | `css/components/forms.css` `.cf-menu`(rise-in;hover 80ms) | `context-menu` | 已实现 | [录屏:recordings/14-context-menu.webm](recordings/14-context-menu.webm) |
| 25 | 工具提示浮现(80ms) | `css/components/forms.css` `.cf-tooltip` + `js/ui/tooltip.js` | `tooltip` | 已实现 | [录屏:recordings/12-button-tooltip.webm](recordings/12-button-tooltip.webm) |

## 五、状态

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 26 | 保存中→已保存脉冲点(rev 翻牌处) | `apps/web/index.html` `#rev-dot` + `js/main.js`(rev patch 重放 cf-dot-pulse)+ `.rev-dot`(timeline.css) | `rev-dot` | 已实现 | [录屏:recordings/17-rev-dot-pulse.webm](recordings/17-rev-dot-pulse.webm) |
| 27 | 断连/网络错误横幅滑入 | `apps/web/index.html` `#conn-banner` + `js/ui/banner.js`(uiStore.connBanner 渲染路径)+ `.banner-row` 滑入 + `.conn-dot` 呼吸点 | `banner-conn` | 已实现(A2 遗留①已接线:wire-wave3.js 写 uiStore.connBanner;net 断→滑入,恢复→收起) | [录屏:recordings/18-conn-banner.webm](recordings/18-conn-banner.webm) |

## 六、降级与豁免登记

- `prefers-reduced-motion: reduce`:`css/motion.css` 一处总控(`*` 选择器压 animation/transition 时长);
- 循环动画豁免(仅两处):`.exp-bar.active`(导出进行中不确定进度)、`#conn-banner .conn-dot`(断连呼吸点);
- 拖拽全程零动画:ghost/trim/框选矩形几何每帧直写,无 transition/animation;
- 已知边界:面板离场为即时隐藏(display:none),只有入场动效——离场动画需要延迟卸载,
  与 e2e「切换后立即断言 active」冲突,列为有意取舍(见 ADR-0014 取舍段口径)。

## 七、册四 A4 新增微交互(FE1:T4.1/T4.2/T4.3;实现位置登记,录屏候 e2e 波补录)

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 28 | 素材缩略卡懒加载(视口内才发 media_thumbnail;缩略 2 路限流) | `js/panels/media-card.js`(IntersectionObserver)+ `js/core/media-cache.js`(排队/去重/负缓存) | `media-item .thumb` | 已实现 | 待录 |
| 29 | 音频卡迷你波形(peaks coarse;失败回落纹理) | `js/panels/media-card.js` loadThumb + `js/render/waveform.js` drawMiniWave | `.thumb canvas.thumb-wave` | 已实现 | 待录 |
| 30 | 缩放联动波形密度档(coarse/standard/fine) | `js/core/media-cache.js` peaksLevelFor + `js/render/waveform.js`(PX_PER_MS live binding) | `clip canvas.clip-wave` | 已实现 | 待录 |
| 31 | 代理徽标点击生成(等待省略号→✓/✗ 文本态) | `js/panels/media-card.js` proxyBadge + `js/ui/menu.js` 生成代理 | `media-proxy` | 已实现 | 待录 |
| 32 | 锁定轨拒绝编辑:lane 抖动一次(cf-shake 160ms)+ toast | `js/render/clip-gestures.js` flashLocked + `css/motion.css` cf-shake | `.lane-locked / .lane-locked-flash` | 已实现 | 待录 |
| 33 | 静音/独奏音频片段灰化(saturate/brightness 快过渡,非颜色单线索) | `css/components/timeline.css` `.lane-muted/.lane-soloed` | `track-lane-<id>` | 已实现 | 待录 |
| 34 | roll/slide 共享边界指示线(2px 实线,区别于吸附虚线) | `js/render/playhead.js` drawOverlay(trimLineMs)+ `js/render/clip-gestures.js` frameRoll | `tl-overlay` | 已实现 | 待录 |
| 35 | slip 内容平移预览(占位不动,波形纹理反向平移 + 源时间码气泡) | `js/render/clip-gestures.js` frameSlip(`.slip-preview`) | `clip.slip-preview` | 已实现 | 待录 |
| 36 | 轨道高度拖拽(拖拽期零 Op 直写高度;松手一笔 track_update) | `js/render/track-head.js` mountGrip | `.lane-grip` | 已实现 | 待录 |
| 37 | 轨名双击内联重命名(Enter/Esc/失焦收束) | `js/render/track-head.js` startRename | `track-name-<id>` | 已实现 | 待录 |
| 38 | 历史回跳确认弹窗(「将撤销 N 笔」;复用 confirm-dialog) | `js/panels/history.js` confirmDialog | `history-row / confirm-dialog` | 已实现 | 待录 |
| 39 | 历史快照分隔线(导出/批量前自动打标,ephemeral 不落盘) | `js/core/edit-commands.js` markHistory + `js/panels/history.js` markRow | `history-mark` | 已实现 | 待录 |

降级登记:素材缩略图/波形不可用(无 ffmpeg/无音轨)回落图标/装饰纹理,不假图;外部文件拖入导入面板 = 诚实提示(无拷入通道,登记候 BE 补);「在资源管理器打开」= 禁用项(浏览器沙箱),不假实现。

## 八、册五 A5 新增微交互(E-FE1:T5.1 关键帧 UI / T5.2 调色 UI;实现位置登记,收口波已补录)

| # | 微交互 | 实现位置 | testid/锚点 | 状态 | 录屏 |
|---|---|---|---|---|---|
| 40 | 秒表开/关双态(accent 描边 + aria-pressed;开启即打点,零动画) | `js/panels/kf-watch.js` + `css/components/ui-a5.css` `.kw-watch.on` | `kw-toggle-<prop>` | 已实现 | [录屏:recordings/40-kf-watch.webm](recordings/40-kf-watch.webm) |
| 41 | 秒表关闭确认弹窗(复用 confirm-dialog;空数组清除通道已随 BE 收口打通:schema minItems 移除 + 单 Op 清空/undo 还原) | `js/panels/kf-watch.js` confirmKfOff | `confirm-dialog` | 已实现 | [录屏:recordings/41-kf-watch-confirm.webm](recordings/41-kf-watch-confirm.webm) |
| 42 | 关键帧菱形拖拽跟手(邻居夹取 + 时间码气泡;拖拽零 Op 零动画,松手单 Op,Esc 取消) | `js/panels/kf-row.js` mountDrag + gesture-kit showBubble | `kf-row / kw-diamond` | 已实现 | [录屏:recordings/42-kf-diamond-drag.webm](recordings/42-kf-diamond-drag.webm) |
| 43 | 曲线画布锚点/贝塞尔柄拖拽(拖拽期采样点云降淡 + 直线示意,提交后投影采样刷新真曲线;ADR-0018 壳零插值) | `js/panels/kf-curve.js` + `js/panels/kf-editor.js` | `kf-curve-canvas` | 已实现 | [录屏:recordings/43-kf-curve-drag.webm](recordings/43-kf-curve-drag.webm) |
| 44 | 色轮指针拖拽跟手(角度=色相 半径=强度;盘面光谱逐像素数学生成=取色器数据面,轮圈/指针经 token) | `js/panels/grade-wheel.js` + `.grade-wheel` | `grade-wheel-lift|-gamma|-gain` | 已实现 | [录屏:recordings/44-grade-wheel.webm](recordings/44-grade-wheel.webm) |
| 45 | 示波器采样状态文本态(采样中…→已采样@ms;数据域标注「精确帧含调色」/降级「源素材未调色」) | `js/panels/scopes.js` | `scopes-status / scopes-note` | 已实现 | [录屏:recordings/45-scopes-status.webm](recordings/45-scopes-status.webm) |
| 46 | 分屏割线拖拽跟手(clip-path inset 直写,零动画;Esc/双击复位 50%) | `js/panels/compare.js` mountDividerDrag | `compare-divider` | 已实现 | [录屏:recordings/46-compare-divider.webm](recordings/46-compare-divider.webm) |

降级登记(本册新增):色轮盘面光谱与矢量示波器热图为 JS 逐像素数学生成的展示色
(取色器/示波器数据面,同 `<input type=color>` 原生光谱口径),非主题色,不落
tokens.css 之外的任何色值字面量;示波器 render_frame 不可用时降级直吃源素材
(「未调色」如实标注);分屏对比因内核 render_frame 无「不带 grade」开关且壳零 Op
纪律禁止临时清写,落地为「基准帧快照 vs 当前帧」口径(面板恒显说明)。

## 九、册五 A5-FE2 面板演示录屏(E-FE2:T5.3/T5.4/T5.5/T5.6 新面板;收口波补录)

| # | 演示 | 面板/工具 | 录屏 |
|---|---|---|---|
| D1 | 混音台轨道 EQ(整组替换单 Op) | `panels/mixer.js` / track_update patch.eq | [录屏:recordings/06-mixer-eq.webm](recordings/06-mixer-eq.webm) |
| D2 | 总线响度单(audio_loudness 测量 + 偏差徽标如实) | `panels/mixer.js` / audio_loudness | [录屏:recordings/07-mixer-bus-loudness.webm](recordings/07-mixer-bus-loudness.webm) |
| D3 | 复合片段说明卡 + 解包(compound_unbind 单 Op) | `panels/compound.js` / compound_create→compound_unbind | [录屏:recordings/08-compound.webm](recordings/08-compound.webm) |
| D4 | 多机位同步分析 → 切换点 → 生成序列(multicam_cut 单 Op) | `panels/multicam.js` / multicam_sync→multicam_cut | [录屏:recordings/09-multicam.webm](recordings/09-multicam.webm) |
| D5 | 场景检测(硬切夹具;可选自动切段单 Op) | `panels/scenetool.js` / scene_detect | [录屏:recordings/10-scene.webm](recordings/10-scene.webm) |
| D6 | 编码探测(AMF/NVENC 在位如实) | `panels/export.js` / encode_probe | [录屏:recordings/11-encode-hw.webm](recordings/11-encode-hw.webm) |
| D7 | 渲染队列生命周期(入队→暂停→恢复→取消) | `panels/queue.js` / render_queue | [录屏:recordings/12-queue-lifecycle.webm](recordings/12-queue-lifecycle.webm) |
| D8 | OTIO 导出→导入(新工程;往返最小子集) | `panels/export.js` / otio_export→otio_import | [录屏:recordings/13-otio.webm](recordings/13-otio.webm) |

录制口径:`docs/design/record-captures.py` 册五补录段(#40-46 微交互 + D1-D8 面板
演示)统一 720×450 低分辨率,单文件 ≤2MB、目录总量红线 ≤8MB;复跑命令
`python docs/design/record-captures.py [--only <子串>]`。

## 十、册六 A6 录屏(T6.1 工程库/迁移/模板 + T6.3 导出矩阵/preflight 门 + T6.2 素材库;F4 补录)

| # | 演示 | 面板/工具 | 录屏 |
|---|---|---|---|
| R1 | 工程库视图七操作(搜索/重命名/复制/归档/恢复/删除/打开命令一键复制;归档区并入) | `panels/library.js` + `library-ops.js` / library_manage | [录屏:recordings/50-library-ops.webm](recordings/50-library-ops.webm) |
| R2 | 布局迁移 v3(v2 提示条 → 确认框 → 迁移后重载;工程菜单常驻入口) | `panels/library.js` 迁移入口 / migrate_layout | [录屏:recordings/51-migrate-v3.webm](recordings/51-migrate-v3.webm) |
| R3 | 新建向导三模板(口播/双机位/方形,选即预填画幅帧率轨道)+ 布局 v3 选择 | `panels/wizard.js` / library_manage new | [录屏:recordings/52-wizard-template.webm](recordings/52-wizard-template.webm) |
| R4 | 导出矩阵(七格式说明行/画幅·清晰度·码率三档位/区域 in-out/仅视频) | `panels/export-matrix.js` / render_run 扩参 | [录屏:recordings/53-export-matrix.webm](recordings/53-export-matrix.webm) |
| R5 | 导出前 preflight 检查门(「导出成片」自动触发;缺失素材 warn 行 → 裁决「先不导出」;「仍要导出」同面) | `panels/export-matrix.js` runPreflight(auto) / export_preflight | [录屏:recordings/54-preflight-gate.webm](recordings/54-preflight-gate.webm) |
| R6 | 素材库页签(库根扫描入库/类型 chips/标签整组替换/一键导入工程) | `panels/media-lib.js` + `media-panel.js` 双页签 / media_library + media_import | [录屏:recordings/55-media-library.webm](recordings/55-media-library.webm) |

录制口径:同第九节(册六补录段 50-55 前缀,720×450;裁决按钮只在导出前自动门路径
渲染——手动「检查」按钮无裁决面,54 走「导出成片」触发);录屏经 VP9 重编码压总量
(时长逐支不变,单文件 ≤2MB)。

## 十一、册七 A7 录屏(T7.3 脚本页签 / T7.5 批准流+线程报告 / T7.2 插件;F4 补录)

| # | 演示 | 面板/工具 | 录屏 |
|---|---|---|---|
| R1 | 脚本页签运行(内置「批量变色」载入 → 运行 = preview_plan 副本预演 → 结构化逐步回执) | `panels/script.js` / preview_plan | [录屏:recordings/56-script-run.webm](recordings/56-script-run.webm) |
| R2 | 计划批准流(以 plan 提交 → 差异面板预演卡 → 逐项批准/拒绝 → apply_plan 回执 + planId) | `panels/diff.js` 批准流 + `core/plan.js` / preview_plan + apply_plan | [录屏:recordings/57-plan-approve-flow.webm](recordings/57-plan-approve-flow.webm) |
| R3 | 标注线程两轮(note_reply 不改结案态)+ session_report 人话 Markdown 渲染 | `panels/notes.js` 线程块/报告区 / note_reply + session_report | [录屏:recordings/58-note-thread-report.webm](recordings/58-note-thread-report.webm) |
| R4 | 插件安装确认(选文件 → plugin_validate 校验卡+权限五面 → 写入注册表 → 首启确认对话框 → 运行) | `plugins/manager.js` + `manifest.js` / plugin_validate | [录屏:recordings/59-plugin-install-confirm.webm](recordings/59-plugin-install-confirm.webm) |
| R5 | 贡献点生效(命令贡献点进时间线右键菜单并调用回执)+ 越权拦截(只读插件调写工具 GUARD_FAILED/FORBIDDEN toast) | `plugins/contributes.js` + `host.js` / 宿主权限裁决镜像 | [录屏:recordings/60-plugin-contribs-forbidden.webm](recordings/60-plugin-contribs-forbidden.webm) |

录制口径:同第九/十节(册七补录段 56-60 前缀,720×450;录完即 VP9 CRF46 重编码压
总量,时长逐支不变;旧档三支大文件 09-multicam/16-export-progress/13-dialog-zoom
同 CRF46 重编码腾挪余量,目录合计 ≈7.77MB ≤ 8MB 红线)。
