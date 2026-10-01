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

## T6.3 导出矩阵与后台任务 + T6.2 内置能力收全——BE 完成(F2)

实码(工具 72→76 = 18 查询 + 38 写 + 20 编排;render/render_run 同批扩导出参数面):

- **导出矩阵渲染端**(`cutforge-render::export` 单源 + `lib.rs::render_export`):
  - 格式出口(encode 步分派):`mp4-h264`(与既有编码面同源,缺省 remux+bt709)/
    `mp4-h265`(libx265+hvc1,软件编码,hw 请求 WARN 不冒充)/`mov`(容器随扩展名)/
    `gif`(palettegen+paletteuse 两段,fps=12 固定如实声明,调色板写缓存 tmp 用完即清)/
    `m4a`(纯音频短路:probe→mix→copy,不渲视频链)/`mp3`(libmp3lame 192k)/
    `png-seq`(image2 %04d,帧数清点入进度)/`frame-png`(复用 render_frame 单帧管线,
    不进整片导出);缺省(无导出参数)路径逐字不变,RENDERER_VERSION 9.0→10.0;
  - 预设:`preset`(vertical/horizontal/square + 9x16/16x9/1x1/3x4/4x5 别名)画幅映射
    与 render_variants 同源;`qualityTier` 1080p/720p/480p = **短边**缩放(四舍五入到偶数,
    1080p 竖屏=1080x1920 平台惯例);`bitrateTier` high/medium/low = 1080p 基准
    (12000/8000/4000 kbps)按高度线性折算的**估算值**(显式 bitrate 覆盖);
  - 区域导出:`inMs/outMs` = 工程级时间窗裁剪(`window_project` 纯函数)——全轨种钳制
    平移(输出时间轴=in 起)、头部裁剪按速度分段积分折算源域(`source_advance_ms`)、
    各视频轨新首段入向转场清除、textass/叠加层/调整层同窗、BGM 保留(相位从源 0 起,
    已登记口径);`videoOnly` = 音轨静音 + BGM 摘除(文本轨不吃 mute)。
    已登记近似:窗口边界恰入 freezeMs 定格区时源域折算按速度段计算。
- **MCP 面**:`render`/`render_run` 增 format/preset/qualityTier/bitrateTier/inMs/outMs/
  videoOnly 参数(缺省零旗标;`render_frame` 的 format=png/jpeg 不受撞车,构建面隔离);
  frame-png 在 render_run 分派到单帧管线(atMs=inMs)。
- **`export_preflight`(查询)**:轻探测不渲全片——缺失素材(计划源在位性)/黑帧风险
  (首末视频片段源域 32x18 灰度抽样,16 级量化,<16 判风险)/静音段(声轨覆盖间隙>200ms
  + 逐段源 RMS<0.005 启发式,100ms 网格合并,上限 20 段)/时长(窗口裁剪后)/响度预估
  (audio_loudness 复用:最新成片在位则实测,否则 null 不虚标);`heuristic:true` 如实标注。
- **`export_all_variants`(编排)**:ratios 缺省取工程 outputs(再缺省 9x16/16x9/1x1),
  逐变体入既有渲染队列(独立 runId,进度/取消/重试复用 render_queue 面与并发上限);
  `action=status` 按 parentRunId 拉取式聚合(all ok→ok;有 fail/canceled→fail)。
- **素材库(ADR-0023 口径,不虚标)**:
  - `media_library`(查询):库根 `library.json` manifest(类型/标签/时长/引用),扫描
    合并幂等(在位文件↔条目按引用对齐,标签持久、字节/时长刷新、消失除名,条目上限
    200 截断 WARN);kind/tag/query 过滤;action=tag 改条目标签;库根 = env
    `CUTFORGE_MEDIA` 缺省 `%USERPROFILE%\CutForge\Media`;
  - `media_import`(写):拷贝导入工程——src=绝对路径或素材库相对引用,落点布局感知
    (v3=media/、v2=01_原始素材/、v1=01_materials/),同名同内容幂等覆盖、异内容追加
    序号,tmp+rename 原子落盘,不产 Op 不改 IR——**FE1 登记的「拷贝导入通道候 BE」
    欠账就此收口**。
- **剪映草稿随包收编(ADR-0023 落地,端到端实证)**:`rs_jy_draft.py` 整文件收编为随包
  资产 `tools/jianying/`(带归属声明;ADR-0008 边界由 ADR-0023 显式修订)——**依赖闭包
  整收**:rs_paths/rs_common/rs_codes/segmentation + `templates/jy59_empty_draft.json` +
  `vendor/pyJianYingDraft`(MIT),运行时零 CutFlow 仓库依赖;两处**路径解析适配补钉**
  (算法零变化):rs_common 配置路径 env `CUTFLOW_CONFIG`(剪映机器路径 config.json
  自备,如实 NO_CONFIG 不虚标)+ rs_jy_draft 工程根三态派生(v3 扁平)与模板自包含;
  orchestrate 脚本定位序 = **env CUTFLOW_REPO(显式,调试/对拍)→ 工程内 → 随包资产
  (<exe>/scripts/ 安装器落点 + 开发树 tools/jianying/)→ CutFlow 仓库回退(一个版本期;
  含祖先发现,与第 1 层纯 env 判别分离——来源标注不混)**;
  来源经编排响应 `scriptSource` 如实标注(cutflow-env/workspace/bundled/cutflow-fallback);
  **dispatch 端到端缺省**:`export_jianying` 契约(root+name)此前实际不接线(scriptArgs
  缺省空 → 真脚本必败,golden 只锁桩面)——补 project 路径(三态布局感知)+ `--name`
  缺省注入(显式 scriptArgs 仍整组透传);无 CUTFLOW_REPO 全链冒烟实证:
  `scriptSource=bundled` + DRAFT_OK(1 轨/1 段/180 帧门禁全 PASS,草稿落工程区)。
