# PROJECT-FORMAT · CutForge 公开工程格式(T5.5)

> 本文是工程文件(project.json)的**公开格式文档**:IR 全字段 + 语义 + 版本迁移策略。
> 逐节从 `schemas/project.schema.json`(唯一手写真相源)与 `crates/cutforge-core/src/model.rs`
> 实码生成式核对;独立化的信任基础(册六要用)。字段语义与 schema `description` 同源,
> 冲突时以 schema + 实码为准。约束引擎:JSON Schema draft-07 子集(Rust
> `cutforge-schema::engine` 与 Python `tools/_generated/cf_validate.py` 双端同源对拍,
> `additionalProperties: false` 全量启用——幽灵字段显式报错)。
> OTIO 子集范围见第八节;多机位展开口径见第七节;本文遵守 ADR-0019。

## 一、总体形态

工程 = **目录 + 单文件真相源** `05_时间线工程/project.json`(V2 布局;V3 为根 `project.json`)
+ 媒体素材(单文件真相源纪律,ADR-0019 复合片段否决外部子工程引用的公共根据)。
目录契约(0.5 中文布局;旧英文布局 0.4.x 兼容读,不迁移;**V3 扁平布局**见 §1b):

| 目录 | 内容 |
|---|---|
| `00_制作简报/` `01_原始素材/` `02_转写与校对/` `03_创作素材/` | 上游输入(CutFlow 同源) |
| `04_粗剪决策/` | cutlist.json / cutlist.applied.json |
| `05_时间线工程/` | project.json + wordline.json(旧布局 `05_ir/`) |
| `06_成片输出/` | 渲染成片与导出派生物(SRT/ASS/VTT/OTIO/EDL) |
| `.cutforge/` | 渲染缓存/代理/缩略图/LUT 库/oplog/rev/锁/快照(派生物不入真相源,ADR-0013) |

时间统一**毫秒**(`*Ms` 后缀);剪映微秒只存在于适配层。工程 `version` 恒为 `1`
(兼容旧读法),契约演进由 `schemaVersion` 表达。

## 1b、V3 扁平布局(册六 ADR-0021,独立模式)

独立化心智:**真相源全部平铺工程根**,素材一个 `media/`,导出一个 `exports/`。
目录名唯一真相源仍是 `cutforge-io::paths`(`V3_*` 常量;三态判定 `LayoutKind::Legacy/V2/V3`,
并存时优先级 V2 > V3 > V1):

| 路径(相对工程根) | 内容 |
|---|---|
| `project.json` / `wordline.json` | 工程真相源 + 全片时间真相源(V2 的 `05_时间线工程/` 两文件上提) |
| `cutlist.json` / `cutlist.applied.json` | 粗剪决策单(V2 的 `04_粗剪决策/` 两文件上提) |
| `notes.json` / `.cutforge/` | 不变(本就随根;oplog/rev/快照/缓存三态同构) |
| `media/` | 素材池(`media_browse` 默认视图;迁移映射 `01_原始素材`/`01_materials` → `media`) |
| `exports/` | 导出产物目录(`06_成片输出`/`06_output` → `exports`) |

- **迁移器**:`cutforge-cli migrate <工程> --to v3` / MCP `migrate_layout`——一次性、幂等
  (v3 工程再跑 = NOOP)、冲突整体拒绝(任一映射目标已存在 → CONFLICT,盘面不动);
  project.json **字节零改动**,`.cutforge/`(OpLog/rev)原地不动——OpLog 完整性与撤销链保留(AC-6.2)。
  迁移映射:真相源文件上提 + 素材/输出两目录整体改名;`00/02/03/_内部状态` 等非契约
  目录原地保留并在报告 kept 如实列出;腾空目录移除。
- **冻结口径(F-R1)**:V1/V2 工程**兼容读写原地保留,只修 bug 不双写**;不做 v3→v2 回迁。
- **缺省策略(过渡期)**:scaffold 缺省仍产 V2(19 份 e2e 断言锚定面);V3 经显式开关
  (`cutforge-cli new --layout v3` / MCP `project_new layout="v3"` / 迁移)启用;缺省翻转
  待壳侧工程库页与安装器收编后单独变更(ADR-0021 决策 4,登记遗留)。
- 迁移后的工程不再被 CutFlow 侧 rs_* 桥按 V2 契约识别(独立化语义本身;ADR-0021)。

