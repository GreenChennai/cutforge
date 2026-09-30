# A4 册四进度总账(核心工具与媒体管线)

> 执行依据:《CutForge-迭代计划》册四(核心工具与媒体管线)+ 总纲 §1.3 全局纪律。
> 本文件沿用 [A3-PROGRESS.md](A3-PROGRESS.md)(上溯 [A2-PROGRESS.md](A2-PROGRESS.md)、
> [A1-PROGRESS.md](A1-PROGRESS.md))的滚动台账惯例:复盘发现的问题**强制折入下一册
> 任务单**(总纲 §1.3.5);每指标可判定、可存档,不得为通过而放宽;未完成项如实写
> 「进行中 / 未达标」。
> 批次落库:BE1(a2ba870)/ BE2(b8feb55)/ BE3a(9a69c0a)/ BE3b(d6ba931)/ FE1(77cade5)
> 五波已提交;FE2+FE2 收尾+收口波与本文档同处未提交工作面,提交切分由主控决定。
> 本文所引数字均为收官实测值(2026-09-30 盘面;bench 存档 `docs/bench/2026-09-30.json`
> release 档、`docs/bench/perf-a3.json` 收官重生成;听觉存档 `docs/design/audio-samples/`)。
> **主控终验已跑**:`gate.py A4` 18/18 全绿;15 份 e2e 全绿;`cargo test --workspace`
> 307 passed;tool_parity 56×2 双通道零漂移;`check_doc_counts` 56=13+30+13。

## 一、任务逐项(+FE2 收尾+收口波)

