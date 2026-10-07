# 更新日志(CHANGELOG)

格式参照 Keep a Changelog;版本号遵循语义化版本(SemVer)。

## 未发行(Unreleased)

> 册一「内核重构与架构加固」批次(128856c / 3ce2884 / 7157178)+ 册二「前端壳重写」
> 批次(7f02955 之后的未提交工作面)+ 册三「UX 动效/键位/可访问性」批次 +
> 册四「核心工具与媒体管线」批次(a2ba870~77cade5 已提交;FE2/收口波为未提交工作面)+
> 册五「专业深度」批次(E-BE1 6de2f23 / E-BE2 492bc1b / E-BE3 a45f52f / E-FE1 9b33927 已提交;
> E-FE2/收口波为未提交工作面)+ 册六「独立化」批次 + 册七「AI 原生与开放生态」批次
> (G0 68927f7 / G1 ea3065a / G2 ec23bbe+a7fad76 已提交;G2 壳侧与 G3 收口波为未提交
> 工作面);版本号待发行时定(沿用仓库惯例:发行时才把本段改为版本号。
> 此前的 v0.6 批次——编辑器 NLE 化、schema v2 M14 双仓同步——发行时一并补记)。
> 册二台账见 [docs/A2-PROGRESS.md](docs/A2-PROGRESS.md);册三台账见
> [docs/A3-PROGRESS.md](docs/A3-PROGRESS.md);册四台账见
> [docs/A4-PROGRESS.md](docs/A4-PROGRESS.md);册五台账见
> [docs/A5-PROGRESS.md](docs/A5-PROGRESS.md);册六台账见
> [docs/A6-PROGRESS.md](docs/A6-PROGRESS.md);册七台账见
> [docs/A7-PROGRESS.md](docs/A7-PROGRESS.md)。

### 新增

- **审片台闭环(MV 审片台资产包 20261007 → cutforge web 壳移植,零新真相源)**:
  **逐帧锚定意见**——标注面板「锚定此帧」把当前 atMs 钉进下一条意见
  (`notes_add` anchor.kind=time;之后拖时间线锚不变,逐帧审阅核心语义),
  tags=审片 口径随行;**元素拾取**——`Ctrl+Shift+C`/「拾取元素」进 F12 式
  检查(实时高亮 + 尺寸/坐标/当前帧 tooltip),点击壳内元素 → selector+位置
  锚进意见并预填正文模板(DOM selector 是壳事实,tags `dom:` 前缀显式声明
  易变性,不进工程真相);**拾取器零残留**(Esc 退出摘除全部 listener,
  e2e 断言);键位注册表新增 review.inspect(审片组);样式落
  `css/components/review.css`(tokens 同源);**审阅工作流铁律**落
  `apps/web/references/review-loop.md`(铁律⑨意见=Agent 接口/⑩叠加层不改源/
  ⑬抗打/⑮行内编辑/⑧缓存三防线 + 人/Agent 分工与机检口径,32 条铁律出处
  标注);回归 `tools/e2e_review_loop.py`(4/4:锚定帧落内核/拾取零残留/
  预填模板/open 意见机检面)。

- **V2-W3a OpLog 压实接线(R-13 收口)+ 桌面壳视觉基础层(A-08/A-09/§9.7)**:
  **io**——open 快照优先装载(`replay_from_snapshot`+增量双栈,**引擎内存保全量虚拟
  历史**,护城河「OpLog 即历史」在压实后成立;不可信自动回退全量路径绝不静默);
  persist 写尾自动压实(廉价头窗门+整行校验+逐分片原子重写);真实文件级回归
  TC-IO-SNAP-003(压实前后四面相等)/004(kill -9 三态自愈)/005(快照缺失拒绝压实);
  **desktop**——`ui/theme.rs` 三层 token 与 web tokens.css **自动平价测试逐值对拍**
  (62 原始+32 语义/组件槽,任一侧漂移即红;跨壳同源优先于 §9.3 提案色,裁决已注记),
  sable `theme::inject` 桥接,WCAG AA 对比表+守门测试,三处既有裸色值清零;
  `ui/icon.rs`+46 个内嵌 SVG(web 15 个逐字同源+31 个 NLE 自绘),panels 全部文本
  字形清零(含"卑"乱码,TC-DESK-ICON-002 零命中);`ui/fx.rs` 动效 token
  (80/160/240+三族 easing+reduced-motion 总控接 sable anim,zone 角标散写收编);
  纯度扫描三项零命中(可转阻断);桌面壳测试 95 全绿;ui-taste 8.5/10。
  **io 另登记新发现既有 bug 候选**:serde_json 缺省解析 17 位浮点 1 ULP 漂移可致
  check_window_drift 假阳性 CONFLICT(建议 feature 开关或比较容差,另立票)。
