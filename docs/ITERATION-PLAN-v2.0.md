# CutForge × CutFlow 审查报告与 V2 升级迭代计划书

| 项 | 值 |
|---|---|
| 文档版本 | v2.0-draft1(评审稿) |
| 日期 | 2026-09-19 |
| 性质 | **先文档后动工**:本文是 V2 迭代的唯一依据;M8 启动前不写修复代码 |
| 审查方法 | 双路只读代码审查(内核/结合面各一路,证据到 file:line)+ 交叉自查拷问 + 关键 P0 实机验证 + 竞品调研(达芬奇/剪映,来源见附录) |
| 证据等级 | ★★★ 实机复现 / ★★ 代码路径推演(行号确凿) / ★ 文档与实现比对 |
| 前置阅读 | `docs/FLOW.md`(现状流程地图)、CutFlow `docs/adr/0033-0039` |

---

## 0. TL;DR(一分钟版)

1. **V1(M0–M7)交付的骨架是真实的**:7549 行 Rust、77 测试全绿、CI 全绿、双端契约对拍、渲染对拍 6/6、Release 三平台产物——这些不是空壳。
2. **但本次审查发现 5 个 P0**:最重的一条是**结合面断裂——CutForge 打不开任何 CutFlow 真实生成的工程**(`_meta` 字段被 v2 契约拒收);其次 **undo 对文件级 Op 是静默伪造**(已实机复现:返回成功、rev 上涨、OpLog 记录了回退,文件却原封不动)。
3. **能力矩阵存在系统性虚标**:13 项"必达达成"中 4 项无任何代码支撑(变速/转场/BGM ducking/punch-in),2 项半虚标,1 项(音效落点)是真 Bug——sfx 不做 adelay,所有音效在 0 秒同时炸响。
4. **双向同步目前是"拉式死代码"**:merge_from_disk 与 watcher 全仓零生产调用,baseRev 快照链仍是占位——M3 的冲突检测在真实工作流中永远不会触发。
5. **V2 路线一句话**:先修信任(M8 止血)→ 再通链路(M9 同步落地)→ 再会编辑(M10 对齐剪映核心操作)→ 再追平渲染(M11 矩阵从纸面到实码)→ 最后专业分轨(M12 达芬奇向,可选)。沿用 V1 的门禁制,每阶段指标可判定、可存档、不可放宽。

**结合度总评分:8/20(契约 1/5 · 流程 2/5 · 工具 2/5 · 发布 3/5)。** 文档层融合叙事完整,运行时主链路断裂——这就是 V2 要解决的核心矛盾。

---

## 1. 审查方法与自查记录(grill 闭环)

按全权委托,拷问转为自查,共三轮,结论均已折入后文:

- **第一轮(验证主张)**:对"undo 伪造"嫌疑写临时例程实机执行——`notes_add → undo` 返回 `Ok rev=2`,notes 总数仍为 3,盘面 notes.json 仍含新标注。**P0 实锤(★★★)**。复现:`Workspace::open → notes_add → undo → 读 notes.json`。
- **第二轮(诚实性拷问)**:把 `docs/capability-matrix.md` 的 13 个"✅ 达成"逐项对 `cutforge-render/src/lib.rs` 做 grep 对照(grep xfade/setpts/atempo/zoompan/ducking/overlay/bgm),证实 4 项零代码、夹具不覆盖任何虚标项。**矩阵按实码降级重写(§5)**。
- **第三轮(证伪我的两个预判)**:①"v1 schema 会拒收 v2 IR"——**证伪**(v1 模板无 `additionalProperties:false`,CutFlow→CutForge 方向宽容;真实断裂是反向的 `_meta`);②"rs_cleanup 会删 notes.json"——**证伪**(classify() 白名单不匹配 notes.json/.cutforge)。审查报告里的每条 P0 都过了这道反问。

---

## 2. 结合度评估(四层)

