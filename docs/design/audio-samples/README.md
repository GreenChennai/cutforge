# 听觉对比存档(册四 AC-4.6)

> 如实标注:**机器听觉对比为人工项**——自动化只保证产物存在、参数与内核渲染链一致,
> 降噪/变调的「听感是否自然」须人耳复核;本存档即复核输入。

## 文件(命名规整:`<处理>_before|after.wav`;48kHz/mono/16-bit,各 4s)

| 文件 | 内容 | 滤镜(与内核同一映射,首段均为 aformat 48k 归一) |
|---|---|---|
| `denoise_before.wav` | 440Hz 正弦 + 白噪声(anoisesrc seed=42,可复现) | 混音(未处理) |
| `denoise_after.wav` | 同上,降噪后 | `afftdn=nr=15:nf=-35:tn=1`(内核 denoise=mid 档) |
| `pitch_before.wav` | 440Hz 干净正弦 | 直通 |
| `pitch_after.wav` | 同上,+4 半音保速变调 | `asetrate=60476,aresample=48000,asetpts=N/SR/TB,atempo=0.793701`(atempo = 1/k 减速拉回,与内核 speed/pitch 同口径,时长不变) |

- 生成入口:`python tools/e2e_media_perf.py`(每次运行确定性重建;滤镜参数与
  `crates/cutforge-render/src/across.rs` 的 denoise/pitch 映射逐字一致;四支样本
  时长 = 源 4s 由 e2e 保速断言把守)。
- 预算:四文件总体积 ≤3MB(本目录由 e2e 断言把守)。
- 复核口径:对比 before/after 的「残留噪声 / 高频发闷(降噪)」与「音高是否整
  +4 半音、时长与速度是否不变(变调)」。
