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

## T6.1+T6.3 UI(F3)——工程库视图/向导模板/导出矩阵面板/素材库双页签

实码(apps/web 无构建 ESM;工具数不变,纯壳活 + TESTIDS.md 第九节全量登记):

- **工程库视图**(panels/library.js + library-ops.js,「工程 ▾」菜单常驻入口):卡片栅格
  (缩略图/fps/画幅/时长/rev/修改时间,徽标 current/archived/locked/invalid)、七操作
  (open 给 serve 命令一键复制——一个 serve 一个工程,诚实口径;rename/copy/archive/
  unarchive/delete 确认框含 `.trash` 捞回提示)、搜索(服务端 query)、归档区并入、
  恢复清单(library_recover list→recover)、**v2/v1 迁移提示条 + 迁移确认框**、
  启动静默扫描可恢复项的顶栏提示条。
- **新建向导升级**:模板三套(竖屏单轨口播 9:16·V+A / 横屏双机位 16:9·V+V+A /
  方形社媒 1:1·V+A+文本;选即预填,手改即脱离)+ 布局 v2/v3 选择。
- **导出矩阵面板**(panels/export-matrix.js,挂 #export 面板):七格式出口说明行
  (gif=12fps、m4a/mp3=仅音频、png-seq=序列帧)、画幅/清晰度/码率三档位、区域 in/out
  (selectionStore 回填可手改)、videoOnly、**preflight 检查门**(清单行 data-warn 双通道,
  问题项裁决「仍要导出/先不导出」)、export_all_variants 批量入队。
- **媒体面板双页签**:工程素材(原面红线不动)/ 素材库(库根手填偏好记忆/kind chips/
  标签过滤/搜索/一键导入/标签整组替换);**media_import 拷贝导入三通道**(绝对路径输入/
  文件选择器/素材库引用,FE1 登记的拷贝导入通道就此闭环)。
- 顺手修复:media-pane flex 滚动链回退(e2e_media_perf P95 63.7fps 不降反升)。

## T6.4 应用化 + T6.5 独立验收 + gate A6 + 文档收官(F4)

实码(crates 少量 + tools/packaging/docs;工具数 **76 = 18+38+20 不变**——cfproj 面全走
既有 library_manage 参数面,零新工具):

- **`.cfproj` 工程描述文件(T6.4)**:轻量 JSON `{kind:"cutforge-project",version,name,root,
  rev,createdAt}` 快照——**不是真相源**(root 指向工程目录),失效给可读错误(工程被移走/
  kind 不匹配/缺 root)。单一实现进 `cutforge_io::library`(export_cfproj/parse_cfproj,
  写走 atomic 唯一落盘点);生成入口 = `library_manage action=export_cfproj`(MCP)+
  `cutforge-cli library cfproj`(CLI,同实现);关联打开 = `serve <x.cfproj> --open`
  (cli serve 与 mcp serve 共用 `resolve_root_arg`,安装器文件关联即此命令)。
- **端口占用自动换端口 + 横幅(T6.4)**:收敛 `serve_workspace` 单一实现——请求端口被占
  → 自动换 +1..+20 首个空闲并打横幅(排查命令内联);两面旧行为(CLI 静默预选 /
  MCP 直接 bind 失败)不一致且用户不知情,就此统一;`.cutforge/session` 的 port 改为
  **绑定后**写入(实际端口,不再可能记账假端口);全窗占满才失败(退出码 4 不变)。
- **`doctor --bundle`(T6.4)**:诊断包 = 单文件 zip(doctor.json + environment.txt +
  session/session-summary/rev/project.json 快照,在位才收、单件 512KB 上限)。zip 为
  **零依赖手写 store 形**(method 0 + UTF-8 名 + CRC-32;新增 Cargo 依赖违反纪律而包体
  KB 级压缩收益趋零),落盘走 atomic;Python zipfile 实测可解(testzip 干净)。
  顺带:CLI `library` action 面同步增 `cfproj`。
- **安装器与验收面(T6.4/T6.5,诚实登记)**:`packaging/cutforge.iss`(Inno Setup 6:
  core 固定组件 + ffmpeg 默认勾选可选组件(构建时 `packaging/ffmpeg/` 放入即启用,
  ADR-0022)、快捷方式、卸载器(用户工程不删)、`.cfproj` 关联、勾选内嵌时写
  HKCU\Environment 两个变量;`cutforge://` 协议注册注释留白未做)、
  `packaging/README.md`(内容清单/启动方式/便携版与安装器差异;**顺带诚实登记 release
  job zip 只拷 web 三件的现状**,全树化留 A6-L)、`packaging/pure-checklist.md`
  (AC-6.5/6.6 人工清单:装机→全流程→卸载核查)。**ISCC 不在开发机,实际编译与 VM
  验收为人工项**(脚本经结构人工核对,未经编译的 Pascal 代码一律不写)。
- **`tools/e2e_independence.py`(T6.5,AC-6.3 判定器)**:
  - 环境隔离:剥离全部 `CUTFLOW_*` 再起 serve(serve 子进程 env 干净,断言"独立机器"
    语义;CI ubuntu 无 CUTFLOW_REPO 天然隔离,恰是主场);
  - 四大件全走内置工具链:clip_add+裁 → subtitle_import(SRT,纯绿 textStyle)→
    text_add 花字(hz.neon)→ audio_beats(onset-energy,脉冲夹具 onsetCount=19)→
    reframe 承载防丢(种子片段在真相源,工具链写链后 anchorY 原样在——渲染端零消费
    与编辑面未承接均为登记口径,不虚标)→ render_run 导出;
  - **AC-6.3 核心判定**:采样线程对 serve 进程子树全程采样(psutil 优先,Windows wmic /
    POSIX /proc 兜底),断言零 python 子进程(本机实证子树名单
    `['cutforge-render.exe','ffmpeg.exe','ffprobe.exe']`;剪映导出豁免——本脚本不调用
    export_jianying,ADR-0023 注明);
  - 产物校验:ffprobe 时长对拍(6.00s ≈ 6s)+ 像素抽样(t=2.0s 深灰底上纯绿字样
    1704 像素 = 字幕烧录实证,零 PIL 依赖 rawvideo 解析);
  - 纯 CLI 面(AC-7.4 预演):v3 布局新工程 → run-script 无头建卡(沙箱零逃逸)→
    `cutforge-cli clip-update` 改字段 → `cutforge-render` 直渲 → v3 产物落 exports/ →
    ffprobe 校验——全程无 UI 无 serve。
- **gate A6 注册**(D-A2 制):A5 二十一项一字不动全部继承 + e2e-independence,
  **22 项全阻断**;CI web-e2e 只加 e2e_independence 一步(gate.yml 结构校验过)。
- **录屏 6 支补录**(record-captures 口径,册六 50- 段低分辨率):工程库七操作/迁移 v3/
  向导模板/导出矩阵/preflight 检查门/素材库(见下验证节)。
- **文档收官**:本台账 F3/F4 补账 + AC-6.1~6.7 状态表(下);CHANGELOG 册五收口波补记 +
  册六 F3/F4 条目;README(册六段/门禁表 A6/e2e ×18/快速开始 cfproj+bundle+端口横幅/
  gate 命令 A6);FLOW(gate 段 A6 注册 + 工具面 76);capability-matrix #43–#51(F2
  已落,本轮零结构变化)。

