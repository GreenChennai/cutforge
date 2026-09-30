# 更新日志(CHANGELOG)

格式参照 Keep a Changelog;版本号遵循语义化版本(SemVer)。

## 未发行(Unreleased)

> 册一「内核重构与架构加固」批次(128856c / 3ce2884 / 7157178)+ 册二「前端壳重写」
> 批次(7f02955 之后的未提交工作面)+ 册三「UX 动效/键位/可访问性」批次 +
> 册四「核心工具与媒体管线」批次(a2ba870~77cade5 已提交;FE2/收口波为未提交工作面);
> 版本号待发行时定(沿用仓库惯例:发行时才把本段改为版本号。此前的 v0.6 批次——
> 编辑器 NLE 化、schema v2 M14 双仓同步——发行时一并补记)。
> 册二台账见 [docs/A2-PROGRESS.md](docs/A2-PROGRESS.md);册三台账见
> [docs/A3-PROGRESS.md](docs/A3-PROGRESS.md);册四台账见
> [docs/A4-PROGRESS.md](docs/A4-PROGRESS.md)。

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
  降级保留(册二后新壳以 SSE 为主通道,长轮询转为断线降级路径,去留册三定夺);连接纪律
  (读超时/请求体上限/总时限/慢连接隔离)。
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
- **Web 壳模块化重写(册二)**:单文件旧壳拆为 **core/render/panels/ui 四层无构建 ESM**
  (37 个 js 共 3,555 行,单文件最大 354 行;index.html 78 行),六 store + projector 只读
  投影 + keyed 增量渲染(一次 clip move 相关 DOM 变更 **7 次**,旧壳数千次)+ 播放解耦
  媒体元素池 + 1k clips 虚拟化;SSE 主通道 + 长轮询断线降级;`data-testid` 全量锚点
  (apps/web/TESTIDS.md);新增交互:右键菜单、轨头眼睛开关(ephemeral 视图隐藏)、
  导出剪映草稿按钮、数字字段拖拽调节、向导模态 a11y、快捷键调度器。旧壳保全
  `apps/web/legacy/` 经 `/assets/legacy/` 回退(册三收尾删)。决策见 ADR-0011(无构建
  ESM)/0012(canvas 重绘层+DOM 交互层)/0013(临时投影三原则与 `ephemeral.*`);
  台账见 [docs/A2-PROGRESS.md](docs/A2-PROGRESS.md)。
- **`render_frame` 单帧精确预览工具(41→42 = 13 查询 + 21 写 + 8 编排)**:壳「精确预览」
  按钮消费;帧缓存键 = 工作区指纹 + atMs(100ms 量化)+ 画幅 + 版本 + ASS 哈希,
  改一笔必 miss;超时 10s 中止、未落账、可恢复。
- **e2e 体系扩容(册二)**:三份旧脚本选择器迁 `data-testid` 全绿;新增
  `e2e_ui_smoke`(DOM 变更预算 / selfTest 重建铁律 / 超时-401-SSE 降级三场景)、
  `e2e_playback_survival`(播放零中断:283 帧采样 currentTime 回跳 0ms)、
  `e2e_perf_timeline`(1k clips 虚拟化 + 滚动 P95 60.2fps;`--min-fps` 参数化,
  负载敏感不进 CI);CI web-e2e 增前两步。
- **壳纯度门禁升级 v2**(`check-shell-purity`):R1 持久化语义禁令 / R2 投影只读
  (timelineStore 只准 projector 写)/ R3 禁裸 fetch(白名单 api.js)/ R4 legacy 豁免
  (删 legacy/ 时同步收口);`gate.py A2` 册级门禁注册(11 阻断 + 1 观察 legacy-reminder)。
- **设计系统与主题策略(册三,ADR-0014)**:主题本册仅深色(类达芬奇蓝灰 13 档 +
  单强调色);tokens 三层(原始/语义/组件),**唯一色值定义点 `css/tokens.css`**;
  对比度 19 组正文 + 4 组图形/大字全 AA(`docs/design/contrast-table.md`);壳纯度门禁
  升级 **v3 新增 R5 零硬编码色值**(css+js+html 三面扫描,注释不豁免,注入样例必抓;
  豁免登记仅 tokens.css + assets/icons.js);canvas 取色收口 `js/render/theme.js`,
  与 DOM 面同源。
- **动效与微交互(册三)**:时长梯度 80/160/240ms + 三条缓动 token;一律
  transform/opacity 合成器路径;**27 项微交互清单全实现**(`docs/design/micro-interactions.md`);
  **19 条录屏存档 `docs/design/recordings/`(≈4.7MB,`record-captures.py` 可重录)**;
  `prefers-reduced-motion` 一处总控;降噪纪律(同屏并发 ≤3 / hover 无位移 / 拖拽零动画)。
