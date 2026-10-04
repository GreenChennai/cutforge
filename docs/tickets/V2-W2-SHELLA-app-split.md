# V2-W2-SHELLA 桌面壳拆分与键位注册表(A-02/BUG-22/A-07)

范围:apps/desktop/src/**(app.rs 1971 行拆分)+ 与 Web 键位同源的注册表。

条目(报告 v2 §7 A-02 / §4 BUG-22 / A-07):
- A-02 拆四模块:`app/mod.rs`(<300 行,生命周期/窗口装配)、`app/command_surface.rs`
  (菜单/快捷键→MCP 命令映射,纯数据驱动)、`app/workspace_view.rs`(面板布局+工程事件订阅)、
  `app/playback_facade.rs`(播放状态机门面);command_surface 不认识 GPUI 组件,
  workspace_view 不认识命令协议——双向解耦后可脱离桌面壳单测;
- BUG-22 键位升级命令注册表(id/标题/快捷键/上下文同源),设置页速查表从注册表生成;
  两壳共用一份键位语义表(JSON 常量,Web 读 JSON、桌面 include_str!);上下键改为跨轨遍历
  (对齐 Web 语义);TC-DESK-KEY-001 跨壳平价断言、TC-DESK-KEY-002 冲突绑定拒绝;
- A-07 桌面壳测试 0→≥40:投影纯函数 golden(state.rs)、rpc parse_envelope 表驱动 ≥15 例、
  command_surface 拼装单测(close_gap/paste 轨道反查既有手测路径)、TC-DESK-CMD-001/002;
- 行为零变化:拆分是纯移动,启动冒烟+既有交互逐项过;单文件 ≤600 行(G-1 口径);
- 与 Render/MCP2 并行时文件域不相交(它们不碰 apps/**)。
状态:待第 1 波集成后开工
