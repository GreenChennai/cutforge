# A3 册三进度总账(UX 动效 / 键位 / 可访问性)

> 执行依据:《CutForge-迭代计划》册三(UX 动效/键位/可访问性)+ 总纲 §1.3 全局纪律。
> 本文件沿用 [A2-PROGRESS.md](A2-PROGRESS.md)(上溯 [A1-PROGRESS.md](A1-PROGRESS.md))的
> 滚动台账惯例:复盘发现的问题**强制折入下一册任务单**(总纲 §1.3.5);每指标可判定、
> 可存档,不得为通过而放宽;未完成项如实写「进行中 / 未达标」。
> 批次落库:册三实码与本文档同处 feat/plan-A3-ux-motion 分支未提交工作面,提交切分由
> 主控决定;本文所引数字均为收官实测值(2026-09-29 盘面;性能存档
> `docs/bench/perf-a3.json`)。`gate.py A3` 15 项已注册齐备,**主控终验待跑**——
> 本文状态以册内实测为准,终验结论由主控复跑落章。

## 一、七任务逐项(+A2 遗留收口)

| 任务 | 做了什么 | 关键证据 | 落点 |
|---|---|---|---|
| T3.1 设计系统与主题策略 | 主题策略定案 **ADR-0014**(本册仅深色,类达芬奇蓝灰 13 档 + 单强调色);tokens 三层(原始/语义/组件),**唯一定义点 `css/tokens.css`(202 行)**;对比度自查表 19 组正文 + 4 组图形/大字**全 AA**;壳纯度门禁升级 **v3 新增 R5 零硬编码色值**(css+js+html 三面扫描,注释不豁免,注入样例必抓;豁免登记仅 `css/tokens.css` + `assets/icons.js`);canvas 取色收口 `js/render/theme.js` 读语义 token,与 DOM 面同源 | `docs/design/contrast-table.md` 全 AA;gates.rs R5 注入样例单测必抓;theme.js 与 DOM 同色源 | `docs/adr/0014`、`apps/web/css/tokens.css`、`apps/web/js/render/theme.js`、`crates/cutforge-cli/src/gates.rs`(v3) |
| T3.2 动效与录屏存档 | 时长梯度 fast/base/slow = **80/160/240ms**(token `--cf-dur-*`)+ 三条缓动 token;一律 transform/opacity 合成器路径;**27 项微交互清单全实现**;**19 条录屏存档 `docs/design/recordings/`(4.7MB,`record-captures.py` 可重录)**;`prefers-reduced-motion` 在 `css/motion.css` 一处总控;降噪纪律(同屏并发 ≤3 / hover 无位移 / 拖拽零动画) | micro-interactions.md 27 项逐行回链实现位置与录屏;recordings/ 19 条 webm 落库 | `apps/web/css/motion.css`、`docs/design/{micro-interactions.md,record-captures.py,recordings/}` |
| T3.3 精确拖拽手势 | pointer capture 管线(`gesture-kit.js` 通用内核 + `gestures.js` 业务手柄):ghost 复用节点跟手 **≤1 帧**、3px 位移阈值、**Esc/失焦取消零 Op**;trim 相邻片段碰撞夹取 + 实时时长气泡;框选多选;边缘 60px 自动卷入;Ctrl/⌘+滚轮以视口中心缩放;拖拽全程零 Op、**松手一次手势单命令** | `e2e_drag_perf`:拖拽 P95 **60.2fps**(`--min-fps 55`)/ ghost 跟手 ≤1 帧 / Esc 取消零 Op / 松手单命令 / 非法落点红态 | `apps/web/js/render/{gesture-kit,gestures,timeline-view}.js`、`tools/e2e_drag_perf.py` |
| T3.4 快捷键体系 | **45 条数据化注册**(`keymap-registry.js`,导出面 `window.__cfKeymap.table()`):全部可重绑定 + 冲突检测(强制 = 停用被占)+ 恢复默认,localStorage 偏好(`prefs.js`);J/K/L 倍速链、I/O 入出点、M 会话级标记(ephemeral 不落盘,持久化判定见 A3-L5)、B 切割模式;输入态屏蔽(`shortcut-gate` data-gate 可断言);「?」帮助面板全表 + 搜索;右键菜单逐项带按键提示 | `e2e_hotkeys`:注册表 45 条遍历 + 实按 26 条(含 J/K/L、I/O、B、S 等点名链路)+ 重绑定冲突闭环 + 输入态屏蔽 + 帮助搜索 | `apps/web/js/ui/{keymap-registry,keymap,shortcuts,help-panel,prefs}.js`、`tools/e2e_hotkeys.py` |
| T3.5 性能预算与可视 | dev 性能面板(**Shift+D,默认关**;打开才起采样 rAF,零常态开销):帧率 / rAF 分布 / DOM 数 / 未完成请求 / 投影耗时 / 媒体池占用 + 预算表逐行可视(`data-pass` 非颜色线索);媒体池 **POOL_MAX=24 LRU + 播放头窗口锚定**([t−2s, t+8s] 不淘);预算口径真相源 `docs/design/perf-budget.md` | `e2e_perf_budget`:boot **219ms** < 1s / 页签 **43.1ms** < 100ms / 导出进度回调 **2.4Hz** ≥2Hz 达标 + 条平滑 / 池峰值 24 有界(200 轮导航淘汰 6768);结果落盘 `docs/bench/perf-a3.json`(收口后重生成) | `apps/web/js/ui/{perf-panel,perf}.js`、`apps/web/js/render/media-pool.js`、`docs/design/perf-budget.md` |
| T3.6 可访问性 | 键盘编辑全链路(选择 Alt+方向 → 移动/跨轨 Ctrl(+Shift)+方向 → 删除 Del → 撤销 toast 按钮 / Ctrl+Z,**全程 rev/OpLog 断言**);模态 aria-modal + 焦点归还;右键菜单四上下文带快捷键提示与禁用原因;交互元素全 title;状态线索非颜色单依赖;错误消息含修复提示;**axe-core 4(4.13.0,`tools/vendor/axe.min.js` 580,491 字节入库存档)全页扫描 0 critical/serious**(原登记两笔 label/aria-tooltip-name 已修复,KNOWN_REGISTERED 保留兜底) | `e2e_a11y`:键盘闭环 + axe 全页扫描 0 新增 critical/serious;`docs/design/contrast-table.md` 非颜色线索口径 | `apps/web/js/ui/*`、`tools/e2e_a11y.py`、`tools/vendor/axe.min.js` |
| T3.7 新手路径 | **脚本盲测三流程(导入→剪切→导出 / 加转场 / 加 BGM)零卡点通过**(`docs/design/novice-blind-test.py`:只凭界面可见文案驱动,禁 testid 与源码知识);卡点修复对照表 `docs/design/novice-audit.md`:素材落点被占**自动顺接到轨尾** / 分割菜单自适应(播放头 / 右键位置 / 禁用给修法)/ 首启引导条(「知道了」可关,localStorage 记忆)/ 时间线空态下一步 / 删除 toast 5s **真撤销** / 批量撤销前确认(设置可关) | novice-audit §一卡点对照 5 行全闭合(2 修壳 / 2 修驱动 / 1 无需修);§二自明性盘点 | `docs/design/{novice-audit.md,novice-blind-test.py}`、`apps/web/js/ui/onboarding.js`、`apps/web/js/core/commands.js` |
| 附:A2 遗留收口 | **net 错误横幅接线**(`banner-conn`,恢复自动收起,A2-L1 闭合);**SSE 连接态徽标**(`conn-badge`,data-state = ok/reconnecting/polling/retry,A2-L5 闭合);**`apps/web/legacy/` 整树删除**(A2-L2 闭合:purity R4 豁免收口 + gate A2 legacy-reminder 观察项移除 + 全仓引用清理);长轮询降级去留定夺 = **保留为正式降级面**(A1-L3 了断,FLOW §5.6 同步) | TESTIDS.md banner-conn/conn-badge 登记;gate.py legacy-reminder 移除注释;FLOW §5.6 定夺标注 | `apps/web/js/ui/banner.js`、`apps/web/js/ui/wire-wave3.js`、`tools/gates/gate.py`、`docs/FLOW.md` |

