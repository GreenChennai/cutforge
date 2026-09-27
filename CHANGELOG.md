# 更新日志(CHANGELOG)

格式参照 Keep a Changelog;版本号遵循语义化版本(SemVer)。

## 未发行(Unreleased)

> 册一「内核重构与架构加固」批次(128856c / 3ce2884 / 7157178);版本号待发行时定
> (沿用仓库惯例:发行时才把本段改为版本号。此前的 v0.6 批次——编辑器 NLE 化、
> schema v2 M14 双仓同步——发行时一并补记)。

### 新增

- **渲染七步分解(RenderPlan)**:`render()` 单函数(≈440 行)拆为 `plan.rs`(纯函数计划)
  + `steps.rs`(命令行生成纯函数,可单测)+ 执行器;步骤 probe → segment → compose-video →
  overlay → mix → subtitle → encode,产出结构化 `StepReport`(并作 SSE 渲染进度数据源)。
- **渲染缓存内容寻址**:`composed/overlaid/subbed` 固定文件名废除,改五层键值目录
  `.cutforge/render-cache/{seg,mix,compose,overlay,sub}/` + `cache-index.json` 清单;
  新增 `cutforge-cli cache {info,gc,clear}`(LRU + 容量上限默认 10GB;清单外孤儿/tmp 件
  24h 超龄即清);多画幅变体真分叉(画幅无关层全共享)。
- **HTTP 层**:`/assets/*` 目录托管 `apps/web/`(前缀白名单 + canonicalize 穿越防护 +
  MIME 表 + ETag/304;前端新增文件零 Rust 改动);`/events` 升级 SSE(`text/event-stream`),
  事件面从"仅 project.json"扩展到 notes/cutlist 外部改动与 `render.progress`,旧长轮询
  降级保留(册二完成后移除);连接纪律(读超时/请求体上限/总时限/慢连接隔离)。
  决策见 ADR-0009(继续纯 std 手工加固,零新增依赖)。
- **错误码命名空间**:结果协议新增加法维度 `ns`(`io.*`/`core.*`/`mcp.*`/`render.*`),
  单一真相源 `registry::CODE_NS`,三面同码;**既有 code 取值逐字不变**。
- **`cutforge-cli doctor`**:六项环境诊断(工程/ffmpeg/ffprobe/web 资源/缓存目录可写/端口),
  每项失败给可复制执行的修复命令。
- **性能基准**:`tools/bench/bench.py`(1k clips/8 轨合成工程,open/query/apply/undo/render,
  `--check` 阈值判定);基线落盘 `docs/bench/baseline.json`(render 6586.1ms,min 口径)。
- **工具面黄金对拍**:`tools/bench/tool_parity.py` + `tools/bench/golden/`(41 工具逐键
  响应库,行为漂移即红)。
- **验收载体与门禁**:新增 `e2e_static` / `e2e_events` / `http_hardening` / `cache_addressing`;
  `gate.py A1` 册级门禁注册(决策 D-A2:每册一个 `A<n>` 入口);CI 增补 parity/static/events
  三步;docs/CONTRACT-WORKFLOW.md(新增 IR 字段的标准七步流水线);ADR-0009/0010。

### 变更

- **三巨石拆分(纯移动,零行为变化)**:`cutforge-mcp/src/lib.rs`(2,146 行)→
  registry/dispatch/orchestrate/progress/session/tools_nolock/workspace_svc +
  `transport/{stdio,http,events,static_files}`;`cutforge-io/src/lib.rs`(1,120 行)→
  `workspace/` 显式步骤管线(apply 八步状态机,先文件后记账实序在代码注释声明);
  `cutforge-core/src/engine.rs`(1,060 行)→ `engine/{apply,undo,replay,projection,invariants}`。
- 全仓 `cargo clippy --workspace -D warnings` 清零;非测试源文件全部 ≤800 行(最大 791)。
- 性能:bench 基准达标(1k 工程查询 18.6ms≤50ms、提交 71.3ms≤100ms;经常驻工作区
  +watcher 免开合并专项优化)。

## 0.5.0(2026-09-25)

### 变更(Breaking · 目录契约)

- **工程目录契约中文化(v2)**:阶段目录 `00_brief`/`01_materials`/`02_sensed`/`03_assets`/
  `04_cut`/`05_ir`/`06_output`/`_state` 更名为 `00_制作简报`/`01_原始素材`/`02_转写与校对`/
  `03_创作素材`/`04_粗剪决策`/`05_时间线工程`/`06_成片输出`/`_内部状态`;新增交付区
  `成品/`(NEVER_CLEAN)。文件名全部 ASCII 不变;`03_创作素材/artboard` 子目录保留英文;
  工程根 `notes.json` 与 `.cutforge/` 不动。目录名唯一真相源:
  `crates/cutforge-io/src/paths.rs`(与 CutFlow `rs_paths.py`,ADR-0046 同构)。
- 新建工程(`cutforge-cli new` / MCP `project_new`)一律产中文目录;需 **CutFlow ≥ v0.19**。
- **兼容**:0.4.x 旧布局(英文目录)工程在 0.5.0 中原地读写、不自动迁移
  (`tests/layout_compat.rs` 门禁);MCP 工程发现/真相源读取/render_probe/stage_status、
  cutforge-render 工程读取均按盘面布局自动择路。
- `schemas/mcp-tools.json`:stage_rebuild 的 `dir` 枚举增补中文目录(旧枚举值兼容保留);
  render_probe/stage_status/project_new 描述同步目录契约。
- Web 壳:`/session` 新增 `projectRel`(工程相对路径由服务端下发,壳不再硬编码目录名)。

### 升级指引

- 与 CutFlow 联动的场景:两侧同步升级(CutFlow ≥ v0.19 + cutforge ≥ 0.5.0);
  单侧升级期间,旧布局工程仍可被读写,不阻塞。
