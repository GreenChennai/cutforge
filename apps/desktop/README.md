# apps/desktop(GPUI 桌面壳)

**状态:M5-4 复活落地(2026-10-03,C-FE1~C-FE5 首波,docs/upstream/03)。**
Sable UI v4.0(sable-dock/sable-video/sable-widgets,484 测试,Windows 冒烟多轮)满足
ADR-0039 恢复条件(GPUI 稳定生态 + Windows 冒烟);下文为降级期历史记录,保留备考。

## 实现现状(C-FE1~C-FE3 + C-FE4 部分)

- `cutforge-desktop` bin:GPUI 开窗 + sable-dock 三段式工作台(左媒体库/中预览/
  右检查器/下时间轴);
- 内核子进程自拉起(`cutforge-cli serve --root --port --token`,健康等待 30s,
  Drop 随壳退出;`--attach` 可接已运行实例);
- `/rpc` MCP tools/call 客户端(root 每调用注入 + envelope 从 result.content[0].text
  解包)+ `GET /events?root&since` 长轮询线程 → rev 对账重投影(壳零真相缓存);
- 时间轴:sable-video `Timeline` 视图投影(place_clip 铸造视图 id,回调还原内核
  字符串 id)→ widgets `TimelineView` 渲染 + 播放头 + 拖拽/点选回调;
- 检查器:`GET /ui-fields` editable 分组驱动(数值字段 ± 步进提交 `clip_update`
  patch;其余组只读展示)——字段集单一真相源在内核,壳零字段清单;
- 预览:`render_frame` 单帧 → PNG → gpui image(需 cutforge-render/ffmpeg 就位,
  缺失时错误信封如实上状态栏);
- 工具栏:undo/redo/播放头步进/预览此帧;`start-desktop.cmd` 一键启动。

**冒烟(2026-10-03)**:pure-animation 工程,壳存活、内核自拉、timeline 3 clips 投影、
clip_update(volume)→ rev 0→1 → undo 全闭环;重叠编辑被内核 GUARD 拒绝并如实展示。

**待办(C-FE4/C-FE5 收口)**:媒体缩略图/拖拽导入、布局序列化、渲染依赖打包说明。

---

上游 OpenCut 的桌面壳自述 "Very early. Right now this is just a window that opens",
GPUI 0.2.2 生态尚在演进且对 Linux/WSL 有硬性平台约束。按计划书既定降级路径:
本项目先以 **wasm 内核 + Web 壳(apps/web)** 交付多端能力,桌面壳待 GPUI
0.3+ 或上游 crates/* 落地后复评——内核单一实现不变,壳只是薄皮,届时以
`cutforge-wasm` 同源投影接入 GPUI。

恢复为阻断门禁(M5-4)的条件:GPUI 稳定版发布 + 本仓引入 `apps/desktop`
二进制并通过 Windows 冒烟(开窗+加载工程+渲染时间线)。
