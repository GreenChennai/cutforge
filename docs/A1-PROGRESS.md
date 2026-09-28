# A1 册一进度总账(内核重构与架构加固)

> 执行依据:《CutForge-迭代计划 · 01-册一-内核重构与架构加固.md》+ 总纲 §1.3 全局纪律。
> 本文件沿用 [V2-PROGRESS.md](V2-PROGRESS.md) 的滚动台账惯例:复盘发现的问题**强制折入
> 下一册任务单**(总纲 §1.3.5);每指标可判定、可存档,不得为通过而放宽;未完成项如实写
> 「进行中 / 未达标」。
> 批次提交:128856c(T1.1–T1.3)→ 3ce2884(T1.4–T1.6/T1.9/T1.10)→ 7157178(T1.7/T1.8/D-A2)
> → 文档同步批(ADR-0010 / CHANGELOG / README / FLOW 清扫 / 本台账)。

## 一、十任务逐项

| 任务 | 做了什么 | 关键证据 | 落点 |
|---|---|---|---|
| T1.1 拆分 mcp 巨石 | `lib.rs` 2,146 行 → registry / dispatch / orchestrate / progress / session / tools_nolock / workspace_svc + `transport/{stdio,http}`;旧 API `pub use` 重导出,CLI 零改动 | protocol_conformance 全绿;tool_parity 41/41 | `crates/cutforge-mcp/src/*` |
| T1.2 拆分 io 巨石 | `lib.rs` 1,120 行 → `workspace/{open,apply,persist,query,sync,bases,conflicts,notes}`;apply 编译为显式步骤函数管线(step1~7 具名函数),实码序「先文件后记账」在模块注释声明 | workspace 内联测试迁移保全绿;行数红线内 | `crates/cutforge-io/src/workspace/*` |
| T1.3 拆分 engine 巨石 | `engine.rs` 1,060 行 → `engine/{apply,undo,replay,projection,invariants,mod}` 五模块;领域语义零变化 | `file_level_undo` / `oplog-replay` / `merge-property` 全绿 | `crates/cutforge-core/src/engine/*` |
| T1.4 render() 分解 | ≈440 行单函数 → `plan.rs`(纯函数计划,收集段清单)+ `steps.rs`(命令行生成纯函数)+ 执行器;七步 probe → segment → compose-video → overlay → mix → subtitle → encode,产出结构化 `StepReport`(含 `to_progress()` 供 SSE) | steps.rs 内联单测 18 个、plan.rs 5 个;parity_matrix 九项实渲全绿 | `crates/cutforge-render/src/{plan,steps,lib}.rs` |
| T1.5 缓存体系统一 | composed/overlaid/subbed 固定文件名废除 → 五层内容寻址目录 + `cache-index.json` 清单;`cutforge-cli cache {info,gc,clear}`(LRU + 默认 10GB 上限;清单外孤儿/tmp 件 24h 超龄即清) | **改 clip 后同键必 miss 实测**(cache_addressing);gc 容量/孤儿单测 | `crates/cutforge-render/src/cache.rs`、`crates/cutforge-cli/src/cache.rs`、`tests/cache_addressing.rs` |
| T1.6 HTTP/事件层升级 | `/assets/*` 目录托管(前缀白名单 + canonicalize 穿越防护 + MIME + ETag/304,4 条硬编码路径删除);`/events` 升级 SSE,事件面扩 notes/cutlist/render.progress,长轮询降级保留;连接纪律(读超时/体上限/总时限/慢连接隔离);D-A1 决策=std 手工加固(ADR-0009) | 12 条穿越路径 100% 拒绝;ETag 304;SSE P95 73–81ms / 长轮询 254–262ms;挂死连接隔离 | `crates/cutforge-mcp/src/transport/*`、`tools/e2e_static.py`、`tools/e2e_events.py`、`docs/adr/0009` |
| T1.7 错误模型与诊断 | 结果协议新增加法维度 `ns`(`CODE_NS` 单一真相源,三面同码;既有 code 逐字不变);`cutforge-cli doctor` 六项检查(工程/ffmpeg/ffprobe/web 资源/缓存可写/端口),失败必给可复制修复命令(单测锁定) | FLOW §5.5 码表;doctor「失败必须带 fix」单测 | `crates/cutforge-mcp/src/registry.rs`、`crates/cutforge-cli/src/doctor.rs`、`docs/FLOW.md` §5.5 |
| T1.8 性能基线 | `tools/bench/bench.py`:1k clips / 8 轨合成工程,open/query/apply/undo/render 五项,`--check` 阈值判定 + 对 baseline 劣化 >20% 阻断;基线落盘 | 首跑报告 `docs/bench/2026-09-27.json`;基线 `docs/bench/baseline.json`(render 6586.1ms,min 口径) | `tools/bench/bench.py`、`docs/bench/*` |
| T1.9 契约工作流文档化 | 新增 IR 字段的标准七步流水线说明书(schema → 双端生成 → 内核模型 → ClipPatch → ui-fields → 壳控件 → 对拍夹具),每步附实码核实的命令与门禁兜底 | 文档落库;顺带发现契约链断点(见遗留 L-1) | `docs/CONTRACT-WORKFLOW.md` |
| T1.10 变体真分叉 | render_variants 画幅无关层(seg/mix 等)全缓存共享,仅 video/encode 分叉;parity 夹具断言从「mix 仅一份」扩展到画幅无关层 | parity_matrix 变体断言扩展 | `crates/cutforge-render/tests/parity_matrix.rs`、`src/cache.rs` |

