# A7-PROGRESS · 册七(AI 原生与开放生态)台账

> 只增不改的历史台账(F-R 口径:完成一项记一项,不回头改写)。
> 计划书:册七「AI 原生与开放生态」;分支 `feat/plan-A7-ai-native`。

## G0 · T7.1 Editor API(版本化)+ T7.6 工程打包——BE 完成

实码(工具 76→78 = 18 查询 + 40 写 + 20 编排):

- **`/api/v1` REST 面**(ADR-0025 URL 版本;旧 `/rpc` 保留为别名,至少两册):
  `POST /api/v1/tools/<tool>` 单表转发零副本 + 7 个 GET 查询别名
  (`/api/v1/project|timeline|notes|oplog|conflicts|stages|tools`)+ SSE 事件流
  `/api/v1/events` + 5.4 状态码映射(ok→200/PRECONDITION_FAILED→400/
  GUARD_FAILED→403/SCHEMA_INVALID→422/鉴权→401/NOT_FOUND→404);纯 std 手工
  前缀判定,零新增依赖(ADR-0009 延续)。
- **OpenAPI 3.1 生成器**(`tools/gen_openapi.py`):描述文件由工具注册表**单一真相源
  生成**(docs/api/openapi.json;92 路径 = 83 工具入口 + GET 别名 7 + tools 清单 +
  events),`--check` 漂移门(生成物 ≠ 注册表即红,ADR-0003 同纪律)。
- **TS SDK 生成物**(`tools/gen_ts_sdk.py` → `apps/sdk/cutforge.ts`):83 个类型化
  调用面 + 83 工具类型;`tools/test_ts_sdk.mjs` 真 serve 实测 16 断言;`--check` 漂移门。
- **`.cfpkg` 工程打包/解包**(T7.6):`project_package`/`project_unpackage` 两写工具;
  zipstore 单源实现(manifest + 工程 + oplog + 素材 + 产物五段;zip-slip 防线;
  打包→解包往返 byte 等价;空工程零媒体引用 counts 如实 0)。
- **ADR**:0024(插件沙箱分级:JS Worker 先行/外部进程补能力/WASM 远期)、
  0025(API 版本策略 = URL 版本)、0026(多实例协作明确暂缓,冲突停写保持)。
- 顺手修复:**v3 布局不发 SSE 事件潜伏缺陷**(watcher 根目录选择漏 v3 真相源位)。

## G1 · T7.5 AI 协作面 + T7.4 Headless 收口 + T7.2 插件服务端面——BE 完成

实码(工具 78→83 = 21 查询 + 42 写 + 20 编排):

- **`preview_plan`(查询)**:批量变更集在**同卷副本工程**上逐项 dry-run——
  `cutforge_io::scratch` 复制(硬链接优先零拷贝)→ 逐项 `root` 改写副本根重入
  `dispatch` 单表(与真实写通道同一实现,不建第二套业务逻辑)→ 副本 OpLog 增量
  提取字段级 before/after(大值 slim 折叠)→ 返回前销毁副本并逐出常驻缓存;
  依赖冲突按序暴露(逐项 ok/err + rev 链)。
- **`apply_plan`(写)**:default-deny 逐项批准——只有 approvals 显式批准
  (逐项索引/id 或 approveAll;显式 reject 恒胜出)的项才逐项重入单表执行,
  actor 保持调用方,每项 Op causedBy 绑 planId(审计链);未批准/被拒项一律
  跳过并回执(`rejected_by_caller`/`not_approved`),绝不落地。
- **`note_reply`(写)**:标注线程化——同一标注多轮追加回复(线程 id = 标注 id),
  不改 state、不碰 resolved_by(讨论史与结案回执分离)。
- **`session_report`(查询)**:一次会话(sinceRev 起)改动摘要——操作分布/
  参与者/改动段/轨/耗时/结案回执率;人话 Markdown + JSON 双形态。
