# FLOW · CutForge 整体流程与工作区梳理

> 本文件是项目的**流程总览与工作区地图**:谁写什么、经过哪里、被谁校验。
> 决策依据见 CutFlow 仓 `docs/adr/0033~0038`;契约细节见 `schemas/`。

## 一、项目定位与分工

- **CutForge**(本仓,Rust):编辑器内核 + 多端接入。管"人的手"——交互、预览、标注、精确编辑。
- **CutFlow**(Python,`E:\平日资料\GitHub\CutFlow`):S0–S11 视频管线。管"机械臂"——转写对齐、粗剪、合成、字幕、烧录、自检。
- 两者**读写同一份工程文件**,通过文件系统同步,不共享进程、不共享实现。

## 二、里程碑路线(门禁制,全绿才许前进)

| 里程碑 | 内容 | 状态 | 门禁入口 |
|---|---|---|---|
| M0 | 立项与合规(许可三件套/命名/瘦身/CI 骨架) | ✅ | `gate.py M0` |
| M1 | 契约固化(五份 schema/双端生成/常量单源/迁移器) | ✅ | `gate.py M1` |
| M2 | Rust 内核(模型/命令通道/撤销栈/OpLog/IO/CLI) | ✅ | `gate.py M2` |
| M3 | 双向同步与标注(合并/冲突/notes/阶段脏传播/延迟) | ✅ | `gate.py M3` |
| M4 | MCP 与脚本(28 工具双通道为 M4 时点;数量现以 schemas/mcp-tools.json 为准/批式脚本沙箱/桥脚本) | ✅ | `gate.py M4` |
| M5 | 多端壳(wasm/Web/GPUI 桌面) | ⬜ | — |
| M6 | 渲染后端(七步管线/能力对等矩阵) | ✅ 经 M11 渲染追平(必达 13/13 实码) | `cargo test -p cutforge-render` |
| M7 | 开源发布(非 Fork/CI 全绿/律师复核) | ✅ 已发布 v0.1.0 | `gate.py M7` |
| **V2** | **M8 止血 → M9 主链路 → M10 编辑 → M11 渲染追平 → M13 发布** | ✅ 台账见 [V2-PROGRESS.md](V2-PROGRESS.md) | 见 [ITERATION-PLAN-v2.0.md](ITERATION-PLAN-v2.0.md) |

## 三、仓库布局(每个部件一句话)