配套:`gate.py A3` 册级门禁注册(**15 项全阻断**,D-A2 制)、CI web-e2e 增 hotkeys/a11y
两步(drag/perf/budget 负载敏感不进 CI)、`docs/design/` 四份口径文档 + 录屏存档落库、
README/CHANGELOG/FLOW 同步(本批)。

## 二、AC-3.1 ~ AC-3.7 状态表

| # | 验收项 | 判定方式 | 状态 | 当前实测 |
|---|---|---|---|---|
| AC-3.1 | 设计系统:色值唯一定义点;正文/图形对比度全 AA;零硬编码色值 | `docs/design/contrast-table.md` + `gate.py A3` shell-purity v3 R5 | ✅ | tokens 三层唯一定义点(tokens.css **202 行**);**19 组正文 + 4 组图形全 AA**;R5 三面扫描(css+js+html,注释不豁免)注入样例必抓;canvas 经 theme.js 与 DOM 同源 |
| AC-3.2 | 动效:微交互清单实现 + 录屏存档 + reduced-motion 总控 | `docs/design/micro-interactions.md` + `recordings/` | ✅ | **27 项全实现**;**19 条 webm(4.7MB)**存档可重录;motion.css 一处总控全量降级 |
| AC-3.3 | 拖拽手感:帧率 / 跟手 / 取消零 Op | `tools/e2e_drag_perf.py --min-fps 55` | ✅ | P95 **60.2fps**;ghost 跟手 **≤1 帧**;Esc/失焦取消**零 Op**、松手单命令;非法落点红态 |
| AC-3.4 | 键位:数据化注册 ≥40 / 实按抽样 / 重绑定闭环 | `tools/e2e_hotkeys.py` | ✅ | 注册 **45 条**;实按 **26 条**(含点名链路全覆盖);重绑定冲突检测(强制 = 停用被占)闭环;输入态屏蔽可断言 |
| AC-3.5 | 性能预算:首屏 / 页签 / 导出节奏 / 媒体池有界 | `tools/e2e_perf_budget.py`(结果 `docs/bench/perf-a3.json`) | ✅ | boot **219ms** < 1s;页签 **43.1ms** < 100ms;导出进度回调**实测 2.4Hz ≥2Hz 达标**(轮询已收口 500ms,A3-L3 已于 A3 期闭合)+ 条平滑恒在;池峰值 **24** 有界(200 轮导航淘汰 6768);4h 长跑为人工项(A3-L1) |
| AC-3.6 | 可访问性:键盘编辑闭环 + axe 0 critical/serious | `tools/e2e_a11y.py` | ✅ | 键盘链(Alt+方向 → Ctrl+方向 → Del → 撤销)rev/OpLog 全程断言;axe 全页扫描 **0 critical/serious**(原登记两笔已修,登记表保留兜底) |
| AC-3.7 | 新手路径:盲测三流程零卡点 | `docs/design/novice-blind-test.py` + `novice-audit.md` | ✅ **脚本盲测通过,真人复验为人工项(A3-L2)** | 三流程零卡点;卡点对照 5 行全闭合;自明性盘点(引导条/空态/toast 撤销/帮助面板)落库 |