- **Headless 收口**(T7.4):`cutforge-cli batch`(JSON/最小 YAML 子集清单,多工程
  单排队 + `--report` 落盘)+ `docs/schemas/batch-report.schema.json`(生成物,
  e2e 对拍)+ `cutforge-cli watch`(文件变更自动重渲,防抖 + `--max-runs`)+
  `tools/ci-example/`(渲染批清单 + 报告校验器样例,供"自动出片" CI 使用)+
  `tools/e2e_headless.py`(AC-7.4 全链验收判定器)。
- **插件服务端面**(T7.2):plugin-manifest schema(cutforge-schema 单一真相源;
  id/版本/形态/入口/权限五面/贡献点)→ `plugin_validate`(查询;8 组负例单测)+
  `plugin-call` 通道(CLI;actor=plugin 归因,权限裁决 = manifest 声明 vs 工具
  分类 查询→read/写→write/编排→exec,越权 GUARD_FAILED/FORBIDDEN,7 断言)+
  `docs/PLUGIN-SPEC.md`(双形态生命周期与权限模型契约)。

## G2 · 壳侧(FE)+ parity 容差——FE 完成(apps/web 未提交工作面)

实码(apps/web 无构建 ESM;TESTIDS.md 第十节全量登记;工具数不变):

- **脚本页签**(`panels/script.js`,T7.3):cutforge-script-v1 编辑区(行号槽)+
  片段库(**内置 3 项按会话态生成**:按标记切割/批量变色(调色)/批量转场;
  库脚本存 localStorage)+ 运行(= preview_plan 副本预演,结构化逐步回执)+
  以 plan 提交(进批准流)+ 导出 plan JSON + AI 粘贴区(人在环);
  **运行通道勘察口径(诚实)**:后端无「直接跑脚本」RPC(脚本宿主在 CLI
  run-script 侧,浏览器不可达)——壳以 plan 形态运行,`script_run` RPC 登记候 BE
  (A7-L)。
- **计划批准流**(`panels/diff.js` 第二区 + `core/plan.js` 状态,T7.5):preview
  逐项卡片(字段级 before/after;大值折叠提示经 oplog_tail 查)→ 逐项批准/拒绝
  双钮(再点撤销;未决=默认拒绝)→ 全批(ok 项)/全拒/清 approvals → apply_plan
  回执(落地/跳过/拒绝 + rev 链 + planId 归因入口)→ OpLog 行内 causedBy 显式
  成行(plan-* 高亮);三入口(检查器批量区/脚本页签/AI 粘贴)共用同一 draft。
- **标注线程 + 会话报告**(`panels/notes.js`,T7.5):thread[] 逐条(author 徽标
  AI/人)+ open 行回复输入(note_reply);已结案/否决留档区(线程留档);session_report
  极简 Markdown 渲染(白名单解析,零 innerHTML)+ 下载 .md。
- **插件宿主**(`plugins/{host,manager,manifest,contributes}.js`,T7.2/ADR-0024):
  JS Worker 形态——本地文件安装(manifest.json+入口)→ plugin_validate 校验展示卡
  (valid/errors/warnings + 权限五面)→ 首启确认对话框(权限明示)→ 启用/禁用/
  卸载(localStorage 注册表);贡献点三类:命令(keymap 注册 + 时间线右键菜单项,
  可逐条隐藏)、面板(专属页签,插件推 HTML 片段宿主**受控渲染**:标签白名单+
  剥属性+禁脚本);宿主桥 postMessage 单通道(token 不下发;每次 API 代理请求先过
  manifest 权限裁决镜像,与服务端 plugin-call 同口径——越权 GUARD_FAILED/
  FORBIDDEN 原样回传);崩溃隔离(onerror → 终止 Worker + 自动禁用 + toast)。
- **示例插件三件套**(`apps/web/examples/plugins/`):demo-command(命令)/
  demo-menu(菜单)/demo-panel(面板)——AC-7.2「命令+右键菜单+自定义面板三类」
  开箱可跑。
