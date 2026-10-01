# CutForge

> **与 OpenCut 的关系**：CutForge 是一个**独立项目**，不是 OpenCut 的官方版本、
> 分支或衍生发行版，与其维护者无隶属或背书关系。我们在领域模型与工程结构上
> 参考了 [OpenCut](https://github.com/OpenCut-app/OpenCut)（MIT，**活跃开发中**）
> 与其经典版 [opencut-classic](https://github.com/OpenCut-app/opencut-classic)
> （MIT，**已于 2026-05-17 归档**）。其 MIT 许可全文见 `LICENSE-OPENCUT.MIT`，
> 完整归属见 `NOTICE.md`。本项目原创部分适用 `LICENSE`（ARL-1.0）。

<h1 align="center">CutForge · Rust 视频编辑器内核</h1>

<p align="center">
  <strong>可独立起步,也为 AI 而生</strong><br>
  新建空工程 → 导入素材 → 多轨编辑 → 导出,全程不依赖任何管线;<br>
  同时也能直接打开 <a href="https://github.com/GreenChennai/CutFlow">CutFlow</a> 工程,与它共用同一份工程文件——<br>
  人的每一次拖拽都是一个可审计、可撤销的 Op,AI 改了什么,<b>时间线上看得见,日志里查得到</b>。
</p>

- **一个内核,多个壳**：时间线模型、命令与撤销、操作日志（OpLog）、渲染调度只实现一次，Web 壳与桌面壳都是薄壳，杜绝双实现语义漂移。
- **文件是真相源**：工程（`project.json` / `wordline.json` / `cutlist.json` / `notes.json`）即同步面，任何写入者（人、AI、脚本）经同一命令通道产生可审计、可撤销的 Op。
- **为 AI 而生**：内置 MCP server 与脚本宿主；AI 的每次改动可 diff、可回滚，用户在时间轴上打的标注 AI 能读到、能执行、能回执。
- **与 CutFlow 分工而非合并**：[CutFlow](https://github.com/GreenChennai/CutFlow)（Python）负责 S0–S11 视频管线的批量机械工作（转写对齐、粗剪、合成、字幕、烧录、自检）；CutForge 负责"人的手"——交互、预览、标注与精确编辑。两者读写同一份工程文件。

## 🆕 阶段二 · 从"能打开"到"像剪映一样用"

> 最小可用集凑齐:**新建 → 导入 → 预览 → 全字段编辑 → 保存 → 导出 → 改动被 CutFlow 识别**。

- **从零剪**:空工程模板(`cutforge-cli new` 或编辑器「＋ 新建工程」向导)→ 素材面板双击/拖拽导入(落点=播放头,帧磁吸;时长自动 ffprobe 探测)→ 多轨编辑 → 导出,**全程不依赖 CutFlow**(e2e 固化);
- **检查器不再缺斤短两**:可编辑字段集由 [schemas/ui-fields.json](schemas/ui-fields.json) **单一真相源**声明(基础/画面/音频/变速/文本五组),机械校验保证"壳允许编辑的 ⊆ 内核 `ClipPatch` 支持的"——内核有而壳没有的漂移从此被门禁拦住;转场/淡变等先做只读展示;
- **并发不卡顿**:查询类工具改只读打开,不再持排他锁——**长渲染期间查询/编辑照常响应**(并发 e2e 断言,实测 ≤0.02s);渲染编排保持子进程,不占 workspace 锁;
- **改动识别闭环**:编辑器每次落盘即 Op+rev,`.cutforge` 留**会话变更摘要**;回 CutFlow 一侧 `rs_run --status` 精确标脏、`rs_editor.py diff` 输出人话差异("V1 第 1 段延长 1.5s""V2 新增 1 卡 5.0–6.8s"),再决定重建范围——人改完,AI 接得住;
- **写盘纪律**:全部落盘收口到 `atomic.rs`(旁路写入审计 0 命中),CutFlow 的记账文件进 watcher 忽略表,跑管线不再惊动编辑器。

## 状态

M0–M4 已完成并通过门禁。路线图：M0 合规立项 ✓ → M1 契约固化 ✓ → M2 Rust 内核 ✓ → M3 双向同步与标注 ✓ → M4 MCP 与脚本 ✓ → M5 多端壳 → M6 渲染后端 → M7 开源发布。

**册一(A1 · 内核重构与架构加固)已完成**：三巨石拆分（mcp / io / core 模块化）、渲染
RenderPlan 七步分解、渲染缓存内容寻址（+ `cache` CLI）、`/assets` 目录托管、SSE 事件面、
HTTP 连接加固（ADR-0009）、错误码命名空间 + `doctor` 诊断、`bench` 性能基准、
`tool_parity` 黄金对拍、`gate.py A1` 册级门禁注册；台账见
[docs/A1-PROGRESS.md](docs/A1-PROGRESS.md)。

**册二(A2 · 前端壳重写)已完成**：Web 壳重写为 **core/render/panels/ui 四层无构建 ESM**
（37 个 js 文件单文件最大 354 行；旧壳 legacy/ 已于册三收尾删除，回退期结束）、
六 store + 只读投影 + keyed 增量渲染（一次 clip move 相关 DOM 变更 7 次）+ 播放解耦 +
1k clips 虚拟化（滚动 P95 60.2fps）、`render_frame` 精确预览工具（41→42 工具，时点口径）、
顺带修复 Windows 字幕烧录路径 bug、三份新 e2e + `data-testid` 全量锚点
（[apps/web/TESTIDS.md](apps/web/TESTIDS.md)）、`check-shell-purity` v2 + `gate.py A2`
册级门禁；决策见 ADR-0011/0012/0013,台账见
[docs/A2-PROGRESS.md](docs/A2-PROGRESS.md)。

**册三(A3 · UX 动效/键位/可访问性)已完成**：设计系统 ADR-0014(本册仅深色,tokens 三层
唯一色值定义点,对比度 19 组正文 + 4 组图形全 AA,壳纯度 v3 新增 R5 零硬编码色值)、
27 项微交互全实现 + **19 条录屏存档**（[docs/design/recordings/](docs/design/recordings/),
可重录）、精确拖拽手势（ghost 跟手 ≤1 帧 / Esc 取消零 Op / 拖拽 P95 60.2fps）、
**45 条可重绑定快捷键**（冲突检测 + 「?」帮助面板全表搜索）、dev 性能面板（Shift+D）
+ 媒体池 POOL_MAX=24 有界、可访问性（键盘编辑闭环 + axe 全页扫描 0 critical/serious）、
新手路径脚本盲测三流程零卡点；顺带收口 `apps/web/legacy/` 整树删除、net 错误横幅 +
SSE 连接态徽标；e2e 扩至 **12 份** + `gate.py A3` 15 项全绿;决策见
ADR-0014,台账见 [docs/A3-PROGRESS.md](docs/A3-PROGRESS.md)。

**册四(A4 · 核心工具与媒体管线)已完成**：决策 ADR-0015/0016/0017(画布 64–7680 偶数
范围约束/文本 ASS 路线/音频分离延后册五评估);工具面 **42→68 = 15 查询+34 写+19 编排**(册五 T5.2/T5.3/T5.6 增 lut_import/scope_data/audio_loudness/encode_probe/render_queue 五工具;T5.4/T5.5 增 compound_create/compound_unbind/multicam_cut/scene_detect/multicam_sync/otio_export/otio_import 七工具)——
时间线编辑六工具(`clip_trim` 四件套 trim/roll/slip/slide、`clip_split_all`、
`track_update` 七字段、`clip_gap_delete`、`clip_copy`/`clip_paste_at` 会话剪贴板)、
speedCurve 分段积分曲线变速 + reverse + 变换链、**转场库 7→58**(五分类 ffmpeg 实测
枚举 + acrossfade 音频转场,顺手修转场 offset 截断虫)、fx 注册表 11 特效(combo≤3)+
motion 真实渲染 19 项、文本渲染(`textStyle` 14 字段→确定性 ASS 所见即所得)+ 花字 12
模板 + 卡拉OK、字幕工作流四工具(SRT 往返 byte 级相等)、音频降噪四档/保速变调/卡点、
媒体缩略图/代理/peaks;前端媒体池缩略懒加载 + 波形、四件套手势(一次手势恰一 Op)、
轨道头七字段、**历史面板**、转场/特效/文本/字幕/音频工具面板、画布五档预设、曲线点集
编辑器;三份新 e2e(×12→**15**)+ 听觉存档四样本([docs/design/audio-samples/](docs/design/audio-samples/));
`gate.py A4` **18 项全绿**;决策见 ADR-0015/0016/0017,台账见
[docs/A4-PROGRESS.md](docs/A4-PROGRESS.md)。

- **M0**:ARL-1.0 混合授权三件套、命名核查存档、工具链 pin(与上游一致)、统一门禁入口、CI 骨架。
- **M1**:五份 schema(唯一手写契约)+ 双端代码生成(Python 生成校验器 / Rust `cutforge-schema`)+ 常量单源零漂移 + 迁移器幂等 + 回归集对拍(双端结论逐样本一致)。
- **M2**:`cutforge-core`(领域模型/命令通道/撤销栈/OpLog/三路合并骨架/锚点,行覆盖 ≥80%,wasm32 可构建)+ `cutforge-io`(工程读写/原子写唯一落盘点/锁/备份/媒体探测/轮询 watcher)+ `cutforge-cli`(打开/查询/应用/撤销重做/OpLog + 门禁判定器)。
- **M3**:双向同步全链——三路合并九行判定表零静默覆盖(12,000 组属性测试)、OpLog 回放等价(含 undo/redo 混入)、冲突三方快照落盘(`.cutforge/conflicts/`)、标注(notes.json)读写/结案回执绑定 opIds/锚点重定位(100 组场景零丢失)、阶段脏传播(改 IR 只标 S3+;改字幕只重烧 S8)、往返延迟基准(AI 可见 P95 ≤100ms,实测个位数毫秒)。
- **M4**:MCP 层——单注册表(28 工具为 M4 时点;阶段二新增 clip_add/media_probe/media_browse/project_new,阶段三新增 transition_set/motion_set/bgm_set;册二 A2 新增 render_frame;册四 A4 新增 clip_trim/clip_split_all/track_update/clip_gap_delete/clip_copy/clip_paste_at;册四 A4-BE3b(时点 56)新增 text_add/subtitle_import/subtitle_replace/subtitle_export/media_peaks/media_thumbnail/media_proxy/audio_beats;册五 A5 新增 lut_import/scope_data/audio_loudness/encode_probe/render_queue;A5-BE3(T5.4/T5.5)新增 compound_create/compound_unbind/multicam_cut/scene_detect/multicam_sync/otio_export/otio_import 后为 **68 工具 = 15 查询+34 写+19 编排**,当前一律以 `schemas/mcp-tools.json` 为准),stdio 与内嵌 HTTP(127.0.0.1+token)双通道共用同一 dispatch;脚本宿主 `cutforge-script`(批式步骤+策略沙箱,六类逃逸零到达派发器);CutFlow 侧四个桥脚本(rs_editor/rs_notes/rs_oplog/rs_gate)入 doctor 体检与命令速查表。

## 🚀 快速开始(编辑器,4 步)

**无需 Rust 工具链**:GitHub Release 下载对应平台压缩包(`cutforge-windows.zip` / `cutforge-linux.zip` / `cutforge-macos.zip`,内含 `cutforge-cli` / `cutforge-mcp` / `cutforge-render` 三个二进制与 `web/` 静态资源),解压即用;校验见随包 `SHA256SUMS-<os>.txt`。

1. **启动**:`cutforge-cli serve --open`(推荐;无参数时交互选择工程,回车 = 最近工程),或 `cutforge-mcp serve --root <工程目录> --open`(无 --root 同样交互选择)。Windows 也可双击仓库根的 [start-editor.cmd](start-editor.cmd)。
2. **浏览器**:带 `--open` 自动打开;否则手动访问控制台打印的 `http://127.0.0.1:<端口>/?token=<T>`。
3. **编辑与导出**:素材面板双击/拖拽导入(时长自动探测)、时间线精确拖拽(ghost 跟手、Esc 取消、trim 时长气泡)、四件套微调(Alt=slip/Ctrl=slide/Shift+边缘=roll,一次手势一个可撤销 Op)、分割 / 波纹删,预览(画质代理,空格播放、←/→ 逐帧;「精确预览」按钮按播放头出单帧最终效果图,走 `render_frame`),**转场库 58 + 特效栈 + 动效/曲线变速**、**文本工具与字幕编辑器**(SRT 导入导出、花字、卡拉OK,画布拖位置所见即所得)、**音频降噪/变调/卡点**、**历史面板**(回跳 N 笔撤销),分组检查器(字段集由 `schemas/ui-fields.json` 单一真相源约束),标注,差异面板;**45 条快捷键全部可重绑定**(按「?」查全表 + 搜索;J/K/L 倍速、I/O 入出点、B 切割),**Shift+D 性能面板**(帧率/预算逐行可视,默认关);导出选 `cutforge` 后端即由本机内核出片,不依赖 CutFlow,亦可一键导出剪映草稿。
4. **从零新建**:编辑器顶部「＋ 新建工程」向导,或命令行 `cutforge-cli new <目录> --slug 名字 --fps 30 --track video,audio` 生成空工程后 `serve` 打开——对没有任何 CutFlow 工程的目录同样成立。

要点:服务仅监听 127.0.0.1;数据面(/rpc /media /session)经 Bearer token 鉴权,重启服务会换新 token;改动经 `/events` SSE 实时推送(断线自动转长轮询降级),静态资源由 `/assets/*` 目录托管(前端新增文件零 Rust 改动);退出 = 在服务窗口按 Ctrl+C。启动自检(`cutforge-cli doctor`)逐项报告工程 / ffmpeg / ffprobe / Web 资源 / 缓存目录 / 端口的就绪状态与可复制执行的补救命令。画质代理预览不含转场 / 特效 / 字幕烧录的最终效果(单帧最终效果用「精确预览」),成片请用导出。

### 给 AI 用户的打开方式

```text
「用 cutforge 打开这个工程,我想手动调几刀」
「把 V1 第 2 段删了,结尾留 1 秒黑场前把 BGM 淡出」
「我刚才在编辑器里改了什么?帮我只重跑受影响的阶段」
「新建一个 1080x1920 的空工程,我把素材拖进去」
```

## 🧬 门禁与自检(人眼判断不作为通过依据)

| 门禁 | 拦的是什么 |
|---|---|
| `check-shell-purity`(v2) | 壳里不准算时间线语义(R1 持久化禁令)、投影只读(R2:timelineStore 只准 projector 写)、禁裸 fetch(R3,白名单 api.js);色值纪律(R5,唯一定义点 tokens.css)。legacy/ 豁免(R4)已随 legacy/ 删除收口 |
| `check-write-paths` | 文件写入只允许走 `atomic.rs`,旁路写入=0(连注释里的字样都算命中) |
| `check-ui-fields` | 检查器可编辑字段 ⊆ 内核 `ClipPatch`(从实码解析,单一真相源) |
| `check_doc_counts` | 文档里的工具数口径必须与 `schemas/mcp-tools.json` 一致,漂移点名到文件:行 |
| `protocol_conformance` | MCP 注册表与 dispatch 逐一相等,stdio 与内嵌 HTTP 差异恒为 0 |
| `tool_parity` | 工具黄金响应库逐键对拍,行为漂移即红(册一 AC-1.2 建 41;册二 A2 增 render_frame 后 42;册四 A4 增六编辑工具后 48;册四 A4-BE3b 增八工具后 56;册五 A5 增调色/音频/队列五工具后 61,A5-BE3 增专业编辑/互操作七工具后 68;数量以 schemas/mcp-tools.json 实配为准) |
| `cargo test` | 领域模型/合并/OpLog/渲染对拍(ffmpeg 实测矩阵)/脚手架/只读并发/HTTP 加固/缓存寻址 |
| e2e × 17 | 编辑操作全链(Playwright)、预览(画面/声音/seek/像素非黑)、**从零剪**(新建→导入→改字段→导出+并发+会话摘要)、静态托管(穿越 100% 拒绝/ETag 304)、事件推送(SSE P95/长轮询降级)、UI 冒烟(data-testid 驱动/DOM 变更预算/selfTest/超时-401-降级)、播放生存(播放零中断)、时间线性能(1k clips 虚拟化+帧率,本机跑)、**拖拽手感**(P95/跟手 ≤1 帧/Esc 取消零 Op)、**键位体系**(45 条注册表遍历+实按+重绑定)、**可访问性**(键盘闭环+axe 扫描 0 critical/serious)、**性能预算**(首屏/页签/导出节奏/池有界)、**编辑全工具**(四件套恰一 Op+实渲逐差+锁定拒编辑)、**字幕编辑器**(SRT byte 级往返+帧证位置+卡拉OK+花字)、**媒体池千素材**(P95 63.3fps+懒加载+听觉存档)、**关键帧闭环**(秒表打点单 Op/投影采样对拍/曲线拖锚写回/菱形拖移)、**调色闭环**(色轮写回/帧色偏/LUT/示波器三画布/分屏割线)——册三新增四份、册四新增三份、册五新增两份,帧率类本机跑 |
| `gate.py A3`(册级) | UX/键位/可访问性册级聚合:**15 项全阻断**(纯度 v3 含 R5 色值/行数红线/tool_parity 42,时点口径/九份 e2e 聚合/pytest);负载敏感项本机册收官跑 |
| `gate.py A4`(册级) | 核心工具与媒体管线册级聚合:**18 项全阻断**(A3 十五项一字不动全部继承 + editing_tools/subtitle_editor/media_perf 三份新 e2e;tool_parity 61);千素材帧率等负载敏感项本机册收官跑 |
| `gate.py A5`(册级) | 专业深度册级聚合:**21 项全阻断**(A4 十八项一字不动全部继承 + keyframes/color 两份新 e2e + bench 阈值 AC-5.7 ≤20%;tool_parity 68);负载敏感项本机册收官跑 |

```bash
python tools/gates/gate.py M0 --json     # 统一门禁入口(结果协议见下)
cargo test --workspace --locked
python tools/e2e_edit_ops.py             # 需先 cargo build -p cutforge-mcp
python tools/e2e_preview.py
python tools/e2e_from_zero.py
python tools/e2e_static.py
python tools/e2e_events.py
python tools/e2e_ui_smoke.py             # 册二新增(data-testid 驱动 UI 冒烟)
python tools/e2e_playback_survival.py    # 册二新增(播放零中断)
python tools/e2e_perf_timeline.py --min-fps 55   # 册二新增(负载敏感,本机跑,不进 CI)
python tools/e2e_drag_perf.py --min-fps 55       # 册三新增(拖拽手感;负载敏感,本机跑)
python tools/e2e_hotkeys.py                      # 册三新增(键位注册表遍历+实按+重绑定)
python tools/e2e_a11y.py                         # 册三新增(键盘闭环+axe 扫描)
python tools/e2e_perf_budget.py                  # 册三新增(首屏/页签/导出节奏/池有界;含真实导出)
python tools/e2e_editing_tools.py                # 册四新增(四件套恰一 Op+实渲逐差+锁定拒编辑)
python tools/e2e_subtitle_editor.py              # 册四新增(SRT byte 级往返+帧证位置+卡拉OK+花字)
python tools/e2e_media_perf.py --min-fps 55      # 册四新增(千素材 P95+懒加载+听觉存档;负载敏感,本机跑)
python tools/gates/gate.py A1 --json     # 册级门禁(clippy/≤800 行红线/tool_parity/e2e/bench 聚合)
python tools/gates/gate.py A2 --json     # 册二册级门禁(12 项:壳行数/纯度 v2/e2e 面;1 观察)
python tools/gates/gate.py A3 --json     # 册三册级门禁(15 项全阻断:纯度 v3 含 R5/拖拽/键位/可访问性/性能预算)
python tools/gates/gate.py A4 --json     # 册四册级门禁(18 项全阻断:继承 A3 + 编辑工具/字幕/千素材三份新 e2e)
```

结果协议:`{"ok":bool,"code":str,"message":str,"data":object}`;退出码 `0`=通过、`2`=门禁失败、`3`=前置/环境缺失、`4`=内部错误。

## 📁 目录结构

```
cutforge/
├── crates/
│   ├── cutforge-core/      # 领域模型/命令通道/撤销栈/OpLog/三路合并/锚点(engine/ 五模块)
│   ├── cutforge-schema/    # 契约代码生成(与 Python 生成端对拍)
│   ├── cutforge-io/        # 工程读写/atomic.rs 唯一落盘点/锁/备份/探测/watcher(workspace/ 八步状态机)
│   ├── cutforge-render/    # 渲染后端(ffmpeg 直出;RenderPlan 七步 + 内容寻址缓存)
│   ├── cutforge-mcp/       # MCP server:registry/dispatch + transport/{stdio,HTTP,SSE,/assets 静态托管}
│   ├── cutforge-cli/       # CLI:serve/new/查询/应用/撤销重做/cache/doctor/门禁判定器
│   └── cutforge-script/    # 脚本宿主(批式步骤+策略沙箱)
├── apps/web/               # 薄壳:core/render/panels/ui 四层无构建 ESM(模块化结构见 ADR-0011~0017;锚点登记 TESTIDS.md;旧壳 legacy/ 已删)
├── schemas/                # mcp-tools.json(工具契约唯一真相源)/ui-fields.json/project.schema.json
├── tools/                  # gates/gate.py 门禁入口 + bench(基准/黄金对拍)+ e2e × 17 + 夹具生成器 + 文档对拍
├── tests/                  # 跨仓桥测试(四桥冒烟/剪映出口对拍)
└── docs/                   # FLOW.md 工作区地图 / V2-PROGRESS·A1~A4-PROGRESS 台账 / adr/ 决策记录 / bench/ 基准 / design/ 设计口径与录屏
```

## 🔗 与 CutFlow 的双向闭环

| 方向 | 通路 |
|---|---|
| CutFlow 工程 → CutForge 编辑 | 同一份 `project.json`(schemaVersion + 稳定 id);B8 护栏识别编辑器痕迹,`rs_run --status` 标脏 |
| CutForge 改动 → CutFlow 重建 | `.cutforge` 会话变更摘要 + `rs_editor.py diff` 人话差异 → `rebuild.py` 定向重建 |
| 四桥 | `rs_editor`(视图/变更识别)/ `rs_notes`(标注)/ `rs_oplog`(OpLog 审计)/ `rs_gate`(门禁透传),错误码与 5.4 码表对拍 |
| 剪映出口 | `export_jianying` 与 CutFlow `rs_jy_draft.py` 编排**同一个脚本**,映射真相只有一份 |

### 兼容矩阵(CutFlow ↔ cutforge ↔ 目录契约)

| CutFlow 版本 | cutforge 版本 | 目录契约 | 说明 |
|---|---|---|---|
| ≥ v0.19 | ≥ 0.5.0 | v2(中文目录) | `00_制作简报`/`01_原始素材`/`02_转写与校对`/`03_创作素材`/`04_粗剪决策`/`05_时间线工程`/`06_成片输出`/`_内部状态`/`成品`(NEVER_CLEAN);唯一真相源 `crates/cutforge-io/src/paths.rs` ↔ CutFlow `rs_paths.py`(ADR-0046) |
| < v0.19 | 0.4.x | v1(英文目录) | `00_brief`…`05_ir`/`06_output`/`_state` |
| 任意(旧工程) | ≥ 0.5.0 | v1 盘面 | 0.4.x 旧布局工程在 0.5.0 中**原地读写、不自动迁移** |

架构决策见 [docs/adr/](docs/adr/);整体流程与工作区地图见 [docs/FLOW.md](docs/FLOW.md)。

## 📄 许可

混合授权，三点必须读清（全文见各文件）：

1. 本项目**原创部分**适用 [ARL-1.0](LICENSE)（弱传染：核心文件的修改须回传开源；插件、壳、商业应用可闭源）。**正式发布前协议文本尚待执业律师复核。**
2. 派生/参考自 OpenCut 的部分必须遵守其 **MIT 许可**，全文见 [LICENSE-OPENCUT.MIT](LICENSE-OPENCUT.MIT)（逐字保留，未修改）。
3. CutFlow 的**已发布 MIT 版本（`v0.1.0`–`v0.12` 等 tag）授权不可撤回**；任何许可变更仅对其后发布的新版本生效。

> **定位注记（[ADR-0010](docs/adr/0010-定位转向个人自用闭源非商业许可简化.md)，2026-09-27）**：
> 本项目已转向**个人自用、闭源、非商业**；上述「发布前律师复核」义务挂起，仅当恢复对外
> 公开发行时重新生效。许可三件套原样保留，已发布版本的授权效力与 MIT 归属保留义务不受影响。

## 🛠️ 开发

工具链与上游 OpenCut 保持一致（`proto` + `moon` + `bun` + `rust 1.97.0`，edition 2024），保留未来接口层回流上游的可能。**整体流程与工作区地图见 [docs/FLOW.md](docs/FLOW.md)。**

```bash
# 门禁(统一完成判定入口)
python tools/gates/gate.py M0 --json
python tools/gates/gate.py M1 --json
python tools/gates/gate.py M2 --json
python tools/gates/gate.py M3 --json
python tools/gates/gate.py M4 --json
python tools/gates/gate.py A1 --json   # 册级门禁(每册一个 A<n> 入口,决策 D-A2)
python tools/gates/gate.py A2 --json   # 册二册级门禁(壳行数红线/纯度 v2/e2e 面,11 阻断+1 观察)
python tools/gates/gate.py A3 --json   # 册三册级门禁(UX/键位/可访问性,15 项全阻断)
python tools/gates/gate.py A4 --json   # 册四册级门禁(核心工具与媒体管线,18 项全阻断)
```

参与贡献前请读 [CONTRIBUTING.md](CONTRIBUTING.md)。
