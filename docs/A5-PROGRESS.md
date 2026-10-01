# A5-PROGRESS · 册五「专业深度」进度册

> 口径:status 只认实码 + 夹具证据;工具数以 `schemas/mcp-tools.json` 为准
> (册五后 **68 = 15 查询 + 34 写 + 19 编排**)。RENDERER_VERSION 随本册升 **8.0**
> (BE2),BE3(T5.4/T5.5)再升 **9.0**(compound 递归展开 + adjust 步 + VTT)
> (grade 链 / 轨道组建流 / loudnorm 目标入键 / encode 带 bt709 标签重编码,旧缓存整体失效)。

## 一、任务面

| 任务 | 内容 | status | 关键证据 |
|---|---|---|---|
| T5.1 关键帧引擎 | IR v3 + 五条表达式通路 + 逐样本对拍 | ✅(BE1,commit 6de2f23) | parity K1–K6;RENDERER_VERSION 7.0 时点 |
| T5.2 调色 | clip.grade 整对象替换:一级校色(色温/色调/曝光/对比/高光阴影/饱和度/Lift/Gamma/Gain)+ 二级(曲线/LUT;HSL 登记降级)+ 示波器数据后端 + lut_import | ✅(BE2,本批) | `render::grade` 链单测;parity G1–G5;`lut_import`/`scope_data` 工具 |
| T5.3 音频工作站 | track.eq(≤8 段 biquad 链)+ track.dyn(acompressor 参数子集 + alimiter)+ ducking 参数化 + audio_loudness 响度计 + loudnormTarget 导出参数化 | ✅(BE2,本批) | parity A1–A3(频带能量/动态范围/响度 ≤1LU);`across.rs` 分组建流单测 |
| T5.6 渲染质量与控制 | encode 缺省 **remux+bt709 标签**(零重编码零代损)+ 显式选项重编码参数面(encoder/quality/crf/bitrate/gop/pixFmt)+ ffprobe 复验自检 + 硬件探测/优雅降级 + 渲染队列(排队/暂停/恢复/取消/重试)+ 每步耗时/命令回显开关 | ✅(BE2,本批) | `render::encode` 单测;encode_probe;render_queue 冒烟;parity G1 标签复验;bench 复跑 -15.5% |
| T5.4 专业编辑工具 | compound 内联子时间线 + adjust 调整层 + 多机位展开 + scene_detect | ✅(BE3,本批) | parity C1–C4(compound 两级实渲/adjust 窗/multicam 500ms 恢复/硬切检测);单 Op 原子(create/unbind/cut/split) |
| T5.5 互操作 | OTIO 手写最小子集往返 + EDL CMX3600 + VTT + PROJECT-FORMAT.md | ✅(BE3,本批) | 往返语义等价(出→入→再出 diff=0 + 工程投影等价);docs/PROJECT-FORMAT.md 生成式落档 |

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

## 六点五、T5.4 专业编辑工具(BE3)

- **复合片段(ADR-0019 内联子时间线)**:`clip.compound = {clips, canvas?}`——子 clips
  局部时间域、升序/不重叠/首尾相接(`CompoundSpec::validate` 拒重叠与间隙,与主轨同
  契约);深度上限两级(子 clip 带 compound 即拒,语义层校验与 keyframes 同模式)。
  `compound_create`(选区 ≥2 视频片段、两两相接、不含复合 → 打包单 Op 原子)与
  `compound_unbind`(局部→全局时间域平移回主时间线,id 重分配,单 Op)互为逆操作,
  undo/replay 零新机制;patch.compound 整对象替换(显式 null 拒绝);merge 按
  compound.clips id 数组递归(单测锁定内层不同叶零冲突/同叶 CF-001)。