- **parity 响度容差**(BE,ec23bbe):响度测量字段族(deviation/inputI/inputTp/
  inputLra/targetI)跨 ffmpeg build 小数漂移以绝对容差吸收(CI 实证 11.5 vs 11
  跨档场景),超差仍严格——export_preflight 跨 build 漂移 CI 实证驱动。

## G3 · P0 修复 + 夹具 83 + 正式 e2e + gate A7 + 文档 + 录屏(收口波,F4)

实码(crates/tools/docs;apps/web 禁碰):

- **P0 修复(`crates/cutforge-io/src/scratch.rs`,最高优先)**:G2 实测发现
  `copy_tree` 对 `.cutforge/oplog/*.jsonl` 与 `.cutforge/rev` 走硬链接——OpLog
  append-only(`atomic::append_line` 是全仓唯一 append 模式写面,直写 inode),
  副本预演 Op **穿透写回真工程**(实证:真 oplog 出现 causedBy=null 预演 Op、
  真 rev 被推高;G1 的隔离测试夹具零 Op、oplog 文件不存在时副本侧新建 inode
  不穿透,故目录指纹级测试检不出)。修法:**append-only 强制整拷贝**——
  `force_copy_file`(oplog 目录与 rev 文件)跳过硬链接回退;副本建出即与真工程
  盘面完全独立。补**文件级单测** `scratch_append_only_files_never_penetrate`:
  种子写产真实 OpLog → 副本连续写两笔 → 真工程 oplog 逐文件字节一致 + rev 不变
  + `.cf-scratch` 零残留 + 重开后 rev 对账不吞穿透 Op;**红绿证据**:临时撤销
  修复该测试即红(「真工程 oplog/20261001.jsonl 被副本追加穿透」),复修即绿。
- **夹具 83**(`tools/bench/tool_parity.py`):阶段 I 扩五新工具——preview_plan
  两项小 plan 一错一对(错项暴露逐项错误码,对项出字段级 before/after)/
  apply_plan 批准面三态全谱(批准→落地/显式拒绝→rejected_by_caller/未列→
  not_approved;product_assert 锁 rev 恰 +1)/note_reply 一轮(replies=1)/
  session_report(sinceRev 0 全会话;墙钟 durationMs 走新 `session` 归一化模式
  占位 + markdown 耗时文本正则占位)/plugin_validate 合法+非法各一;golden
  `--update-golden` 重录 **83**,连跑两次 `82 PASS/1 WARN(audio_beats 启发式,
  既有容忍)/0 DRIFT`。
- **`tools/e2e_ai_native.py`**(G2 冒烟正式化,AC-7.3/7.5 判定器):① 脚本页签
  三内置片段载入→运行(预演)→结构化输出(2/2/1 步;真工程零写入);② 以 plan
  提交→preview 卡片→逐项批准/拒→apply 回执(落地 1/拒绝 1)→causedBy=planId
  对账(恰一 Op,actor=user)+ 被拒项真相源零落地 + **P0 全 rev 断言恢复**
  (预演后真工程 rev 不变 + oplog 逐文件字节一致 + 工程父目录零 .cf-scratch
  残留);③ 插件九步:安装→校验→权限确认→三贡献点生效(右键菜单 ×2 调用回执
  + 面板页签受控渲染)→越权双拦截(宿主镜像 + 服务端 plugin-call 均
  GUARD_FAILED/FORBIDDEN,工程零变化)→崩溃隔离(Worker 抛错→自动禁用)→
  禁用(贡献点即时摘除)→卸载(注册表移除);④ 标注线程两轮(user+agent,
  state 仍 open)+ session_report 渲染(人话 Markdown 上帧 + 下载解锁)。
  风格对齐既有(pathlib/testid/服务端断言/退出码 0/2/ubuntu 可移植)。
- **gate A7 注册**(`tools/gates/gate.py`,D-A2 制):A6 二十二项一字不动全部
  继承 + e2e-ai-native + e2e-headless,**24 项全阻断**;CI web-e2e 只加
  e2e_ai_native 一步(e2e_headless 含 watch 真实渲染与批处理,分钟级,本机册
  收官跑);`gate.yml` yaml.safe_load 校验过。