| 层 | 得分 | 断裂点(证据) |
|---|---|---|
| **契约层** | **1/5** | ① CutFlow `rs_ir.py:176/334` 两个 build 路径**必写**顶层 `_meta`;cutforge v2 schema `additionalProperties:false` 且无 `_meta` → `Workspace::open` 对一切真实工程直接失败(★★)。② 实际存在**三份**契约表示:CutFlow v1 模板 schema、cutforge v2 schema、`rs_ir.validate` 手写校验器——ADR-0034"唯一真相源"未落地(v1 模板仍被 `tests/test_v10.py` 断言)。 |
| **流程层** | **2/5** | ③ `stage.rs` 脏传播映射无任何生产消费者(仅测试引用);与 rs_run 缓存语义系统性矛盾(rs_run.py S3 缓存键不含 project.json 自身输出 → cutforge 标了脏,`rs_run --status` 依旧判 done)。④ `cut_apply` 不重算 keep/removedMs,而 `rs_cut --apply` 会从 cuts[].action **重算并覆盖 keep** → 经 MCP 编辑 keep 是幻觉路径;更糟:cutlist 的脏提示指向 `04_cut/rebuild.py` → S2 `--force` **从 wordline 重新 detect 并重写 cutlist.json**,把触发重建的那次编辑本身冲掉。⑤ B8 手注护栏(`rs_ir.py:345-363` `_manual_edits`)只认 chroma/background/_meta.manualEdit,识别不了 CutForge 写入 → `05_ir/rebuild.py` 无提示冲掉编辑器改动。 |
| **工具层** | **2/5** | ⑥ `orchestrate()` 无条件追加 `--json`,而 rs_run/rs_render/rs_jy_draft 的 argparse 均无此旗标 → **stage_run/render/export_jianying 三个 MCP 编排工具必崩**(退出码 2);数字参数被 `filter_map(as_str)` 静默丢弃。⑦ 裸 `python` 起子进程:Windows 仅装 py 启动器的机器 6 个编排工具全灭。⑧ `stage_status` 只查 `_state/S*.json` 文件存在性,不区分 done/stale/failed。⑨ `rs_editor timeline` 对 CutFlow 原生 IR(轨道无 id)输出 `id/track = null`。⑩ 双侧硬编码本机 `E:\` 路径;`rs_gate --probe` 不检查 gate.py 存在即返回 OK(doctor 体检假绿)。⑪ MCP `render` 工具只封装 rs_render.py,与 cutforge-render 零集成(两座孤岛)。 |
| **发布层** | **3/5** | 做得好:ADR/CONTEXT/README 交叉引用扎实,CI 浅克隆 CutFlow 跑 M0/M1,四桥命令与 README 逐字一致。欠缺:两仓互无桥级集成测试(P0 正是从这条缝漏过);`SKILL.md`(Agent 实际入口)未登记四桥;CI 注释有误导;组织迁移/律师复核/人工签字三项 PENDING。 |

**契合性判断**:分工模型本身成立(CutFlow=批量机械臂,CutForge=人的手),但"手"目前**只会看不会动**(Web 壳是纯只读查看器,文件靠 `<input type=file>` 加载,没有任何编辑落盘路径),且"手"与"臂"之间当前**只有单向只读桥是通的**。V2 的契合性目标:编辑器侧可写、管线侧可感知、双向同步真正触发。

---

## 3. Bug 清单(双仓合并,分级)

### P0(正确性/数据损坏,共 5 条)

| # | 标题 | 证据 | 级别 |
|---|---|---|---|
| P0-1 | **undo 对文件级 Op 静默伪造**:notes/cutlist 的 Op 进了撤销栈,undo 把 before 写回 **project**(指针 /items 不存在→insert 后被反序列化静默丢弃;指针 / 是纯 no-op)——返回成功、rev 上涨、OpLog 谎称已回退,文件原封不动;撤销深度被重定位 Op 系统性灌水 | engine.rs:233(不查 target.file)/ :351 / :514;io/lib.rs:255;**已实机复现** | ★★★ |
| P0-2 | **`_meta` 契约拒收:CutForge 打不开任何 CutFlow 真实工程**;且 typed Project 无 _meta 字段,即使收下,任何 CutForge 写回都会丢它 | rs_ir.py:176/334 → project.schema.json:5 → schema/engine.rs:131 → io/lib.rs:65;migrate.rs 不剥 _meta | ★★ |
| P0-3 | **渲染段缓存跨画幅污染**:缓存键=hash(clip)+版本,**不含 canvas**;先渲 9x16 再渲 16x9,后者直接命中前者分段——文件名标 1920x1080,画面是 1080x1920 内容 | render/lib.rs:120-123;`render_variants` 注释"只分叉 encode"亦不实(:291-307 全流程重跑) | ★★ |
| P0-4 | **人声混音只取第一个 voice 素材的完整源音频**:无 per-clip 裁剪(sourceInMs 失效)/无延迟/无逐段音量,其余 voice 片段全部丢失——任何粗剪/多段工程音画必然错位 | render/lib.rs:195(voice_files.first)/ :201-221 | ★★ |
| P0-5 | **并发丢更新**:MCP 每次 dispatch 各自 `Workspace::open`(锁外读),锁只在单次操作内持有 → 双通道并发写同一工程时后写者整文件覆盖先写者,而先写者的 Op 已进 oplog → project.json 与 OpLog 永久分叉 | mcp/lib.rs:81;io/lib.rs:58-117(open 不做 oplog 对账) | ★★ |

### P1(功能断/虚标,14 条)

1. **能力矩阵虚标**(详见 §5):变速/转场/BGM ducking/punch-in 零代码;位置缩放旋转 overlay/音量淡入淡出半虚标(字段不消费);`capability_matrix` MCP 工具还返回"必达/可选"规划值,与文档口径互矛盾。
2. **音效落点缺 adelay**:sfx_specs 收集了 start_ms 但滤镜从未使用——所有 sfx 在 0 秒同时炸响(render/lib.rs:154 vs :211-215)。
3. **merge_from_disk 以本地充当 base**:本地修改对合并器不可见 → CF-001/CF-002 **永不触发**,本地改动被磁盘静默覆盖,"禁止最后写入者获胜"被架空;且 merge_from_disk 与 Watcher **全仓零生产调用**——M3 同步是拉式死代码。
4. notes_resolve 错误码是占位符 `"REJECTED-if-missing-else-INTERNAL"`(mcp/lib.rs:284),不在 5.4 表内;协议一致性测试只测缺参分支,逃过门禁。
5. orchestrate 追加 `--json` 使 3/6 编排工具必崩(结合面⑥)。
6. 裸 `python` 子进程(Windows py-launcher 机器全灭)+ CutFlow 路径硬编码。
7. stage_status 仅查文件存在性(结合面⑧)。
8. **ARL-1.0 核心文件标记缺口**:LICENSE 1.3 条只认 ARL-CORE 头/核心目录/CORE-FILES 清单三口径;现实仅 2 个文件带 ARL-CORE 头,目录名不匹配,`[package.metadata.arl]` 是许可未定义的自定义机制——**互惠义务对绝大多数核心源文件的归属存在争议空间**(engine/model/oplog/merge/notes/anchor 均未标)。
9. cut_apply 先记账后写文件:写失败则 oplog 已留"已改"、rev 已涨,文件没变(审计与盘面分叉)。
10. cut_apply keep 编辑被 rs_cut --apply 静默丢弃 + cutlist 重建路径自我覆写(结合面④)。
11. MCP render 工具不接 cutforge-render(结合面⑪)。
12. B8 护栏不识别 CutForge 写入(结合面⑤)。
13. rs_editor timeline 对 v1 IR 输出 null id(结合面⑨)。
14. rs_gate --probe 假绿 + 硬编码路径(结合面⑩)。

### P2(质量/边界,18 条摘要)

serve_http 可被单连接挂死(读循环无超时)/全缓冲区子串鉴权+可枚举默认 token/无 Connection:close;锁 stale 阈值 30s 但持锁方不刷新心跳(慢盘双写者);persist 崩溃窗口无对账;redo Op 的 before 记错(engine.rs:258);open 把 notes.json 一切读错误当空(限权/IO 抖动→数据丢失,应只认 NotFound);notes_add 幂等分支返回无关标注 id;CLI 错误码自成一套(与 5.4 表漂移);apply_value_at 终点缺键静默 insert;watcher.debounce 存而不用;渲染文件名直接拼 slug(非法字符写坏路径);fps 硬编码 30 忽略 project.fps;joinCrossfadeMs 不消费;OpLog 半行恢复语义两侧分叉(rs_oplog 跳过续读 vs cutforge 截断);轨道/片段 id 跨重建按下标重派→标注**错锚**(比孤儿更糟);字幕时间域断层(subtitle_retime 改 IR 不联动 S7/wordline.final);契约三重表示(ADR-0034 自我违背);SKILL.md 未登记四桥;两仓无桥级集成测试。

---

## 4. 功能契合性:与"编辑器"的差距(对标基线)

### 4.1 对标矩阵(功能轴)

依据:达芬奇官方 Cut 页文档与 New Features Guide、剪映/CapCut 官方教程与中文社区手册(链接见附录 A)。

| 能力域 | 达芬奇 Resolve | 剪映/CapCut | CutFlow(管线) | CutForge(编辑器)现状 | 差距判定 |
|---|---|---|---|---|---|
| 时间线模型 | 双时间线(Cut 磁吸+Edit 传统)、智能指示器 | 多轨+磁吸、逐帧细剪 | IR 静态描述 | 只读投影,无操作 | **本质差距** |
| 剪切/分割/删除/移动 | ✅ 全套 + ripple/roll/slide/trim | ✅ 全套 + 简易 | rs_cut 决策式 | 命令层已有(clip_split/delete/move),**无 UI** | 中 |
| 撤销/重做 | 无限层级+可命名快照 | 多级 | rebuild/rollback 工程级 | 有而**伪造**(P0-1) | 先修后谈 |
| 变速 | clip speed + 曲线 speed ramp | 线性+**曲线变速**(SVIP) | clip.speed 字段 | 字段在,**渲染不消费**(虚标) | 大 |
| 关键帧动画 | 全属性关键帧+曲线编辑器 | 关键帧(缩放/位移/不透明度) | 无此概念 | **IR 无 keyframes 概念** | 大(V2 需 IR v3) |
| 转场 | 库+自定义 | 丰富转场库+时长设置 | 三级语法(ADR-0026) | 不消费 transition | 大 |
| 字幕 | 手动+语音转写(付费版 AI) | **AI 智能字幕**、样式卡、卡拉OK | rs_subtitle 全套+卡拉OK(强项) | ASS 只读烧录 | CutFlow 已领先 |
| 音频 | Fairlight:混音台/降噪/侧链/响度计 | 音频分离/降噪/淡入淡出 | loudnorm 总线/ducking(强项) | 混音残缺(P0-4) | 先修后谈 |
| 调色 | 行业标杆(节点/Lift-Gamma-Gain/LUT) | 滤镜+调节(HSL/曲线) | 无 | 无 | 远期(M12) |
| AI 能力 | Neural Engine(魔法遮罩/隔离) | AI 抠图/智能字幕/粗剪 | **自带 ASR 字级对齐+粗剪决策器**(强项) | — | CutFlow 已领先 |
| 互操作格式 | XML/AAF/EDL/**OTIO** | 草稿专有(5.9 后加密) | 剪映 5.9 草稿导出 | project.json v2 | 中(加 OTIO 进出) |

### 4.2 界面/操作轴(结论)

- **剪映的胜因是"零学习成本+素材库+AI"**;达芬奇的胜因是"分页工作流(Cut 快剪/Edit 精剪/Color/Fairlight/Deliver)+专业深度"。CutForge 的差异化定位应当是第三条路:**"AI 原生编辑器"——每一个操作都是可审计的 Op,人和 AI 在同一条时间线上互相可见**。这是两个商业产品都不具备的结构性优势(V1 已打好地基:OpLog/标注/冲突模型)。
- V2 的界面原则(决策 D3,§8):**Web 壳先达到"剪映核心操作可用"**,不追素材库(归 CutFlow/artboard),不做调色页(M12 评估)。

---

## 5. 诚实性修正:能力矩阵降级重写(V2 第一步)

M6 交付的 `docs/capability-matrix.md` 必须按实码改判,并在 M8 作为门禁项(矩阵-代码-对拍夹具三方一致):

| # | 能力 | 原判 | 实码改判 | M11 目标 |
|---|---|---|---|---|
| 2 | 变速 | ✅ 达成 | ❌ 未实现(字段不消费) | setpts/atempo+曲线 |
| 5 | 转场 | ✅ 达成 | ❌ 未实现(compose 直通) | xfade 链+尾帧扩展 |
| 9 | BGM ducking | ✅ 达成 | ❌ 未实现(bgm 零引用) | 侧链 amix→asplit |
| 12 | punch-in | ✅ 达成 | ❌ 未实现 | zoompan |
| 4 | 位置/缩放/旋转 | ✅ 达成 | ⚠️ 半实现(仅画面适配) | overlay/rotate |
| 3 | 音量/淡入淡出 | ✅ 达成 | ⚠️ 半实现(音量不生效) | per-clip volume/afade |
| 8 | 音效落点 | ✅ 达成 | 🐛 实现=Bug(无 adelay) | adelay 落点+密度护栏 |
| 14 | 多画幅 | ✅ 达成 | 🐛 缓存污染(P0-3) | 真·共享缓存分叉 encode |

修正后真实达成:必达 13 项中**实码 5 项**(裁剪排序/花字样式/冻结帧/字幕烧录/工程可精修),整体 ≈ 6/15 = 40%。**这不是倒退,是把地基从沙地换成岩层**——V1 的门禁体系恰恰保证了我们能诚实度量这件事。

---

## 6. V2 迭代路线(M8–M13)

> 沿用 V1 门禁纪律:每指标五元组(判定命令/阈值/退出码/阻断性),**不得为通过门禁放宽阈值**;每阶段结束跑全量 M0–M(M) 回归。排序逻辑:**信任 → 链路 → 编辑 → 渲染 → 体验 → 发布**——正确性不修,后面全部是沙上建塔。

### M8 · 止血与信任修复(最高优先,~5-8 人日)

**目标**:消灭全部 P0;把"纸面达成"降级为"实码达成";让每个返回 Ok 的操作语义为真。

交付与门禁(全部阻断):
1. **文件级 Op 撤销语义**(修 P0-1):undo/redo 按 `target.file` 路由——project.json 走现有指针回写;notes.json 回滚 NotesStore;cutlist.json 走文件逆写;重定位类 Op 不入撤销栈。
   - 门禁 M8-1 `undo_file_level_roundtrip`:notes_add→undo 盘面字节级还原;cut_apply→undo cutlist 还原;撤销深度=真实用户手势数。
2. **`_meta` 契约收编**(修 P0-2):迁移器剥 `_meta` 入内存旁路,`persist()` 原样回写;v2 schema 显式收录或迁移语义写入 ADR。
   - 门禁 M8-2 `open_real_cutflow_ir`:夹具直接调 CutFlow `rs_ir.py build` 生成真实 IR(含 `_meta`)→ cutforge open/migrate/roundtrip 三过。
3. **渲染三修**(P0-3/4 + sfx adelay):缓存键并入 canvas(+fps);voice 逐段(-ss/-t/adelay/volume)拼接进总线;sfx 落点 adelay。
   - 门禁 M8-3 `render_matrix_fixture`:双画幅变体互不污染(分辨率断言)+ 多 voice 段各就各位(音轨时间断言)+ sfx 起播时间断言。
4. **并发写安全**(P0-5):锁覆盖 open→apply→persist 全程或锁内 rev 重验。
   - 门禁 M8-4 `concurrent_writes_no_loss`:两线程各 N 次写,oplog 数=盘面 diff 数=2N。
5. 诚实性落地:capability-matrix 按 §5 降级重写;MCP capability_matrix 与文档同源(单一 JSON 生成两份);错误码表内化(占位符清理+CLI 对齐 5.4);orchestrate --json 白名单+python 探测(py -3→python3→python)+ 路径去硬编码。
   - 门禁 M8-5 `protocol_conformance_v2`:错误码穷举测试(含 notes_reject/resolve 失败路径)+ 6 编排工具在 Windows py-launcher 模拟下全通。

### M9 · 主链路贯通:双向同步从死代码到心跳(~8-12 人日)

**目标**:用户改文件,AI 真的知道;AI 改工程,编辑器真的看见;阶段状态与 CutFlow 缓存同口径。

1. **baseRev 快照链**:`.cutforge/bases/<rev>.json`(仅存与上版 diff,防膨胀);merge_from_disk 用真祖先——CF-001/002 从"永不触发"变为可触发。
   - 门禁 M9-1 `conflict_real_repro`:外部改 A 字段+本地改 A 字段→必须 CF-001(当前实现下该测试必红,V2 转绿)。
2. **watcher 进程接线**:MCP 常驻进程内起 watcher 线程,事件去抖→merge_from_disk→冲突落 `.cutforge/conflicts/`→HTTP 通道推 `workspace.changed` 事件。
   - 门禁 M9-2 `external_edit_visible`:外部手改 project.json 后,常驻 MCP 客户端 ≤1s 收到事件且视图一致。
3. **阶段状态同口径**:stage_status 解析 rs_run --status(或 _state 内容),脏传播 impact_for 接入提示面;rs_run S3 记录输出 hash 识别带外改写;B8 护栏识别 `schemaVersion`/轨道 id 视为手注。
4. **cut_apply 语义修复**:服务端按 cuts[].action 重算 keep/removedMs;cutlist 脏提示改为 `rs_cut --apply` + 从 S3 级联;先写文件后记账。
   - 门禁 M9-3 `cut_apply_roundtrip`:MCP 改 action → rs_cut --apply 消费 → S3 重建 → IR 反映,全程无静默丢弃。
5. 桥升版:rs_editor timeline id 回退(`V1#2`+idSource 标注);rs_gate probe 真检 gate.py;rs_oplog 半行语义对齐(截断);SKILL.md 登记四桥;两仓互加桥级集成测试(CI 各浅克隆对方跑最小冒烟)。
   - 门禁 M9-4 `bridges_ci`:双侧 CI 桥冒烟绿。