配套:gate A1 册级门禁注册(D-A2,七项)、CI 增补 parity/static/events 三步、全仓
`clippy --workspace -D warnings` 清零、README/CHANGELOG/FLOW 同步(本批)。

## 二、AC-1.1 ~ AC-1.10 状态表

| # | 验收项 | 判定方式 | 状态 | 当前实测 |
|---|---|---|---|---|
| AC-1.1 | 非测试源文件 ≤800 行;clippy 新增告警 = 0 | `gate.py A1` rust-line-limit / cargo-clippy | ✅ | 最大 **791 行**(steps.rs,7157178 时点);clippy `-D warnings` **0** |
| AC-1.2 | 41 工具拆分前后逐一对拍 | `tools/bench/tool_parity.py`(golden 响应库) | ✅ | **41/41** 逐键相等,零漂移 |
| AC-1.3 | render() 分解:步骤函数 ≥6 且有单测;parity 九项全绿 | `cargo test -p cutforge-render` | ✅ | 七步;steps.rs 18 + plan.rs 5 单测;parity 九项实渲绿 |
| AC-1.4 | 改 clip 后重渲零陈旧复用;gc 遵守容量上限 | `tests/cache_addressing.rs` | ✅ | 同键必 miss 实测;LRU/容量/24h 孤儿单测绿 |
| AC-1.5 | SSE P95 ≤200ms;长轮询降级 ≤1s | `tools/e2e_events.py` | ✅ | SSE P95 **73–81ms**;长轮询 **254–262ms**(多轮) |
| AC-1.6 | `/assets` 目录托管;穿越 100% 拒;ETag 304;零 Rust 改动可达 | `tools/e2e_static.py` | ✅ | **12 条穿越全拒**;ETag/304 生效;新增文件可达 |
| AC-1.7 | 挂死连接不阻塞其余请求;超时回收 | `tests/http_hardening.rs` | ✅ | 4 项断言(隔离/回收/413 上限)全绿 |
| AC-1.8 | 基准报告产出且阈值内 | `tools/bench/bench.py --check` | ✅ **达标** | 终测三轮全 PASS:open **5.02ms** ✓(阈 200)、query **18.58ms** ✓(阈 50)、apply **71.34ms** ✓(阈 100)、render min ≈6055–6066ms 对基线 6586.1ms 约 **−8%** ✓(不劣化);profile 定位每 RPC 重开 38.4ms / watcher 守护抢锁 47.3ms 主因,经 serve 常驻工作区复用(fingerprint 新鲜度协议,`crates/cutforge-mcp/src/resident.rs` + `crates/cutforge-io/src/fresh.rs`)+ watcher 本进程写入免开合并 + 锚点双 persist 合一收口(专项明细见 L-9) |
| AC-1.9 | 全量回归:cargo 全绿 + 三 e2e 绿 + gate M0/M1 + `gate.py A1` | §1.3 判定 | ✅ | cargo 全绿、三 e2e 绿;**gate A1 七项阻断全绿(exit 0)**;gate M0 绿;**M1 6/7**(唯一红项 adr-unique 为外部环境项 L-8,非本仓问题) |
| AC-1.10 | CONTRACT-WORKFLOW 落库;ADR 落库;check-doc-counts 过 | `python tools/check_doc_counts.py` | ✅ | CONTRACT-WORKFLOW.md、ADR-0009/0010 落库;文档同步即本批,门禁绿 |

> 口径说明:行数/实测值以 7157178 提交时点为准;AC-1.8/1.9 为性能专项收口后的终测值
> (bench `--check` 三轮全 PASS、gate A1 exit 0),按诚实性纪律如实更新为全绿。

## 三、遗留台账(强制折入下一册;总纲 §1.3.5)