- **V2-W2 性能与架构轮(工单 docs/tickets/V2-W2-*,审查报告 v2 §5/§7/§4)**:
  **KERNEL2(R-12/R-13/A-05)**——apply 去全量 clone(`engine/revert.rs` 逆向回滚令牌,
  五条失败路径逐字节相等)+ schema 增量校验(`engine/validate.rs` 受影响子树+全局
  不变量,全量降级装载面与 `Engine::validate_full()`);**bench 1k×500 命令 71,158ms→
  396ms(降 99.4%,验收线 ≥80%)**;OpLog 压实纯函数面(`compact.rs` plan/retained_ops,
  rev>1000+快照存在才压,旧日志缺 rev 放弃)+ `rebuild_stacks_incremental`/
  `replay_from_snapshot`(10 万 Op open 1.1s;io 层接线归 V2-W3);include! 清理,
  pro_tests 27 测试迁 `tests/pro_commands.rs`;core 164 测试全绿。
  **MCP2(A-01)**——dispatch.rs 1150→162 行,`handlers/` 注册表化(85 handler 按域
  13 文件,`CommandHandler` trait + 三执行面,新增命令=一文件+一行);行为逐字节
  零变化(parity 0 DRIFT,97→103 测试,TC-MCP-DISPATCH-001 正反向);schema 由
  mcp-tools.json 编译期嵌入派生;豁免注释升级为永久登记(mcp→render 纯函数消费边,
  解耦方案留档)。**SHELLA(A-02/BUG-22/A-07)**——app.rs 1971 行拆 app/ 六模块
  (mod 257/workspace_view 540/playback_facade 591/source_map 451/command_surface 483/
  keymap 317,全 ≤600 行);纯度约束(command_surface 无 GPUI、workspace_view 无
  工具名);键位命令注册表 40 行同源(on_key 查表、上下键跨轨遍历、设置页速查表
  生成、冲突检测);桌面壳测试 30→80 全绿。**RENDER(BUG-13/14/21、R-06/R-07)**——
  escape_text 收口唯一实现(两导出路径逐字节相等);花括号中和经像素级实证定为
  「仅配对且含合法 ASS 标签形态才全角中和,未闭合/无标签半角保真」(U+200B 方案
  经 PSNR=inf 实证无效,证据链入代码注记),generation_warnings 逐条列出被中和行;
  SRT 毫秒 1~3 位补零+行号诊断;缓存键 FNV-1a+索引 hash_algo 版本位+cache-index.lock
  锁内合并写;`ff.rs` 单源 ffmpeg 看门狗(超时 kill+FfmpegTimeout{stage}+stderr 4KB
  尾环,100MB 泼入内存峰值 <50MB),RENDERER_VERSION 11.0→12.0;render 199 测试。
  **全 workspace 697 测试 0 失败**。
