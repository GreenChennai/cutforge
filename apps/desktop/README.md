# apps/desktop(GPUI 桌面壳)

**状态:C-FE2 交互收口(2026-10-03)。** Sable UI v4.0+(sable-dock/sable-video/
sable-widgets,486 测试)底座;本壳 = 常规 NLE 布局的可编辑工作台,真相在内核,
壳只做「投影 + 意图提交」(docs/upstream/03 §1.1 铁律)。

## 界面布局(常规视频剪辑软件形态)

```
┌──────────────────────────────────────────────────────────────┐
│ 工具栏:CutForge │ 撤销 重做 │ 分割 副本 删除 │ +视频轨 +音频轨 +字幕轨 │
├─────────┬────────────────────────────────┬───────────────────┤
│ 媒体库   │ 预览监视器(信箱式取景)          │ 检查器             │
│ 缩略图   │ ⏮ ◀ ▶/⏸ ▶ ⏭  时间码           │ (ui-fields 分组)   │
│ 双击插入 │                                │                   │
├─────────┴────────────────────────────────┴───────────────────┤
│ 时间轴:轨道头(V/A/T + M 静音)│ 标尺+轨道+播放头;缩放 −/+/适配  │
├──────────────────────────────────────────────────────────────┤
│ 状态栏:● 连接 · rev · clips · 最近消息 · 工程路径                │
└──────────────────────────────────────────────────────────────┘
```

## 实现现状(C-FE1~C-FE4 主体)

- `cutforge-desktop` bin:GPUI 开窗 + sable-dock 三段式工作台;`sable::dock::init`
  与 `gpui_component::theme::Theme::change(Dark)` 双主题全局各自初始化(只调后者
  丢 sable 主题会启动 panic);
- 内核子进程自拉起(`cutforge-cli serve --root --port --token`,健康探针只验
  HTTP 服务面 `/ui-fields`——**不要求工程合法**,坏工程开窗后在状态栏如实报错,
  不再白等 30s;Drop 随壳退出;`--attach` 可接已运行实例);
- `/rpc` MCP tools/call 客户端(root 每调用注入 + envelope 解包)+ `/events`
  长轮询。**对账两信号刻意分离**(实测教训:事件回流不带全量数据,原地重投影
  是陈旧数据的幻觉):dirty(事件/提交成功)→ 后台 `load_once` 重拉全量快照;
  snapshot_rev ≠ seen_rev → 本地重投影。reloading 守卫防重入;
- 媒体库:`media_browse` files 清单(滤 `.cutforge` 缓存)→ `media_thumbnail`
  串行抽帧缩略图网格(内核返回工程内相对路径,壳侧 `Rpc::absolutize` 挂根再读);
  双击 = `clip_add` 到播放头(素材类型路由首个同类未锁轨);
- 预览:`render_frame {atMs}`(100ms 网格量化;壳侧请求点钳在最后一帧之前——
  精确 =duration 抽帧会落空)→ PNG → gpui image,纯黑监视器风取景区 + 细边框。
  进度条(可点击/拖动 seek,canvas 回写 bounds 换算比例)+ 居中传输条
  ⏮◀▶/⏸▶⏭(播放键 accent 放大)+ 左时间码大字/右出帧状态徽标;播放 = 壳侧
  按真实流逝时间推进播放头 + 帧追逐(稳态 ~0.17s/帧,内核有帧缓存;幻灯片式
  预览,精确播放走 render 导出);单飞 + pending 折叠 + 60s 卡死自愈;
- 检查器:ui-fields editable 分组驱动。数值字段 = sable NumberField 实体池
  (拖拽/滚轮/键入全交互;选片变化才重建,避免每帧重建丢编辑态),Binding 直提
  `clip_update`;文本片段 = gpui-component Input(IME 全支持,Enter 提交);
  转场/动效 = 目录快捷按钮 + **时长 NumberField**(transition.durMs /
  motion.inMs / motion.outMs,读当前对象 merge 后整对象 patch);flip/denoise
  分段钮;reverse 开关;文本样式/花字行只对真文本片段渲染(text 为 null 的
  音视频片段跳过);其余对象字段只读摘要。片段操作:分割/副本@播放头/删除;
- 时间轴:sable `TimelineView` v0.3(横滚标尺同步/选中 2px 高亮/素材名标签/
  播放头红帽把手/行高 44 NLE 档/**点轨道空白 = scrub**)+ 壳侧轨道头列
  (V/A/T 徽标 + 名称 + M 静音,`track_update`,缩放组嵌入标尺对齐位)+
  底部信息条(倍速/全长/操作提示);字幕片段显示文本内容截 16 字;
  (px_per_second 是壳侧视图参数,不属时间线语义);
- 键盘:空格 播放/暂停;←/→ 步帧(Shift=1s);Home/End;Del 删选中;
  S 分割@播放头;D 副本;Ctrl+Z/Ctrl+Y 撤销重做;+/− 缩放(输入框聚焦时
  自动让位,不劫持输入);
- 工具栏:撤销/重做、分割/副本/删除(选中态启用)、三轨新增;`start-desktop.cmd`
  一键启动(依赖 CUTFORGE_FFMPEG / CUTFORGE_FFPROBE / CUTFORGE_RENDER 环境变量
  或同目录二进制)。

**冒烟(2026-10-03,真实点击回归,PostMessage 注入)**:选中高亮/标尺 scrub/
播放推进+帧追逐/暂停/分割@播放头(V1-001 → 4983+7017ms,入点语义正确)/删除
(rev+1 时间轴即时刷新)/双击插入(同轨重叠被 GUARD 拒并展示 CF-004,空位成功)/
轨道 M 静音(track_update → rev 推进 → 重拉 → 重投影)/缩放适配。

**内核侧配套修复(本轮)**:`cutforge-render` frame.rs 抽帧时间域映射——
时间线出现空隙(删段/移动)或片段带转场时,合成片与时间线时间轴错位,直接
-ss atMs 越过合成 EOF(rc=0 无输出 → "抽帧未产出文件")。映射与 compose 两
路径 offset 口径同源(xfade 名义累计 / concat 段实际时长),空隙钳前段末帧定格。

**待办(C-FE4/C-FE5 收口)**:媒体拖拽导入、布局序列化、波形/缩略图轨内渲染、
精确播放(render 管线)、渲染依赖打包说明。
