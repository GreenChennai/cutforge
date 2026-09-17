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
| M4 | MCP 与脚本(27 工具/沙箱/桥脚本) | ⬜ 下一步 | `gate.py M4` |
| M5 | 多端壳(wasm/Web/GPUI 桌面) | ⬜ | — |
| M6 | 渲染后端(七步管线/能力对等矩阵) | ⬜ | — |
| M7 | 开源发布(非 Fork/CI 全绿/律师复核) | ⬜ | — |

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
│   └── mcp-tools.json          MCP 27 工具契约(M4 填 schema)
├── crates/
│   ├── cutforge-schema/        契约层(叶子):include_str! 嵌入五 schema + draft-07 子集校验引擎 + v1→v2 迁移器
│   ├── cutforge-core/          内核(ARL-CORE):不碰文件系统、不调 ffmpeg
│   │   ├── model.rs            Project/Track/Clip 领域模型 + 唯一性/重叠不变量 + id 生成
│   │   ├── engine.rs           Engine:query(纯投影) 与 apply/undo/redo(唯一写入口) 分离;record_file_change;replay;rebuild_undo_stack
│   │   ├── command.rs          Command 六种 + ClipPatch 字段级变更派生
│   │   ├── oplog.rs            Op/OpLog(opId 去重、request_id 幂等、tail 过滤)
│   │   ├── merge.rs            三路合并九行判定表(CF-001/002/003)
│   │   ├── anchor.rs           锚点五类 + 重定位三规则(跟随→重挂→orphan)
│   │   ├── notes.rs            NotesStore(创建/结案回执绑 opIds/重定位联动)
│   │   └── timeutil.rs         RFC3339/紧凑日期(全仓唯一日期算法)
│   ├── cutforge-io/            IO 层:原子写唯一落盘点 + 锁/备份/探测/轮询 watcher + Workspace 编排 + stage.rs 脏传播
│   └── cutforge-cli/           CLI(lib+bin):查询/命令/撤销/OpLog/标注/冲突 + check-write-paths/check-deps 判定器
├── tools/
│   ├── gates/gate.py           ★ 统一门禁入口(M0–M3 已注册)
│   ├── gen_constants.py        常量生成器(--check 零漂移)
│   ├── schema_gen.py           生成 tools/_generated/cf_validate.py(Python 校验器)
│   └── validate_regression.py  回归集校验入口(16/16)
├── tests/regression/           三类 videoType 样本(各 5 文件)
└── .github/workflows/gate.yml  CI(跨仓检查拉 CutFlow;rust job 有 crates 才启用)
```

## 四、工程目录契约(CutFlow 既有 + CutForge 增量)

```
<工程>/
├── 00_brief/ 01_materials/ 02_sensed/ 03_assets/artboard/
├── 04_cut/          cutlist.json / cutlist.applied.json / rebuild.py
├── 05_ir/           project.json / wordline.json / rebuild.py
├── 06_output/       final_*.mp4 / subtitles.ass / rebuild.py
├── _state/          backup/<ts>/   ← 每次覆写前的备份
├── notes.json       ★ 标注(人的意图,进 git)
└── .cutforge/       ★ 同步与审计(可整目录删除重建,不进 git)
    ├── oplog/YYYYMMDD.jsonl   追加式操作日志(按天切分)
    ├── rev                    单调修订号
    ├── lock                   写锁(pid+时间戳,过期可接管)
    └── conflicts/             冲突三方快照(CF-*)
```

## 五、核心流程

### 5.1 唯一写入路径(任何写入者都走这条,计划书 4.2 八步)

```
CLI / 未来的 MCP / 编辑器
  └─► Workspace::apply(cmd, actor, opts)                    [cutforge-io]
        1. 申请工程锁(atomic::create_exclusive,过期接管)      [lock.rs]
        2-3. Engine::apply:baseRev 前置校验 → 变更 → schema+重叠不变量
             (失败即回滚快照,拒绝码 Reject)                    [engine.rs]
             before==after → 幂等短路(不升 rev 不产 Op)
        4. Op 追加 .cutforge/oplog/<日>.jsonl(append-only)     [persist]
        5-6. 备份旧 project.json → atomic_write 原子替换       [atomic.rs ★唯一落盘点]
        7. rev 落盘;标注锚点重定位联动(notes 有变则落盘+留痕)  [sync_notes_after_change]
        8. 释放锁
```

### 5.2 双向同步

- **AI/脚本改动 → 编辑器可见**:`apply` 落盘后,编辑器(现阶段的 CLI/未来 UI)重开 Workspace + `Query::Timeline/ProjectView` 即见;基准 P95 9ms(阈值 100ms)。
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

## 六、契约与单源体系(杜绝双栈漂移)

```
schemas/*.json(唯一手写)
  ├─► cutforge-schema(Rust 引擎 + include_str! 嵌入 + build.rs 存在性闸)
  └─► tools/schema_gen.py → tools/_generated/cf_validate.py(纯 stdlib;勿手改)
对拍:回归集(3 类 videoType×5 文件)双端结论逐样本一致;迁移器两端输出语义相等且幂等。
常量:rs_common.RATIOS + platforms.json → gen_constants.py → constants.ratios.json(--check 零漂移)。
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

结果协议:`{"ok","code","message","data"}`;退出码 0 通过 / 2 门禁失败 / 3 环境缺失 / 4 内部错误。

## 八、观察项与已知占位(诚实清单)

1. **回归样本是构造的**(9-14 磁盘清理后无现网工程):待下一真实工程用真实产物替换 `tests/regression/` 并复跑。
2. **baseRev 快照链占位**:`merge_from_disk` 目前以"本地充当祖先",完整三方快照链在引入编辑器(M5)时落地——门禁不依赖它。
3. **CI 远端全绿**需推送后在 GitHub Actions 确认(M0-5 的远端半边)。
4. **ARL-1.0 发布前须执业律师复核**(计划书附录 A 免责条款)。
5. watcher 为轮询基础版(M2 决策);M4/M5 若接 notify crate 须先补 ADR(第三方依赖纪律)。