```
cutforge/
├── schemas/                    ★ 契约唯一手写真相源
│   ├── project.schema.json     项目 IR v2(version 恒 1 + schemaVersion "2.0.0")
│   ├── wordline.schema.json    字级时间轴(全片时间唯一真相源)
│   ├── cutlist.schema.json     粗剪决策(x-removeRequiresGuardOk/x-keepCoversTimeline 断言)
│   ├── notes.schema.json       时间轴标注(锚点+人机对话)
│   ├── oplog.schema.json       操作日志(append-only,baseRev 必填)
│   ├── constants.ratios.json   生成物:比例/平台/帧率单源(勿手改)
│   ├── mcp-tools.json          MCP 工具契约(唯一手写面;数量以 tools 数组为准,B7)
│   └── ui-fields.json          壳可编辑字段集单一真相源(E4-2;check-ui-fields 机械校验 ⊆ ClipPatch)
├── crates/
│   ├── cutforge-schema/        契约层(叶子):include_str! 嵌入五 schema + draft-07 子集校验引擎 + v1→v2 迁移器
│   ├── cutforge-core/          内核(ARL-CORE):不碰文件系统、不调 ffmpeg
│   │   ├── model.rs            Project/Track/Clip 领域模型 + 唯一性/重叠不变量 + id 生成
│   │   ├── engine/             Engine 五模块(册一 T1.3 拆分):apply(唯一写入口)/undo/replay/projection/invariants
│   │   ├── command.rs          Command 六种 + ClipPatch 字段级变更派生
│   │   ├── oplog.rs            Op/OpLog(opId 去重、request_id 幂等、tail 过滤)
│   │   ├── merge.rs            三路合并九行判定表(CF-001/002/003)
│   │   ├── anchor.rs           锚点五类 + 重定位三规则(跟随→重挂→orphan)
│   │   ├── notes.rs            NotesStore(创建/结案回执绑 opIds/重定位联动)
│   │   └── timeutil.rs         RFC3339/紧凑日期(全仓唯一日期算法)
│   ├── cutforge-io/            IO 层:原子写唯一落盘点 + 锁/备份/探测/轮询 watcher + stage.rs 脏传播 + scaffold.rs 空工程模板(B11);Workspace 编排在 workspace/ 子模块(册一 T1.2:open/apply/persist/query/sync/bases/conflicts/notes,apply 为显式步骤函数管线)
│   ├── cutforge-cli/           CLI(lib+bin):查询/命令/撤销/OpLog/标注/冲突 + cache(缓存治理)/doctor(环境诊断) + check-write-paths/check-deps 判定器
│   ├── cutforge-mcp/           MCP 层:单注册表(数量以 schemas/mcp-tools.json 为准),registry/dispatch/workspace_svc + transport/{stdio,http,events,static_files};stdio 主通道 + 内嵌 HTTP 辅通道(127.0.0.1+token)共用同一 dispatch
│   └── cutforge-script/        脚本宿主:cutforge-script-v1 批式步骤 + 策略沙箱(白名单/路径/步数/超时,逃逸面结构性为零)
├── apps/web/                   壳:core/render/panels/ui 四层无构建 ESM(SSE 主通道 + 长轮询断线降级;data-testid 锚点登记 TESTIDS.md;旧壳 legacy/ 已于册三收尾删除,回退期结束;ADR-0011~0014)
├── tools/
│   ├── gates/gate.py           ★ 统一门禁入口(M0–M7 与册级 A1/A2/A3 已注册,决策 D-A2)
│   ├── bench/bench.py          性能基准(T1.8;--check 阈值判定,基线 docs/bench/baseline.json)
│   ├── bench/tool_parity.py    工具黄金响应库对拍(册一 AC-1.2 建 41;册二 A2 增 render_frame 后 42;册四 A4 增六编辑工具后 48;数量以 schemas/mcp-tools.json 为准)
│   ├── gen_constants.py        常量生成器(--check 零漂移)
│   ├── schema_gen.py           生成 tools/_generated/cf_validate.py(Python 校验器)
│   └── validate_regression.py  回归集校验入口(16/16)
├── tests/regression/           三类 videoType 样本(各 5 文件)
└── .github/workflows/gate.yml  CI(跨仓检查拉 CutFlow;rust job 有 crates 才启用)
```

## 四、工程目录契约(目录契约 v2,0.5.0 起中文化)

单一真相源:`crates/cutforge-io/src/paths.rs`(与 CutFlow 侧 `rs_paths.py`,ADR-0046 同构)。
**文件名全部 ASCII 不变**;`artboard` 子目录保留英文;工程根 `notes.json` 与 `.cutforge/` 不动。

```
<工程>/
├── 00_制作简报/
├── 01_原始素材/            (只读语义不变)
├── 02_转写与校对/
├── 03_创作素材/artboard/   ← artboard 子目录保留英文
├── 04_粗剪决策/            cutlist.json / cutlist.applied.json / rebuild.py
├── 05_时间线工程/          project.json / wordline.json / rebuild.py
├── 06_成片输出/            final_*.mp4 / subtitles.ass / rebuild.py
├── 成品/                   ★ 交付区(0.5 新增;NEVER_CLEAN,任何清理不得触碰)
├── _内部状态/              backup/<ts>/   ← 每次覆写前的备份;阶段记账 S*.json
├── notes.json              ★ 标注(人的意图,进 git)
└── .cutforge/              ★ 同步与审计(可整目录删除重建,不进 git)
    ├── oplog/YYYYMMDD.jsonl   追加式操作日志(按天切分)
    ├── rev                    单调修订号
    ├── lock                   写锁(pid+时间戳,过期可接管)
    ├── bases/                 baseRev 快照链(LRU 上限 32,M9-1)
    ├── session                serve 会话 token
    ├── conflicts/             冲突三方快照(CF-*)
    └── render-cache/          渲染中间产物缓存(册一 T1.5 起内容寻址)
        ├── {seg,mix,compose,overlay,sub}/   五层键值产物
        └── cache-index.json   条目键/大小/时间清单(gc 与调试用)
```