> 口径说明:性能数字以存档 `docs/bench/perf-a3.json`(2026-09-29 捕获,debug 本地档)
> 为准;帧率/拖拽/预算三类负载敏感项为本机安静时段实测,不进 CI(同 A2-L7 口径);
> gate A3 15 项注册齐备,主控终验待跑,终验结果出来前本表不写「全绿」。

## 三、遗留台账(强制折入下一册;总纲 §1.3.5)

编号自 A3-L1 起,与 A1(L-1~L-9)/ A2(A2-L1~A2-L7)**并行衔接**:A1/A2 各条继续有效,
按其原处置建议折入后续册,本表不重复收录。**本册处置变化追记**:
A1-L3(长轮询去留)册三定夺 = **保留为正式降级面**(conn-badge 以 polling 态可视,
e2e_events 降级断言保留,FLOW §5.6 已同步);A1-L2(渲染进度 steps/cacheHits 透传壳
消费)本册未做,**继续留账**折入册四/册五;A2 交接六条:#1(错误面)/#2(legacy 删除)/
#3(门禁延续)/#4(性能对照)闭合,#5(微瑕清账)部分闭合——main.js 未收敛反涨转
A3-L7,A1-L4/A1-L7 未动到对应面继续留账,#6(契约纪律)兑现(本册零新增后端工具,
工具数 42 不变);A2-L6(purity R2 文本级局限)册三未升级判定器,该绕行形态是否出现
以主控复跑 shell-purity 终验为准(出现即先升级判定器再动代码,口径不变)。