- **内置能力四件套收口实况(诚实口径)**:
  - ASS 引擎(收全):册四 textStyle(字体/字号/颜色/描边/底衬/阴影/九宫对齐/行距/
    透明度/位置/卡拉OK \kf)+ 花字 12 模板 + VTT/SRT/ASS 导入导出已在位;盘点缺口 =
    VTT 不承载样式(VTT 往返只锁文本/时轴,样式互导以 ASS 为唯一通道)——格式能力
    边界如实,不做过度互导;
  - reframe 全自动(诚实降级):无可靠自动构图算法(运动能量/响度重心启发式对构图
    质量不可判定,人脸检测不可做)——**不做伪 AI 构图**;clip.reframe(anchorY) 契约
    字段承载防丢、渲染端零消费如实登记(能力矩阵 #50 missing,触发条件登记);
  - 转写:按 ADR-0023 **明确不做**(能力矩阵 #51 missing=决策,非待办);
  - 卡点/节拍:册四 audio_beats(onset-energy 启发式)+ 吸附已在位,本册无新增欠账。
- **capability-matrix**:json/md 同步追加 #43–#51(7 achieved + 2 missing 诚实标注)。
- **收口波补钉(F2 验证期发现,当轮修复)**:
  - **v3 工程导出产物落点**:`RenderPlan::build_full` 的 out_dir 原只有 legacy/v2
    二态,v3 工程渲染误落 `06_成片输出/`(与 `paths::output_dir` 的布局感知清单面
    不一致)——补 v3 → `exports/` 分支(v2/legacy 语义逐字保留,双目录并存的 v1
    边角不改判),`render_matrix::out_dir_follows_layout_tri_state` 集成测试钉坑;
    手工冒烟实证:v3 工程 mp4 落 `exports/`;
  - **export 编码分派归位**:`exec_export_encode` 自 lib.rs(815 行,超 A1-3
    800 行红线)平移至 `export.rs`(单源更彻底;`run_ff`/`strs` 转 pub(crate)),
    lib.rs 760 / export.rs 710 / plan.rs 794 全部 ≤800;clippy 新告警三处
    (doc 懒延续/可折叠 if/useless format)同轮清零;

验证(2026-09-30 本机,F2;收口波补钉后复跑):

- `cargo test --workspace --locked` 全绿 44 套件 0 FAILED(render 新增 export.rs 11 项
  单测含参数面/窗口折算/路径互斥 + plan.rs 布局三态;mcp 新增 export_tools 4 项 +
  media_library 4 项;protocol_conformance 76=18+38+20 与缺参面同步);
- `cargo clippy --workspace --all-targets -- -D warnings` 零告警;
- `check_doc_counts`:76 = 18 + 38 + 20 四文档零漂移;
- `tool_parity --update-golden` 后连跑两次 0 DRIFT(76 工具;导出矩阵/素材库夹具:
  m4a/gif/预设导出产物落盘断言、预检 missingAssets=0、导入落盘 01_原始素材/libbgm.mp3、
  标签过滤命中、两变体终态全 ok;1 WARN 为 audio_beats 加法键既有容忍);
- 19 份既有 e2e 全绿(17 份 python + e2e_ai_edit_visible/e2e_note_loop;
  零适配——缺省渲染路径参数逐字不变);
- gate A1 复跑全绿(rust-line-limit ≤800:lib.rs 760/export.rs 710/plan.rs 794);
- 手工冒烟:gif 导出(2s 窗 × 12fps = 24 帧实证/调色板两段)/区域导出时长=out-in
  (in=1000,out=3000 → ffprobe 2.000000s)/m4a 纯音频(无视频流,3s 窗 → 3.000000s)/
  export_preflight(缺失素材=[]/黑帧 luma=128 risk=false/静音段=[]/窗口化 totalMs=2000/
  响度实测偏差 0.05LU)/media_import 真实落盘(byte 级一致,v3 → media/)/
  export_jianying 无 CUTFLOW_REPO 走随包 scriptSource=bundled + DRAFT_OK 门禁全 PASS。

## 待办(册六后续任务)

- T6.1 模板面(画幅/轨道/品牌色预设)、FE 工程库页;
- T6.2 内置能力收编(字幕/花字/卡点/重构图,AC-6.3 零外部脚本);
- T6.3 导出矩阵与编辑不阻塞(AC-6.4);T6.4 应用化(安装器/文件关联/单实例/内嵌 ffmpeg);
- T6.5 纯净机全流程;缺省布局翻转为 V3(前提:FE + 安装器收编,e2e 断言同步)。