**兼容口径**:0.4.x 旧布局(`00_brief`/`01_materials`/`02_sensed`/`03_assets`/`04_cut`/
`05_ir`/`06_output`/`_state`)的既有工程**原地读写、不自动迁移**;
新建工程(`cutforge-cli new` / MCP `project_new`)一律产新布局。

## 五、核心流程

### 5.1 唯一写入路径(任何写入者都走这条,计划书 4.2 八步)

```
CLI / MCP / 编辑器(同一命令通道)
  └─► Workspace::apply(cmd, actor, opts)                    [cutforge-io]
        1. 申请工程锁(atomic::create_exclusive,过期接管)      [lock.rs]
        2-3. Engine::apply:baseRev 前置校验 → 变更 → schema+重叠不变量
             (失败即回滚快照,拒绝码 Reject)                    [engine/apply.rs]
             before==after → 幂等短路(不升 rev 不产 Op)
        4. Op 追加 .cutforge/oplog/<日>.jsonl(append-only)     [persist]
        5-6. 备份旧 project.json → atomic_write 原子替换       [atomic.rs ★唯一落盘点]
        7. rev 落盘;标注锚点重定位联动(notes 有变则落盘+留痕)  [sync_notes_after_change]
        8. 释放锁
```

> **实码序说明(册一 T1.2 拆分时声明)**:上图为计划书 4.2 的理想序;实码是
> `crates/cutforge-io/src/workspace/apply.rs` 的显式步骤函数管线,**先文件后记账**
> (P1-9:新内容先落盘,再执行"备份 → 原子写 → OpLog 追加 → rev 落盘")——写失败时
> oplog 尚未记账,崩溃恢复的 rev/oplog 对账依赖该实序,故不按理想序重排(偏差已在
> `apply.rs` 模块注释声明)。

### 5.2 双向同步

- **AI/脚本改动 → 编辑器可见**:`apply` 落盘后,编辑器(CLI / Web 壳)经 `/events`(SSE,§5.6)感知,或重开 Workspace + `Query::Timeline/ProjectView` 即见;基准 P95 9ms(阈值 100ms)。
- **用户改动 → AI 感知**:所有改动都带 `actor` 落在 OpLog;`Query::OpLogTail {since_rev, actor_kind}` 过滤读取。
- **外部改动**(编辑器直接改文件):`merge_from_disk()` 三路合并(祖先/磁盘/内存)→ 可合并则采纳(**保留 OpLog/rev 历史**)→ 冲突则写 `.cutforge/conflicts/` 三方快照并停写,**禁止选边**。
- 撤销/重做:每个 Op 的逆 = before↔after 互换;undo/redo 本身也产生新 Op;`Engine::replay` 从日志重建任意状态(回放 hash 等价有门禁)。

### 5.3 标注生命周期(4.9)

```
用户 notes-add(anchor+body, state=open)        → notes.json + Op(insert)
  → AI notes_list/OpLog 读到 → apply 改动(--caused-by n-XXXX)
  → AI notes-resolve(reply + opIds, state=resolved) → 结案回执可回看
元素位移 → 锚点跟随(吸附进元素);id 消失 → ≤500ms 重挂最近元素;再不行 → state=orphan(显式保留,面板可见)
拒绝走 notes-reject(reason)。
```

### 5.4 阶段脏传播(4.10;只标记不重跑)