- **渲染递归展开**(render::compound):segment 步遇 compound 先按**同一管线**
  (exec_segment+exec_compose 共享内容寻址缓存)渲子时间线为中间段——键 = compose_key
  (子 seg keys),即「子内容寻址键 = 子 clips 内容指纹」;中间段再作普通素材走外层
  全通路(变换/变速/转场)。子 clips 音频暂不渲染(诚实降级 WARN 留痕,登记遗留);
  compound 壳不产混音事件(中间段纯视频,防 mix 空 audio 流炸)。parity C1:子时间线
  红/蓝各 1s + fade 500ms 转场,外层 lime 叠加片段——总时长 2.0s/中间帧红→混合→蓝/
  叠加上层正确/重渲二级缓存全命中。
- **调整层**:`track.kind=adjust`(轨 id 字母 X);adjust 片段 fx/grade 按时间窗作用于
  主合成结果——渲染新增 **adjust 步**(overlay 之后 mix 之前;STEP_NAMES 7→8 加法):
  每片段 trim 抽窗→setpts 归零→fx+grade 链(窗内流,不依赖滤镜级 enable)→overlay
  enable 贴回;空 = 透传零产物;缓存层 `adjust`(键 = 基片键 + 片段 JSON)。调整层
  文本随 textass 同通道烧录;hidden 轨整轨不生效。parity C2:fx.blur 时间窗内红蓝
  边界峰值梯度骤降(<50%),窗外画面与颜色逐位不变。
- **多机位(ADR-0019 展开方案)**:`multicam_sync`(免锁纯计算)——PCM 波形互相关
  (engine=pcm-xcorr,8 倍抽取均值减除 + NCC,重叠 <60% 守卫;degraded=true +
  confidence 诚实标注;offset 语义 = 角度源时间轴相对基准滞后)。`multicam_cut`——
  切换点列表展开为普通片段序列落视频轨(sourceInMs 已含同步偏移,单 Op 原子;首切点
  必须 0/严格递增/角度越界拒绝)。parity C3:双素材固定 500ms 偏移(非周期六响锚点,
  周期节拍 xcorr 有等分歧义峰)→ 恢复 499ms,切换序列渲染色块无缝(1.5s 切点两侧同绿)。
- **场景检测**:`scene_detect`——灰度 64x36 @5fps 抽帧差分(engine=frame-diff,阈值 =
  mean+(max−mean)×k,严格局部极大防平台误报,200ms 最小间隔;degraded 诚实标注)+
  可选 autoSplit(Command::TrackSplitAt 单 Op 多点切,切点严格包含才切)。纯函数面在
  cutforge-render::analyze(与 mcp 工具/parity 同源)。parity C4:硬切色块检测点
  1.0s ±100ms;TrackSplitAt 切两段断言。

## 六点六、T5.5 互操作(BE3)

- **OTIO 手写最小子集**(ADR-0019,否决 otio crate):`cutforge-core::interop` 单一
  映射函数两侧——`otio_export`(工程→OTIO JSON:Timeline/Track(Video/Audio)/Clip
  (source_range + 相对路径 ExternalReference)/Gap/Marker/Transition 基础型/Stack↔
  复合;time = 秒值 RationalTime rate=fps,毫秒零损失)与 `otio_import`(从零新建工程,
  project_new 同类免锁;子集外元素 subset_scan 逐节点 WARN 留痕不静默丢;未知轨型/
  转场降级留痕)。**往返语义等价**:出→入→再出 `otio_semantic_eq` diff=0(name/id
  不透明句柄不比)+ 工程投影等价(src/startMs/durationMs/sourceIn/transition 逐键;
  sourceIn 缺席≡0 归一),单测锁定(含复合 Stack 往返)。
- **EDL CMX3600**:`otio_export format=edl` 手写导出——视频轨逐事件(硬切 C/转场 D,
  D 帧数 = 声明 durMs),头注释写生成器与版本,音频/文本/调整层轨头注释如实声明略过
  (不冒充全量);外部工具解析人工验证一次(AC-5.5 后半)候用户执行。