验证(2026-09-30 本机,F4):

- `cargo test --workspace --locked` 全绿(io 新增 cfproj_export_parse_roundtrip;cli 新增
  bundle 三测:CRC-32 已知向量/DOS 时间抽样/zip 结构逐字段回读);
- `cargo clippy --workspace --all-targets -- -D warnings` 零告警;
- 18 份 e2e 全绿(17 份既有 + e2e_independence;负载敏感项安静时段复跑);
- `tool_parity --update-golden` 后连跑两次 0 DRIFT(76 工具;library_manage 增 action
  为加法参数面,golden 夹具无键变化);
- `check_doc_counts`:76 = 18 + 38 + 20 四文档零漂移;
- gate A6 二十二项全绿 + A1–A5 复跑全绿 + M0/M1 绿(M2–M7 外部红项如实注明);
- `doctor --bundle` 产物 zipfile 实测:testzip 干净、entries=[doctor.json, environment.txt,
  project.json];serve 端口占用横幅实测(CLI 与 MCP 两面);`.cfproj` 导出→解析→serve
  打开与坏描述报错实测;
- 录屏:34+6=40 支,目录总量 7.89MB ≤ 8MB 红线(册六 6 支 50- 段 720×450 低分辨率
  录制 + VP9 CRF46 重编码压总量,时长逐支不变、单文件 ≤2MB;54-preflight 走「导出成片」
  自动门触发——裁决按钮只在 runPreflight(auto) 路径渲染,手动检查面无裁决;
  回链 micro-interactions.md 第十节,TESTIDS 第九节为 F3 已登记面)。

## AC-6.1~6.7 状态表(册六收官口径)