| CutForge 改了 | 变脏起点 | 提示执行 |
|---|---|---|
| 05_ir/project.json | S3(下游 S3–S11) | `python 05_ir/rebuild.py` + B8 护栏提示 |
| 06_output/subtitles.ass | 仅 S8 | `python 06_output/rebuild.py`(不碰 IR) |
| 04_cut/cutlist*.json | S2(级联) | `python 04_cut/rebuild.py` |
| 03_assets/artboard/** | S4(下游) | `python 03_assets/artboard/rebuild.py` |
| 05_ir/wordline.json | S2(级联) | `python rebuild.py --from S2` |
| 其他文件 | 不标脏 | — |

### 5.5 结果协议错误码表(计划书 5.4;T1.7 命名空间化)

结果协议 `{"ok","code","message","data"}` 为 CLI/MCP/HTTP 三面共用;T1.7 起错误码带
**命名空间加法维度 `ns`**——由单一真相源 `crates/cutforge-mcp/src/registry.rs` 的 `CODE_NS`
表派生,三面同源取值;**既有 code 取值逐字不变**(线上断言兼容红线):

| code | ns | 语义 |
|---|---|---|
| OK | ok | 成功(非错误,单列) |
| CONFLICT | core | 编辑冲突:合并冲突 / rev 前置不满足 |
| SCHEMA_INVALID | core | schema 契约校验失败 |
| PRECONDITION_FAILED | core | 命令前置校验失败:缺参 / 对象不存在 |
| GUARD_FAILED | core | 内核守护拒绝:密度 / 跨 kind / keep 守卫 |
| JIANYING_RUNNING | mcp | 编排面:剪映占用工程 / 草稿 |
| NO_CONFIG | io | 工程 / 文件 / 环境资源不存在 |
| DEP_MISSING | io | 外部依赖缺失(ffmpeg/ffprobe/CutFlow 环境) |
| GREEN_SCREEN_INPUT | render | 绿幕输入校验失败 |
| INTERNAL | mcp | 协议面内部错误(未知工具 / 未实现分支 / 意外失败) |

- 命名空间口径:`io.*` 文件系统 / 环境资源;`core.*` 内核编辑语义(命令 / 契约 / 守护 / 合并);
  `mcp.*` 协议与编排面;`render.*` 渲染链路;`ok` 成功码。
- 三面同码:MCP/HTTP 工具面在 `registry::envelope` 统一派生 `ns`;CLI 在 `emit` 经
  `cutforge_mcp::code_namespace` 同源取值。表外码(CLI 门禁判定器专用码)诚实派生为
  `unknown`,不冒充表内命名空间;新增码必须先登记 `CODE_NS`(单测锁定与 `CODES` 同序同值)。

### 5.6 事件面、静态托管与渲染缓存(T1.5/T1.6;ADR-0009)

- **事件推送**:`/events` 升级为 **SSE**(`text/event-stream`);事件面从「仅 project.json」
  扩展到 notes.json / cutlist.json 外部改动 + `render.progress`(渲染结构化进度,
  RenderPlan 步骤边界发数)。复用 cutforge-io watcher(SyncHub),只扩发布面(补 M9-R4)。
  册二起壳以 SSE 为主通道,长轮询(`?since=`)转为**断线降级路径**(AC-2.6③ 实测:
  SSE 毒化后壳自动转长轮询);**册三定夺(A1-L3 / A2-L5 了断):保留为正式降级面**——
  壳顶 `conn-badge` 以 polling 态可视降级中,e2e_events 降级断言保留。
- **静态托管**:`/assets/*` 目录映射 `apps/web/`(前缀白名单 + canonicalize 穿越防护 +
  MIME 表 + ETag/304);此前 4 条硬编码路径白名单已删除,前端新增文件**零 Rust 改动**。
- **渲染缓存**:`.cutforge/render-cache/{seg,mix,compose,overlay,sub}/` 五层**内容寻址**
  (键 = hash(输入 spec)+RENDERER_VERSION+相关画幅/帧率),`cache-index.json` 记录
  条目键/大小/时间;治理走 `cutforge-cli cache {info,gc,clear}`——LRU + 容量上限
  (默认 10GB),清单外孤儿与 tmp 件 **24h 超龄即清**(新于 24h 视为并发在写,不动)。
- **连接纪律**:读超时 / 请求体上限 / 总请求时限 / 显式 `Connection: close`(SSE 流除外)
  以具名常量落码;`crates/cutforge-mcp/tests/http_hardening.rs` 断言「一条挂死连接
  不阻塞其余请求、超时后连接被回收」。

## 六、契约与单源体系(杜绝双栈漂移)

```
schemas/*.json(唯一手写)
  ├─► cutforge-schema(Rust 引擎 + include_str! 嵌入 + build.rs 存在性闸)
  └─► tools/schema_gen.py → tools/_generated/cf_validate.py(纯 stdlib;勿手改)
对拍:回归集(3 类 videoType×5 文件)双端结论逐样本一致;迁移器两端输出语义相等且幂等。
常量:rs_common.RATIOS + platforms.json → gen_constants.py → constants.ratios.json(--check 零漂移;ADR-0015 起 canvasAllowed=预设推荐集,合法域 = CutForge schemas/project.schema.json 的 canvas 范围约束 64–7680 偶数,生成物 canvasRange 键承载)。
跨字段断言:x-removeRequiresGuardOk(remove 刀 guard 必须真)、x-keepCoversTimeline(keep 覆盖全轴)双端同实现。
```

## 七、门禁矩阵(完成判定唯一入口;人眼不作数)

| 门禁 | 里程碑 | 关键阈值 | 状态 |
|---|---|---|---|
| naming/license/repo-size/toolchain/ci | M0 | clone ≤300MB(实测 26.5);无"停更/archived"表述 | ✅ |
| tests-layout | M0(观察) | 12 probe 归置 | ✅ |
| schemas-complete/regression-schema/constants-no-drift/cargo-schema-tests | M1 | 回归 16/16;双端对拍一致;迁移幂等 | ✅ |
| adr-unique/glossary/doc-consistency | M1 | 38 编号唯一;活文档零旧表述 | ✅ |
| core-coverage | M2 | 行覆盖 ≥80%(实测 84.6%) | ✅ |
| roundtrip-semantic-eq / undo-redo-roundtrip | M2 | 语义 diff=0;全 undo 回初始 | ✅ |
| write-paths / wasm-core / deps-direction | M2 | 旁路写入=0(仅 atomic.rs);wasm32 构建;违规边=0 | ✅ |
| sync-latency | M3 | AI 可见 P95 ≤100ms(实测 ~9ms) | ✅ |
| merge-property | M3 | 12,000 组零静默覆盖 | ✅ |
| oplog-replay / merge-table / notes-anchor / workspace-rebuildable / stage-dirty / e2e-note-cli | M3 | 各自测试全绿 | ✅ |
| mcp-tools / mcp-e2e-visible / mcp-note-loop / sandbox-escape / protocol / bridge-doctor | M4 | 28 工具双 schema 齐备(M4 时点;阶段二起 38,以 schemas/mcp-tools.json 为准);双通道一致;≤1s/≤3s 闭环;逃逸=0;协议 ∈5.4 表;桥 4/4 登记 | ✅ |

结果协议:`{"ok","code","message","data"}`;退出码 0 通过 / 2 门禁失败 / 3 环境缺失 / 4 内部错误。
CI 只跑 M0+M1(跨仓检查拉 CutFlow;M2-M6 依赖本机 ffmpeg/wasm-pack/llvm-cov/CutFlow 工程,为本地阻断项——CI 上缺依赖会以退出码 3 如实暴露,不会误报通过)。

### 7.1 V2 门禁现状与册级门禁(D-A2 口径,明文说明)

`tools/gates/gate.py` 注册了 **M0–M7**;**V2 的 M8–M13 不在 gate.py 里**,它们以
cargo 测试与 e2e 脚本形式存在,由 `cargo test --workspace` 与 CI 的 web-e2e job 承载:

| 里程碑 | 门禁载体(测试/脚本名) |
|---|---|
| M8 渲染管线 | `cutforge-render` 单元/集成测试(`cargo test -p cutforge-render`) |
| M9 主链路(合并/快照/守护) | `cutforge-io` m9_tests(`conflict_real_repro_and_stop_writes` 等)、`external_edit_visible_within_1s` |
| M10 编辑器 e2e | `tools/e2e_edit_ops.py`(M10-1/M10-2/M10-3)、`tools/e2e_preview.py`、`tools/e2e_from_zero.py`(册二起三份迁 `data-testid` 选择器;另新增 `e2e_ui_smoke.py` / `e2e_playback_survival.py` / `e2e_perf_timeline.py`,载体见册级 A2;册三再增 `e2e_drag_perf.py` / `e2e_hotkeys.py` / `e2e_a11y.py` / `e2e_perf_budget.py`,载体见册级 A3) |
| M11 渲染矩阵 | `crates/cutforge-render/tests/parity_matrix.rs`(九项实渲对拍)、`capability-matrix.json` 同源断言 |
| M12/M13 | ADR-0005 暂缓决议 + 发布验收(ACCEPTANCE.md);无自动化门禁 |

即:**V2 里程碑的完成判定 = `cargo test --workspace` 全绿 + 三份 e2e 全绿(V2 时点口径;
此后册一/册二/册三把 e2e 面扩至十二份)+ gate.py M0/M1**;
不新增 gate.py 里程碑注册(避免双份判定器漂移,与"工具数量以 schemas/mcp-tools.json 为准"同一纪律)。

**册级门禁注册制(册一起,D-A2)**:多册计划的验收以 `gate.py A<n>` 聚合注册,避免六册后
验收碎片化。已注册 **`gate.py A1`**(册一),七项:cargo 全绿 / clippy -D warnings /
非测试源文件 ≤800 行 / tool_parity 黄金对拍 / e2e_static / e2e_events / bench --check
(末项为观察项)。已注册 **`gate.py A2`**(册二,前端壳重写),**12 项 = 11 阻断 + 1 观察**:
cargo 全绿 / clippy / js 行数红线(js 单文件 ≤400、index.html ≤120)/ shell-purity-v2
(R1 持久化禁令/R2 投影只读/R3 禁裸 fetch/R5 色值;R4 legacy 豁免已随 legacy/ 删除收口)/ tool_parity(42,时点口径)/
e2e_events / e2e_static / e2e_ui_smoke / e2e_playback_survival / e2e_perf_timeline
(`--min-fps` 参数化)/ pytest-suite。legacy-reminder 观察项已随 legacy/ 删除移除
(册三收尾,A2-L2 了断)。已注册 **`gate.py A3`**(册三,UX 动效/键位/可访问性),15 项全阻断:
cargo 全绿 / clippy / js 行数红线 / shell-purity(v3 含 R5)/ pytest / tool_parity(42,时点口径)/
e2e_static / e2e_events / e2e_ui_smoke / e2e_playback_survival / e2e_drag_perf(`--min-fps 55`,
安静时段复跑口径)/ e2e_hotkeys / e2e_a11y(axe 扫描,登记违规见脚本)/ e2e_perf_budget /
e2e_perf_timeline(负载敏感项均不进 CI);载体 tools/e2e_{drag_perf,hotkeys,a11y,perf_budget}.py。CI 只跑 M0/M1 + web-e2e(含册二 ui_smoke/playback_survival + 册三 hotkeys/a11y 四步;perf/拖拽帧率负载敏感不进 CI),A<n> 本机册收官跑;台账见 [A1-PROGRESS.md](A1-PROGRESS.md) /
[A2-PROGRESS.md](A2-PROGRESS.md) / [A3-PROGRESS.md](A3-PROGRESS.md)。

## 八、观察项与已知占位(诚实清单)

1. **回归样本是构造的**(9-14 磁盘清理后无现网工程):待下一真实工程用真实产物替换 `tests/regression/` 并复跑。
2. ~~baseRev 快照链占位~~ **已闭合(M9-1)**:`.cutforge/bases/` 快照链已落地(LRU 上限 32),冲突触发的真实窗口与三方合并口径见 V2-PROGRESS M9 架构决策。
3. **CI 远端全绿**需推送后在 GitHub Actions 确认(M0-5 的远端半边)。
4. **ARL-1.0 发布前须执业律师复核**(计划书附录 A 免责条款;ADR-0010 起定位转向个人自用/闭源/非商业,该义务挂起至恢复对外公开发行)。
5. watcher 为轮询基础版(M2 决策);M4/M5 若接 notify crate 须先补 ADR(第三方依赖纪律)。