- **精确拖拽手势(册三)**:pointer capture 管线(gesture-kit + gestures):ghost 跟手
  ≤1 帧、3px 阈值、**Esc/失焦取消零 Op**;trim 碰撞夹取 + 实时时长气泡;框选多选;
  边缘 60px 自动卷入;Ctrl/⌘+滚轮视口中心缩放;拖拽全程零 Op、松手单命令
  (e2e_drag_perf:拖拽 P95 60.2fps)。
- **快捷键体系(册三)**:**45 条数据化注册**(`window.__cfKeymap.table()` 可导出全表),
  全部可重绑定 + 冲突检测(强制 = 停用被占)+ 恢复默认,localStorage 偏好;J/K/L 倍速链、
  I/O 入出点、M 会话级标记、B 切割模式;输入态屏蔽(data-gate 可断言);「?」帮助面板
  全表 + 搜索(e2e_hotkeys:遍历 45 条 + 实按 26 条 + 重绑定闭环)。
- **dev 性能面板(册三,Shift+D 默认关)**:帧率 / rAF 分布 / DOM 数 / 未完成请求 /
  投影耗时 / 媒体池占用 + 预算表逐行可视(`data-pass` 非颜色线索);媒体池
  **POOL_MAX=24 LRU + 播放头窗口锚定**;预算口径真相源 `docs/design/perf-budget.md`;
  `e2e_perf_budget` 结果落盘 `docs/bench/perf-a3.json`(boot 245ms<1s / 页签 44.9ms<100ms /
  池 200 轮导航有界)。
- **可访问性(册三)**:键盘编辑闭环(选择→移动→删除→撤销,toast 撤销按钮,全程
  rev/OpLog 断言);模态 aria-modal + 焦点归还;右键菜单带快捷键提示与禁用原因;
  交互元素全 title;状态线索非颜色单依赖;**axe-core 4 全页扫描 0 critical/serious**
  (`tools/vendor/axe.min.js` 入库存档;e2e_a11y)。
- **新手路径(册三)**:脚本盲测三流程(导入→剪切→导出 / 加转场 / 加 BGM)零卡点
  (`docs/design/novice-blind-test.py`);卡点修复对照表 `docs/design/novice-audit.md`:
  素材落点被占自动顺接 / 分割菜单自适应 / 首启引导条(可关)/ 时间线空态下一步 /
  删除 toast 5s 真撤销 / 批量撤销确认(可关)。
- **错误面收口(册三,A2 遗留)**:net 错误横幅(`banner-conn`,恢复自动收起,A2-L1);
  SSE 连接态徽标(`conn-badge`:已连 / 降级轮询 / 重连中,A2-L5);长轮询降级定夺为
  **正式降级面**(A1-L3 了断)。
- **`apps/web/legacy/` 整树删除(册三收尾)**:回退期结束;purity R4 豁免收口、
  gate A2 legacy-reminder 观察项移除、全仓引用清理(A2-L2 了断)。
- **e2e 体系扩容(册三,×8→×12)**:新增 `e2e_drag_perf`(拖拽手感:帧率/跟手/取消零 Op)、
  `e2e_hotkeys`(键位注册表遍历 + 实按 + 重绑定)、`e2e_a11y`(键盘链 + axe 扫描)、
  `e2e_perf_budget`(首屏/页签/导出节奏/池有界,结果落 `docs/bench/perf-a3.json`);
  CI web-e2e 增 hotkeys/a11y 两步(drag/perf/budget 负载敏感不进 CI);
  `gate.py A3` 册级门禁注册(**15 项全阻断**)。
- **时间线编辑全工具(册四 A4-BE1)**:`clip_trim` 四件套(trim/roll/slip/slide,
  碰撞守护)、`clip_split_all`、`track_update`(七字段)、`clip_gap_delete`、
  `clip_copy` + `clip_paste_at`(会话剪贴板);Track 字段进 IR + merge 承接 +
  roundtrip 证明;工具 42→**48 = 13 查询 + 27 写 + 8 编排**;parity golden 48 重录。
  决策见 ADR-0015/0016/0017(画布范围约束/文本 ASS 路线/音频分离延后评估)。
- **曲线变速与画布扩宽(册四 A4-BE2)**:`speedCurve` 分段积分曲线变速(单一真相源
  `speed_segments`,投影/渲染时长一致三道对拍)+ `reverse`(areverse,先于变速)+
  变换链 crop→flip→rotate + 定格组合;画布自定义扩为 **64–7680 偶数**范围约束 +
  推荐集(双端 multipleOf 对拍;ADR-0015);parity 夹具 9→14 全绿。
- **转场/特效/动效库(册四 A4-BE3a)**:转场库 **7→58**(ffmpeg 实测枚举五分类,
  目录 `tr.*` 直通;`GET /catalogs` 下发 + 缩略图生成器)+ `acrossfade` 音频转场
  (构造性零漂移);fx 注册表 **11** 特效(combo ≤3,未注册 fxId 逐项降级 WARN);
  motion 真实渲染 **19** 项(不能真实渲染的不进目录,诚实纪律)。