| 任务 | 做了什么 | 关键证据 | 落点 |
|---|---|---|---|
| 决策 ADR-0015~0017 | **ADR-0015** 画布自定义扩为范围约束(64–7680 偶数)+常用预设(生成物 `canvasAllowed` 键承载);**ADR-0016** 文本渲染主路线=确定性 ASS 生成、复用烧录链(drawtext 仅记录兜底,否决 HTML 截帧/Canvas 两方案);**ADR-0017** 音频分离延后册五评估(三条标准:能力增益/依赖代价/工期置换,不达标以「明确不做+原因」落档) | 三份 ADR 落库并互相回链(0016↔AC-4.4、0015↔AC-4.8、0017↔册五交接) | `docs/adr/0015`、`docs/adr/0016`、`docs/adr/0017` |
| T4.2 时间线编辑全工具(BE1) | **六编辑工具**:`clip_trim` 四件套(trim/roll/slip/slide,**碰撞守护**)/`clip_split_all`/`track_update`(七字段)/`clip_gap_delete`/`clip_copy`+`clip_paste_at`(会话剪贴板);Track 字段进 IR+merge 承接+roundtrip 证明;工具 42→**48 = 13 查询+27 写+8 编排**;golden 48 重录 | edit_ops 引擎扩展测试;parity golden 48 对拍 | `crates/cutforge-mcp/src/edit_ops.rs`、`crates/cutforge-core/src/command.rs`、`tools/bench/golden/` |
| T4.4+T4.9 曲线变速与画布(BE2) | `speedCurve` **分段积分曲线变速**(单一真相源 `speed_segments`,投影/渲染时长一致**三道对拍**)+`reverse`(areverse,先于变速)+变换链 crop→flip→rotate+定格组合语义;画布扩为 **64–7680 偶数**范围约束(双端 multipleOf 对拍+推荐集,ADR-0015);parity 夹具 9→14 全绿;工具数不增 | 曲线变速时长语义/倒放首末帧对调/旋转朝向/crop 象限/flip 镜像五项实渲 | `crates/cutforge-render/src/`、`schemas/project.schema.json`、`schemas/constants.ratios.json` |
| T4.5+T4.6 转场/特效/动效(BE3a) | 转场库 **7→58**(ffmpeg 实测枚举五分类,目录 `tr.*` 直通;`GET /catalogs` 下发+缩略图生成器);**修转场 offset 截断虫**:尾帧扩展时长误入累计致后段整段丢失,旧夹具被容器时长骗过——夹具升级容器与视频流双 4.000s+120 帧锁断言;acrossfade 音频转场(**构造性零漂移**);fx 注册表 **11** 特效(combo≤3,未注册 fxId 逐项降级 WARN);motion 真实渲染 **19** 项(不能真实渲染的不进目录,诚实纪律);parity 增 18 项夹具 | 五分类各 ≥2 实渲视频流零漂移(120 帧/4s);mono 去色/vignette/grain/mosaic 像素级断言;fadeIn 亮度+slideIn 平移夹具 | `crates/cutforge-render/src/catalog.rs`、`crates/cutforge-render/tests/parity_matrix.rs`、`docs/design/transitions/` |
| T4.7+T4.8+T4.1 后端(BE3b) | 文本渲染落地:`textStyle` **14 字段**→确定性 ASS 复用烧录链(**PlayRes=画布**,所见即所得;ADR-0016)+花字 **12 模板**+卡拉OK `\kf`;字幕工作流 `text_add`/`subtitle_import`/`subtitle_replace`/`subtitle_export`(SRT 往返 **byte 级相等**;ClipsInsert/Patch 单 Op 原子);音频 `denoise` 四档(afftdn)/`pitch` 保速变调/`audio_beats` 启发式卡点(诚实标注);track **mute/solo/hidden 渲染联动收口**(BE1 欠账);媒体缩略图/代理/peaks(**mtime+size 内容寻址**)+`useProxy` 显式 opt-in;工具 48→**56 = 13 查询+30 写+13 编排** | SRT byte 级往返;denoise/pitch 滤镜映射与内核 `across.rs` 逐字一致 | `crates/cutforge-mcp/src/`、`crates/cutforge-render/src/`、`schemas/mcp-tools.json` |
| T4.1+T4.2+T4.3 前端(FE1) | 媒体池**缩略卡懒加载**(视口外零请求)+音频波形(peaks 密度档)+代理开关;时间线 **A/B/T 工具模式**+**四件套手势**(Alt=slip/Ctrl=slide/Shift+边缘=roll,**一次手势恰一 Op** 实证)+统一吸附候选;轨道头七字段(锁定拒编辑/高度拖拽/色板);**历史面板**(oplog 驱动回跳 N 笔撤销/快照标记);导入/track_reorder 缺通道诚实禁用登记 | e2e_editing_tools oplog 计数差值断言恰一 Op | `apps/web/js/render/{clip-gestures,gesture-kit,timeline-view}.js`、`apps/web/js/panels/` |
| FE2+FE2 收尾 | 转场库 58 网格+**特效栈编辑器**(combo≤3)+动画选择器+**文本工具**(画布拖位置所见即所得,`render_frame` 渲染帧字节随动实证)+花字库+**字幕编辑器全流程**+音频降噪/变调/卡点 UI+**画布五档预设**+PiP 变换把手+**曲线点集编辑器**;收尾修 3 UI 缺陷+fx 投影架桥+隐藏页签虚拟化 bug | 帧证文字位置断言;隐藏页签虚拟化回归 | `apps/web/js/panels/{transitions,fxlib,textool,subtitles,curve,insp-groups}.js`、`apps/web/js/core/catalogs.js` |
| 收口波 | BE 三小修:`timeline_projection` 补 fx 键/`render_frame` 内容末端 PRECONDITION/花字显式清除 `patch.huazi={}`;三份新 e2e(`e2e_editing_tools`/`e2e_subtitle_editor`/`e2e_media_perf`)+**听觉存档四样本**(修 atempo 方向反写真 bug,保速 4s 断言把守);`gate.py A4` 注册 **18 项**;CI 增 editing_tools/subtitle_editor 两步;perf/bench 收官落盘(`docs/bench/perf-a3.json` 重生成、`docs/bench/2026-09-30.json`) | gate A4 18/18;e2e_media_perf P95 63.3fps;四支 WAV ≤3MB 落库 | `tools/e2e_{editing_tools,subtitle_editor,media_perf}.py`、`tools/gates/gate.py`、`.github/workflows/gate.yml` |