### M10 · 编辑能力对齐剪映核心(~15-25 人日,首次"会动")

**目标**:Web 壳从查看器变编辑器,覆盖剪映入门操作的 80%。

1. **本地服务化**:`cutforge serve`(内嵌 HTTP 已有骨架→升级为常驻:WebSocket 事件 + 静态托管 apps/web + 打开工程目录对话框),替代 `<input type=file>`。安全:仅 127.0.0.1+随机 token 落盘 `.cutforge/session`。
2. **操作集**(全部走 MCP 工具→Op,无旁路):选择/拖拽移动(吸附帧网格)/边缘 trim/分割(S)/删除(Delete,带波纹选项)/复制粘贴/多轨管理/撤销重做(Ctrl+Z/Y + AI 批量撤按钮)/磁吸开关(默认开,对齐剪映心智)。
   - 门禁 M10-1 `edit_ops_e2e`:浏览器自动化(webapp-testing 技能)跑"导入 CutFlow 真实工程→分割→移动→波纹删→undo 全还原→redo"脚本,OpLog 与盘面全程一致。
3. **标注 UI 闭环**:时间轴点选→标注框→AI 处理(经 MCP)→回执气泡;孤儿面板可视。
4. **差异面板升级**:按 actor 过滤(AI 改了什么)、逐字段 before/after、一键撤销该批。
   - 门禁 M10-2 `note_loop_ui`:3 条标注创建→AI 执行→回执,浏览器内全链 ≤5s。