- **文档收官**:本台账新建 + AC-7.1~7.6 状态表(下)+ A7-L 遗留登记;
  CHANGELOG(册五收口波补记查漏 + 册六补漏 + 册七 G0/G1/G2/G3 条目);
  README(册七状态段/门禁表 A7 行/e2e ×18→×20/tool_parity 78→83 口径/
  快速开始两份新 e2e + gate A7 命令);FLOW(gate 段 A7 注册 + 工具面 83 口径);
  check_doc_counts(83)绿。

验证(2026-10-01 本机,G3):

- `cargo test --workspace --locked`:46 套件全绿 451 passed 0 FAILED
  (scratch.rs 新增 append-only 文件级回归测试在内);
- `cargo clippy --workspace --all-targets --locked -- -D warnings`:零告警;
- 20 份 e2e 全绿(19 份既有 + e2e_ai_native 两跑 14.3s/10.0s);
- `tool_parity --update-golden` 后连跑两次 0 DRIFT(83 工具;1 WARN 为
  audio_beats 加法键既有容忍);
- `check_doc_counts`:83 = 21 + 42 + 20 四文档零漂移;
- `gen_openapi --check`(92 路径)与 `gen_ts_sdk --check`(83 调用面)零漂移;
- gate A7 二十四项全绿 + A1–A6 复跑全绿 + M0/M1 绿(M2–M7 外部红项如实注明);
- 录屏 45 支(册七补录 56-60 五支 720×450 + VP9 CRF46 重编码;旧档三支大文件
  同 CRF46 重编码腾挪余量),目录总量 ≈7.77MB ≤ 8MB 红线,单文件 ≤2MB,
  时长逐支不变;回链 micro-interactions.md 第十一节。

## AC-7.1~7.6 状态表(册七收官口径)

| # | 验收项 | 状态 | 判定证据 |
|---|---|---|---|
| AC-7.1 | OpenAPI 注册表生成漂移=0;TS SDK 跑通 ≥10 典型调用;/rpc 兼容可用 | ✅ | G0:`gen_openapi --check` 零漂移(92 路径);TS SDK 生成物 `test_ts_sdk.mjs` 真 serve 16 断言(≥10);`/rpc` 保留为别名(全部既有 e2e/parity 仍走 `/rpc` 即兼容实证) |
| AC-7.2 | 示例插件三类贡献点可装/启/禁/卸;写操作入 OpLog(actor=plugin);越权被拒(≥6 负例) | ✅ | G1 服务端(plugin_validate 8 组负例单测 + plugin-call 权限裁决 7 断言;写插件 OpLog actor=plugin 归因)+ G2 壳侧三件套示例 + G3 `e2e_ai_native` 插件九步(安装→校验→确认→三贡献点→越权双拦截→崩溃隔离→禁用→卸载)+ `e2e_headless` 服务端面 |
| AC-7.3 | 脚本标签页编辑→运行→输出全链;3 个示例脚本开箱可跑 | ✅ | G2 脚本页签(编辑器/片段库/运行/输出/AI 粘贴)+ G3 `e2e_ai_native` 场景 1(三内置片段 2/2/1 步预演结构化输出);**诚实登记**:运行=preview_plan 副本预演(plan 形态),落地走批准流——`script_run` RPC 候 BE(A7-L 1) |
| AC-7.4 | Headless 纯 CLI 全链;批清单 3 工程排队;报告 JSON schema 稳定 | ✅ | G1 `tools/e2e_headless.py`(单链 new→run-script→clip-update→直渲→ffprobe/像素;批清单 3 工程混合布局 + batch-report.schema.json 生成物对拍;watch;ci-example);gate A7 阻断项,本机册收官跑 |
| AC-7.5 | AI 预览 dry-run→批准→落地逐条可拒;OpLog 归属正确;全程无越权写入 | ✅ | G1 preview_plan/apply_plan(default-deny,causedBy 绑 planId)+ G3 `e2e_ai_native` 场景 2(逐项批准/拒→回执→causedBy=planId 对账恰一 Op actor=user→被拒项零落地)+ **P0 文件级断言**(预演后真工程 oplog 字节逐一致/rev 不变/零残留) |
| AC-7.6 | 全量回归零劣化 | ✅ | gate A7 二十四项全绿(A1–A6 一字不动继承);tool_parity 83 工具两次 0 DRIFT;20 份 e2e 全绿;bench 阈值 AC-5.7 照跑 |