## 二、顶层字段

| 字段 | 类型 | 语义 |
|---|---|---|
| `version` | const 1 | 兼容旧读法,恒 1(计划书 3.3 第 5 项) |
| `schemaVersion` | enum `2.0.0`/`3.0.0` | 契约版本(semver);v1 工程由迁移器补齐为 3.0.0;新工程与迁移产物一律写 3.0.0 |
| `slug` | string | 工程标识(工程目录名) |
| `fps` | enum 24/25/30/50/60 | 允许集来自 `schemas/constants.ratios.json`(生成物,零漂移门禁) |
| `canvas` | object | `{width,height}`;ADR-0015:64..7680 且必须为偶数(yuv420p/x264 约束前移到 schema) |
| `backends` | array enum | `ffmpeg`/`jianying`/`cutforge`;N 后端注册表(ADR-0037),minItems 1,v1 迁移补 `["ffmpeg"]` |
| `notes` | string | 时间轴标注文件路径(相对工程根;缺省 notes.json) |
| `tracks` | array | 轨道数组(第三节),必填 |
| `bgm` | object | 工程级背景乐:`{src, assetId?, gainDb=-18, ducking=true, loop=true, duckThreshold?…}`;ducking 侧链参数 T5.3 参数化(缺省 = 既有常量 0.03/8/80/500,行为零变化) |
| `outputs` | array enum | 产出比例 9x16/3x4/16x9;canvas 是主比例,第二比例走 reframe |
| `markers` | array | `[{ms, label}]`(OTIO Marker 映射,T5.5) |
| `subtitle` | object | `{ass?, source?, style?}`(外部字幕路径) |
| `joinCrossfadeMs` | number | 0<值<1帧 的转场提升为该时长交叉溶解(ADR-0023);0=禁用提升 |
| `font` / `effects` | object | 工程级字体/渲染效果开关(CutFlow S7 消费;模型层承接防丢) |

## 三、轨道(Track)

