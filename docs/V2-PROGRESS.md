# V2 迭代进度总账(M8 → M13)

> 唯一执行依据:`docs/ITERATION-PLAN-v2.0.md`。本文件是滚动台账:每阶段收尾时,
> 复盘发现的问题与可改进项**强制折入下一阶段任务单**(全权委托条款)。
> 门禁纪律不变:每指标可判定、可存档、不得为通过而放宽。

## M8 · 止血与信任修复(2026-09-19 完成)

**交付**:5 个 P0 全清 + M8-5 诚实性,门禁 M8-1~M8-5 全绿。

| 门禁 | 判定 | 结果 |
|---|---|---|
| M8-1 文件级撤销为真 | `file_level_undo`(内核 4 例)+ IO 侧 notes/cutlist 还原 + auto 不入撤销栈 | ✅ |
| M8-2 真实工程可打开 | 夹具 = CutFlow rs_ir.py 真实生成(含 `_meta`),open/apply/reopen/roundtrip 后 `_meta` 逐字节相等 | ✅ |
| M8-3 渲染三修 | `render_matrix_fixture`:双画幅分辨率互不污染 / 人声 startMs 落点 / sfx 1500ms 起播;ffmpeg 缺失即失败 | ✅ |
| M8-4 并发零丢失 | 双线程 × 5 写,rev=oplog=2N,盘面不分叉 | ✅ |
| M8-5 协议诚实 | 错误码穷举(占位符清零)、矩阵单一真相源(7/15 诚实计数锁定)、6 编排工具协议完整、python 探测 | ✅ |

**全量回归**:cargo 91/91(基线 77 → +14 门禁)、pytest 3/3、gates M0/M1 绿。
**提交**:59f7ed6(P0-1/2/5)→ f4765da(P0-3/4)→ e5d6f5e(M8-5)+ ADR-0001~0003。

### 复盘:M8 新发现(全部折入 M9+)

| # | 发现 | 处置 |
|---|---|---|
| M8-R1 | "字节级还原"门禁对外来文件格式不可达(undo 走语义域重序列化),已降为 canonical 语义等价并记录于 ADR-0001 | M9 评估可选的会话内字节快照日志;不阻塞 |
| M8-R2 | CI rust job 此前**无 ffmpeg**——渲染类门禁在 CI 上根本无法运行,这是 V1 虚标能溜过门禁的基建性原因 | 已装(本次);M11-1 全量对拍依赖它,须保持 |
| M8-R3 | `render_variants` 仍每变体全链重跑(计划书 6.5 的"共享缓存只分叉 encode"未兑现) | M11 真分叉 |
| M8-R4 | `merge_from_disk` 若在 `open_exclusive` 之外被调用则无锁保护 | M9 watcher 调用点必须显式持锁,或收编为 Workspace 内部方法 |
| M8-R5 | MCP dispatch 对只读工具也走 `open_exclusive`(防 TOCTOU),长操作(渲染编排)持锁会阻塞并发工具 | M9-2 watcher 设计约束:merge 只在短临界区;渲染编排保持子进程(不持 workspace 锁) |
| M8-R6 | `stage_status` 仍只查 `_state` 文件存在性(计划书结合面⑧) | M9-3 正题 |
| M8-R7 | `tests/__pycache__`、`tools/**/__pycache__` 曾被跟踪 | 已解除跟踪 + .gitignore |

## M9 · 主链路贯通(2026-09-19 完成)

**交付**(M9-1~M9-4 全绿):