- **VTT**:subtitle.rs 补 webvtt(vtt_parse 兼容短形时间戳/cue settings/NOTE 块,
  vtt_format WEBVTT 头 + 点分隔;规范形 byte 级往返 + 与 SRT 同内容互转等值单测);
  subtitle_export format=vtt / subtitle_import parse_auto 按 WEBVTT 头识别。
- **docs/PROJECT-FORMAT.md**(公开工程格式):IR 全字段+语义+复合/调整层/多机位/
  OTIO 子集范围+版本迁移策略(v1/v2/v3+加法扩展零改写)+结果协议——从 schema 与
  model 实码生成式撰写,逐节核对实码(册六独立化信任基础)。
- 顺手修:**overlay 步层输入索引潜伏 bug**(`[i:v]`→`[i+1:v]`,旧实现把基片自身缩放
  叠加——红底红 logo 不可见;compound 夹具 lime 检出;单测锁定 + 既有 parity 断言
  不受影响);RENDERER_VERSION 8.0→9.0。

## 七、工具数与 golden

- 56 → **61**(BE2:lut_import/scope_data 编排,audio_loudness/encode_probe 查询,
  render_queue 编排)→ **68**(BE3:compound_create/compound_unbind/multicam_cut/
  scene_detect 写 + multicam_sync/otio_export/otio_import 编排);口径
  **68 = 15 查询 + 34 写 + 19 编排**。
- golden 重录(以 tools/bench/tool_parity.py 热修线最终盘面为准,工具序列覆盖以该文件
  实配为准);**对比模式连跑两次 0 DRIFT**(elapsedMs 经 JSON-Lines 行归一;
  LUFS 值 probe_mode 下 0.5LU 量化;硬件探测布尔占位 `<HW_PROBE>`——跨机稳定)。
  BE3 新七工具的协议面由 protocol_conformance(缺参探针 + pro_ops_tools_full_chain
  dispatch 级闭环)与 pro_ops/analyze 单测覆盖。
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
7. **compound 子时间线音频**(T5.4):单轨中间段只取视频面,子 clips 音频不渲染
   (WARN 留痕 + 壳不产混音事件);候选路线 = 子管线 mix 步产物作中间段音轨
   (须解决音频内容寻址键与外层变速耦合),真实多轨复合需求出现时按逐案 ADR 重开。
8. **EDL 外部解析人工验证一次**(AC-5.5 后半)候用户执行;多轨音频 EDL(A 通道映射)
   未做(诚实声明略过)。
9. **CutFlow 侧 schema 模板同步**(compound/adjust 字段)未做——跨仓改动不在本批
   授权面(crates/schemas/tools/docs);双边落库候单独批次(CONTRACT-WORKFLOW §1)。
10. **进出复合编辑视图无损 e2e** 壳侧半(投影 compound 概要已下放;壳视图是 FE 活,
    ADR-0019 落地证据登记)。
11. multicam_sync 只支持音频同源素材(视频画面差分对齐未做;诚实标注 engine=pcm-xcorr)。
12. scene_detect 阈值只收硬切级跳变(溶解/渐变检出候 ffmpeg scdet 选件评估)。

## 十一、收口波(2026-10-01,续作子代理实测)

**BE 两补**(E-FE1 钉坑收口):
- `patch.keyframes` 空数组=清空全部关键帧:schema `clip.keyframes` `minItems:1` 移除
  + description 写明「空数组 [] 合法 = 清除(整组替换语义)」;生成校验器
  `tools/_generated/cf_validate.py` 同步重生成(keyframes 块已无 minItems);
  单测 `crates/cutforge-core/tests/keyframes_clear.rs`:空数组单 Op/after=[](非 null)/
  before 携原整组/undo 还原整组/redo 复现清空/`to_validated_value` 放行。