| 字段 | 类型 | 语义 |
|---|---|---|
| `id` | string `^[VATX]\d+$` | 稳定 id:kind 首字母+序号(V/A/T;**adjust 取 X**,如 X1);首次生成写回不变(标注锚点依赖) |
| `kind` | enum | `video`/`audio`/`text`/**`adjust`**(调整层,T5.4:轨上片段 fx/grade 按时间窗作用于下方全部视频轨;不占主时间线、不进混音;文本随 textass 烧录) |
| `name`/`locked`/`mute`/`solo`/`hidden` | — | 显示名 / 防误编辑 / 静音(只作用音频面)/ 独奏(只作用音频面)/ 隐藏(只作用视觉面);渲染联动见 render::plan |
| `heightPx`/`color` | — | 壳显示偏好(不影响渲染) |
| `eq` | array ≤8 | 轨道 EQ(T5.3):peaking/lowshelf/highshelf 段,渲染混音链 per-track biquad;整组替换(track_update patch.eq),null 清除 |
| `dyn` | object | 轨道动态(T5.3):`{thresholdDb?, ratio?, attackMs?, releaseMs?, limitDb?}` acompressor+alimiter;整对象替换 |
| `clips` | array | 片段数组(第四节;带稳定 id 的对象数组——三路合并按 id 逐元素套表) |

## 四、片段(Clip)

必填 `id`/`startMs`/`durationMs`;稳定 id `^[VATX]\d+-\d{3}$`(`<轨id>-<序号三位零填>`,
如 V1-003;调整层轨片段 X1-001),禁止依赖数组下标。同轨时间重叠由引擎不变量拒绝
(CF-004 前提)。全字段(⌀ = 整对象/整组替换语义):

| 字段 | 语义 | 渲染消费 |
|---|---|---|
| `src` | 媒体相对路径(复合片段壳无 src) | 段提取输入 |
| `sourceHash` | 源内容寻址 hash(缓存/素材变更检测) | 缓存键辅助 |
| `startMs`/`durationMs` | 时间线占位;复合片段时长 = 子时间线总时长 | concat/adelay/-t |
| `sourceInMs` | 源域入点 | `-ss` 输入侧 |
| `speed` 0.25..4 | 线性常速(speedCurve 兼容回退) | setpts/atempo |
| `speedCurve` ⌀ | 分段速度曲线 `[{atMs,speed}]`,段间不内插(左点区间恒速);总源消耗 = ΣΔt×speed 分段积分,投影 endMs 与渲染时长严格一致 | speed_segments 单一真相源 |
| `reverse` | 倒放(先于变速;内存警示:整段读入内存) | reverse/areverse |
| `rotation`/`crop`⌀/`flip` | 变换链 = 裁剪→翻转→旋转→画幅归一→punchIn→reverse→变速 | transpose/rotate/crop |
| `volume` 0..2 | **None = 自然音量**(voice 1.0/sfx 0.8);显式 0 才是静音 | 混音 gain |
| `role` | voice/sfx/music/ambient | 缺省音量/密度护栏 |
| `text`/`textStyle`⌀/`huazi`⌀/`font` | 文本与样式(册四 T4.7;ADR-0016 确定性 ASS) | textass 烧录链 |
| `denoise`/`pitch` | 降噪档 off/low/mid/high;保速变调半音 ±12 | afftdn/asetrate+atempo |
| `position`/`scale`/`opacity`/`reframe` | 画面变换(overlay 存在时优先) | overlay 表达式 |
| `motion`⌀ | 入场/出场动画(in/out 枚举 + inFx/outFx=mo.* 直通,未注册降级 WARN) | 段滤镜链 |
| `transition`⌀ | 与前一片段的转场:`{type(7 基础枚举), durMs, reason(jumpcut/topic), fx(tr.* 直通 58 项)}`;type=cut/none 显式硬切;ADR-0023 尾帧扩展零时间漂移 | xfade 链 |
| `overlay`⌀ | 叠加层(rs_brand 变体轨口径):绝对像素 `{x,y,w,h,opacity=1}`;不占主时间线 | overlay 步 |
| `fade` | 音频淡入淡出 ms | afade |
| `loop` | 素材循环铺满 | stream_loop |
| `punchIn`⌀ | 变焦取紧构图(factor 1..2) | 段链 |
| `freezeMs` | 冻结帧补长(源播到该毫秒,tpad 克隆尾帧) | tpad |
| `assetId` | 素材库 manifest 稳定 id(溯源与换素材键;渲染仍由 src 驱动) | — |
| `fx`⌀ | 单片段特效:`{in?,out?,combo≤3}`(数组顺序即应用序;未注册 fxId 逐项降级 WARN) | 段链(fx 目录三态) |
| `keyframes` ⌀ | 关键帧(IR v3):`[{property,timeMs,value,interp?,bezier?}]`;白名单 position.x/y/scale/rotation/opacity/volume/speed/fx.*.*(整组替换;未知属性 SCHEMA_INVALID) | 五条表达式通路(ADR-0018) |
| `grade` ⌀ | 调色(T5.2):一级校色/曲线/LUT/HSL(登记降级);链序 = colorbalance→curves→eq→colorchannelmixer→lut3d;整对象替换,null/{} 清除 | grade 链 |
| `compound` ⌀ | **复合片段(内联子时间线)**,见第五节 | 递归展开 |

## 五、复合片段(`clip.compound`,T5.4/ADR-0019)

```json
{ "id": "V1-001", "startMs": 0, "durationMs": 2000,
  "compound": { "clips": [ {…子片段…}, {…} ] } }
```

- **内联子时间线**:`clips` 结构与顶层时间线同构,时间域为复合片段**局部域**
  (子 clip startMs 相对复合起点);`canvas` 可省略(缺省 = 工程画幅)。
- **嵌套深度上限两级**(主时间线 + 一层复合;子 clip 不得再带 compound——层级校验在
  内核语义层 `CompoundSpec::validate`,与 keyframes 同模式,拒绝三层)。
- **子时间线单轨语义**:子 clips 必须按 startMs 升序、两两不重叠且**首尾相接**
  (无间隙;校验器拒绝重叠/间隙——与主时间线视频轨同契约)。
- **渲染递归展开**:子时间线先按同一管线渲染为中间段(`.cutforge/render-cache/compose/`
  内容寻址,键 = 子 clips 内容指纹 + 画幅/帧率/尾帧/渲染版本/LUT 哈希),再作为普通素材
  参与外层既有合成(壳上的变换/变速/转场全通路复用)。**子 clips 携带的音频暂不渲染**
  (诚实降级 WARN 留痕,登记遗留)。
- **编辑 = 解包→改→重打包**(`compound_unbind` → 常规编辑 → `compound_create`),
  子 clips 不经 clip_update 直接 patch(个人自用心智,简单可审计);patch.compound
  整对象替换存在(undo/对拍面),显式 null 拒绝(摘除走 compound_unbind)。
- 打包/解包各为**单 Op 原子**(`Command::CompoundCreate/CompoundUnbind`),undo/redo/replay
  原样复用;投影 `timeline_get` 携带 `compound: {clipCount, durationMs, canvas}` 概要。

## 六、调整层(`track.kind = "adjust"`,T5.4)

- 轨上片段携带 **fx/grade**(与文本)时,按其时间窗叠加到**下方全部视频轨**的合成结果上;
  不占主时间线 concat 序列、不进混音、不撑时长上界;hidden 轨整轨不生效。
- 渲染 = **主合成(含叠加层)后新增 adjust 步**:每片段 `trim 抽窗 → setpts 归零 →
  fx+grade 链作用于窗内流 → overlay enable 贴回`(不依赖滤镜级 enable,任意链可窗内
  生效);空链片段跳过。缓存层 `adjust`(键 = 基片键 + adjust 片段 JSON)。
- 调整层上的**文本片段**随 textass 与文本轨同通道烧录(语义 = 顶层叠加,完全同源)。

## 七、多机位(展开方案,ADR-0019)

- **IR 不建 multicam 实体**。同步分析(`multicam_sync`:同源多角度素材数组 → PCM 波形
  互相关,engine=pcm-xcorr,启发式 degraded=true + confidence 诚实标注)→ 各角度偏移
  ms(offset 语义 = 角度源时间轴相对基准的滞后:读角度 i 内容于时间线 T,
  `sourceInMs = T + offsetMs`;angles[0] 为基准,恒 0)。
- 落账(`multicam_cut`):切换点列表 `[{tMs(相对 startMs), angle}]` → 展开为**普通片段
  序列**落同一条视频轨(单 Op 原子;sourceInMs 已含同步偏移)。"换角度" = 一次普通
  片段编辑,OpLog/撤销/对拍零新机制。

## 八、OTIO 子集与互操作(T5.5/ADR-0019)

**手写最小 OTIO JSON 子集**(否决引 otio crate,C4 最小依赖纪律)。导出与导入是同一份
确定性映射函数的两侧(`cutforge-core::interop`);往返语义 = **出→入→再出语义等价**
(`otio_semantic_eq`,name/id 为不透明句柄不参与比较;工程投影等价)。

| CutForge | OTIO(子集) | 备注 |
|---|---|---|
| 工程 | `Timeline.1`(`name`=slug,`metadata.cutforge{generator,schemaVersion,fps}`) | fps 落允许集外取 30 并 WARN |
| 轨(video/audio) | `Stack.1.tracks → Track.1`(kind Video/Audio) | text/adjust 轨子集外:导出略过 + WARN 留痕;导入整轨跳过 + WARN |
| 片段 | `Clip.1`:`source_range`(sourceInMs→start_time,durationMs→duration,RationalTime rate=fps,值=秒)+ `media_reference=ExternalReference.1(target_url=src 相对路径)` | 无 src 无 compound 的片段跳过 + WARN |
| 复合片段 | `Clip.1.children = Stack.1`(内嵌单 Video 轨,局部时间域原样) | 复合 ↔ Stack 映射;深度上限两级 |
| 间隙 | `Gap.1`(前置/片段间间隙;trailing 不导出) | 导入按占位还原 startMs |
| 转场 | `Transition.1`(transition_type = cutforge 基础型 verbatim;durMs 存 `metadata.cutforge`) | 导入:SMPTE_Dissolve/Wipe → fade;未知类型降级 fade + WARN |
| markers | `Stack.1.markers → Marker.1`(`marked_range.start_time`) | 双向映射 |

- **子集外元素**(未知 OTIO_SCHEMA/未知轨型/效果栈/时间变形/未知键):导入**诚实跳过并
  WARN 留痕,不静默丢**(`subset_scan` 逐节点点名);导入 = 从零新建工程
  (`otio_import`,project_new 同类,拒绝覆盖),不动已开工作区。
- **EDL(CMX3600)**:`otio_export format=edl` 手写导出——视频轨逐事件(C/D),有效转场
  映射 D 溶解事件(时长 = 声明 durMs 帧数);头注释写生成器与版本;音频/文本/调整层轨在
  头注释如实声明略过(不冒充全量;外部工具人工可读判定,AC-5.5)。
- **媒体引用策略**:恒工程相对路径(单目录可搬运);绝对/越出工程根的 URL 由导入方
  resolve 校验拒绝。

## 九、字幕格式(SRT/ASS/VTT,T4.7 + T5.5)

- **SRT**:parse→format 对规范形输入 byte 级幂等;时间精度 = 毫秒零损失;多行保留
  (clip.text 内含 \n)。
- **ASS**:导入只取 Dialogue 时窗与纯文本(剥 override 标签,\N → \n);导出样式恒
  Default 基线(样式契约面在 textStyle 字段)。
- **VTT**(T5.5):`WEBVTT` 头 + 点分隔时间戳(`HH:MM:SS.mmm`,兼容短形 `MM:SS.mmm`);
  NOTE/STYLE/REGION 块与 cue settings 容错;与 SRT 同毫秒内容互相转换等值(差异仅
  逗号→点 + 头)。`subtitle_export format=vtt` / `subtitle_import`(parse_auto 按
  WEBVTT 头识别)。
- **剪映草稿**(ADR-0023):导出**保留**,收编为随包独立脚本(`rs_jy_draft.py` 算法归属不变,
  随安装器分发;册六 T6.2 落地:随包资产在 `tools/jianying/`(开发树)与 `<exe>/scripts/`
  (安装器落点),带归属声明;orchestrate 定位序 = env CUTFLOW_REPO(显式,调试/对拍)→
  工程内 → 随包资产 → CutFlow 仓库回退(保留一个版本期),来源经响应 `scriptSource`
  如实标注);诚实标注仍依赖 Python 运行时,不冒充零依赖能力;headless 化(Rust 直写
  草稿 JSON)挂册七按使用频率再评估。本格式只保证 IR 字段兼容读。

## 十、版本迁移策略(v1/v2/v3)

| 版本 | 新增 | 迁移 |
|---|---|---|
| v1(无 schemaVersion) | — | 迁移器补 `schemaVersion=3.0.0`、`backends=["ffmpeg"]`、轨道/片段稳定 id(字母+序号确定性生成) |
| v2(2.0.0) | 契约化(baseRev/anchors/oplog 面) | 读兼容:v2 工程原样打开,不强制改写 |
| v3(3.0.0,册五 T5.1) | `clip.keyframes` | v2 读兼容:缺 keyframes = 无关键帧(与 v3 空集语义等价),**零迁移成本不强制改写** |
| v3 加法扩展(T5.2–T5.5,仍 3.0.0) | `clip.grade`/`track.eq`/`track.dyn`/`bgm.duck*`/`clip.compound`/`kind=adjust`/轨道 id X 前缀 | 全部 Optional + skip_serializing_if 缺省不落盘;**存量工程零改写**(serde default + 迁移幂等,migrate_idempotent 单测锁定) |

- 迁移器双端同源(Rust `cutforge-schema::migrate` / Python `cf_validate`),
  `migrate_py_rust_semantic_eq` + `migrate_idempotent` 门禁对拍。
- 模型层铁律:**schema 加字段,模型必须同批承接**("不加载 = 必丢",CONTRACT-WORKFLOW
  §3;防御 = roundtrip 语义相等夹具 `roundtrip_semantic_eq`)。

## 十一、结果协议与工具面

- 所有 MCP/HTTP 工具返回 `{ok, code, message, data}` + 加法字段 `ns`;code 取值限于
  5.4 表(OK/CONFLICT/SCHEMA_INVALID/PRECONDITION_FAILED/GUARD_FAILED/
  JIANYING_RUNNING/NO_CONFIG/DEP_MISSING/GREEN_SCREEN_INPUT/INTERNAL)。
- 工具集单一真相源 = `schemas/mcp-tools.json`(册六 A6 后 **72 = 16 查询 + 37 写 +
  19 编排**);`_doc` 口径句与 tools 数组机械对拍(`check_doc_counts`)。
- 写通道唯一入口 `Workspace::apply`(八步);工程级文档变更经 `record_change`
  (先文件后记账);渲染/同步分析/导出为免锁面(不产 Op 不改 IR)。

——本文以 `git grep -n "compound\|adjust" schemas/project.schema.json` 与
`cargo test -p cutforge-core interop` 为最低验收;册六独立化时按本文逐节复核。