| 门禁 | 判定 | 结果 |
|---|---|---|
| M9-1 冲突真触发 | `conflict_real_repro_and_stop_writes`:写入窗口内外部同字段异改 → CF-001 + 本地待写弃用 + 停写;`window_drift_different_fields_auto_merge` 异字段自动合并;baseRev 快照链 `.cutforge/bases/`(LRU 32,40 次写后 ≤32 份) | ✅ |
| M9-2 外部改动可见 | `external_edit_visible_within_1s`:守护线程 + SyncHub 长轮询;HTTP `/events?root=&since=` 推 `workspace.changed`;MCP dispatch 幂等注册守护 | ✅ |
| M9-3 同口径 + keep 重算 | cutforge:stage_status 优先 rs_run --status(降级如实标注)、cut_apply 服务端重算 keep/removedMs(金样 = rs_cut.finalize_cutlist 真实生成,6 例对拍);CutFlow:rs_run outputs_hash 带外改写检测、B8 护栏识别 CutForge 编辑痕迹(schemaVersion)、REBUILD/SKILL cutlist 脏提示改道 rs_cut --apply | ✅ |
| M9-4 桥升版 + 双侧冒烟 | rs_editor id 回退(V1#2+idSource)、rs_gate probe 真检 gate.py、rs_oplog 半行截断对齐、SKILL.md 登记四桥;cutforge 新增 4 桥冒烟 pytest,CutFlow 新建 CI(.github/workflows/gate.yml)浅克隆 cutforge 跑桥探针+烟测 | ✅ |

**回归**:cutforge cargo 97/97(+6)、pytest 7/7、gates M0/M1 绿;CutFlow 300/300。

### 架构决策(M9,已实施)

- **冲突触发的真实窗口**:同步点快照(`synced_disk`)+ persist 前漂移检测。
  "每写必先预合并 + 立即持久化"使两个 cutforge 进程之间结构性地不会产生 CF-001
  (后写者总以真祖先看到先写者);真正可触发的是**锁外外部写者**(CutFlow 管线/文本
  编辑器)落在 pre-merge 与 persist 之间——此时三路合并,同字段异改 → 冲突落盘、
  本地弃用、停写。快照 LRU 上限 32(计划书 V2-R4;未用 diff 存储,偏离已记录于 M8 折入项)。
- **v1 工程独占打开即升级规范形**:使外部检测与守护合并都有稳定的 v2 基准。

### 复盘:M9 新发现(折入后续阶段)

| # | 发现 | 处置 |
|---|---|---|
| M9-R1 | CutFlow `derive_keep` 对**尾部 remove** 产出的 keep 不覆盖片尾,`finalize_cutlist` 与 schema 断言会双双拒绝——结尾静音删除场景在现行语义下不可表达(CutFlow 自身矛盾,与本次改动无关的存量问题) | 折入 CutFlow 待办:需 ADR 澄清"keep 覆盖片尾"语义(允许尾删 vs 禁尾删);金样夹具已避开该形态 |
| M9-R2 | MCP stage_status 走 rs_run --status 时是子进程调用(~秒级);编辑器高频轮询应改走 `/events`+oplog rev,不要反复拉 stage_status | M10 Web 壳遵守 |
| M9-R3 | 守护线程与 MCP dispatch 各自 open_exclusive:冲突落盘可能由守护先写入,dispatch 停写提示需引导用户看 conflict_list(已实现,UI 侧待展示) | M10 冲突面板 |
| M9-R4 | `/events` 只推 project.json 变更;notes.json/cutlist.json 的外部改动(如 rs_cut --apply)不产生事件 | M10 扩展事件面 |

## M10 · 编辑能力(2026-09-19 完成)

**交付**:

| 门禁 | 判定 | 结果 |
|---|---|---|
| M10-1 edit_ops_e2e | `tools/e2e_edit_ops.py`(Playwright,3 连跑稳定):真实 IR 导入→分割→检查器移动→波纹删→undo 全还原(逐字段相等 + OpLog 数==rev + 盘面 rev 一致)→redo 等量回放(oplog 计数相等 + 片段逐字段一致) | ✅ |
| M10-2 note_loop_ui | 同脚本:3 条标注创建→AI 执行(clip_update,causedBy 绑定)→浏览器内逐条回执结案(resolvedBy.opIds 校验),全链 0.54s ≤ 5s | ✅ |

- **本地服务化**:`cutforge-mcp serve --root <工程>` 常驻服务——静态托管 apps/web +
  `/session`(token 落盘 `.cutforge/session`)+ `/rpc`(全部 UI 操作走 MCP 工具→Op,
  无旁路)+ `/events` 长轮询(外部改动可见);随机 token(pid+时钟 hash,零依赖)。
- **操作集**:选择/拖拽移动(帧网格磁吸开关)/边缘 trim/S 分割/Del 删除/Shift+Del
  波纹删/Ctrl+C·V 复制粘贴(新工具 `clip_duplicate`)/`clip_move`/`track_add`
  多轨管理/Ctrl+Z·Y 与按钮撤销重做(跨 dispatch 可用)。
- **标注 UI 闭环**:时间轴锚定新建→open 列表→回执结案表单(opIds)→孤儿面板计数。
- **差异面板升级**:actor 过滤/limit/逐 Op before→after/勾选批量撤销(undo ×N)。
- **注册表 28→32**:新增 `clip_move`/`clip_duplicate`/`track_add`/`timeline_get`
  (时间线投影由内核计算 endMs,壳零时间线语义,check-shell-purity 绿)。
- **IR v3 预研**:ADR-0004——keyframes 只进 position/scale/opacity/volume 四属性,
  ease 封闭枚举,不做曲线编辑器,实现随 M12 按需。

### 复盘:M10 新发现(折入 M11/M13)

| # | 发现 | 处置 |
|---|---|---|
| M10-R1 | **Engine::restore 把 redo_stack 置空**——MCP 每次 dispatch 重开工程,redo 跨 dispatch 永远失效(NOTHING_TO_REDO)。已修:`rebuild_stacks` 对称重建双栈(M10 e2e 抓出) | 已修 |
| M10-R2 | serve 线程读循环读到 header 即 break 时,POST body 滞留内核缓冲,关闭触发 Windows RST→客户端间歇 ConnectionReset。已修:读满 Content-Length + shutdown(Write)+drain 优雅关闭 | 已修 |
| M10-R3 | serde Map 的 `value["key"]` Index 在键缺失时 panic(notes_add 的 anchor.ref 缺键打挂整个连接线程)。已改 `.get()`;**全仓应排查同类 Index 用法** | M11 前清零 |
| M10-R4 | 浏览器子资源(css/js)不带 Authorization——鉴权必须只锁数据面,静态资源公开。已修 | 已修 |
| M10-R5 | e2e 对 Playwright 点击竞态敏感:已改为服务端真相(oplog 计数)驱动重试;后续 e2e 一律遵循"以服务端状态为断言依据,UI 只作驱动"的写法 | 写入约定 |

## M11 · 渲染追平(2026-09-19 完成)

**交付**(RENDERER_VERSION 2.0→3.0,旧缓存全失效):

- **转场(5)**:xfade 链 + 尾帧扩展(ADR-0023 口径)——总时长保持 sum(dur) 零吞切;
  三级语法(jumpcut 亚帧/topic 300ms)按 clip.transition durMs/type 消费。
- **变速(2)**:video setpts=PTS/speed + audio atempo 链(0.5–2 分解,覆盖 0.25–4);
  音画同步:源读取 dur×speed,成片落点不变。
- **punch-in(12)**:中心裁剪 factor 紧构图(静态,zoompan 语义)。
- **位置/缩放(4)**:overlay 字段 clip = 叠加层(rs_brand 变体轨口径),绝对像素 +
  opacity + between(t) 时间窗合成;旋转:契约无字段(CutFlow 同),evidence 如实标注。
- **BGM ducking(9)**:bgm 循环铺满 + gainDb + sidechaincompress 侧链(on/off 能量差可测);
  bgm-only 工程 anullsrc 占位总线。
- **淡入淡出(3)**:clip.fade → per-seg afade( loudnorm 前生效)。
- **真·多画幅分叉(14)**:mix 缓存键画幅无关 → 变体共享 mix 只重做 video/encode
  (mix-*.m4a 两变体仅一份,parity 夹具断言)。
- **文件名消毒**:slug 敌对字符 → `_`;文本轨:结构性锚点不渲染(与 CutFlow 同口径)。

| 门禁 | 判定 | 结果 |
|---|---|---|
| M11-1 parity_matrix_full | `crates/cutforge-render/tests/parity_matrix.rs` 九项 ffmpeg 实测(转场零漂移 4.000s/变速时长语义/punch-in 帧差/overlay 时间窗/ducking on-off 能量差/afade RMS ≥6dB/文件名消毒/文本轨口径/mix 真分叉) | ✅ |
| 矩阵回填 | capability-matrix.json:achieved 7→**13**(必达 13/13),evidence 全部指向实测夹具;MCP 同源断言同步 13 | ✅ |

**回归**:cargo 98/98(+parity)、pytest 7/7、gates M0/M1 绿、e2e 2/2。

### 复盘:M11 新发现(折入 M12/M13)

| # | 发现 | 处置 |
|---|---|---|
| M11-R1 | 转场只做视频 xfade,音频无 acrossfade(8ms 硬接);CutFlow rs_render 同口径,对拍容差内 | 观察项,不立项 |
| M11-R2 | ducking 参数(threshold/ratio/attack/release)为经验值,未与 rs_render 数值对拍 | M12 若立项"专业分轨"再对拍 |
| M11-R3 | parity 夹具跑一次 ~12s(九项实渲);CI web-e2e + rust-gates 总时长可接受 | 已入 CI |

## M12 · 决策(2026-09-19)

**ADR-0005:专业分轨整体暂缓**——调色/OTIO/曲线变速 UI/音频分离均不进入 V2.0。
理由:V2 核心矛盾(主链路断裂)已解决;无真实素材牵引不立项(防重蹈 V1 虚标);
OTIO 违背最小依赖需独立论证。触发重启条件与启动资产(parity 基建、keyframes 预研)已记录。

## M13 · v2.0 发布(2026-09-19)

- **ARL-CORE 补齐(P1-8 闭合)**:27 个核心源文件加 `ARL-CORE` 头 +
  `CORE-FILES` 清单 37 项(LICENSE 1.3 ③ 口径);律师复核列入人工验收。
- **版本**:workspace 0.1.0 → **0.2.0**(V2 迭代);tag v0.2.0,Release 名
  "V2 迭代(M8–M13):主链路贯通 + 编辑能力 + 矩阵必达 13/13"。
- **验收**:ACCEPTANCE.md 增 V2.0 机制验收表(全部 Agent 实测);人工四项
  (真机走查/律师复核/组织迁移/CI 首跑)待用户签字——与 V1 口径一致。
- **组织迁移/律师复核**:人工事项,保持 PENDING(全权委托不可代签)。

## V2 终局口径

| 指标 | V1 宣称 | V2 实测 |
|---|---|---|
| 能力矩阵 | 93.3%(虚) | **87%(13/15,必达 13/13 实码+夹具)** |
| 结合度 | 8/20 | 契约/流程/工具/发布四层全通(双侧 CI 桥冒烟) |
| 测试 | 77 | cargo 98 + pytest 7 + e2e 2 门禁 + CutFlow 300 |
| P0/P1 | 5 P0 未知 | 5 P0 全清;P1-8/P1-9 等顺带闭合 |

## E1/E5/E2 迭代批次(2026-09-20,来源:《20260920-CutFlow×CutForge 迭代更新笔记》P0 清单)

- **E1 启动入口**:`cutforge-cli serve` 子命令(薄转发 `cutforge_mcp::serve_workspace`,同一实现;
  check-deps 白名单增 cli→mcp 边);无 `--root` 时交互列工程(回车=最近);`--open` 自动开浏览器;
  `serve_preflight` 启动自检(工程/Web/ffmpeg/ffprobe/cutforge-render 逐项 ✓/△ + 补救命令,缺工程退出 3);
  release 打包纳入 `cutforge-mcp` + `apps/web/`(E1-1/E1-2);仓库根 `start-editor.cmd/.sh`;README Quick Start。
- **E5 导出接线**:`render` 工具按 `backend` 分派(B6);`render_run`/`render_progress` 异步渲染+轮询
  (子进程跑 cutforge-render,全程不持 workspace 锁);`CUTFORGE_FFMPEG`/`CUTFORGE_FFPROBE`/
  `CUTFORGE_RENDER` 环境变量定位(B13,与 CutFlow `WPI_FFMPEG` 口径对齐);壳加导出面板+进度。
  **顺带修掉两个"真实工程导不出片"的阻断**:①render 二进制未走 `_meta` 旁路(ADR-0002),
  真实 CutFlow IR 一律 SCHEMA_INVALID;②volume 缺省被当静音 → 真实 IR(clip 不带 volume)整片无声,
  全静音混音又令 loudnorm 测得 -inf、linear=true 应用崩溃。语义改为:None=自然音量(人声 1.0/sfx 0.8),
  显式 0 才静音;混音加数字静音守卫。
- **E2 预览**:`/media` 端点(数据面鉴权/canonicalize 防穿越/Range 206);`timeline_get` 扩全字段投影
  (endMs 服务端算好,壳零时间线语义);壳 canvas 预览+空格播放+←/→ 逐帧+标尺拖拽联动
  (诚实标注"画质代理");新增 `tools/e2e_preview.py` 门禁:鉴权/穿越/Range、readyState、
  seek 对齐 ≤1 帧、canvas 非全黑、播放推进、编辑器内导出→产物时长断言、壳纯度仍绿。
- **顺带清账**:工具数 34(11 查询+16 写+7 编排)三方对拍(`_doc`/protocol_conformance/CLI 自述补全 17 子命令,
  B7/B8 部分);mcp session 落盘改走 `atomic.rs`(write-paths 判定器命中 7→4,余 4 处为 render 测试夹具×2
  与 io lib×2 的**基线既有**命中;**已专项清账**:四处全改走 `atomic.rs`,判定器 remove_file 模式
  拼接 bug 修正(join("::")→join(""))后审计 0 命中);Cargo/文档描述去陈旧数量。
- **验证**:cargo 全测试 0 failed;M10-1/M10-2 e2e PASS;e2e_preview 全 PASS;shell-purity/check-deps/M0/M1 绿。
- **版本**:workspace 0.2.0 → **0.3.0**(E1/E5/E2 批次);tag v0.3.0,release 产物改为每平台单 zip(三二进制 + web/,SHA256SUMS 对应)。

## 阶段二批次(2026-09-20,来源:《副文档 02 · 阶段二:CutForge 编辑器闭环与"从零剪"》)

- **E3 素材导入**:新增 MCP 工具 `clip_add`(内部走既有 `Command::ClipInsert`,requestId 去重;
  durationMs 缺省由 `cutforge_io::probe` 探测自动填)与只读工具 `media_probe`(时长/分辨率/音轨,
  B12 死代码接线)、`media_browse`(列可导入媒体;`GET /media/browse` 同一 payload 实现);
  路径校验统一收敛 `resolve_within_root`(/media 的 canonicalize 函数化,/media、browse、clip_add、
  probe 共用,不建并行实现);壳加素材面板(双击 = 播放头帧磁吸落点,拖拽 = 轨道任意落点)。
- **E4 检查器**:壳检查器按语义分组(基础/画面/音频/变速/文本),字段分组由单一真相源
  `schemas/ui-fields.json` 声明并经 `GET /ui-fields` 下发(壳不读文件);新增机械校验
  `cutforge-cli check-ui-fields`(可编辑字段集 ⊆ ClipPatch 字段集);ClipPatch 未承接的
  position/transition/fade/punchIn 等投影字段做只读展示(E4-3)。
- **E6 工程/首次运行**:`cutforge-mcp serve` 无 `--root` 时交互列工程(picker 迁至 mcp,与 CLI
  同一实现);serve 启动建齐 `.cutforge/{session,bases,oplog}` 并打印位置,token 失配时页面顶部
  横幅提示;**只读工具(project_get/timeline_get/notes_list/conflict_list/oplog_tail/cutlist_get/
  wordline_get/render_probe/stage_status/media_*)不再持排他锁**(E6-3/B14),写通道仍全程锁;
  e2e 断言"渲染进行中查询/编辑不被阻塞"。
- **B11 新建工程**:空工程模板 + `cutforge_io::scaffold`(CLI `new` 子命令与 MCP `project_new`
  工具同走单一实现;模板必须过 v2 schema;已存在拒绝覆盖);壳新建向导(画幅/帧率/工程名);
  README 定位更新为"可独立起步的编辑器(也能打开 CutFlow 工程)"。
- **RT-1/RT-5**:工作区数据面 dispatch 归因 `Actor::user("editor")`(stdio/内嵌 HTTP 仍 agent),
  serve 期间每次成功写后增量落盘 `.cutforge/session-summary.json`(actor=human 的 Op 清单 +
  rev 区间,Ctrl+C 也不丢账);watcher 忽略表加入 CutFlow 记账文件(`05_ir/pipeline.json`、
  `_state/*.json`),跑阶段不再给编辑器推假变更。
- **B 系扫尾**:B7 工具数四处同源(34→**38** = 13 查询+18 写+7 编排:`_doc`/protocol_conformance/
  gate 契约比对/README·FLOW·ACCEPTANCE 注明时点);B8 CLI 自述补全至 19 子命令(+new/check-ui-fields);
  B9 CLI `clip-update` 补齐 source-in-ms/speed/opacity/scale/text/freeze-ms;B10 于 FLOW.md §7.1
  明文说明 M8–M13 门禁以 cargo test + e2e 承载(不新增 gate.py 注册)。
- **壳(E8)**:工具栏(播放/首尾/时间) + 素材面板 + 预览 + 分组检查器 + 轨道区 + toast 状态
  (替代单行 footer,#status 保留为诊断锚点);冲突顶部停写横幅;未选中空态引导与按钮置灰;
  快捷键补 Home/End。**RT-2/RT-3/RT-4(CutFlow 侧)不在本仓波次,移交后续。**
- **验证**:cargo 全测试 0 failed(含 protocol_conformance 38 口径、readonly 不持锁、project_new
  从零、clip_add 穿越/探测断言、scaffold 模板校验);M10-1/M10-2/M10-3 e2e PASS;e2e_preview 全 PASS;
  新增 `tools/e2e_from_zero.py`(CLI new → 导入 → 改 4 字段 → 导出成片时长对拍 + 渲染期间并发
  + RT-1 摘要)入 CI;check-shell-purity / check-ui-fields / check-write-paths / check-deps 绿;
  clippy 无新告警。

## 版本 v0.4.0(2026-09-21 发行)

- **版本**:workspace 0.3.0 → **0.4.0**(阶段二编辑器闭环批次);tag v0.4.0。
- **本批内容**:E3 素材导入(clip_add/media_probe/media_browse + 壳素材面板)/ E4 检查器
  ui-fields 单一真相源 + check-ui-fields 机械校验 / E6 只读并发(查询类不持排他锁,
  并发 e2e 断言)/ B11 从零新建(scaffold + project_new + 新建向导)/ RT-1 会话变更摘要
  / RT-5 watcher 忽略 CutFlow 记账 / write-paths 基线 4 处清账(判定器 remove_file
  漏报一并修复)/ J6 剪映出口对拍(tests/test_jy_bridge.py,与 CutFlow rs_jy_draft
  编排同一脚本)/ ADR-0006~0008 / NOTICE·README 定位升级(可独立起步的编辑器)。
- **CI**:三 job 全绿;tests job 的 pytest 含跨仓桥测试(CUTFLOW_REPO 浅克隆同跑)。
