# A5-PROGRESS · 册五「专业深度」进度册

> 口径:status 只认实码 + 夹具证据;工具数以 `schemas/mcp-tools.json` 为准
> (册五后 **61 = 15 查询 + 30 写 + 16 编排**)。RENDERER_VERSION 随本册升 **8.0**
> (grade 链 / 轨道组建流 / loudnorm 目标入键 / encode 带 bt709 标签重编码,旧缓存整体失效)。

## 一、任务面

| 任务 | 内容 | status | 关键证据 |
|---|---|---|---|
| T5.1 关键帧引擎 | IR v3 + 五条表达式通路 + 逐样本对拍 | ✅(BE1,commit 6de2f23) | parity K1–K6;RENDERER_VERSION 7.0 时点 |
| T5.2 调色 | clip.grade 整对象替换:一级校色(色温/色调/曝光/对比/高光阴影/饱和度/Lift/Gamma/Gain)+ 二级(曲线/LUT;HSL 登记降级)+ 示波器数据后端 + lut_import | ✅(BE2,本批) | `render::grade` 链单测;parity G1–G5;`lut_import`/`scope_data` 工具 |
| T5.3 音频工作站 | track.eq(≤8 段 biquad 链)+ track.dyn(acompressor 参数子集 + alimiter)+ ducking 参数化 + audio_loudness 响度计 + loudnormTarget 导出参数化 | ✅(BE2,本批) | parity A1–A3(频带能量/动态范围/响度 ≤1LU);`across.rs` 分组建流单测 |
| T5.6 渲染质量与控制 | encode 缺省 **remux+bt709 标签**(零重编码零代损)+ 显式选项重编码参数面(encoder/quality/crf/bitrate/gop/pixFmt)+ ffprobe 复验自检 + 硬件探测/优雅降级 + 渲染队列(排队/暂停/恢复/取消/重试)+ 每步耗时/命令回显开关 | ✅(BE2,本批) | `render::encode` 单测;encode_probe;render_queue 冒烟;parity G1 标签复验;bench 复跑 -15.5% |
| T5.4/T5.5 | 复合/多机位/调整层;OTIO/EDL | ⬜ 未开工 | 册五后续批次 |

## 二、T5.2 调色:grade IR 与滤镜链映射

**链序定档**(ADR-0018 段链图上扩,`render::grade` 模块注释同文):

```text
[源] → crop → flip → rotate → scale+pad+fps(画幅归一) → punchIn
     → ★ grade → fx.combo → reverse → 变速 → kf 合成 → motion → tpad
```

- 挂载位置 = **画幅归一后、fx 前**(调色喂给特效;胶片颗粒/暗角叠在调色后的画面)。
- grade 内部链内序(一级在前、LUT 收尾,达芬奇「LUT 在节点末端」心智):
  `colorbalance`(色温/色调/高光/阴影/Lift)→ `curves`(RGB/亮度曲线,master=亮度近似)
  → `eq`(曝光 brightness/对比/饱和度/Gamma 分通道)→ `colorchannelmixer`(Gain 对角)
  → `lut3d`(LUT)。全部逐像素静态,与 reverse/变速可交换。