| # | 发现 | 处置建议 |
|---|---|---|
| A3-L1 | **4h 长跑内存为人工项**:e2e_perf_budget 以 200 轮导航模拟 + 媒体池有界(POOL_MAX=24,淘汰 6768)替代长跑断言(gate.py 注释与 perf-a3.json 均如实注明) | 后续册视需要安排 4h 真机长跑人工复验;自动化替代口径(池有界)不变 |
| A3-L2 | **真人盲测复验为人工项**:AC-3.7 本册为脚本盲测(novice-blind-test.py 三流程零卡点),真人盲测录屏存档未做(novice-audit §三 已登记) | 册四安排真人按同三流程复验,录屏归档 `docs/design/recordings/`(与 19 条自动化录屏同库) |
| A3-L3 | ✅ **已闭合(A3 期收口)**:导出进度原为壳侧纯轮询(`js/core/render-commands.js` setTimeout **800ms ≈1.25Hz**,当时预算表行 6「≥2Hz」未达,e2e 只断言节奏有界 + 平滑 + 恒在不冒充达标)→ 轮询间隔已按原处置收口 **500ms**,复测面板行「✓ 导出进度回调 2.4Hz ≥2Hz」pass=1,`docs/bench/perf-a3.json` 已按修复后重生成 | 已按原处置执行:间隔 800→500ms 一行改(render-commands.js)+ 复测 e2e_perf_budget 以落盘达标口径闭账;条目与证据链保留,不再折入后续册 |
| A3-L4 | **Tab 键让位折中未经 ADR**:view.panels 绑定 Tab(焦点在控件上时仍走焦点移动,keymap.js 绑定表已注明行为),属可访问性折中 | 册四补 ADR 定夺:Tab 让位范围、焦点可见性与 WCAG 2.1.2(无键盘陷阱)口径,定了再动 |
| A3-L5 | **M 标记持久化判定「不需要」**:会话标记为剪辑辅助线,ephemeral 不落盘/不进 IR(ADR-0013 口径;novice-audit §三 已登记判定),非遗留缺陷而是已了断的设计决定 | 判定已闭账,不再折入后续册;如未来需求变化,以「markers 持久化」新立字段走 CONTRACT-WORKFLOW 七步 |
| A3-L6 | **CI 跨仓门禁钉扎 CutFlow faad005**:CutFlow v0.21.0(e1745c1)ADR-0060 先行落地但其 README 索引未跟上,adr-unique 会红,故 CI 钉已验证截面(gate.yml 注释声明) | CutFlow README 索引补 0060 后删除 checkout 行回到 main 即解除(跨仓事项,非本仓代码);解除前 CI 侧不受影响 |
| A3-L7 | **A2-L3 顺手收敛未做反涨**:main.js 102→**124 行**(建议级 ≤100;红线 ≤400 达标,全壳最大 js 仍为 commands.js 393 行) | 册四动装配面时收敛;建议级纪律口径不变 |

## 四、与册四的交接清单

1. **代理转码**:导出/预览的代理媒体链路(转码编排走既有 render 编排面,不加新写路径);
   媒体池真缩略图随代理转码一并落(A2-L4 后半)。
2. **真缩略图**:素材面板/时间线缩略图接真实帧(复用 `render_frame` 帧缓存键口径),
   替换占位;受 POOL_MAX=24 有界纪律约束(docs/design/perf-budget.md §三)。
3. **波形真数据**:`js/render/waveform.js` 装饰纹理 → 真实音频数据(A2-L4 前半);
   `wire-wave3.js` 已留接线位,接通后回填 micro-interactions 第 11 项录屏。
4. **标记持久化**:A3-L5 已判定「不需要」(会话级 ephemeral),**册四不再实现**;
   如需求变化走 CONTRACT-WORKFLOW 七步新立字段,不动既有判定。
5. **协同编辑预留**:多写入者底座(OpLog/rev/三路合并/冲突快照)册四只预留接口面
   (presence/锁语义的壳侧占位),不改合并语义;动语义前先补 ADR。
6. **门禁与基线延续**:册四收官跑 `gate.py A3` 同款 15 项(UX 面不劣化)+ 按 D-A2 注册
   `gate.py A4`;bench 收官照跑落盘新日期 JSON(A1-L5 口径);A1-L2(渲染进度
   steps/cacheHits 透传壳)留册四排期;A3-L3(导出轮询 500ms 收口)已于 A3 期闭合,
   不再占用册四排期。
