# ADR-0013:临时投影边界——不进 IR、不落盘、不参与撤销,`ephemeral.*` 命名空间白名单

日期:2026-09-29 · 状态:已采纳(册二 D-B3,动工前落库)
关联:册二迭代计划 T2.6/D-B3 · 总纲 §0.4 R6(壳内语义计算回潮风险)· check-shell-purity 门禁 · ADR-0011/0012

## 背景

总纲 R6 指出:壳内一旦出现自算时间线语义,就会出现"视图与盘面两套真相"的回潮。但交互体验(修 W7 拖拽跟手)又要求壳在**鼠标按住期间**展示"如果松手会是什么样"的预览——拖拽 ghost、trim 实时变形、吸附指示线,这些值随指针每帧变化,既不可能也不应该每帧打一次内核命令。需要一条可门禁判定的边界,把"临时视觉投影"与"持久语义计算"切开。

## 决策

**三原则**——壳内临时投影(ephemeral projection)允许存在,但必须同时满足:

1. **不进 IR**:临时值不得写进任何与 project.json/timeline 投影同构的状态容器(六 store 中的 project/timeline 两 store 是投影只读面,ephemeral 值只住在独立 ephemeral store);
2. **不落盘**:临时值不得出现在任何请求载荷的持久化字段里(clip_update 的 patch 只能来自已投影值与用户输入,拖拽中间值不进 patch);
3. **不参与撤销**:临时值的产生与消失不产生 Op、不触碰 rev。

**命名空间白名单**——ephemeral store 的每个键在 patch 记录与调试快照中一律以 `ephemeral.` 前缀登记(当前白名单:`ephemeral.dragGhost`(拖拽/trim 的视觉预览)、`ephemeral.snapMs`(吸附指示线)、`ephemeral.hiddenTracks`(轨头眼睛开关的视图隐藏集));白名单之外新增临时键必须先改本 ADR。

**生命周期铁律**——ephemeral 值的存在期 = 手势按住期/会话期(视图开关),手势结束(mouseup/Esc)即清除并以服务端重投影结果为准;`workspace.changed` 重投影到达时 ephemeral 视图层必须让位于投影(EphB:投影到达 → 清 ghost → 重渲染)。

## 取舍

- 拖拽期间的 ghost 位置是"壳自算的 startMs 候选值"——本质上是把语义计算放在了壳里,只是**不承认它为真相**。换来:拖拽逐帧跟手(W7),而提交仍是单次内核命令、以服务端重投影收敛。
- 门禁可判定性:check-shell-purity 升级(T2.6,册二后续任务)按「投影只读 + ephemeral 白名单」建模;本 ADR 先把边界用命名空间在代码层固化,store 层保证 ephemeral 键在 patch 记录里永远带前缀,审计可 grep。
- 代价:存在一个独立的 ephemeral store 与"投影到达清 ghost"时序规则;复杂度远低于每帧 RPC 或全 canvas 重算。

## 被否决的替代

- **禁止一切临时投影(拖拽期间只显示吸附刻度数字)**:否决——W7(拖拽不跟手)是本册修 W 清单明文项;达芬奇式"交互零阻塞"是册二参考对齐的明确目标。
- **临时值进 timeline store、用 `isGhost` 标记位区分**:否决——真值与临时值同容器,污染"store 可由内核投影完全重建"铁律(T2.2),且 `isGhost` 位会渗进渲染与断言。
- **临时值每帧同步给内核(原子草稿通道)**:否决——为视觉效果引入高频写路径,rev/OpLog 噪声不可接受,与「断言以服务端状态为准」的 e2e 纪律冲突。

## 后果

- 壳内出现的一切非投影时间数值必须能指出其 `ephemeral.*` 键;否则按纯度违规处理。
- `ephemeral.hiddenTracks` 让轨头眼睛开关无需任何内核支持(W12 补课零 Rust 改动);该开关不是工程状态,刷新/重投影后按会话内选择保留,换会话即回默认——这是"不落盘"的直接推论。
- 册三(主题化/多面板)与册四(真实缩略图/预热)复用同一白名单扩展流程:改本 ADR → 加键 → 门禁同步。

## 落地证据

- `apps/web/js/core/store.js`:ephemeral store(patch 键强制 `ephemeral.` 前缀,与投影 store 物理分离)。
- `apps/web/js/render/gestures.js`(拖拽/trim 写 `ephemeral.dragGhost`/`ephemeral.snapMs`,mouseup 提交命令)、`render/timeline-view.js`(投影到达即清 ghost)。
- `apps/web/TESTIDS.md`:轨头眼睛开关 `track-visibility-<trackId>` 的 ephemeral 语义标注。
