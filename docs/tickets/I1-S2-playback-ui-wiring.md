# I1-S2 壳:播放接线 + M3 播放 UI

来源:docs/upstream/05-ui-feature-iterations.md §I1 M1/M3。

## 目标
1. DesktopApp 接 PlaybackEngine:播放泵从幻灯片切到引擎帧(纹理上传 gpui);
   zone 文件优先,未命中回落直解码;引擎失败自动回落幻灯片模式并在状态栏给可展开原因。
2. M3 UI:JKL(←→帧步/空格播放/ L 快放)、倍速 0.25~4x、循环区间、静音键;
   预览面板小按钮组(画质切换含代理 use_proxy / 截图=render_frame 落工程 / 全屏);
   断连/引擎故障横幅。

## 边界
- 允许:apps/desktop/src/app.rs、panels/preview.rs、panels/mod.rs、settings.rs(速查表)、kernel.rs(如需 env 透传)。
- 禁止:playback/**(只消费其 API)、crates/**、git 写操作。

## 验收
- 真实启动冒烟 + 截图;播放路径有日志证据(帧率/回落事件);
- 幻灯片降级路径保留(引擎不可用不白屏);门禁全绿。

状态:已完工——M1/M2/M3 全勾;收口修复:帧序号门控(17→30fps)、临时端口、play_log 落盘;1080p 实测 0~50s≥29.3fps
