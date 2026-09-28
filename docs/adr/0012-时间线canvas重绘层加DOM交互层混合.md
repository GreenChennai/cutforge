# ADR-0012:时间线渲染采用 canvas 重绘层 + DOM 交互层混合,不搞全 canvas 或全 DOM

日期:2026-09-29 · 状态:已采纳(册二 D-B2,动工前落库)
关联:册二迭代计划 T2.4/D-B2/AC-2.3/AC-2.4 · 参考 OpenCut Classic(Timeline Canvas)· ADR-0011(无构建 ESM)

## 背景

时间线是编辑器性能与交互密度的中心:标尺/网格/播放头/吸附线是高频重绘的"绘制型"内容,clip 块与轨道头是命中选择、无障碍、动画的"交互型"内容。旧壳全 DOM:每秒刻度是一个 span、播放头推进触发全量 innerHTML 重建(W2/W6),1k clips 必卡。动工前决策点 D-B2:时间线渲染走全 canvas、全 DOM,还是混合?

## 决策

1. **重绘层用 canvas**:标尺(刻度/网格/播放头三角标记)进 `#ruler` 内 sticky 定位的 canvas,按视口窗口绘制(不随内容宽度撑爆画布);拖拽吸附线等瞬时指示画进时间线视口 overlay canvas。这些层每帧重画成本与可见像素数成正比,与 clip 数量无关。
2. **交互层用 DOM**:clip 块与轨道头保留 DOM——命中选择(mousedown 目标区分主体/左右 trim 边)、无障碍(可聚焦/aria 标注)、CSS 过渡动画、以及既有 e2e 依赖的 `.clip[data-id]` 锚点都依赖真实 DOM 节点。
3. **播放头走 DOM transform 线 + canvas 标记的组合**:竖线是单个绝对定位元素,更新走 `transform: translateX`(合成器路径,零重排),三角形标记画在标尺 canvas;播放循环每帧只碰这两处与时间码文本,**绝不触发时间线 DOM 重渲染**。
4. clip 块内部的小画布(音频片段纹理)由 waveform 子模块按需绘制,虚拟化器保证只画视口内的片段。

## 取舍

- 混合方案要维护两套坐标换算(内容坐标 ↔ 视口坐标,sticky 偏移)——一次封装进 render 层,换算只发生在 ruler/overlay 两个模块内。
- 全 canvas 更极致(千级 clip 也零 DOM)但重构面大:命中测试、焦点管理、读屏、既有 e2e 锚点全部重写,与 B-R1「功能不回退」冲突;列为**册五可选优化**(计划书 §2.D 原文口径)。
- 全 DOM 维持 W2/W6 病灶,直接否决。

## 被否决的替代

- **全 canvas(OpenCut 式 Timeline Canvas 一步到位)**:否决(本册)——重构面与回退风险失控;保留为册五候选,届时需重开 ADR 评估 e2e 锚点迁移。
- **全 DOM + CSS will-change 硬扛**:否决——innerHTML 全量重建病灶不除,60fps 判据(AC-2.4)无解。

## 后果

- keyed reconciliation 只服务 DOM 交互层(clip/轨头);canvas 层自绘自管,两层通过"视口窗口 + pxPerMs"单一换算约定对齐。
- 播放解耦有了结构保证:播放循环的写入面 = transform + 两个文本节点 + canvas 重画,物理上碰不到 clip DOM。
- 时间线刻度不再是可选中/可命中的 DOM(旧壳 .tick span 删除);无 e2e 或快捷键依赖刻度 DOM。

## 落地证据

- `apps/web/js/render/ruler.js`(canvas 标尺)、`render/playhead.js`(DOM transform 线 + overlay canvas 吸附线)、`render/waveform.js`、`render/timeline-view.js`(DOM 交互层 keyed reconciliation + 虚拟化)。
- `apps/web/css/components/timeline.css`:sticky canvas 定位约定。