- **V2-W1 内核数据正确性轮(工单 docs/tickets/V2-W1-KERNEL-data-correctness.md,审查报告 v2 §4)**:
  **BUG-01/02(P0)**——切分 sourceIn 改走 `source_read_ms` 单一真相源(变速/倒放/曲线
  分段积分),新建深模块 `clip_ops.rs`(`Clip::split_at`:关键帧 rebase、fade/transition
  归属),ClipSplit/ClipSplitAll/track_split_at 三处私有换算删除;**BUG-03(P0)**——
  Roll/Slide 负 sourceIn 拒绝式守卫 + schema `sourceInMs` 24h 上界;**BUG-04**——Merge
  语义连续性(同 src/同速/sourceIn 相接 ±1ms)+ `Clip::merge_with`(split→merge 逐字节
  往返不变量);**BUG-05**——跨轨移动保 id + 目标轨 partition_point 有序插入 + 轨道有序
  debug_assert(插入族全量有序化);**BUG-06**——undo 稳定 id 寻址(Op 增可选 targetId,
  旧日志零迁移)+ project 级前置校验(不一致报 CF-002);**BUG-07**——replay rev 链校验
  (断链/乱序/重号报 RevGap,缺 rev 顺推兼容);**BUG-08**——三路合并序敏感(单侧纯重排
  保留、双侧异序 CF-003);**BUG-09**——关键帧 per-property 值域表。24 个 TC 用例先红
  后绿,`cutforge-core` 150 测试全绿;schemas/oplog.schema.json 补可选 `targetId`。
- **V2-W1 io 耐久性轮(工单 docs/tickets/V2-W1-IO-durability.md)**:
  **R-01(P0)**——atomic_write 父目录 fsync + append_line sync_data + `append_lines`
  组提交(批量 100 Op 108ms→2ms);**R-02(P0)**——锁接管三重前置链(锁龄≥30s + pid/启动
  时间不存活 + 心跳过期),持锁方 5s 心跳,recover 判据同源;**R-03**——oplog 半行截断
  显式修复模式(自动备份 recovery-* + 截断到最后完整 Op + RepairReport 三出口不静默);
  **R-04**——记账缺失差异自愈(ReconciledOp 屏障,undo 到此明确拒绝);**R-05**——oplog
  指纹升级(首/末行 rev + 末行 FNV);**R-10**——watcher 空闲指数退避 + Weak 线程退出;
  **R-13①**——快照缺省开(5min/LRU10,env 可关);**BUG-12**——备份目录名三段式防同秒
  互覆 + 50 代保留清理。
- **V2-W1 门禁纪律轮(工单 docs/tickets/V2-W1-GATES-discipline.md)**:
  gate.py 1609 行单体拆为 `tools/gates/` 包(m0-m7/a1-a7/common,行为零变化:结构
  SHA256 diff 为空 + 22/23 检查逐字节一致);CORE-FILES 存在性校验(反面测试 TC-GATE-001
  真实退出码 2)与条目订正;`check_changelog`(G-4 警告级)与 `check_desktop_line_limit`
  (G-1 报告模式)落地;CI 增 `clippy-gates`(-D warnings 全仓)与 `kernel-gates`
  (`gate.py M2`)两 job,A1 行数红线与 bench 阈值启用条件见 gate.yml 注释;§8.2 三纪律
  入 CONTRIBUTING.md;A8/README「巨石清零」不实宣称订正为实测口径;桌面壳纯度/Box::leak/
  行数三个观察项扫描器落地(TC-GATE-002/003)。
