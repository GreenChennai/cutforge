# 微交互清单(册三 T3.2)· 实现位置登记

> 纪律:C-R1 降噪——动效只用于**状态变化**;时长梯度 fast 80ms / base 160ms / slow 240ms
> (token:`--cf-dur-fast/base/slow`);缓动标准 `cubic-bezier(.2,0,0,1)`(`--cf-ease`),
> 入场 `--cf-ease-enter` / 出场 `--cf-ease-exit`;一律 transform/opacity 合成器路径
> (box-shadow/filter 仅用于小面积状态色,禁 width/left/top/height 动画);
> **拖拽过程零动画(直接跟手)**;同屏并发入场动画 ≤3(全部为 ≤240ms 一次性短动效,
> 循环动画仅两处豁免:导出不确定进度条、断连横幅呼吸点);
> `prefers-reduced-motion` 在 `css/motion.css` 一处总控全量降级(动画/过渡压到 0.01ms)。
> 录屏存档(AC-3.2,record-captures.py 可重录):docs/design/recordings/ 共 19 条 webm(960×600,单条 ≤2MB,合计 ≈4.7MB);27 项逐行回链见下表「录屏」列;关键项覆盖:拖拽 ghost 跟手+吸附脉冲(02)、非法落点红态(03)、trim 时长气泡(04)、框选多选(07)、右键菜单(14)、模态缩放入场(13)、toast 撤销(15)、断连横幅滑入(18)、首启引导(01)、性能面板(19-perf-panel.webm)。

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