- grade 入 `timeline_projection`(同 fx 先例,壳读回桥退化)+单测
  `timeline_projection_carries_grade_key_always`(挂=整对象/未挂=null 键恒在);
  `gradeCurves` description 回写「每通道 minItems=2 契约」(渲染端 curves 两点定一段;
  壳拖点前须补齐两端点)。

**parity 68**:新七工具夹具落库(multicam_sync 双素材=同源字节复制锁零偏移/
multicam_cut/scene_detect 硬切 2s 红+2s 蓝(含静音轨绕混音图 :a specifier)/
compound_create→compound_unbind/otio_export→otio_import→otio_export 往返,
`product_assert` 产物级断言:unbound=2、OTIO 语义 diff=0);golden 重录 **68**,
连跑两次 `67 PASS/1 WARN(audio_beats 启发式,既有口径)/0 DRIFT`。

**两份新 e2e**:`e2e_keyframes.py` 七断言 PASS(12.7s;秒表打点单 Op/投影
keyframes+keyframeSamples 121 点对拍/求值一致性帧亮度比 0.751≈0.75/曲线拖锚
round4(vAt) 写回/菱形拖移 60px=1000ms/easeIn 预设);`e2e_color.py` 九断言 PASS
(8.0s;基线中性帧/色轮 Lift 写回单 Op/帧色偏 meanR−meanB=105/LUT 帧压暗/
示波器三画布非空/分屏割线 50%→72.27%/grade 拷贝粘贴逐字段等价)。
TESTIDS.md 已登记 testid 全覆盖,apps/web 零改动。

**gate A5 注册**(`tools/gates/gate.py`,D-A2 制:A4 十八项一字不动全部继承,
另纳 e2e-keyframes/e2e-color/bench-threshold,**21 项全阻断**);CI web-e2e
只加两新步;`gate.yml` yaml.safe_load 校验过。

**录屏补录**:前任缺 `10-scene.webm`——收口修 capture 剧本两处 bug
(等待条件错对原始键名 `cutCount`→改等 `scene-summary` 摘要行;`scene-auto`
testid 挂 checkbox 本体,选择器去掉 ` input`)后补录成功(238KB)。目录 34 支
总量 **7.27MB ≤ 8MB 红线**(册五 15 支新录屏 VP9 CRF40 重编码压溃,时长逐支
不变;单文件 ≤2MB 口径不破)。micro-interactions.md #40-46 录屏回链 + 新增
第九节(E-FE2 面板演示 D1-D8)。

**全量验证(本机收官实测)**:
- `cargo test --workspace --locked`:40 套件全绿零失败(含 keyframes_clear 新测);
  `cargo clippy --workspace --all-targets --locked -- -D warnings` 零告警;
- e2e 17 份全绿(15 既有+2 新):轻量 11 份 + 负载敏感 4 份安静时段复跑
  (drag_perf P95 60.2fps/perf_timeline P95 60.2fps/media_perf P95 64.1fps 掉帧 0/
  perf_budget boot 408ms,perf-a3.json 重生成);
- `check_doc_counts`:**68 = 15 查询+34 写+19 编排** 与 schema 一致;
- `bench --check`(AC-5.7):render 中位 **6063ms vs 基线 6586ms(Δ −8.0%)**,
  阈值 ≤+20% PASS,存档 `docs/bench/2026-10-01.json`;
- pytest 12 passed。

**收口新增遗留**:
13. ~~秒表「末属性清空」壳侧接线~~ **已闭合(主控收尾,2026-10-01)**:kf-watch/kf-editor/
    kf-row 三处删除路径统一改为直写 `patch.keyframes: []`(空数组=整组清空,内核已接受),
    撤 emptyBlocked/CLEAR_BLOCKED 拦截与过期文案;node --check 过,e2e_keyframes/e2e_color
    复跑 PASS。「末属性清空」路径的 e2e 专项断言留待册六 e2e 扩充时补(现状:三路径
    手工冒烟一致,自动化未覆盖该单点)。