配套:`gate.py A4` 册级门禁注册(**18 项全阻断**,D-A2 制:A3 十五项一字不动全部继承,
另纳册四三份新 e2e;media_perf 千素材帧率负载敏感不进 CI)、CI web-e2e 增
editing_tools/subtitle_editor 两步、`docs/design/audio-samples/` 听觉存档 +
`docs/design/transitions/` 缩略帧落库、README/CHANGELOG/FLOW 同步(本批)。

## 二、AC-4.1 ~ AC-4.8 状态表

| # | 验收项 | 判定方式 | 状态 | 当前实测 |
|---|---|---|---|---|
| AC-4.1 | 工具面与差距表:§4.B 差距表逐域闭合 | `schemas/mcp-tools.json` + `tools/check_doc_counts.py` + §4.B 差距表逐域核对 | ✅ | 工具 42→**56 = 13 查询+30 写+13 编排**(check_doc_counts 绿)。差距表逐域闭合:编辑域(四件套/split_all/gap_delete/copy+paste/track_update 七字段)、变速域(speedCurve 分段积分+reverse+变换链)、转场/特效/动效域(58/11/19)、文本字幕域(textStyle 14 字段+字幕工作流四工具+花字 12+卡拉OK)、音频域(denoise 四档/保速变调/卡点/mute-solo-hidden)、媒体域(缩略/代理/peaks)、画布域(AC-4.8)全闭合。**候 BE 清单**:track_reorder/track_delete、批量原子、拷贝导入通道、已建工程画幅修改工具、壳 huazi-clear 真清除、mcp-tools.json huazi null schema(逐条见 §三 A4-L7~L12) |
| AC-4.2 | 时间线编辑全工具:四件套语义+恰一 Op+锁定拒编辑 | `tools/e2e_editing_tools.py` | ✅ | 四件套(Alt=slip/Ctrl=slide/Shift+边缘=roll)各一次手势=**恰一 Op**(oplog 计数差值);clip_split_all/clip_gap_delete/clip_paste_at 交互;四件套各实渲一次**时长语义逐差对拍**;锁定轨拒编辑零 Op |
| AC-4.3 | 转场/特效/动效:库扩容+实渲零漂移 | `parity_matrix.rs` ⑮ + `capability-matrix.json` 同源断言 + `GET /catalogs` | ✅ | 转场 **7→58**(五分类各 ≥2 实渲视频流零漂移,120 帧/4s)+acrossfade 音频零漂移;fx **11** 特效(combo≤3);motion **19** 项真实渲染;**修 offset 截断虫**(流时长双断言防回归) |
| AC-4.4 | 文本/字幕:渲染落地+编辑器全流程 | `tools/e2e_subtitle_editor.py` + ADR-0016 | ✅ | textStyle 14 字段→确定性 ASS(PlayRes=画布,画布拖位置所见即所得,**渲染帧字节随动**);SRT 导入导出 **byte 级相等往返**;批量替换单 Op+幂等;卡拉OK `\kf` 主色前沿推进;花字 12 模板上帧+`patch.huazi={}` 显式清除;投影逐 clip 含 fx 键 |
| AC-4.5 | 媒体池千素材性能 | `tools/e2e_media_perf.py --min-fps 55` | ✅ | **1000 素材 P95 63.3fps**(≥55);BROWSE_CAP 500 如实截断;缩略图懒加载视口外零请求;代理开关在位;负载敏感不进 CI |
| AC-4.6 | 音频:降噪/变调/卡点+听觉存档 | `e2e_media_perf` §听觉存档 + `docs/design/audio-samples/` | ✅ **存档与参数链自动化;「听感自然」真人复核=人工项(A4-L14)** | denoise 四档(afftdn)/pitch 保速变调(atempo 方向反写真 bug 已修,源 4s 保速断言)/audio_beats 启发式卡点(诚实标注,下拍未做 A4-L5);四支 WAV(48kHz/mono/16-bit 各 4s)滤镜映射与内核逐字一致,总量 ≤3MB |
| AC-4.7 | 回归不劣化:册二/三全部 e2e 与性能预算 | 册四收官全量复跑 + `docs/bench/{perf-a3,2026-09-30}.json` | ✅ | **15 份 e2e 全绿**(册二/三 12 份+册四 3 份)即证零劣化;perf_budget 收官复跑:boot **345ms**<1s / 页签 **45.3ms**<100ms / 导出回调 **2.5Hz**≥2Hz / 池 **24** 有界;bench release 档落盘(gitRev 77cade5,open median 5.59ms) |
| AC-4.8 | 画布:范围约束+双端对拍 | ADR-0015 + schemas 契约 + parity 夹具 | ✅ | canvas **64–7680 偶数**范围约束(双端 multipleOf 对拍+推荐集进 `constants.ratios.json` canvasAllowed);壳画布五档预设;已建工程画幅修改工具候 BE(A4-L10) |

