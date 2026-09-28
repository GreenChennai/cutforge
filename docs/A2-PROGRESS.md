# A2 册二进度总账(前端壳重写)

> 执行依据:《CutForge-迭代计划》册二(前端壳重写)+ 总纲 §1.3 全局纪律。
> 本文件沿用 [A1-PROGRESS.md](A1-PROGRESS.md) 的滚动台账惯例:复盘发现的问题**强制折入
> 下一册任务单**(总纲 §1.3.5);每指标可判定、可存档,不得为通过而放宽;未完成项如实写
> 「进行中 / 未达标」。
> 批次落库:册二实码与本文档同处 7f02955 之后的未提交工作面,提交切分由主控决定;
> 本文所引数字均为收官实测值(结构行数 2026-09-29 盘面复测,bench 报告
> `docs/bench/2026-09-28.json`,gitRev 7f02955)。

## 一、七任务逐项

| 任务 | 做了什么 | 关键证据 | 落点 |
|---|---|---|---|
| T2.1 壳模块化重写 | 旧单文件壳拆为 **core/render/panels/ui 四层 ESM**:37 个 js 文件共 3,555 行 + 13 个 css 共 439 行,单文件最大 `core/commands.js` **354 行**(红线 ≤400);`index.html` **78 行**(红线 ≤120),面板内部结构由 `js/panels/*` 装配期建入;无构建 ESM 多文件(ADR-0011),时间线 canvas 重绘层 + DOM 交互层混合(ADR-0012);旧壳本体保全 `apps/web/legacy/`,经 `/assets/legacy/` 可回退(B-R1 过渡期) | `gate A2 js-line-limit` 绿;`e2e_static` 字节级别名(`/`、`/app.js`、`/style.css` 根三入口)锁定 | `apps/web/{index.html,js/,css/,legacy/}`、`docs/adr/0011`、`docs/adr/0012` |
| T2.2 单向流与重建铁律 | 六 store(单一可变点)+ projector 只读投影 + commands 命令面单向流;keyed 增量渲染(一次 clip move 相关 DOM 变更从旧壳数千次降到 **7 次**);重建铁律自测入口 `window.__cutforgeSelfTest()`(清空四投影派生面 → 全量重投影 → 逐字段 diff) | `e2e_ui_smoke` §11/§12:DOM 变更 7 < 50;selfTest ok=true | `apps/web/js/core/{store,projector,commands,model}.js`、`apps/web/TESTIDS.md` §五 |
| T2.3 API 收口与事件通道 | 网络/事件面收口:零裸 fetch(白名单唯一出口 `api.js`);错误分类(TIMEOUT/NETWORK/HTTP);event-bus;**SSE 主通道 + 长轮询降级**(A1 壳 300ms 轮询废除) | `shell-purity-v2` R3 零违规;`e2e_ui_smoke` AC-2.6③ SSE 毒化后长轮询接管 | `apps/web/js/core/{api,errors,event-bus}.js` |
| T2.4 预览链路升级(播放解耦 + 精确预览) | 媒体元素池(播放中改工程不销毁媒体元素);**后端新工具 `render_frame`**(41→42=13 查询+21 写+8 编排,编排类;帧缓存键=工作区指纹+atMs 100ms 量化+画幅+版本+ASS 哈希,改一笔必 miss);壳侧「精确预览」按钮消费之(工具未落库时如实显示服务端错误,不做假图)。**顺带修复真实 bug:Windows 字幕烧录路径反斜杠被 ffmpeg filtergraph 转义规则吞掉(此前 Windows 烧录必炸)**,并加「烧录前后帧字节必不同」实渲测试 | `e2e_playback_survival`:283 帧采样 currentTime 回跳 **0ms**、外部改动 SSE→重投影 **103ms**;`render_matrix` 烧录差异实渲测试绿 | `crates/cutforge-mcp/src/progress.rs`(render_frame_tool)、`crates/cutforge-render/src/frame.rs`、`apps/web/js/render/media-pool.js`、`apps/web/js/panels/preview.js` |
| T2.5 面板装配与交互增量 | `js/panels/*` 九面板建入;新增能力:右键菜单、轨头眼睛开关(`ephemeral.*` 视图隐藏,不进 IR/不落盘/不参与撤销)、导出剪映草稿按钮(export_jianying)、数字字段拖拽调节、向导模态 a11y(role=dialog+焦点陷阱+Esc)、快捷键调度器;临时投影边界三原则 + `ephemeral.*` 命名空间(ADR-0013,闭合 A1 交接#4) | `shell-purity-v2` R1/R2 零违规;TESTIDS.md 全量锚点登记 | `apps/web/js/panels/*`、`apps/web/js/render/{gestures,timeline-view}.js`、`docs/adr/0013` |
| T2.6 册级门禁与 CI | `gate.py A2` 注册(D-A2 制):**11 阻断 + 1 观察(legacy-reminder)**,主控复跑 12 项全绿;`check-shell-purity` 升级 **v2**(R1 持久化语义禁令 / R2 投影只读 + timelineStore 只准 projector 写 / R3 禁裸 fetch 白名单 api.js / R4 legacy 豁免;注入样例实测能抓);CI web-e2e 增 ui_smoke + playback_survival 两步(perf 不进 CI:帧率阈值负载敏感,`--min-fps` 参数化本机跑) | gate A2 12 项 exit 0;R4 豁免面随 legacy/ 存在而生效(移除条件写进判定器 data.legacyExempt) | `tools/gates/gate.py`、`tools/check_shell_purity*`(判定器)、`.github/workflows/gate.yml` |
| T2.7 e2e 体系与 testid 迁移 | 三份旧脚本(e2e_edit_ops/e2e_preview/e2e_from_zero)选择器迁 `data-testid` 全绿;新增 `e2e_ui_smoke`(AC-2.3/2.5/2.6 内嵌)、`e2e_playback_survival`(AC-2.2)、`e2e_perf_timeline`(AC-2.4);`TESTIDS.md` 登记全量锚点 + 旧 id 兼容红线(两册过渡)。计划编号口径:perf 专项在 gate.py 注释作 T2.9,本表按七任务归并登记 | 三旧 e2e + 三新 e2e 全绿(AC-2.7) | `tools/e2e_{ui_smoke,playback_survival,perf_timeline}.py`、`apps/web/TESTIDS.md` |

配套:A1 交接#6 基线对照已跑——`docs/bench/2026-09-28.json`(release 档)check 全 PASS:
open 4.73ms / query 17.66ms / apply 69.56ms / render min 5802.7ms 对 A1 基线 6586.1ms
**−11.9%**(劣化 >20% 阻断口径不破);A1 交接#2(静态资源零 Rust 改动)与#4(临时投影纪律)
在本册兑现。

## 二、AC-2.1 ~ AC-2.8 状态表

| # | 验收项 | 判定方式 | 状态 | 当前实测 |
|---|---|---|---|---|
| AC-2.1 | 壳行数红线(js 单文件 ≤400;index.html ≤120);重写面零 Rust 改动 | `gate.py A2` js-line-limit | ✅ | 最大 js 单文件 **354 行**(commands.js);index.html **78 行**;legacy/ 外壳重写零 Rust 改动(`/assets/*` 托管册一已备;Rust 侧改动全部归属 T2.4 render_frame 与 bug 修复范围)。微瑕:main.js 102 行超 ≤100 装配入口建议 2 行(A2-L3) |
| AC-2.2 | 播放中受扰,播放状态与位置零丢失 | `tools/e2e_playback_survival.py` | ✅ | 283 帧采样 currentTime 回跳 **0ms**;外部改动 SSE→重投影 **103ms** |
| AC-2.3 | 一次 clip move 相关 DOM 变更 <50(keyed 增量) | `tools/e2e_ui_smoke.py`(MutationObserver) | ✅ | **7 次**(旧壳同操作为数千次量级) |
| AC-2.4 | 1k clips 虚拟化(视口 clip 节点 ≤100)+ 滚动帧率 | `tools/e2e_perf_timeline.py`(--min-fps 55) | ✅ | 视口 **56/48** 节点(≤100);滚动 P95 **60.2fps**、掉帧 **0** |
| AC-2.5 | selfTest 重建铁律 ok(store 可由内核投影完全重建) | `e2e_ui_smoke` §12(`__cutforgeSelfTest`) | ✅ | 清空 → 全量重投影 → 逐字段 diff=0(ok=true) |
| AC-2.6 | render_frame 超时中止可恢复 / 401 分流 / SSE 断→长轮询降级 | `e2e_ui_smoke` 三场景 | ✅ | 10s 超时中止 + 未落账 + 可恢复;401 分流;SSE 毒化后长轮询接管(三场景全过) |
| AC-2.7 | 三份旧 e2e 迁 data-testid 全绿 | `e2e_edit_ops` / `e2e_preview` / `e2e_from_zero` | ✅ | 三份全绿(旧 id 兼容红线按 TESTIDS §四保留) |
| AC-2.8 | purity v2 违规 0;write-paths 0 | `gate.py A2` shell-purity-v2 + check-write-paths | ✅ | 双 0;五处存量定性全合法(socket 流写 / `#[cfg(test)]` 夹具;判定器按类型收窄,不开大口) |

> 口径说明:实测值以册二收官盘面为准(7f02955 之后未提交面);perf 为 debug 档本地
> 空载实测,CI 弱机不复核(负载敏感,`--min-fps` 参数化),详见 A2-L7。

## 三、遗留台账(强制折入下一册;总纲 §1.3.5)

编号自 A2-L1 起,与 A1 台账(L-1~L-9)**并行衔接**:A1 各条继续有效,按其原处置建议
折入册三/册四/册五,本表不重复收录;其中 **A1-L3(长轮询移除)本册有处置变化**,追记见
[A1-PROGRESS.md](A1-PROGRESS.md) §三 L-3。A1 §四交接六条中:#1(事件消费)、#2(静态资源)、
#4(临时投影)、#6(基线对照)已在本册闭合;#3(渲染进度 UI)、#5(契约加字段)留册三与册四/五。

| # | 发现 | 处置建议 |
|---|---|---|
| A2-L1 | 壳对 net 类错误(TIMEOUT/NETWORK)静默收敛、不 toast——用户无感知 | 册三错误面统一时补横幅(与 A1-L2 渲染进度 UI 同一批错误面改造) |
| A2-L2 | `apps/web/legacy/` 为回退期设施(ADR-0011/B-R1):删除时须**同步删** purity v2 R4 豁免 + `legacy-reminder` 观察项 + TESTIDS §四中指向 `/assets/legacy/` 的表述,否则豁免面变僵尸 | ✅ 已了断:册三收尾(2026-09-29)删除 `apps/web/legacy/` 整树,同步收口 purity v2 R4 豁免(gates.rs)与 `legacy-reminder` 观察项(gate.py),TESTIDS §四表述同步 |
| A2-L3 | `js/main.js` 102 行,超 T2.1「装配入口 ≤100 行」纪律 2 行(AC-2.1 红线 ≤400 达标,属建议级微瑕,已如实登记) | 册三顺手收敛(非阻断);纪律口径不变 |
| A2-L4 | 波形为装饰纹理(非真实音频数据);媒体池无真缩略图 | 册四 T4.1 波形接真数据;媒体池真缩略图随册四代理转码 |
| A2-L5 | SSE 流寿命约 30min,依赖浏览器自动重连续流,**无 UI 指示**(断连期间用户不知道自己离线) | 册三错误面统一时加连接状态指示;与长轮询降级路径(A1-L3)一并定夺去留 |
| A2-L6 | purity R2 为**文本级门禁**:「函数参数把 store 传进去再 set」的绕行形态抓不到(判定器注释已声明该局限) | 册三若引入该形态的代码,先升级判定器(如按调用图收窄)再动代码;现状零违规 |
| A2-L7 | perf 实测(AC-2.4)为 debug 档本地空载,CI 弱机不复核——帧率阈值负载敏感 | 不进 CI 的决策保持;`--min-fps` 参数化本机跑;后续册收官照跑并落盘新日期 JSON(同 A1-L5 口径) |

## 四、与册三的交接清单

1. **错误面统一(册三主线之一)**:A2-L1(net 类静默补横幅)+ A2-L5(SSE 断连无指示)
   + A1-L2(`StepReport.to_progress` 数据源已备,HTTP 面透传 steps/cacheHits 给壳消费)
   三件事同批做,避免横幅/进度/连接状态三套 UI 各写一遍。
2. **legacy/ 删除窗口(册三收尾)**:按 A2-L2 清单删 `apps/web/legacy/` + R4 豁免 +
   legacy-reminder 观察项;同时了断 A1-L3——长轮询降级路径是「删」还是「保留为正式
   降级面」,册三定夺后同步改 FLOW §5.6 标注与 e2e_events 降级断言。
3. **门禁延续**:册三收官跑 `gate.py A2` 同款 12 项(壳面不劣化)+ `gate.py A<n>` 按
   D-A2 注册册三项;purity v2 若遇 A2-L6 形态先升级判定器。
4. **性能对照**:bench 收官照跑落盘(A1-L5/A2-L7 口径);1k 工程帧率门禁用
   `e2e_perf_timeline.py --min-fps 55` 本机跑,不进 CI。
5. **微瑕清账**:A2-L3(main.js 102→≤100)顺手收敛;A1-L4(stage_status 顶层 quirk)、
   A1-L7(seg 全量哈希字段级拆分)若册三动到对应面则一并处理,不动则继续留账。
6. **契约纪律不变**:册三新增后端工具先改 `schemas/mcp-tools.json`(唯一真相源),再走
   dispatch/parity golden/文档口径四件套(render_frame 即按此流程 41→42);文档工具数
   以 `python tools/check_doc_counts.py` 为准。