- **文本渲染与字幕/音频工具(册四 A4-BE3b)**:`textStyle` 14 字段 → 确定性 ASS
  复用烧录链(PlayRes=画布,所见即所得;ADR-0016)+ 花字 12 模板 + 卡拉OK `\kf`;
  字幕工作流 `text_add` / `subtitle_import` / `subtitle_replace` / `subtitle_export`
  (SRT 往返 **byte 级相等**,批量替换单 Op 原子);音频 `denoise` 四档(afftdn)/
  `pitch` 保速变调 / `audio_beats` 启发式卡点(诚实标注);track mute/solo/hidden
  渲染联动;媒体缩略图/代理/peaks(mtime+size 内容寻址)+ `useProxy` 显式 opt-in;
  工具 48→**56 = 13 查询 + 30 写 + 13 编排**。
- **编辑器前端工具面板(册四 FE1/FE2)**:媒体池缩略卡懒加载(视口外零请求)+
  音频波形密度档 + 代理开关;A/B/T 工具模式 + 四件套手势(Alt=slip/Ctrl=slide/
  Shift+边缘=roll,一次手势恰一 Op)+ 轨道头七字段 + **历史面板**(回跳 N 笔撤销/
  快照标记);转场库 58 网格 + 特效栈编辑器 + 动画选择器 + 文本工具(画布拖位置,
  渲染帧字节随动)+ 花字库 + 字幕编辑器全流程 + 音频工具 UI + 画布五档预设 +
  PiP 变换把手 + 曲线点集编辑器。
- **e2e 体系扩容(册四,×12→×15)**:新增 `e2e_editing_tools`(四件套恰一 Op +
  实渲时长语义逐差 + 锁定轨拒编辑零 Op)、`e2e_subtitle_editor`(SRT byte 级往返 +
  帧证文字位置 + 卡拉OK推进 + 花字)、`e2e_media_perf`(1000 素材 P95 63.3fps +
  懒加载 + AC-4.6 听觉存档四样本落 `docs/design/audio-samples/`);CI web-e2e 增
  editing_tools/subtitle_editor 两步(media_perf 负载敏感不进 CI);
  `gate.py A4` 册级门禁注册(**18 项全阻断**);台账见
  [docs/A4-PROGRESS.md](docs/A4-PROGRESS.md)。

### 变更

- **三巨石拆分(纯移动,零行为变化)**:`cutforge-mcp/src/lib.rs`(2,146 行)→
  registry/dispatch/orchestrate/progress/session/tools_nolock/workspace_svc +
  `transport/{stdio,http,events,static_files}`;`cutforge-io/src/lib.rs`(1,120 行)→
  `workspace/` 显式步骤管线(apply 八步状态机,先文件后记账实序在代码注释声明);
  `cutforge-core/src/engine.rs`(1,060 行)→ `engine/{apply,undo,replay,projection,invariants}`。
- 全仓 `cargo clippy --workspace -D warnings` 清零;非测试源文件全部 ≤800 行(最大 791)。
- 性能:bench 基准达标(1k 工程查询 18.6ms≤50ms、提交 71.3ms≤100ms;经常驻工作区
  +watcher 免开合并专项优化)。

### 修复

- **Windows 字幕烧录必炸的真实 bug(册二顺带修复)**:烧录滤镜参数内的路径反斜杠会被
  ffmpeg filtergraph 转义规则吞掉,导致 Windows 上字幕烧录路径必然失败;滤镜参数内路径
  统一正斜杠(`crates/cutforge-render/src/frame.rs`),并新增「烧录前后帧字节必不同」
  实渲测试防回归。
- **trim/Esc 取消后几何残留(册三修复)**:Esc/失焦取消 trim 后,trim 期间写入的
  内联几何(宽度/位移)残留在 DOM——重投影按 meta 比对不会重写同值节点;取消路径
  统一把内联几何复位回投影值(`apps/web/js/render/gestures.js`),e2e_drag_perf
  断言取消后盘面几何与投影一致。
- **转场 offset 截断虫(册四 A4-BE3a)**:尾帧扩展时长误入累计,转场点越靠后累计
  偏移越大,致转场后段整段丢失;旧夹具被容器时长骗过未暴露——夹具升级为容器与
  视频流双 4.000s + 120 帧锁断言防回归(`crates/cutforge-render/tests/parity_matrix.rs`)。
- **变调听觉存档链 atempo 方向反写(册四收口)**:存档生成器把 `atempo` 写成变速比
  k 而非 1/k,变调样本时长漂移、与「保速变调」语义相反;改为 1/k 减速拉回,并加
  「源 4s 处理后仍 4s」保速断言把守(`tools/e2e_media_perf.py`;滤镜映射与内核
  `across.rs` 逐字一致)。

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