5. **IR v3 预研决策**(决策 D2):keyframes 数组(属性×时间点×插值)进 schema;本阶段只做 position/scale/opacity/volume 四属性,V2 不做曲线编辑器。

### M11 · 渲染追平:矩阵从纸面到实码(~12-20 人日)

**目标**:§5 表右列全绿,每项有专属对拍夹具。顺序按依赖:转场(xfade 链,复用 CutFlow 尾帧扩展法)→ 变速(setpts/atempo+曲线贝塞尔)→ overlay/position/scale/rotate/punch-in(zoompan)→ BGM+ducking 侧链 → 音量/afade → 文字轨渲染 → fps 尊重+文件名消毒 → 多画幅真分叉(共享 mix/sub,仅 fork segment+encode,修注释)。
- 门禁 M11-1 `parity_matrix_full`:矩阵 15 项每项一个合成用例,cutforge 与 rs_render 双跑对拍(时长/响度/像素抽样),证据回填矩阵文档;整链重渲时旧缓存 100% 失效(RENDERER_VERSION 纪律)。

### M12 · 专业分轨(可选方向,按需启动)

达芬奇向:调色基础(Lift/Gamma/Gain+LUT 进 IR v3)+音频表(Fairlight 式电平/响度计)+ **OpenTimelineIO 导入导出**(生态互操作,OTIO 是 ASWF 标准);剪映向:曲线变速 UI、蒙版、音频分离(demux+afftdn 降噪)。**本阶段各子项独立立项,不设统一门禁,启动前须 ADR。**

