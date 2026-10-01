# A6-PROGRESS · 册六(独立化)台账

> 只增不改的历史台账(F-R 口径:完成一项记一项,不回头改写)。
> 计划书:册六「独立化:完整独立剪辑工具」;分支 `feat/plan-A6-standalone`。

## T6.1 工程库与项目管理(独立化核心)——BE 完成

决策(动工前落库):

- **ADR-0021(D-F1)**:布局 v3 扁平目录——做;v1/v2 兼容读写冻结(F-R1),迁移器
  一次性到位且幂等;落地节奏 = 过渡期 scaffold 缺省仍产 V2,v3 经显式开关
  (`new --layout v3` / `project_new layout="v3"` / 迁移)启用,缺省翻转登记遗留。
- **ADR-0022(D-F2)**:ffmpeg 策略——安装器可选组件(默认勾选内嵌);运行时解析
  env → 系统 PATH → 内嵌随包;缺失给 doctor 级三选一修复指引。
- **ADR-0023(D-F3)**:剪映草稿导出保留(收编随包独立脚本,诚实标注仍依赖
  Python 运行时);本地转写(whisper 系)明确不做(ADR-0017 评估框架)。

实码(工具 68→72 = 16 查询 + 37 写 + 19 编排):

- `cutforge-io::paths`:V3 常量(`media/`、`exports/`、根真相源)与三态判定
  (`LayoutKind::Legacy/V2/V3`;优先级 V2 > V3 > V1)、`truths_for`/`output_dir` 布局感知。
- `cutforge-io::migrate`:V1/V2 → V3 一次性迁移(预检冲突整体拒绝、幂等 NOOP、
  project.json 字节零改动、OpLog/rev/`.cutforge/` 原地不动)。
- `cutforge-io::library`:库根(env `CUTFORGE_PROJECTS` 缺省 `%USERPROFILE%\CutForge\Projects`)
  七操作(list/search/new/rename/copy/archive/unarchive/delete)+ 卡片轻量派生
  (slug/fps/画幅/时长=IR 投影/rev/修改时间/缩略图;不逐工程 ffprobe);
  归档 `.archives/`、删除 `.trash/<ts>-<名>/`(可捞回);活进程持锁拒绝移动。
- `cutforge-io::recover`:残留锁检测(pid 存活性 + 锁龄,附会话摘要证据)→
  恢复 = 清锁 + `Workspace::open_for_write`(OpLog 半行截断/rev 对账复用既有语义)。
- `cutforge-io::snapshot`:自动快照 `.cutforge/snapshots/r<rev>/`(project.json + oplog
  全量;env `CUTFORGE_SNAPSHOT_INTERVAL_MS`/`CUTFORGE_SNAPSHOT_KEEP`,LRU,缺省关)。
- `cutforge-io::scaffold`:`scaffold_project_layout`(V2 缺省/V3 扁平 + media/exports 建盘)。
- CLI:`library`/`migrate`/`recover` 子命令 + `new --layout v3`。
- MCP:`migrate_layout`(写)/`library_manage`(写,action 参数化)/`library_list`(查询)/
  `library_recover`(写);`project_new` 增 `layout` 参数;`wordline_get`/`cutlist_get`/
  `cut_apply`/导出产物路径三态布局感知。
- 顺手收口:`grade_tools.rs` lut_import 的旁路写入改走 `atomic::atomic_write`
  (A5 遗留,`check-write-paths` 复绿);`atomic.rs` 增 `rename`/`remove_dir_all`
  结构性原语(目录级操作收敛唯一落盘点纪律)。

验证(2026-09-30 本机):

- `cargo test --workspace --locked`:44 套件全绿(新增 io 单测:迁移幂等/冲突拒绝/
  残留保留、library 七操作/活锁拒绝/invalid 卡片、崩溃恢复/OpLog 完整性、快照 LRU/缺省关;
  新增 dispatch 级闭环 `library_migrate_recover_full_chain`)。
- `cargo clippy --workspace --all-targets -- -D warnings`:零告警。
- `check_doc_counts`:72 = 16 + 37 + 19 四文档零漂移;`check-write-paths` 旁路写入 = 0。
- `tool_parity --update-golden` 后连跑两次 0 DRIFT(72 工具;1 WARN 为加法键容忍)。
- 19 份 e2e 全绿(17 份 python + `e2e_ai_edit_visible`/`e2e_note_loop`);gate A1 八项全绿。
- 手工冒烟:new --layout v3 布局树 / V2→migrate→树 diff(素材入 media、成片入 exports、
  真相源到根、残留如实 kept)→再 migrate 幂等 NOOP / library 七操作 / 假 PID 残留锁
  recover 清零 / 快照 env 开落盘、缺省关不落盘。

## 待办(册六后续任务)

- T6.1 模板面(画幅/轨道/品牌色预设)、FE 工程库页;
- T6.2 内置能力收编(字幕/花字/卡点/重构图,AC-6.3 零外部脚本);
- T6.3 导出矩阵与编辑不阻塞(AC-6.4);T6.4 应用化(安装器/文件关联/单实例/内嵌 ffmpeg);
- T6.5 纯净机全流程;缺省布局翻转为 V3(前提:FE + 安装器收编,e2e 断言同步)。