> 口径说明:实测值以册四收官盘面为准(BE1~FE1 已提交至 77cade5,FE2/收口波为未提交
> 工作面);千素材帧率/导出节奏等负载敏感项为本机安静时段实测,不进 CI(同 A2-L7
> 口径);gate A4 18 项**终验已跑全绿**(与 A3 收官时「终验待跑」不同,本册闭账);
> 4h 长跑/真人听感/真人盲测为人工项(A4-L14)。

## 三、遗留台账(强制折入下一册;总纲 §1.3.5)

编号自 A4-L1 起,与 A1(L-1~L-9)/ A2(A2-L1~A2-L7)/ A3(A3-L1~A3-L7)**并行衔接**:
前册各条继续有效,按其原处置折入后续册,本表不重复收录。**本册处置变化追记**:
A3-L1(4h 长跑)与 A3-L2(真人盲测复验)册四未排,连同真人听感复核并入 **A4-L14 人工项**
条目继续留账;A1-L2(渲染进度 steps/cacheHits 透传壳)与 A3-L4(Tab 让位 ADR)、
A3-L7(main.js 收敛)册四未动到对应面,按原处置继续留账。

| # | 发现 | 处置建议 |
|---|---|---|
| A4-L1 | **fx.in/out 槽位登记不渲染**:特效入出点已在 schema/投影登记,但渲染链未消费 | 册五补真实渲染路径(与关键帧引擎对接同批,槽位语义已备) |
| A4-L2 | **motion 19 < 30+**:3D 翻转等动效无真实渲染路径,**不硬凑进目录**(诚实纪律) | 后续册按「能真实渲染才进目录」口径补齐;目录面已留扩位 |
| A4-L3 | **morph 等转场本机 ffmpeg 缺席**:枚举按本机 ffmpeg 实测,缺席项未入 58 目录 | 枚举器随 ffmpeg 版本重跑即可自然扩容;不为本机缺席项写死分支 |
| A4-L4 | **字体枚举探测未做**:libass 对缺失字体静默回落,壳无法预警字体不匹配 | 册五加系统字体枚举 + 工程字体缺失提示;先探 libass 回落行为再动 |
| A4-L5 | **audio_beats 为启发式**:能量阈值卡点,下拍(onset)检测未做 | 后续册评估 onset 检测(零依赖算法域);现口径已在 UI 诚实标注 |
| A4-L6 | **peaks 缓存键为 mtime+size**:同秒同尺寸改内容不失效(折衷已声明) | 如需严格,换内容哈希抽样;权衡探针成本后再动 |
| A4-L7 | **批量操作为 N Op**:批量删除/移动逐笔产生 Op(壳 toast 明示非单笔) | 批量原子(单 Op 或 Op 组)候 BE;动前先定撤销语义 |
| A4-L8 | **拷贝导入通道缺**:外部工程/剪贴板素材导入无直连通道(壳诚实禁用) | 候 BE:media_import 类工具;契约走 CONTRACT-WORKFLOW 七步 |
| A4-L9 | **track_reorder/track_delete 缺**:轨序调整与删轨无工具(壳诚实禁用) | 候 BE;Track 字段 IR 已备(BE1),补命令面即可 |
| A4-L10 | **已建工程画幅修改工具缺**:画布范围约束已立(AC-4.8),但存量工程改画幅无工具 | 候 BE;注意渲染缓存键已含画幅(册一),改画幅即全链 miss 属预期 |
| A4-L11 | **壳 huazi-clear 按钮为占位**:花字清除后端已通(`patch.huazi={}`),壳按钮待切换真清除 | 下批壳侧小修(交接清单 #8) |
| A4-L12 | **mcp-tools.json huazi schema 未显式允许 null**(文档级):清除语义靠口头约定 | 下一笔 schema 变更顺带补 `type: ["object","null"]`,走契约七步 |
| A4-L13 | **CutFlow 侧 schema 双仓同步待协调**(BE2 登记):mcp-tools 双仓副本存在时点差 | 跨仓事项,与 CutFlow 协调同批对齐;钉扎机制同 A3-L6 口径 |
| A4-L14 | **人工项三项**:4h 真机长跑(A3-L1)/真人听感复核(降噪/变调)/真人盲测复验(A3-L2) | 册五安排真人复验并录屏归档 `docs/design/recordings/`(与 A3-L2 同库);自动化替代口径不变 |
| A4-L15 | **两笔 e2e 时序抖动**(册四期):单跑全绿;bgm 面板 fillFrom 回填竞态已以重试兜底注释(`e2e_ui_smoke.py`) | 册五根治(投影回填时序),根治后撤兜底重试;非门禁口径放宽 |

## 四、与册五的交接清单

1. **关键帧引擎**:对接 `speed_segments` 单一真相源(BE2 分段积分已为投影/渲染三道
   对拍收口);关键帧语义入内不得破坏既有时长对拍,动语义先补 ADR;顺手接 A4-L1
   (fx.in/out 槽位)真实渲染路径。
2. **文本槽位渲染**:fx.in/out 槽位已登记不渲染(A4-L1),补渲染路径时与卡拉OK/花字
   既有 ASS 链同源,勿另起第二文本实现(ADR-0016 口径)。
3. **批量原子**:批量操作现 N Op+toast 明示(A4-L7),册五定批量原子语义(单 Op 或
   Op 组)并同步撤销/历史面板快照标记口径。
4. **拷贝导入通道**(A4-L8)与 **track_reorder/track_delete**(A4-L9):三个 BE 命令面
   缺口,Track 字段 IR 与会话剪贴板底座已备;一律先 `schemas/mcp-tools.json` 走
   CONTRACT-WORKFLOW 七步。
5. **字体枚举探测**(A4-L4):系统字体枚举 + 工程字体缺失预警;先探 libass 静默回落
   行为,再定 UI 提示口径。
6. **音频分离评估**:按 **ADR-0017 三标准**(能力增益/依赖代价/工期置换)评估;达标
   即立项,不达标以「明确不做+原因(哪条未过)」落档,诚实三分口径(做/延后/未评估)。
7. **壳侧小修两笔**:①huazi-clear 按钮从占位切换真清除(A4-L11);②bgm 面板
   fillFrom 回填竞态根治(A4-L15,撤重试兜底)。
8. **门禁与基线延续**:册五收官跑 `gate.py A4` 同款 18 项(工具/媒体面不劣化)+ 按
   D-A2 注册 `gate.py A5`;bench 与 perf 收官照跑落盘新日期 JSON(A1-L5 口径);
   `check_doc_counts` 56=13+30+13 口径随册五工具面同步(改前先过 schema)。