- 域:ADR-0020 bt709(全程 SDR;LUT 按目标域 bt709 解释,跨色域 LUT 适配属 HDR 重开议题)。
- 缺省 = 链零变化(parity 红线,既有 segment 单测逐字锁定)。
- **HSL 限定器:诚实降级**(IR 承载防丢,渲染 WARN 留痕,同 A4-L1 fx.in/out 口径)
  ——ffmpeg hue 无范围控制,split+maskedmerge 路线重且脆,评估后明确降级并登记
  (capability-matrix #25 missing)。
- 已知盲区(实测留档):colorbalance 的通道权重按像素亮度计,纯饱和极值点
  (如纯蓝 0,0,255)变化被钳制——夹具采样选阴影域暗像素;极值色走 Gain 通路(G5)。

## 三、T5.2 LUT:解析与库

- `.cube` 解析单一实现 = `render::grade::parse_cube`(lut_import 校验面与单测同源):
  3D 主格式(2..=256,主流 33³),数据行数 = SIZE³,三列有限浮点;TITLE/DOMAIN_* /
  INPUT_RANGE 容忍;`LUT_1D_SIZE` 显式拒绝。
- `lut_import` 工具:校验 → 拷入 `.cutforge/luts/<消毒名>.cube`(同名同内容幂等覆盖,
  同名异内容追加序号);返回相对路径供 `clip.grade.lut` 引用。
- 渲染应用:`lut3d=file='<绝对路径,冒号转义>'`(实测:引号+`\:` 缺一不可);
  文件缺失 → WARN 跳过(诚实降级不炸段);**LUT 内容哈希入段缓存键**(改文件必 miss)。

## 四、T5.2 示波器数据协议(scope_data)

- 输入:帧/视频(工程内相对路径)+ atMs + 采样宽(64..512,缺省 256)。
- 输出三类数据(壳 canvas 绘制是 FE 活,本工具只供数据):
  - `waveform`:列采样亮度 min/max/avg(bt709 加权 luma,列数 waveCols 缺省 128);
  - `vectorscope`:UV(BT.601 系数)64×64 网格计数,行优先 V 外 U 内;
  - `histogram`:R/G/B 各 64 桶计数。
- 缓存:`.cutforge/scope-cache/<key>.json`(内容寻址 = 路径+mtime+size+参数);
  纯计算核心(分桶/聚合)模块内单测,跨机确定性(平色帧夹具)。

## 五、T5.3 轨道 EQ/动态链与响度单

- **混音链图**(有 trackProc 时;无声明 = 既有图逐字一致):
  ```text
  逐事件 [i:a](aformat→denoise→areverse→变调→atempo→volume/表达式→afade) ,adelay
    → 同轨多事件 amix 建流 [tg<轨>]
    → per-track 链 [tg<轨>](eq 全段 equalizer/lowshelf/highshelf → acompressor → alimiter)[tp<轨>]
    → 总线 amix [bus] → BGM(ducking 侧链,参数化)→ [mixout]
  ```
  acrossfade 链模式下视频轨链挂在 `[acb]`(全部主线片段同轨且有声明才应用,跨轨诚实跳过)。
- EQ 段映射:peaking→`equalizer=f/w=q/g`,lowshelf/highshelf→`lowshelf|highshelf=f/g`
  (选 biquad 族弃 firequalizer:短段窗效应 + 版本差异,本机实测口径);
  动态:thresholdDb/limitDb(dB)→ 线性域 10^(dB/20)(`acompressor`/`alimiter` 同域)。
- **响度单**:`audio_loudness` 工具(loudnorm 测量,与导出双 pass 同源参数)→
  LUFS/TP/LRA/阈值 JSON;`target` 可选 → deviation/within1LU(AC-5.3 判定面)。
  导出侧 `loudnormTarget` render 选项(I[:TP])注入 mix pass B;**目标入 mix 缓存键**
  (改目标必换键——冒烟实测钉坑:漏键时陈旧 -14 混音复用,偏差 1.95LU)。
- ducking 参数化:`bgm.duckThreshold/duckRatio/duckAttackMs/duckReleaseMs`
  (缺省 = 既有常量 0.03/8/80/500,格式化去尾零后与既有字面串逐字一致)。

## 六、T5.6 编码探测/参数面/队列

- **编码器解析**:`auto`/缺省 = libx264(**确定性基线**,parity 根基);`hw` = 按候选序
  (nvenc→qsv→amf)**试编探测**(256px null 输出,毫秒级)→ 全败优雅降级 libx264 + WARN
  (试编通过但全片会话失败同样降级——AC-5.6);`sw` = 强制软件。
  本机取证:AMD 卡 → `h264_amf usable=true`(hw 实渲成功),nvenc/qsv usable=false(降级路径成立)。
- **质量预设**:fast/balanced/quality → sw(crf,preset)=(28,veryfast)/(23,medium)/(18,slow);
  硬件按编码器各自映射(nvenc p1/p4/p7+cq、qsv veryfast/medium/slow+global_quality、
  amf speed/balanced/quality);显式 crf/bitrate/gop/pixFmt 覆盖(缺省 = 现值零变化)。
- **色彩标签(ADR-0020 决策 1)**:encode 缺省 = **remux + 标签**(`-c copy` +
  `-color_primaries/trc/colorspace bt709 -color_range tv`;mp4 colr 框由输出流参数
  写入,实测复验可见)——零重编码、零代损失,渲染经济与拆分前同量级
  (bench 复跑 **-15.5%**,AC-5.7 劣化 ≤20% 达标)。显式编码选项才重编码:
  双通道写入(`-color_*` 旗标 + `-vf setparams` 帧属性;实测本机 build 下
  旗标单独给不落 VUI,必须 setparams 才复验可见)。
  ffprobe 复验 `color_primaries/color_transfer/color_space`(字段名实测钉坑:
  ffprobe 的传输特性键 = `color_transfer` 非 `color_trc`)入渲染自检(缺失 WARN)。
- **渲染队列**(`progress.rs` 重写):render_run **入队**即回 runId;单例 worker 按
  并发上限派发(env `CUTFORGE_RENDER_CONCURRENCY` 缺省 1);`render_queue`
  list/pause/resume/cancel/retry。**暂停语义诚实声明**:运行中暂停 = 终止子进程转
  paused,resume 重新入队(渲染缓存吸收重跑成本;进程级冻结跨平台不可达)。
  状态机 queued→running→ok|fail;queued/running--pause→paused;--cancel→canceled;
  fail/canceled/ok--retry→queued。状态事件接既有 `render.progress` 事件面
  (queued/paused/canceled 新状态如实发布)。
- **渲染日志**:每步耗时 `elapsedMs` 恒入进度事件;`verboseCmd`(缺省关——命令原文含
  素材路径,安全口径)附加 `cmd`。

## 七、工具数与 golden

- 56 → **61**(lut_import/scope_data 编排,audio_loudness/encode_probe 查询,
  render_queue 编排);口径 **61 = 15 查询 + 30 写 + 16 编排**。
- golden 重录 61 工具;**对比模式连跑两次 0 DRIFT**(elapsedMs 经 JSON-Lines 行归一;
  LUFS 值 probe_mode 下 0.5LU 量化;硬件探测布尔占位 `<HW_PROBE>`——跨机稳定)。
- 四则口径文档同步:README/FLOW/protocol_conformance 单测/check_doc_counts 全绿。

## 八、验证输出(全部实际执行)

| 项 | 结果 |
|---|---|
| `cargo test --workspace --locked` | **355 passed / 0 failed**(e2e_note_cli 首轮偶发一次,temp 目录并发竞争,复跑两轮全绿) |
| `cargo clippy --workspace --all-targets -- -D warnings` | **0 告警** |
| `tools/check_doc_counts.py` | OK:61 工具 = 15+30+16(扫描 4 文件) |
| `cutforge-cli check-ui-fields` | OK:22 片段字段 + 9 轨道字段全部被 Patch 支撑 |
| `tools/schema_gen.py --check` | 生成物与 schema 一致 |
| `tools/gen_constants.py --check` | 常量零漂移 |
| 行数红线(非测试 .rs ≤800) | 全绿(超限三文件拆分:model→grade_ir 150 / command→patch_apply 378 / dispatch→rpc 80) |
| `tool_parity --update-golden` 后连跑两次 | **61/61 PASS,0 DRIFT ×2** |
| `tools/bench/bench.py --check`(release;AC-1.8/AC-5.7 复跑口径) | open 4.4ms/query 46.7ms/apply 72.8ms 全 PASS;render min 5566ms vs baseline 6586ms(**Δ=-15.5%**,劣化阈 ≤20%)PASS;bench 轮询已适配队列 `queued` 暂态 |
| 冒烟:LUT 导入+应用 | 4³ 解析 ✓;帧红均值 95→142(红增益 1.5)✓ |
| 冒烟:色彩标签 | final 输出四枚举 `{primaries,trc,space}=bt709, range=tv` ✓ |
| 冒烟:响度单 | loudnormTarget=-16 → 实测 **-16.05 LUFS,偏差 0.05 ≤1LU**(AC-5.3)✓ |
| 冒烟:硬件编码 | `--encoder hw` → **h264_amf 实渲成功**(本机 AMD 卡;encode_probe:nvenc/qsv usable=false)✓;`--encoder sw --crf 20` → libx264 ✓ |
| 冒烟:队列 | 3 任务并发 1;pause(queued→paused)→ list 见 paused → resume → 双任务 ok ✓ |

## 九、FE 接口说明

- `clip.grade`(检查器「调色」组,ui-fields v8 editable):整对象替换经
  `clip_update patch.grade`;null/空对象 = 清除(grade_clear 承接)。投影
  timeline_get 的 clip JSON 直接含 grade(壳渲染参数面板)。
- `track.eq / track.dyn`(trackEditable「轨道」组):`track_update patch.eq/dyn`,
  null = 清除;混音台推子/电平表是纯壳活(电平实时 RMS/Peak 需播放期驱动,本期未做,
  登记后续)。
- `render_run` 新可选参数:`encoder/quality/crf/bitrate/gop/pixFmt/loudnormTarget/verboseCmd`
  (全部缺省 = 现行为);`render_queue` 五 action;`render_progress` 事件新增
  `elapsedMs`(恒)与 `cmd`(verboseCmd 时)。
- 示波器:FE 取 `scope_data` JSON 画 canvas;分屏对比纯壳活(登记)。
- LUT 库:`lut_import` 后响应 `data.lut` 即 `grade.lut` 引用值;FE 无需读文件系统。

## 十、遗留(候后续批次/册)

1. **HSL 限定器渲染**(登记降级中):候选路线 split+maskedmerge/geq 掩膜或 LUT 化选色,
   须实渲对拍达标再改判;`grade.hsl` IR 已承载。
2. **电平表/混音台实时面**(T5.3 FE 半):预览播放期 RMS/Peak 驱动需要播放链采样口,
   本册只交付轨道处理与响度计数据面。
3. **分屏对比**(纯壳活,登记);**调色关键帧化**(grade 入 keyframes 白名单)候登记册后。
4. encode 常驻**试编探测**不读缓存(毫秒级);encode_probe 工具面有 `.cutforge` 缓存口径
   未做(当前直测,量级可忽略)——如需批量工具面调用再补。
5. hw 质量映射的 amf 分支仅 `-quality`(crf/cq 无对应参数面,诚实映射);nvenc/qsv 全参面。
6. e2e_note_cli 首轮偶发(temp 目录并发竞争)——既有测试基建,候隔离化,非本册引入。