- **V2-W1 MCP 安全与服务质量轮(工单 docs/tickets/V2-W1-MCP-security-service.md,审查报告 v2 §3/§4/§5/§6)**:
  **安全**——BUG-10 Bearer 值精确相等 + 手写 XOR 恒定时间比较(垃圾后缀/头名大小写/
  重复头/续行四案全部收敛);S-04 最小 HTTP 头解析器(`parse_headers`/`bearer_value`/
  `query_param` 两通道单一实现,query 按 key 精确切分,`mytoken=` 不再误命中);
  S-01 `/session` 改发一次性短期凭据(5 分钟/单次/绑会话 id,主 token 不再进任何
  响应体与会话文件;`POST /session/exchange` 兑换会话 token;会话文件 Unix 0600;
  URL `?token=` 兼容一版并带 `Deprecation` 响应头);S-02 连接计数上限 64
  (env `CUTFORGE_HTTP_MAX_CONNS`),超限 503+`Retry-After`;S-03 插件强制层
  (声明 `filesystem` 白名单外路径访问 → `PluginError::PermissionViolation` 运行时
  拒绝 + 插件禁用 + `authorize_call` 参数面裁决;process 插件 Unix `sh -c ulimit`
  包裹,Windows Job Objects 留待)。
  **服务质量**——R-08 `/media` 流式转发(`io::copy` 恒定 64KB,整文件/大区间不再
  read_to_end;单 Range clamp 256MB,显式超限 416);R-09 resident 锁 per-project 化
  (外层 map 短锁 + 各工程 `Arc<Mutex>`,慢工程不再队头阻塞他工程);R-11 渲染队列
  持久化 `.cutforge/render-queue.jsonl`(append-only + 压实,重启重建 running→
  interrupted 可重试、pending 原样);R-14 幂等 `HashSet<RequestId>` 常驻索引
  (随 OpLog 增量维护/重开重建,dispatch 写预检 O(1),10 万 Op 判定 <1ms);
  BUG-11 SSE root 查询参数 `pct_decode`(CJK/空格路径订阅事件可达)。
  **工具契约**——BUG-17 `render_frame` 响应显式 `framePath`(工程内相对路径,
  壳直读取帧,旧 `path`/`media` 保留一版);BUG-19 新工具 `media_thumbs`
  ({root, src, atMs[], width} 一次多帧,磁盘缓存命中合并;**85 工具 = 21 查询+
  42 写+22 编排**)。
- **I1 播放引擎批次(docs/upstream/05 十轮总纲第一轮,硬骨头 B1/B4 部分)**:
  **内核 `preview_zone_render`**——时间线区间半分辨率预渲(fast 档进键,内容寻址
  `.cutforge/preview-cache/`,首渲/命中双路径实测;84 工具 = 21 查询+42 写+21 编排);
  **hidden 轨道渲染尊重性实测钉死**(04 文档缺口④,像素级双通道测试)。
  **桌面壳 `playback/` 模块**(UI 无关、27 测试)——ffmpeg 直解码 RGBA 管道 + 24 帧
  环形缓冲(condvar 背压,慢消费零丢头)/音频主时钟(cpal,WASAPI 不可用自动回落
  墙钟)/停流看门狗/Drop 收尸;接线 DesktopApp(16ms 播放泵、zone→直解码→幻灯片
  三级降级不白屏、播放中编辑 300ms 防抖重同步)+ M3 播放 UI(JKL 倍速/循环/静音/
  画质切换含代理/截图落 `screenshots/`/沉浸预览);工单与验收记录
  [docs/tickets/](docs/tickets/)。
