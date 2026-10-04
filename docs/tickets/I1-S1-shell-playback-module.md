# I1-S1 壳:playback 模块(解码/时钟/音频/zone 纯函数)

来源:docs/upstream/05-ui-feature-iterations.md §I1 M1(硬骨头 B1)。

## 目标
apps/desktop/src/playback/ 新模块,UI 无关、可单测:
- decoder.rs:ffmpeg rawvideo rgba 管道 → 24 帧环形缓冲;3s 停流看门狗;Drop 杀进程。
- clock.rs:音频主时钟,fallback 墙钟;速度 0.25~4x;纯结构可测。
- audio.rs:ffmpeg f32le 48k stereo → cpal;欠载静音;设备失败 → AudioUnavailable。
- zone.rs:zone_key/is_fresh 纯函数(指纹+区间),无 RPC 无重 I/O。
- mod.rs:PlaybackEngine 统一门面(load_clip/play/pause/set_speed/set_muted/position_ms/poll_frame)。

## 边界
- 允许:apps/desktop/src/playback/**、apps/desktop/Cargo.toml(仅加依赖,如 cpal)、Cargo.lock。
- 禁止:app.rs/preview.rs/main.rs/state.rs/rpc.rs/kernel.rs/panels/**、crates/**、git 写操作、删 target。

## 验收
- cargo test -p cutforge-desktop:decoder 集成测试(ffmpeg 缺失时优雅跳过,本机 /d/MomentShift);
  真实声卡测试 #[ignore];时钟单调性、环形缓冲、速度档测试;
- fmt / clippy -p cutforge-desktop -D warnings / test 全绿(引用退出码)。

状态:回炉两次完工——①背压修复(丢头归零,慢消费回归测试)②preview 分辨率解码(累积衰减治理,进行中);27 测试全绿