| # | 发现 | 处置建议 |
|---|---|---|
| L-1 | **契约链断点**(T1.9 梳理七步链时发现):c5b509d 批次新增 9 字段内核模型零承接——`bgm.assetId`、`clip.assetId/fx/font/huazi/matte`、`motion.inFx/outFx`、顶层 `font/effects`,schema 有、内核无,读写一轮即丢;`clip_update.inputSchema.patch` 未声明 transition/motion 而 dispatch 已承接(契约面窄于实现面);`MotionPatch` 缺 inFx/outFx(与 motion 不对称);`check-ui-fields` 判定器未注册进 gate.py | 册四/册五加字段时按 `CONTRACT-WORKFLOW.md` 七步补齐;**先行项:把「字段静默丢弃」加门禁拦截**(roundtrip 后字段蒸发即红),再逐字段承接 |
| L-2 | mcp 侧跟进:`render_progress` 已有结构化 `StepReport`,但 HTTP 面尚未透传 `steps/cacheHits` 全量字段给壳 | 册三导出进度 UI 消费(StepReport.to_progress 已备好数据源) |
| L-3 | SSE 遗留:长轮询降级路径按 A1-R2 标注「**册二完成后移除**」;EvHub 在无订阅者时仍保留扫描候选,空闲自动停扫未做。**册二追记(2026-09-29)**:册二新壳以 SSE 为主通道,长轮询**未移除**,转为断线降级路径并实测(AC-2.6③ SSE 毒化→长轮询接管);「移除与否」推迟至册三,与 A2-L5(SSE 断连无 UI 指示)一并定夺 | ~~移除时同步删 e2e_events 降级断言与 FLOW §5.6 标注~~ 册三定夺去留:删则同步清 e2e_events 降级断言 + FLOW §5.6 标注;留则改标注为正式降级面;空闲停扫仍待做 |
| L-4 | `stage_status` 的 `resp.get("stdout")` 顶层 quirk(stdout 挂在结果对象顶层而非 data 内),golden 已如实锁定 | 保持 golden 锁定;若未来动结果协议须连 golden 一起重建 |
| L-5 | bench 口径两条:debug 档不可作基线(数值失真,基线只认 release);1k 工程直接塞 render 命令行不可行(命令行长度上限),渲染基准用独立迷你工程的口径已写进 bench.py | 口径不变;后续册收官照跑并落盘新日期 JSON |
| L-6 | cli(cache.rs)与 render(cache.rs)双侧各有一份缓存治理逻辑,为「依赖方向门禁」所致的契约镜像 | 未来可论证把缓存治理下沉 cutforge-io(需先调 deps-direction 门禁) |
| L-7 | seg 缓存键对 clip 做**全量哈希**:纯音频字段(如 text)变更也令视频段 miss——保守正确,不产陈旧复用 | 册二可做字段级拆分(视频键/音频键分离),降 miss 率 |
| L-8 | **环境红项(非本仓问题)**:本地 gate M1 的 adr-unique FAIL——CutFlow 仓**未提交改动** `skills/cutflow/scripts/segmentation.py:504` 引用 ADR-0060,而其 `docs/adr/` 仅到 0059;cutforge 的 adr-unique 在 `CUTFLOW_REPO` 不存在时按 NO_ENV 跳过,故 **CI 不受影响** | 由 CutFlow 侧补 ADR-0060 或改引用(属用户待检视的 v0.19 未提交工作,本迭代不越界改动) |
| L-9 | **性能专项已收口,query/apply 已达标**(原 query 70ms>50 / apply 162ms>100):profile 定位每 RPC 重开 38.4ms、watcher 守护抢锁 47.3ms 主因;经常驻工作区复用(fingerprint 新鲜度协议)+ watcher 本进程写入免开合并 + 锚点双 persist 合一后,profile 同口径 timeline_get 46.7→8.9ms、project_get 39.7→2.3ms、clip_update 86.2→35.7ms、undo 117.5→38.5ms、watcher 合并 47.3→1.5ms;`--check` 终测三轮全 PASS。**低优先级后续空间**(非本册遗留):engine apply 18–21ms / persist 12ms 为下一瓶颈(ProjectView 序列化两次可省 ~8ms);常驻缓存可加 LRU。另:渲染 tmp 临时件在渲染失败时不即时清理 | query/apply 已达标,不再列阻断遗留;后续空间仅作低优先级记录,供后续册视情况取用(劣化 >20% 阻断口径不变);tmp 残留由 24h 孤儿规则兜底,不丢数据 |

## 四、与册二(前端重写)的交接提示

1. **事件消费**:壳改走 SSE(`/events`),不再 300ms 轮询;长轮询降级面在册二收尾删除(A1-R2/L-3)。
   *(册二追记:SSE 主通道已落地;长轮询保留为断线降级路径,删除决定推迟至册三——见 L-3 追记与 A2 台账 A2-L5。)*
2. **静态资源**:前端新增文件放 `apps/web/` 即被 `/assets/*` 托管,**零 Rust 改动**(册二模块化拆 js 的前提已备好)。
3. **渲染进度 UI**:册三导出进度直接消费 `StepReport.to_progress()`(steps/cacheHits,L-2)。
4. **临时投影纪律**:壳纯度门禁(check-shell-purity)继续生效;拖拽 ghost 等按总纲 §1.2 第 3 条(不进 IR、不落盘、可重建)。
5. **契约加字段**:册四/册五动字段前先做 L-1 的「字段静默丢弃」门禁,再走七步流水线。
6. **基线对照**:册二收官跑 `gate.py A1` 同款七项 + `bench.py --check`,对照本册基线,劣化 >20% 册内解决或登记豁免。