### M13 · v2.0 发布

组织迁移 cutforge-app(网页人工创建)、ARL-1.0 律师复核 + **CORE-FILES 清单 + 全核心源文件 ARL-CORE 头补齐**(修 P1-8)、ACCEPTANCE 人工验收闭环(真实口播素材 S0→S11 + 3 标注验收)、Release v2.0(三平台)。

---

## 7. V2 里程碑依赖图

```
M8 止血(P0 全清+诚实性)
   │  不修 M8,后续全部作废
   ▼
M9 主链路(同步真触发/阶段同口径/桥升版)
   │
   ▼
M10 编辑能力(Web 可写)          M11 渲染追平(可并行,M9 后即可开)
   │                                │
   └────────────┬───────────────────┘
                ▼
        M12 专业分轨(可选,按需立项)
                ▼
            M13 v2.0 发布
```

---

## 8. 关键决策记录(ADR 式;正式 ADR 于各阶段立项时落库)

- **D1|文件级 Op 的撤销语义**(M8):采用"按 target.file 路由逆写"而非"文件级 Op 排除出撤销栈"。理由:排除会让"AI 加了标注又撤销"不可表达,违背审计完整性;替代方案(排除)已评估,代价是撤销深度语义复杂化。
- **D2|`_meta` 采用"迁移器剥离+回写保留"而非"schema 收编为保留字段"**(M8):`_meta` 是 CutFlow 的实现细节(含 manualEdit 护栏语义),进契约会把两仓实现耦死;旁路保留成本一行代码。
- **D3|Web 壳目标定档"剪映核心操作"而非"达芬奇分页工作流"**(M10):产品差异化在"AI 原生"(Op/标注/冲突),不在专业深度;调色/混音台留 M12 按需。
- **D4|同步架构采用"常驻 watcher 进程+事件推送"而非"每工具调用轮询"**(M9):MCP 已有 HTTP 骨架,watcher 线程化成本最低;轮询无法满足 ≤1s 可见性北极星。
- **D5|能力矩阵改为"生成物"**(M8):矩阵由对拍夹具结果生成(而非手写),文档/MCP 工具/门禁三方同源,杜绝再次虚标。

