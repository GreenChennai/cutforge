# V2-W3-SHELLC 逐面板美化与可用性(§9.6①~⑨ + 壳契约硬化)

范围:apps/desktop/src/panels/**、app/**(消费 SHELLB 的 theme/icon/fx)、kernel.rs、state.rs。

## 条目(报告 v2 §9.6 顺序即落地顺序;BUG/R 项并入对应面板)
① home.rs:hero 首帧封面(render_frame+磁盘缓存)、hover 提亮、空态;TC-UI-HOME-001
② 顶栏:三段式微调、rev 脉冲点(240ms 扩散)、导出进度环占位;TC-UI-TOPBAR-001
③ library.rs:两级导航、16:9 统一卡、时长角标、骨架灰块、媒体库→时间线拖放、
   右键"添加到播放头";TC-UI-LIB-001/002(千素材滚动 ≥55fps)
④ preview.rs:信箱黑+1px 边框、传输条全 SVG、等宽时间码 20px、低对比浮层、
   双色进度段;I/O 入出点键;TC-UI-PREVIEW-001/002
⑤ timeline.rs:clip 硬边轨型色、hover 把手 8px/4px、胶片+波形分区、标尺自适应密度、
   吸附脉冲;trim 四手势+时长气泡、框选 marquee、多选光晕、重叠转场胶囊、
   边缘卷入、锁轨抖动、轨高拖拽、内联重命名;TC-UI-TL-001/002/003
⑥ inspector.rs:**BUG-18 Box::leak 清除**(TC-DESK-LEAK-001 + 静态门禁)、tab 化、
   秒表图标+n 帧徽标、分组折叠 160ms;TC-UI-INSP-001/002
⑦ settings.rs:分组卡片、速查表已注册表化(SHELLA)样式对齐、快照开关/间隔(R-13)、
   reduced-motion 总控、布局方案管理;TC-UI-SET-001/002
⑧ ui/feedback.rs:toast 队列(5s+撤销位)、错误卡(错误码+建议+重试,消费 ns 字段)、
   断连横幅、三套空态;TC-UI-FB-001/002
⑨ BUG-16(speedCurve 投影显式"待内核支持",禁 1.0 占位)、BUG-20(内核 stderr 落
   .cutforge/logs/ + 心跳断流 5s 断连横幅 + 一键重启)、R-15(seq 单调+Arc 快照+
   长轮询退避)、R-16(缩略图缓存指纹)、R-17(shell-state.json 10s 防抖持久化);
   TC-DESK-PUMP-001/002、TC-DESK-CACHE-001、TC-DESK-STATE-001/002、TC-DESK-KERNEL-001/002
⑩ BUG-17 壳侧:render_frame 直读 framePath(旧扫描保留一版);TC-DESK-FRAME-001

## 依赖与纪律
依赖 SHELLB 的 theme/icon/fx 先行(消费其 token,禁裸值);消费 SHELLA 的六模块结构;
每面板交付含录屏;ui-taste-checklist ≥8/10;报告 v2 §9.6 验收判据逐条落。
状态:待 SHELLB 开工后串行(同面板文件域)