## 遗留(A7-L,册七收官登记)

1. **`script_run` RPC 候 BE**:脚本宿主(cutforge-script 策略沙箱)挂在 CLI
   `run-script` 子命令,浏览器壳不可达——脚本页签以 **plan 形态**运行(steps 逐项
   preview_plan 预演 → 人审批准 → apply_plan 落地),与批准流共用同一会合点;
   「直接跑脚本」的 RPC 面候 BE(需要服务端脚本宿主挂进 dispatch 的安全评审:
   沙箱在派发前拦截的现结构下,HTTP 通道直接执行脚本属新增攻击面)。
2. **FE Worker 插件写操作的 actor 归因候 BE**:壳侧宿主代理经 `/rpc` 调用,
   actor=user("editor")(HTTP 通道无插件身份);插件写操作现以
   `causedBy=plugin:<id>` 链在 OpLog 显式留痕(可审计可检索),**真正的
   actor=plugin 归因**只在服务端 `plugin-call` 通道(e2e_headless 已断言)。
   候 BE:`/api/v1` 增插件身份面(token 之外的插件凭证/会话级身份声明)。
3. **P0 修复记录**(G3,当轮修复):`scratch.rs::copy_tree` 硬链接穿透——
   append-only 面(oplog/rev)强制整拷贝;文件级回归测试两道
   (`scratch_append_only_files_never_penetrate` + e2e_ai_native P0 断言);
   红绿证据留档(撤修即红/复修即绿)。硬链接继续用于「临时文件+改名」安全面
   (GB 级媒体零拷贝收益保留)。
4. **Python SDK 未做**:T7.1 以 TS 为先(计划书口径);Python 客户端生成器与
   生成物未落(`gen_ts_sdk.py` 单源结构已就绪,复制面即成,候需求出现)。
5. **WASM 插件远期**(ADR-0024 决策):插件形态分级 JS Worker 先行/外部进程
   补能力;WASM 沙箱列远期(触发条件:真实需求出现再立项)。
6. **壳侧崩溃徽标一次刷新滞后**(G3 实测登记):`crashPlugin` 内 stopPlugin
   触发的列表重渲先于 setEnabled 落账,崩溃后徽标停留「启动中」直至下一次
   刷新(语义面注册表已禁用,重启不会自动拉起);e2e 以 `plugin-refresh`
   观察刷新后断言;壳侧修复候 FE 波(apps/web 本波禁碰)。

## 待办(册七后续任务)

- ~~T7.1 Editor API~~ **已闭合(G0;Python SDK 未做,A7-L 4)**;
- ~~T7.2 插件系统~~ **已闭合(G1 服务端 + G2 壳侧 Worker 宿主;WASM 远期,
  A7-L 5;FE actor 归因候 BE,A7-L 2)**;
- ~~T7.3 脚本标签页~~ **已闭合(G2 + G3 e2e;script_run RPC 候 BE,A7-L 1)**;
- ~~T7.4 Headless~~ **已闭合(G1;e2e_headless 判定器)**;
- ~~T7.5 AI 协作面~~ **已闭合(G1 BE + G2 壳侧 + G3 e2e;P0 修复收口)**;
- ~~T7.6 工程打包~~ **已闭合(G0 .cfpkg;分享导出复现包=既有渲染产物+cfproj
  组合面,不另立工具)**;
- 多实例协作:**明确暂缓**(D-G3/ADR-0026,冲突停写机制保持;触发条件登记)。