## 9. 风险登记(V2 增量)

| # | 风险 | 缓解 |
|---|---|---|
| V2-R1 | undo 路由改动触及核心,可能破坏既有 77 测试 | M8-1 先写红测试再改;全量回归门禁 |
| V2-R2 | watcher 线程与锁的竞态(Windows 文件事件语义) | 事件仅触发 merge(读),写仍走锁;沿用 rev 判真相 |
| V2-R3 | Web 编辑器范围蔓延(素材库/特效库诱惑) | D3 定档;每操作必须有 MCP 工具对应,无工具不做 UI |
| V2-R4 | baseRev 快照链膨胀 | 只存 diff;zstd 可选;上限+LRU 淘汰 |
| V2-R5 | OTIO 依赖引入违背"最小依赖"纪律 | M12 独立 ADR 论证后再引 |

## 10. 立即行动清单(M8 Day 1)

1. `git checkout -b fix/m8-trust` ;2. 写 M8-1 红测试(复用本文 §1 的验证例程);3. 修 engine.rs undo 路由;4. 矩阵降级重写;5. 其余 P0 依次。**全部 P0 修完前,禁止任何新功能 commit 进入 main。**

---

## 附录 A · 竞品调研来源

- DaVinci Resolve:[官方 Cut 页(磁吸时间线/智能指示器)](https://www.blackmagicdesign.com)、[Resolve 19 New Features Guide(官方 PDF)](https://documents.blackmagicdesign.com)、[Larry Jordan: Cut vs Edit 页实践](https://larryjordan.com)
- 剪映/CapCut:[CapCut 官方教程(音频分离/降噪/淡入淡出)](https://www.capcut.com)、[TechBang 剪映教学(AI 字幕/粗剪/转场)](https://www.techbang.com)、[知乎:剪映从入门到精通(关键帧/蒙版/变速)](https://zhuanlan.zhihu.com)、[剪映功能全面介绍 2026-09](https://www.womenofchina.com)
- 开源参照(常识性参考,未逐条核验):Kdenlive/Shotcut(桌面时间线交互)、Olive(关键帧曲线)、LosslessCut(无重编码剪切)、OpenTimelineIO/ASWF(交换格式)、auto-editor(粗剪语义,CutFlow 已引)

## 附录 B · V1 已达成且经本次复核仍站得住的资产

双端契约对拍与迁移幂等(M1)/ 12,000 组合并零静默覆盖(M3,merge 模块本身正确,问题在调用面)/ 沙箱零逃逸(M4)/ wasm gzip 497KB+跨壳等价(M5)/ 响度对拍 0.02LU 与字幕烧录链(M6,基础链真实)/ CI 与三平台发布(M7)/ docs/FLOW.md 流程地图与 ADR 0033-0039。**V2 是在真实地基上加固,不是推倒重来。**