| # | 验收项 | 状态 | 判定证据 |
|---|---|---|---|
| AC-6.1 | 工程库七操作 + 崩溃恢复零数据丢失 | ✅ | F1(io 单测:七操作/活锁拒绝/崩溃恢复 OpLog 完整性 + dispatch 闭环)+ F3(库视图七操作 UI);强杀恢复 e2e 化登记 A6-L(既有单测覆盖 pid 探测/清锁/校验) |
| AC-6.2 | 布局 v3 + v2 兼容读写 + 迁移幂等 | ✅ | F1(迁移器幂等 NOOP/冲突整体拒绝/project.json 字节零改动)+ F3(向导 v3/迁移入口);**缺省翻转 V3 未做**(前提:FE+安装器收编,ADR-0021 过渡期,见 A6-L) |
| AC-6.3 | 四大件内置,零外部脚本调用 | ✅ | `tools/e2e_independence.py`(CUTFLOW_* 剥离 + 进程树断言零 python 子进程 + 剪映豁免注明);诚实口径:重构图=anchorY 承载防丢(渲染零消费登记 #50),转写=明确不做(#51,ADR-0023) |
| AC-6.4 | 导出矩阵格式/预设/区域/队列/多画幅 | ✅ | F2(七出口渲染端 + 队列 + 批量变体,parity 夹具+手工冒烟)+ F3(矩阵面板/preflight 门 UI);导出期间编辑不阻塞为册二既有 e2e 断言 |
| AC-6.5 | 安装器纯净机装/卸干净 + 文件关联 + 单实例 | ◐ | 安装器脚本/关联/env 写入**就绪**;ISCC 编译验证 + VM 装/卸核查 + 关联双击生效 = **人工项**(pure-checklist §1/§3/§4);单实例唤起协议 = 可选未做(A6-L) |
| AC-6.6 | 纯净机全流程(T6.5 脚本在无 Python 环境通过) | ◐ | 清单脚本化 `packaging/pure-checklist.md`;e2e_independence 已在"剥离 CUTFLOW_*"语义下实证;真实 VM(无 Python)执行 = **人工项** |
| AC-6.7 | 全量回归零劣化 | ✅ | gate A6 二十二项全绿(A1–A5 一字不动继承);media_perf P95 63.7fps(F3 修复后)不降;bench 阈值 AC-5.7 照跑 |

## 遗留(A6-L,册六收官登记)

1. **缺省布局翻转 V3**:过渡期 scaffold 缺省仍 v2(ADR-0021);翻转前提 = FE(v3 全路径
   打磨)+ 安装器收编(纯净机首装即 v3),e2e 断言(目录树/media 落点)同步翻转。
2. **serve `projectRel` 迁移后刷新**:`.cutforge/session` 的 projectRel 是启动时快照,
   serve 进行中迁移 v3 后壳读到的是旧相对路径(重启即正;待做:迁移完成通知刷新)。
3. **他工程缩略图**:库卡片缩略图取自 `.cutforge/thumb-cache`,仅当前(或曾打开)工程有
   产物;冷工程卡片无缩略图(诚实留白;候 media_thumbnail 批量派生评估)。
4. **media_library e2e 覆盖**:素材库 manifest 扫描/标签/过滤面当前为 parity 夹具 +
   手工冒烟,无独立 e2e(册七候选)。
5. **单实例协议 `cutforge://`**:计划书标可选——未做;文件关联已用 `.cfproj` + serve
   落地(同一用户诉求的最低成本面),唤起式单实例待真实用例出现再评估。
6. **Inno 编译验证**:`packaging/cutforge.iss` 结构人工核对过,开发机无 ISCC;实际编译、
   VM 装/卸、关联双击、env 即时生效(要不要补 WM_SETTINGCHANGE 广播)按
   pure-checklist 执行——未经编译验证的 Pascal 代码一律不入库。
7. **release job zip 的 web 全树**:gate.yml release 打包目前只拷 web/ 三件
   (index.html/app.js/style.css;packaging/README.md 已如实登记),js/ 全树待补——
   便携版全功能依赖它;修 CI 步骤属一行改动,登记以免遗忘。

## 待办(册六后续任务)

- ~~T6.1 模板面、FE 工程库页~~ **已闭合(F3)**;
- ~~T6.2 内置能力收编(字幕/花字/卡点/重构图,AC-6.3 零外部脚本)~~ **已闭合(F2 收口
  + F4 e2e_independence 判定器;重构图/转写诚实降级与不做)**;
- ~~T6.3 导出矩阵与编辑不阻塞(AC-6.4)~~ **已闭合(F2 BE + F3 面板)**;
- ~~T6.4 应用化(安装器/文件关联/单实例/内嵌 ffmpeg)~~ **已闭合到脚本面(F4);单实例
  协议可选未做、Inno 编译验证人工(见 A6-L 5/6)**;
- ~~T6.5 纯净机全流程~~ **清单与判定器已闭合(F4);真实 VM 执行人工(AC-6.6,见上表)**;
- 缺省布局翻转为 V3(前提:FE + 安装器收编,e2e 断言同步)——**A6-L 1,候册七**。