- **册七(A7)AI 原生与开放生态**:**Editor API 版本化(T7.1)**——`/api/v1` REST 面
  (POST `tools/<tool>` 单表转发 + 7 GET 别名 + SSE 事件流 + 5.4 状态码映射;旧 `/rpc`
  保留为别名,ADR-0025 URL 版本);OpenAPI 3.1 由工具注册表生成(`docs/api/openapi.json`,
  `--check` 漂移门)+ TS SDK 生成物(`apps/sdk/cutforge.ts`,真 serve 16 断言;
  Python SDK 未做,登记 A7-L);**工程打包(T7.6)**——`.cfpkg` 打包/解包
  (`project_package`/`project_unpackage`,zipstore 单源 + zip-slip 防线,往返 byte 等价);
  **AI 协作面(T7.5)**——`preview_plan`(同卷副本工程逐项 dry-run,与真实写通道同一
  dispatch 实现,真工程零落盘)/`apply_plan`(default-deny 逐项批准,causedBy 绑 planId
  审计链,未批准/被拒项绝不落地)/`note_reply`(标注线程化)/`session_report`(人话
  Markdown 会话报告);**插件系统(T7.2,ADR-0024 分级:JS Worker 先行)**——服务端
  manifest schema + `plugin_validate` + `plugin-call`(actor=plugin 归因,权限裁决
  查询→read/写→write/编排→exec,越权 GUARD_FAILED/FORBIDDEN)+ `docs/PLUGIN-SPEC.md`;
  壳侧 Worker 宿主(安装→校验卡→首启权限确认→启停/卸载;命令/菜单/面板三贡献点;
  面板 HTML 受控渲染;宿主权限裁决镜像 + 崩溃自动禁用)+ 三件套示例
  (`apps/web/examples/plugins/`);**脚本页签(T7.3)**——cutforge-script-v1 编辑/
  片段库(内置三片段:按标记切割/批量变色/批量转场)/运行(preview_plan 预演,
  `script_run` RPC 候 BE 登记A7-L)/以 plan 提交进批准流/AI 粘贴区(人在环);
  **Headless(T7.4)**——`cutforge-cli batch`(JSON/YAML 清单 + 报告 schema 生成物)+
  `watch`(防抖自动重渲)+ `tools/ci-example/`;**验收**——`tools/e2e_ai_native.py`
  (AC-7.3/7.5 判定器:脚本三片段预演/批准流 causedBy 对账+被拒项零落地/插件九步/
  标注线程+报告;含 P0 文件级断言)+ `tools/e2e_headless.py`(AC-7.4 纯 CLI 全链);
  **P0 修复(G3)**——`scratch.rs::copy_tree` 硬链接穿透(OpLog append-only 直写
  inode,预演 Op 穿透写回真工程):append-only 面(oplog/rev)强制整拷贝 + 文件级
  回归测试两道(红绿证据留档);**gate A7 注册**(A6 二十二项继承 + 两份新 e2e,
  24 项全阻断;CI 只加 e2e_ai_native);录屏补录 5 支(56-60,总量 ≈7.77MB ≤ 8MB)。
  工具 **76→78→83 = 21 查询 + 42 写 + 20 编排**。决策见 ADR-0024/0025/0026;
  台账见 [docs/A7-PROGRESS.md](docs/A7-PROGRESS.md)。

- **册六(A6)独立化 · T6.1 工程库与布局 v3**:工程布局 **v3 扁平目录**(真相源全部在根
  `project.json/wordline.json/cutlist.json/cutlist.applied.json/notes.json` + `media/` +
  `exports/` + `.cutforge/`;三态判定 Legacy/V2/V3 收敛 `paths` 单点;过渡期 scaffold
  缺省仍产 V2,v3 经显式开关 `new --layout v3` / `project_new layout="v3"` / 迁移启用,
  ADR-0021);**迁移器** `cutforge-cli migrate <工程> --to v3` + MCP `migrate_layout`
  (一次性到位、幂等 NOOP、冲突整体拒绝;project.json 字节零改动,OpLog 完整性不动);
  **工程库** `cutforge-cli library`(list/search/new/rename/copy/archive/unarchive/delete,
  库根 env `CUTFORGE_PROJECTS` 缺省 `%USERPROFILE%\CutForge\Projects`)+ MCP
  `library_manage`/`library_list`(卡片元数据从 project.json 轻量派生,不逐工程 ffprobe);
  **崩溃恢复** `cutforge-cli recover` + MCP `library_recover`(残留锁检测 pid 存活性/
  锁龄 + 会话摘要证据;恢复 = 清锁 + OpLog 一致性校验复用既有装载语义);**自动快照**
  `.cutforge/snapshots/`(env `CUTFORGE_SNAPSHOT_INTERVAL_MS`/`CUTFORGE_SNAPSHOT_KEEP`,
  LRU 上限,缺省关)。工具 **68→72 = 16 查询 + 37 写 + 19 编排**。决策见 ADR-0021/0022/0023;
  台账见 [docs/A6-PROGRESS.md](docs/A6-PROGRESS.md)。

- **册六(A6)独立化 · T6.3 导出矩阵 + T6.2 内置能力收全**:**导出矩阵** render/render_run
  扩参(format=mp4-h264/mp4-h265/mov/gif/m4a/mp3/png-seq/frame-png、preset 画幅预设、
  qualityTier 短边缩放档、bitrateTier 估算码率档、inMs/outMs 区域窗口、videoOnly 仅视频;
  缺省路径逐字不变;gif=palettegen/paletteuse 两段 fps12、纯音频 probe+mix 短路不渲视频、
  png 序列帧数清点、h265=libx265 软件编码如实降级);**区域导出** = 工程级时间窗裁剪
  (全轨种钳制平移、头部按速度分段积分折算源域、新首段入向转场清除、BGM 保留);
  **`export_preflight`**(查询,轻探测:缺失素材/首末帧黑帧风险/静音段启发式/窗口时长/
  响度实测或 null 不虚标)+ **`export_all_variants`**(编排,逐变体入渲染队列 + 父任务
  拉取式聚合);**素材库** `media_library`(查询,库根 manifest 扫描合并幂等/标签持久/
  kind/tag/query 过滤,库根 env `CUTFORGE_MEDIA`)+ `media_import`(写,拷贝导入工程,
  布局感知落点 v3=media//v2=01_原始素材/,同名同内容幂等/异内容追加序号,tmp+rename
  原子,不产 Op);**剪映草稿随包**(`tools/jianying/rs_jy_draft.py` 收编 + 归属声明;
  orchestrate 脚本定位序 env→工程内→随包→CutFlow 回退,`scriptSource` 如实标注);
  **诚实登记**:重构图全自动=不做伪 AI 构图(#50 missing)、本地转写=ADR-0023 明确不做
  (#51 missing);ASS/花字 12 模板/卡拉OK/卡点为册四既有如实盘点。v3 工程导出产物落点
  补齐(exports/)。工具 **72→76 = 18 查询 + 38 写 + 20 编排**;台账见
  [docs/A6-PROGRESS.md](docs/A6-PROGRESS.md)。

- **册六(A6)独立化 · T6.1+T6.3 UI + T6.4 应用化 + T6.5 独立验收**:**壳侧(F3)**——
  工程库视图(卡片七操作/搜索/归档/恢复提示条/迁移 v3 入口,「工程 ▾」常驻)、向导三模板
  (口播/双机位/方形,选即预填)+ 布局 v2/v3 选择、导出矩阵面板(七格式说明/三档位/
  区域回填/videoOnly/**preflight 检查门**+仍要导出/批量变体入队)、媒体面板双页签 +
  素材库一键导入 + media_import 拷贝导入三通道;修 media-pane flex 滚动链回退
  (media_perf P95 63.7fps)。**应用化(F4,工具 76 不变——cfproj 全走既有参数面)**——
  **`.cfproj` 工程描述**(`library_manage action=export_cfproj` / `cutforge-cli library
  cfproj` 生成,`serve <x.cfproj> --open` 关联打开;单一实现 `cutforge_io::library`,
  失效给可读错误不静默);**端口占用自动换端口+横幅**(收敛 serve_workspace 单一实现,
  CLI/MCP 两面一致,session.port 改绑定后写入实际端口);**`cutforge-cli doctor --bundle`**
  诊断包(单文件 zip = doctor.json + 环境信息 + 会话/摘要/rev/project.json 快照;
  零依赖手写 store 形 zip + CRC-32,落盘走 atomic;Python zipfile 实测);**安装器**
  `packaging/cutforge.iss`(Inno Setup:core + ffmpeg 默认勾选内嵌组件(ADR-0022,
  构建时 packaging/ffmpeg/ 放入即启用)/快捷方式/卸载器(用户工程不删)/.cfproj 关联/
  勾选内嵌写 HKCU\Environment)+ `packaging/README.md`(便携版清单与差异)+
  `packaging/pure-checklist.md`(AC-6.5/6.6 人工清单);**诚实登记:开发机无 ISCC,
  编译与 VM 验收为人工项;`cutforge://` 协议可选未做**。**独立验收(F4)**——
  `tools/e2e_independence.py`(AC-6.3 判定器):剥离 CUTFLOW_* 环境隔离起 serve →
  字幕(含花字)/卡点/重构图(anchorY 承载防丢)/导出四大件全内置工具链 → **进程树断言
  全程零 python 子进程**(psutil/wmic/proc 三级采样;剪映导出豁免,ADR-0023 注明)→
  产物时长/像素抽样(纯绿字幕上帧 1704 像素实证)→ 纯 CLI 面(run-script 建卡 →
  clip-update 改字段 → cutforge-render 直渲 → ffprobe,v3 布局产物落 exports/,
  AC-7.4 预演);**gate A6 注册**(A5 二十一项一字不动继承 + e2e-independence,
  22 项全阻断;CI 只加 e2e_independence 一步,ubuntu 无 CUTFLOW_REPO 天然主场);
  录屏补录 6 支(工程库/迁移/模板/导出矩阵/preflight 门/素材库,总量 ≤8MB 红线内)。
  AC-6.1~6.7 状态表与 A6-L 遗留(缺省布局翻转/单实例协议/Inno 编译验证等七项)见
  [docs/A6-PROGRESS.md](docs/A6-PROGRESS.md)。

- **册六(A6)补漏(册七收官查漏补记)**:T6.3 收口波补钉——**v3 工程导出产物落点**
  (`RenderPlan::build_full` 补 v3 → `exports/` 分支,`render_matrix` 布局三态集成
  测试钉坑)+ **export 编码分派归位**(`exec_export_encode` 自 lib.rs 平移至
  `export.rs`,lib.rs 760/export.rs 710/plan.rs 794 全部 ≤800 行红线,clippy 新告警
  同轮清零);T6.1 顺手收口——`grade_tools.rs` lut_import 旁路写入改走
  `atomic::atomic_write`(A5 遗留,`check-write-paths` 复绿)+ `atomic.rs` 增
  `rename`/`remove_dir_all` 结构性原语(目录级操作收敛唯一落盘点纪律)。

- **册五(A5)专业深度**:关键帧引擎 IR v3(schemaVersion 3.0.0 双读兼容;白名单属性
  + closed interp + 贝塞尔精确求值,求值器单源,投影下发 `keyframes`/`keyframeSamples`
  采样点集,壳零插值;五条渲染通路按 ADR-0018 分级:position/rotation/scale/opacity/volume
  表达式,speed 并入 speed_segments,fx 参数三态;parity K1–K6 含 Rust↔ffmpeg 逐样本对拍);
  调色 `clip.grade` 整对象(色温/色调/曝光/对比/高光阴影/饱和度/Lift/Gamma/Gain + 曲线 +
  LUT `.cube` 库;HSL 限定器诚实降级)+ `scope_data` 示波器数据(波形/矢量/直方图);
  轨道 EQ(≤8 段 biquad)/动态(acompressor+alimiter)/响度单(audio_loudness,loudnormTarget
  实测偏差 0.05LU)/ducking 参数化;编码缺省 **remux+bt709 标签**(零重编码零代损)+
  显式重编码参数面(encoder/quality/crf/bitrate/gop/pixFmt)+ 硬件探测优雅降级(本机 AMF
  实证)+ 渲染队列(排队/暂停/恢复/取消/重试);复合片段内联子时间线(两级,递归渲染,
  内容寻址中间段)+ 调整层 adjust 轨(时间窗 fx/grade 作用于下方全轨)+ 多机位(波形互相关
  同步,展开为普通片段)+ 场景检测;OTIO 手写最小子集(出→入→再出语义 diff=0)+ EDL CMX3600
  + VTT + `docs/PROJECT-FORMAT.md` 工程格式文档。工具 **68 = 15 查询 + 34 写 + 19 编排**;
  壳侧:关键帧秒表打点/时间线关键帧行/双面板贝塞尔曲线编辑器、调色面板+LGG 三色轮+示波器
  三画布+分屏快照对比、混音台(EQ/动态/响度单;实时电平表候播放链采样口,诚实占位)、
  复合打包/解包、多机位页签、渲染队列页签、编码设置。决策见 ADR-0018/0019/0020;
  e2e 新增 `e2e_keyframes.py`/`e2e_color.py`(AC-5.1/5.2 壳侧闭环);gate A5 注册。
  **收口波补记**(2026-10-01):`patch.keyframes` 空数组=清空全部关键帧(schema minItems
  移除 + 单测 `keyframes_clear.rs`)+ grade 入投影键恒在;parity 68 新七工具夹具落库,
  golden 重录后连跑两次 0 DRIFT;录屏补齐 34 支(7.27MB ≤ 8MB 红线,册五 15 支 VP9 重编码;顺手修 capture 剧本两处 bug——10-scene 等待条件错对原始键名 cutCount→改等 scene-summary 摘要行、scene-auto testid 挂 checkbox 本体选择器去掉 ` input`);
  gate A5 二十一项全绿,台账收口波节留档。

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

- **compose xfade 链丢段与源窗越素材双缺陷(I1 回炉根治)**:① xfade 链判定收紧为
  「每个边界时间线相接且有效转场>0」——旧判定对 duration=0 边界仍产 xfade,ffmpeg
  会直接丢弃第二输入(实测合成片 13.0s 截断、后段整段消失,单帧抽帧越 EOF 报
  INTERNAL);不满足整链退化 concat 硬切并 WARN 留痕,空隙不再静默折叠。② 源窗超
  素材长度在执行器层(`exec_segment`,整片/单帧/zone/复合四出口共用)钳到素材实长,
  缺额 tpad 末帧定格补满,空素材/0 字节返回可读 PRECONDITION 而非 INTERNAL;钳制写进
  有效片段克隆使 seg/compose/mix 缓存键自动分叉,素材补长后零陈旧复用。③ 单帧内容
  长度口径统一:源耗尽尾段定格计满,只拦「整窗无源」。RENDERER_VERSION 10.0→11.0
  (行为变更升版,旧缓存整体失效);parity_matrix/parity_text_audio/render_matrix
  全绿,新增 9 项回归测试(含 cf-demo 形态:空隙+部分转场三段全出帧)。
- **mix 步纯视频时间线 `[N:a]` 零匹配炸图(I1 回炉根治)**:音频滤镜图对无音轨素材
  仍拼 `[N:a]` → 流说明符匹配零流,整片/zone/区域导出三出口对纯视频工程必炸
  (1080p 验收工程实测暴露;cf-demo 素材一直带音轨故未暴露)。修复:probe 透传
  has_audio,有音轨才实输入;无音轨段 anullsrc 静音占位(amix 输入数恒=段数,时间
  域贡献保留);BGM 无音轨按无 BGM 处理;acrossfade 链回落 anullsrc 垫位。新增
  纯视频三出口 + 有/无声混排能量断言等 6 项测试;既有 parity 全绿零回归。
- **桌面壳播放链路四缺陷(I1 实测收口)**:①帧消费门控用 pos 差值且每次弹帧重置
  基准,16ms 泵 × 33ms 帧距实际 48ms/帧 = 20.8fps 天花板(实测 17fps)——改**累计
  帧序号对账**,30fps 内容实测 29.3~30.4fps;②内核子进程固定 8790 端口,taskkill /F
  强杀壳不触发 Drop,孤儿内核占口,新实例 RPC 打到僵尸内核(实测清出 6 个)——
  缺省改系统临时端口(`--port` 显式指定仍尊重);③play_log 只写 stderr,GUI 形态
  不可见——缺省落系统临时目录 `cutforge-play.log`(CUTFORGE_PLAY_LOG 可覆盖);
  ④**RenderImage 逐帧泄漏**:gpui 0.2.2 RetainAllImageCache 只进不出,预览每帧新建
  RenderImage 不驱逐 = 每帧漏一个帧缓冲(实测 30fps 下 ~90MB/s,75s 工作集 7GB)——
  预览面板 install_image 统一走 `App::drop_image` 驱逐上一帧,复验 75s 工作集平坦
  106MB。播放引擎另改 preview 分辨率解码(短边 ≤540,帧内存 8.3MB→2.0MB,分配率
  250MB/s→60MB/s;DecodedFrame 尺寸反映实际输出,精确画面仍走 render_frame)。
  最终 1080p 验收:75s 连续 28.8~30.4fps、内存平坦、zone→直解码→幻灯片三级降级
  全链路实测(音画漂移项因 RDP 会话无声卡无法实测,留真机)。
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
