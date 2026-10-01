# CutForge Web 壳 · data-testid 登记清单(册二 T2.7)

> 命名规范:全小写 kebab-case,按「区域-语义」命名;轨道/片段类锚点带稳定 id 后缀
> (`track-lane-V1`、`field-startMs` 例外:字段名用驼峰原样,便于与 ui-fields 对表)。
> e2e 迁移纪律(册二 T2.7 / M10-R5):**新 e2e 一律用 data-testid;旧 id 保留两册过渡**,
> 断言以服务端状态(rev/oplog/盘面)为准,UI 只作驱动。

## 一、静态骨架(index.html)

| testid | 元素 | 说明 | 对应旧 id |
|---|---|---|---|
| `banner-token` | div | token 变更/缺失横幅 | #token-banner |
| `banner-conflict` | div | 冲突停写横幅 | #conflict-banner |
| `banner-internal` | div | 契约错误折叠横幅 | #internal-banner |
| `topbar` | header | 顶栏 | header |
| `session-info` | span | root 会话信息 | #session-info |
| `project-new` | button | 新建工程向导入口 | #btn-project-new |
| `to-start` / `play-toggle` / `to-end` | button | 传输控制 | #pv-to-start/#pv-play/#pv-to-end |
| `tb-time` | span | 顶栏时间码 | #tb-time |
| `magnet` / `ripple-toggle` | checkbox | 磁吸 / 波纹删除开关 | #magnet/#ripple |
| `undo` / `redo` | button | 撤销/重做 | #btn-undo/#btn-redo |
| `tabs` | nav | 页签容器 | #tabs |
| `tab-timeline` / `tab-notes` / `tab-diff` / `tab-conflicts` | button | 页签(data-tab 同值) | 同名 button |
| `toolbar` / `sel-info` / `playhead-ms` / `rev` | — | 时间线工具行(rev 为 e2e 就绪锚点,初始 `-`) | 同名 id |
| `workbench` / `media-panel` / `preview` / `bgm-panel` / `export-panel` / `inspector` | div | 工作台三栏容器 | #workbench/#media-panel/#preview/#bgm-panel/#export/#inspector |
| `timeline-wrap` / `ruler` / `timeline-tracks` / `tl-overlay` / `playhead` | — | 时间线骨架(旧 .tick 刻度 DOM 已删,canvas 重绘,ADR-0012) | #timeline-wrap/#ruler/#tracks/#playhead |
| `rev-dot` | span | rev 旁落盘脉冲点(册三 T3.2:rev 翻牌时绿点一次性脉冲 = 已保存;纯视觉,aria-hidden) | — |
| `banner-conn` | div | 断连/网络错误横幅(册三 T3.2 组件 + A2 遗留①接线:wire-wave3.js 写 uiStore.connBanner;录屏 recordings/18-conn-banner.webm) | — |
| `panel-notes` / `panel-diff` / `panel-conflicts` | div | 三个次级页容器 | #tab-notes 等(容器 id 保留 `tab-*`) |
| `wizard` | div | 向导遮罩(动态 id 复用) | #wizard |
| `toasts` / `status` | — | toast 容器(aria-live)/ 读屏诊断锚点 | 同名 id |

## 二、面板动态元素(js/panels/*)

### 素材面板(media-panel.js)
| testid | 元素 | 说明 |
|---|---|---|
| `media-dir` / `media-refresh` / `media-list` | 输入/按钮/列表 | 旧 id 保留 |
| `media-item` | div.draggable | 素材卡(W12:图标卡+文件名+时长;真缩略图候册四 T4.1) |
| `set-bgm` | button(audio 行) | 一键设 BGM(dataset.src) |

### 检查器(inspector.js;字段集以 /ui-fields 真相源为准)
| testid | 元素 | 说明 | 对应旧 id |
|---|---|---|---|
| `insp-fields` / `insp-groups` / `insp-readonly` | div | 信息行/分组容器/只读面 | 同名 id |
| `insp-apply` / `insp-dup` | button | 应用(clip_update)/复制到播放头 | 同名 id |
| `insp-group-<分组名>` | fieldset | 分组(基础/画面/音频/变速/文本/转场/动效) | .insp-group |
| `field-<字段名>` | input/select | 字段控件;带点字段 `.`→`-`(如 `field-transition-type`) | #insp-f-<field> |
| `insp-start` / `insp-dur` / `insp-vol` | input | 基础三字段双锚点(旧 id 兼容红线,见 §四) | 同名 |
| `track-add-video` / `track-add-audio` / `track-add-text` | button | 加轨 | 同名 id |

### 预览(preview.js)
| testid | 元素 | 说明 |
|---|---|---|
| `preview-stage` / `preview-canvas` | div/canvas | 画质代理画布(旧 #pv-stage/#pv-canvas) |
| `preview-transport` / `preview-frame-prev` / `preview-frame-next` / `preview-time` | — | 传输控制(旧 #pv-transport 等) |
| `preview-precise` | button | **精确预览(render_frame)**:工具未落库时如实显示服务端错误,不做假图 |
| `preview-precise-frame` / `preview-precise-hint` | img/div | 精确帧显示面 |
| `pv-media` | div(hidden) | 媒体元素池宿主(e2e_preview ≥4 元素锚点;旧 id 保留) |

### 导出(export.js)
| testid | 元素 | 说明 |
|---|---|---|
| `export-backend` / `export-ratio` / `export-run` | select/select/button | 双后端(cutforge/ffmpeg;旧 #exp-* id 保留) |
| `export-jianying` | button | **新增**:导出剪映草稿(export_jianying;弹 `jy-dialog`) |
| `jy-name` / `jy-run` / `jy-cancel` / `jy-progress` | — | 剪映草稿对话框内元素 |
| `export-progress` / `export-files` / `export-file` | div | 进度 / 产物清单(旧 #exp-progress/#exp-files) |

### BGM(bgm.js)
`bgm-src` `bgm-gain` `bgm-duck` `bgm-loop` `bgm-apply` `bgm-clear` `bgm-state`(全部旧 id 保留)

### 标注 / 差异 / 冲突
| testid | 元素 | 说明 |
|---|---|---|
| `note-body` / `note-create` | input/button | 创建标注(M10-2 锚点) |
| `notes-open` / `note-row` / `note-reply` / `note-opids` / `note-resolve` | — | open 列表;行内依次 [回执][opIds][结案](行文本含标注 id;M10-2 锚点) |
| `notes-orphans` | div | 孤儿标注数 |
| `diff-actor` / `diff-limit` / `diff-refresh` / `diff-undo-batch` / `diff-rows` / `diff-row` / `diff-pick` | — | OpLog 面板(pick=勾选撤销) |
| `conflict-rows` / `conflict-row` | — | 冲突清单 |

### 新建向导(wizard.js)
`wiz-name` `wiz-ratio` `wiz-fps` `wiz-tr-v` `wiz-tr-a` `wiz-tr-t` `wiz-hint` `wiz-create` `wiz-cancel`(全部旧 id 保留)

## 三、时间线动态元素(js/render/*)

| testid | 元素 | 说明 |
|---|---|---|
| `clip` | div.clip | 片段块(dataset.id / dataset.track 保留;`.clip[data-id]` 为旧 e2e 红线) |
| `clip-edge-l` / `clip-edge-r` | span | trim 热区 |
| `track-lane-<trackId>` | div.track | 轨道行(dataset.trackId/dataset.kind) |
| `lane-label-<trackId>` | span | 轨头(sticky 滚动同步) |
| `track-visibility-<trackId>` | button | **轨头眼睛开关**:ephemeral 视图隐藏(ADR-0013:不进 IR/不落盘/不参与撤销;aria-pressed) |
| `drag-ghost` | div.clip.ghost | 拖拽 ghost(ephemeral.dragGhost;仅手势期存在) |
| `ruler-canvas` | canvas | 标尺重绘层(ADR-0012) |
| `snap-pulse` | div | 吸附对齐脉冲(册三 T3.2:80ms 一次性动画;定位走 transform) |
| `marquee-box` | div | 框选矩形(册三 T3.3:空白拉框多选;仅手势期存在) |
| `context-menu` | div | 右键菜单 |
| `tooltip` / `toast` / `toast-err` | — | 提示三件套 |
| `dialog` / `jy-dialog` | div.cf-mask | 模态(role=dialog + 焦点陷阱 + Esc;`wizard` 复用旧 id) |

## 四、e2e 兼容红线:旧 id 保留清单(两册过渡,T2.7)

**必须继续存在的旧 id/选择器**(现役 e2e 依赖,见 tools/e2e_*.py):

- `#rev`(初始 `-`,投影到达翻牌)— e2e_edit_ops / e2e_preview 就绪锚点
- `#ruler`(点击坐标 ×(1/0.06)=ms;默认 pxPerMs=0.06 不可改)— 两份 e2e 的 seek 路径
- `.clip[data-id="<clipId>"]`(class 名 `clip` + `data-id` 属性)— e2e_edit_ops 片段点击
- `#insp-start`、`#insp-apply` — e2e_edit_ops 移动片段
- `#btn-undo`、`#btn-redo`(永远可点:无可撤销时服务端返回 ok:false,按钮不置灰)— e2e_edit_ops undo/redo 循环
- `#tabs button[data-tab="notes"]` — 页签切换
- `#note-body`、`#note-create`、`#notes-open .row`(行内 [input][input][button],行文本含标注 id)— M10-2 全链
- `#status`(sr-only 诊断锚点)— wait_rev 失败诊断
- `#pv-media video, #pv-media audio`(≥4 元素;readyState≥2)— e2e_preview
- `#pv-canvas`(2d 采样非全黑)、`#playhead-ms`(播放推进)、
  `#exp-backend`(option value=cutforge)、`#exp-run`、`#exp-progress`(含 `完成` + `final_cutforge_`)
- 静态托管字节级别名:`/` `#index.html`、`/app.js`、`/style.css`(apps/web 根三文件,e2e_static 锁定;
  旧壳本体 legacy/ 已于册三收尾删除,别名桩文件仅存契约,不含旧壳代码)

## 五、自测入口(T2.2 重建铁律 / AC-2.5)

```js
// 控制台(e2e 亦可驱动):
const r = await window.__cutforgeSelfTest();
// r = { ok: boolean, before: string, after: string }
// 语义:清空四个投影派生面(project/timeline/media/uiFields+conflicts)→ 全量重投影 →
// 逐字段 diff;ok=true 即「store 可由内核投影完全重建」。
```

补充:`window.__cfClipboard`(Ctrl+C 复制的片段 id,会话态)。
T3.4 增:`window.__cfKeymap`(键位注册表观察面:`table()` 全表 / `routeCount()` /
`keyOf(e)` / `comboOf(id)`;下波键位 e2e 遍历口)。

## 六、册三波二新增 testid(T3.4~T3.7 / A2 遗留)### 静态骨架(index.html)
| testid | 元素 | 说明 |
|---|---|---|
| `conn-badge` | span(topbar) | 事件通道连接态徽标(A2 遗留②):`data-state` = ok/reconnecting/polling/retry,文本即状态 |
| `shortcut-gate` | span(footer,sr-only) | 快捷键裁决锚点:每次按键写 `data-gate` = input/dialog/native/hit:<组合>/miss:<组合> |

### 快捷键体系(T3.4)
| testid | 元素 | 说明 |
|---|---|---|
| `help-dialog` | div.cf-mask | 「?」帮助面板(role=dialog) |
| `help-search` / `help-rows` / `help-row` / `help-keys` / `help-empty` | 输入/容器/行/键位 kbd/空结果 | 行带 `data-keybind-id`;搜索过滤后行数可断言 |
| `settings-dialog` | div.cf-mask | 「Ctrl+,」设置面板(role=dialog) |
| `keybind-list` / `keybind-row-<id>` / `keybind-combo-<id>` / `keybind-capture-<id>` | 容器/行/当前组合/捕获按钮 | 行 testid 用绑定 id(与 `__cfKeymap.table()` 对表) |
| `keybind-capture-zone-<id>` / `keybind-conflict-<id>` / `keybind-force-<id>` | 捕获区/冲突提示/强制按钮 | 冲突检测面:强制 = 既有绑定停用 |
| `keybind-reset` | button | 恢复默认键位(清 localStorage 覆盖) |
| `blade-mode` | span(toolbar) | 切割模式徽标(B 键;hidden 切换;时间线 wrap 挂 .blade-mode) |

### 性能面板(T3.5)
| testid | 元素 | 说明 |
|---|---|---|
| `perf-panel` / `perf-close` | div/button | Shift+D 唤起(默认关);role=region |
| `perf-stats` / `perf-stat` | 容器/行 | 行含 帧率/rAF 分布/DOM 节点/未完成请求/投影耗时/媒体池 |
| `perf-budgets` / `perf-budget` | 容器/行 | 预算表逐行;`data-pass`="1"\|"0"\|"na"(非颜色达标线索),`data-budget-id` 对表 docs/design/perf-budget.md |

### 新手路径(T3.7)
| testid | 元素 | 说明 |
|---|---|---|
| `onboard-bar` / `onboard-dismiss` | div/button | 首启引导条;「知道了」写 localStorage 后不再出现 |
| `timeline-empty` | div(#timeline-wrap 内) | 空工程下一步提示(clips>0 即隐;aria-live) |
| `toast-action` | button(toast 内) | 删除类 toast 的「撤销」按钮(5s 窗;真撤销走 undo 队列) |
| `confirm-dialog` / `confirm-ok` / `confirm-cancel` | div/button | 批量撤销前确认(设置 `setting-confirm-batch` 可关) |
| `setting-confirm-batch` | input[checkbox] | 设置面板「批量操作前确认」开关(localStorage) |


## 七、册四 A4 新增 testid(FE1:T4.1 媒体池 / T4.2 时间线全工具 / T4.3 历史面板)

### 静态骨架(index.html)
| testid | 元素 | 说明 |
|---|---|---|
| `tab-history` / `panel-history` | button/div | 新页签「历史」(T4.3;点开即 refresh) |

### 素材面板(T4.1;js/panels/media-panel.js + media-card.js)
| testid | 元素 | 说明 |
|---|---|---|
| `media-filters` / `.chip` | div/button | 类型过滤组(全部/视频/音频/图片/最近);`.chip.on` + aria-pressed |
| `media-search` | input | 名称搜索(会话态) |
| `media-item` | div(保留) | 缩略卡:dataset.path/kind;dblclick 插入;内部缩略位 `thumb`(懒加载) |
| `media-proxy` | button(视频卡内) | 代理状态/生成(media_proxy);`.ready` = 已就绪 |
| `set-bgm` | button(音频卡内,保留) | 一键设 BGM |

### 时间线(T4.2;js/render/track-head.js + clip-gestures.js)
| testid | 元素 | 说明 |
|---|---|---|
| `lane-label-<id>`(保留) | span | 轨头容器:含下方功能钮 |
| `track-name-<id>` | span | 轨名(双击重命名 → 内联 input.lane-rename) |
| `.lane-lock` / `.lane-mute` / `.lane-solo` | button | 锁定/静音/独奏(mute/solo 仅音频轨;aria-pressed;track_update 七字段) |
| `track-visibility-<id>`(保留) | button | 眼睛(ephemeral 视图隐藏,语义不变) |
| `track-lane-<id>`(保留) | div | 新增状态类:`.lane-locked`(拒绝编辑)/`.lane-muted`(波形灰化)/`.lane-soloed` |
| `.lane-grip` | span | 轨道高度拖拽把手(松手一笔 track_update.heightPx,[28,160]px) |
| `trim-mode` | span(toolbar) | 裁剪模式徽标(V 键;T4.7 起 T 让位文本工具;hidden 切换;wrap 挂 .trim-mode) |
| `clip`(保留) | div.clip | 新增:`.overlay` 画中画视觉(虚线边 + `.clip-pip` 角标);T 模式 wrap.classList=`trim-mode` |

### 导出面板(T4.1 代理)
| testid | 元素 | 说明 |
|---|---|---|
| `export-proxy` | input[checkbox] | 「用代理预览(缺省原片导出)」;ffmpeg 后端禁用(useProxy 仅 cutforge 内核) |

### 历史面板(T4.3;js/panels/history.js)
| testid | 元素 | 说明 |
|---|---|---|
| `history-head` | div | 摘要行:当前 rev / 撤销栈深 / 载入笔数 |
| `history-truncated` | div | 截断提示(count > limit 时;更早历史不参与回跳) |
| `history-rows` / `history-row` | div | 行:`data-opId`/`data-rev`/`data-state`(live\|undone\|record);`.current` = 当前指针;`.undone` = 已撤销(点击重做);`.record` = 撤销/重做记录行(不可回跳,中性呈现) |
| `history-mark` | div(role=separator) | 快照标记分隔线(导出前/批量前自动 + 手动;ephemeral 不落盘) |

### 设置面板(T4.2 吸附)
| testid | 元素 | 说明 |
|---|---|---|
| `setting-snap-strength` | select | 吸附强度档:loose(仅帧)/standard(+边缘+播放头+节拍,8px)/strong(+标记,12px);主开关仍为顶栏「磁吸」 |


## 八、册四 A4-FE2 新增 testid(T4.4~T4.9 转场/特效/文本字幕/音频/画布变换)

### 静态骨架(index.html)
| testid | 元素 | 说明 |
|---|---|---|
| `tab-transitions` / `tab-fx` / `tab-subtitles` | button | 新页签「转场库 / 特效 / 字幕」 |
| `panel-transitions` / `panel-fx` / `panel-subtitles` | div | 三个次级页容器(#tab-transitions/#tab-fx/#tab-subtitles) |
| `text-add` | button(toolbar) | 「T 文本」按钮:播放头处 text_add,响应 clipId 自动选中 |

### 键位变更(T4.7)
`text.add` = T(播放头处添加文本);`mode.trim` 由 T 迁至 **V**(裁剪模式;无 e2e 依赖,注册表/帮助表同步)。

### 转场库(T4.5;js/panels/transitions.js)
| testid | 元素 | 说明 |
|---|---|---|
| `trans-status` / `trans-search` / `trans-fav-chip` | div/input/button | 目录加载态 / 搜索 / 只看收藏(localStorage) |
| `trans-grid` / `trans-card` / `trans-fav` | div/div/button | 分类分组网格(58 项,/assets/transitions 懒加载)/ 卡片(点击或拖到片段应用;`.applied`=当前片段已应用)/ 收藏星标 |
| `trans-empty` | div | 搜索/收藏空态 |
| `trans-dur-presets` / `trans-clear` | div/button(检查器·转场组) | 时长预设 250/500/1000(草稿预填)/ 关闭转场(type=none) |
| `insp-transition-clamp` | div(检查器·转场组) | 钳制提示:durMs > 片段时长时显示「后端将钳制」(不阻塞) |
| `field-transition-type/-durMs/-fx` | select/slider/input | 转场子字段(durMs 升滑杆+预设) |

### 特效库与特效栈(T4.6;js/panels/fxlib.js)
| testid | 元素 | 说明 |
|---|---|---|
| `fx-status` / `fx-grid` / `fx-card` | div/div/div | 目录态 / 分类网格(11 项)/ 卡片(点击挂载到选中片段,≤3 前端拦截) |
| `fx-stack` / `fx-stack-item` / `fx-stack-empty` | div(检查器·特效组) | 已挂 combo 栈:调参(schema 滑杆,松手一笔 Op)/↑↓排序/✕移除 / 空态 |
| `fx-param-<参数名>` | input | fx 参数控件(越界前端钳制+提示) |

### 曲线变速(T4.4;js/panels/curve.js,检查器·变速组附加面)
| testid | 元素 | 说明 |
|---|---|---|
| `curve-editor` / `curve-plot` / `curve-rows` | div/canvas/div | 编辑器宿主 / 折线小图(token 取色)/ 点表 |
| `curve-add` / `curve-apply` / `curve-reset` | button | 加点 / 应用曲线(整组替换单 Op)/ 重置匀速(单点曲线≡常速) |
| `speed-presets` | div(检查器·变速组) | 0.5/1/2/4x 草稿预填(「应用」提交) |
| `field-speed` / `field-reverse` / `field-crop-x…h` / `field-rotation` / `field-flip` | 控件 | 画面/变速人话控件(滑杆/开关/整对象 crop) |

### 文本与字幕(T4.7;js/panels/textool.js + subtitles.js + insp-groups.js)
| testid | 元素 | 说明 |
|---|---|---|
| `huazi-picker` / `huazi-card` / `huazi-apply` / `huazi-clear` | div(检查器·文本组)/button | 花字库(12 模板 CSS 近似预览,标注「以渲染为准」)/ 应用(patch.huazi 整对象)/ 移除(空模板诚实降级;真清除候 BE) |
| `field-textStyle-*` | 控件 | 14 样式字段人话控件(fontFamily/fontSize/color/outlineColor/outlineWidth/borderStyle/backColor/shadow/align 九宫格/lineSpacing/opacity/x/y/karaoke;textStyle 整对象替换) |
| `sub-count` / `sub-rows` / `sub-row` / `sub-text` | div(字幕页签) | 计数 / 列表(文本轨片段投影逐条)/ 行(点击选中)/ 文本(双击内联编辑 → subtitle_set) |
| `sub-start` / `sub-dur` / `sub-karaoke` | input/button(行内) | 时间微调(change 即 subtitle_retime)/ 卡拉OK开关(textStyle.karaoke) |
| `sub-find` / `sub-replace` / `sub-track` / `sub-replace-run` | 输入/下拉/button | 批量替换(subtitle_replace;find 字面量) |
| `sub-import-src` / `sub-import-run` / `sub-export-format` / `sub-export-out` / `sub-export-run` | — | SRT/ASS 导入(路径手输,浏览器无 fs 诚实标注)/ 导出(返回工程内相对路径) |
| `sub-card-name` / `sub-card-save` / `sub-cards` / `sub-card` | — | 样式卡(textStyle 存 localStorage;套用选中/全部;全部=逐笔 Op 明示) |

### 音频工具(T4.8;js/panels/bgm.js + insp-groups.js)
| testid | 元素 | 说明 |
|---|---|---|
| `bgm-beats` / `beats-clear` / `beats-sensitivity` | button(背景乐面板) | audio_beats 检测 / 清除会话节拍 / 灵敏度 0–1 |
| `beats-box` / `beats-summary` / `beats-list` / `beats-chip` / `beats-empty` | div/span | 节拍展示(BPM/拍数/置信度 + 启发式 onset-energy 诚实标注;前 16 拍时间) |
| `field-denoise` / `field-pitch` | select/slider(检查器·音频组) | 降噪四档人话下拉(off/low/mid/high)/ 变调 ±12 半音滑杆(0 居中) |
| `field-motion-aliasIn` / `field-motion-aliasOut` / `motion-alias-apply` / `motion-alias-advanced` | input/button/fieldset(检查器·动效组) | mo.* 直通别名(高级可折叠;motion_set inFx/outFx;只写无回读) |
| `field-motion-in/-inMs/-out/-outMs` | select/input | 动效下拉升级:/catalogs motion 真实渲染目录(19 项),legacy 枚举保留 |

### 画布与变换(T4.9;js/panels/wizard.js + render/preview-transform.js + preview.js)
| testid | 元素 | 说明 |
|---|---|---|
| `wiz-ratio`(升级)/ `wiz-cust` / `wiz-cust-w` / `wiz-cust-h` / `wiz-cust-hint` | select/div/input | 预设六档+自定义;自定义 64–7680 偶数前端拦截(画幅创建时定死,已建工程无修改工具 = 勘察诚实口径,候 BE) |
| `pv-canvas-info` | div(预览面板) | 当前画幅/帧率 + 「画幅在新建工程时设定」诚实标注 |
| `pv-transform-hud` | div(预览下方) | 选中片段变换摘要(position 只读标注;flip/crop 渲染链不呈现代理) |
| `pv-handle-scale` / `pv-handle-rot` | div(画布把手层) | 角柄缩放 / 顶柄旋转(松手各一笔 clip_update;仅 overlay/有变换片段出现) |
| `pv-text-ghost` | div | 文本拖位置 ghost(textStyle.x/y,PlayRes=画布像素经预览缩放比换算) |
| `pv-text-dragzone` | div(画布把手层) | 文本选中态全画布拖拽面(图层空置时 CSS :empty 断接,故文本态显式铺面;点选不拖不产 Op) |
| 片段右键「定格帧」/「取消定格」 | 菜单项 | freezeMs=1000 / 0(单 Op) |

## 六、册五 A5(T5.1 关键帧 UI / T5.2 调色 UI;E-FE1)

### 关键帧:秒表与位置行(core/kf-model.js + panels/kf-watch.js)
| testid | 元素 | 说明 |
|---|---|---|
| `kw-toggle-<prop>` | button(检查器可动画字段旁) | 秒表:开 = 播放头时刻打当前值;关 = 确认后整属性移除(aria-pressed;prop ∈ scale/rotation/opacity/volume/position-x/position-y;`.`→`-`) |
| `kw-position-row` / `kw-position-read` | div/span(检查器·画面组) | position.x/y 只读投影的专属秒表行 + 现值读数 |
| `confirm-dialog` / `confirm-ok` / `confirm-cancel` | 模态(复用) | 秒表关闭确认;末属性整组清空受阻时弹窗明示(minItems 1 通道候 BE) |

### 关键帧:双面板编辑器(panels/kf-editor.js + kf-curve.js)
| testid | 元素 | 说明 |
|---|---|---|
| `kw-editor` / `kw-prop-select` / `kw-add` / `kw-count` | div/select/button/span(检查器·画面组) | 属性选择(固定白名单 + fx 动态键)/ 播放头打点 / 帧数 |
| `kw-kf-list` / `kw-time-<n>` / `kw-value-<n>` / `kw-kf-del-<n>` | div/input/button | 关键帧列表(增删改;change 才提交) |
| `kw-interp` / `kw-interp-<interp>` / `kw-bezier-cp` | div/button/span | 插值预设(线性/缓入/缓出/缓入缓出/保持/贝塞尔;回弹不可表达——bezier y 钳 0..1 无过冲,诚实不设) |
| `kf-curve-canvas` | canvas | 贝塞尔画布:采样点云(keyframeSamples,壳零插值)+ 锚点拖拽 + bezier 双柄 + 播放头线;拖拽期直线示意(提交后以采样刷新) |

### 关键帧:时间线关键帧行(panels/kf-row.js;仅选中片段)
| testid | 元素 | 说明 |
|---|---|---|
| `kf-row` / `kw-row-add` | div/button(轨内) | 关键帧行(clip 下方)/ 播放头打点(属性 = 编辑器当前属性,ephemeral.kfProp) |
| `kw-diamond` | button | 菱形 per 关键帧(dataset.prop/timeMs;拖动移动邻居夹取 / 双击删除;悬停 title 给属性@时刻=值) |

### 调色面板(检查器·调色组;panels/grade.js + grade-wheel.js + kf-curve.js points 模式)
| testid | 元素 | 说明 |
|---|---|---|
| `grade-panel` / `grade-preview-hint` / `grade-precise` | div/div/button | 面板宿主 + 「预览代理不含调色」恒显提示 + 精确预览按钮(render_frame) |
| `grade-<field>` | slider(7 杆) | 色温/色调/曝光/对比/高光/阴影/饱和度(松手整对象提交) |
| `grade-wheels` / `grade-wheel-lift|-gamma|-gain` / `grade-wheel-<x>-reset` | div/canvas/button | 三色轮(角度=色相 半径=强度 → RGB 三值;拖拽零 Op 松手单 Op)/ 归零 |
| `grade-advanced` / `grade-curves` / `grade-curve-channel` / `grade-curve-canvas` / `grade-curve-clear` / `grade-curve-hint` | fieldset/div/select/canvas/button/span | 二级折叠:曲线点集(master/red/green/blue;每通道 ≥2 点才提交——minItems 2 实测契约,1 点挂草稿明示) |
| `grade-lut` / `grade-lut-src` / `grade-lut-import` / `grade-lut-select` / `grade-lut-clear` | div/input/button/select/button | LUT:lut_import 导入(工程内相对路径)/ 会话库下拉(导入记录 ∪ 工程 grade.lut 引用;media_browse 不列 .cube,诚实口径)/ 清除 |
| `grade-copy` / `grade-paste` / `grade-clear` / `grade-clipboard` | button | 调色会话剪贴板(ephemeral.gradeClipboard)/ 清除(patch.grade=null → grade_clear) |
| 片段右键「复制调色」/「粘贴调色」 | 菜单项 | 同上(会话态;粘贴 = clip_update patch.grade 单 Op) |

### 示波器(panels/scopes.js;开启才采样)
| testid | 元素 | 说明 |
|---|---|---|
| `scopes-toggle` | button(预览传输行) | 示波器开关(aria-pressed;关 = 摘订阅零采样) |
| `scopes-panel` / `scopes-sample` / `scopes-status` / `scopes-note` | div/button/span | 面板 / 采样当前帧(render_frame→scope_data)/ 状态 / 数据域标注(精确帧含调色;降级 = 源素材域未调色,如实标注) |
| `scope-wave` / `scope-vector` / `scope-hist` | canvas | 亮度波形(luma 0..255 min-max-avg)/ 矢量(UV 64×64 密度热图)/ RGB 直方图(64 桶;通道色 = token) |

### A/B 分屏对比(panels/compare.js;登记项落地)
| testid | 元素 | 说明 |
|---|---|---|
| `compare-toggle` | button(预览传输行) | A/B 对比开关(aria-pressed) |
| `compare-bar` / `compare-baseline` / `compare-refresh` / `compare-status` | div/button | 基准抓取(render_frame 快照,调色前抓 = 调色前参照)/ 当前帧刷新 / 状态 |
| `compare-overlay` / `compare-img-base` / `compare-img-cur` / `compare-divider` | div/img/div | 叠层(clip-path 分割;左基准右当前;恒显口径:render_frame 无「不带 grade」开关,零 Op 基准快照方案) |

## 七(续)、册五 A5-FE2 新增 testid(T5.3 混音台 / T5.4 复合·多机位 / T5.6 队列·编码)

### 静态骨架(index.html)
| testid | 元素 | 说明 |
|---|---|---|
| `tab-mixer` / `panel-mixer` | button/div | 新页签「混音台」(T5.3) |
| `tab-multicam` / `panel-multicam` | button/div | 新页签「多机位」(T5.4;含场景检测) |
| `tab-queue` / `panel-queue` | button/div | 新页签「渲染队列」(T5.6) |

### 混音台(panels/mixer.js)
| testid | 元素 | 说明 |
|---|---|---|
| `mixer-tracks` | div | 轨道条容器(video/audio 轨各一条) |
| `mix-strip-<轨id>` | div | 每轨条:头部徽标/M/S + 推子占位 + EQ/动态折叠 + 源响度 |
| `mix-mute-<id>` / `mix-solo-<id>` | button | 静音/独奏(track_update.mute/solo;aria-pressed) |
| `mix-fader-<id>` | input[range disabled] | 轨道音量推子占位(IR 无 track volume/pan,候 BE 登记;片段音量走检查器) |
| `mix-eq-<id>`(组)/ `mix-eq-add|apply|clear-<id>` | fieldset/button | EQ ≤8 段(整组替换单 Op);清除 = patch.eq null |
| `mix-eq-band` / `mix-eq-type|freq|gain|q-<id>` | div/input | 段行(peaking/lowshelf/highshelf;20..20000/-24..24/0.1..16) |
| `mix-dyn-<id>`(组)/ `mix-dyn-apply|clear-<id>` / `mix-dyn-<字段>-<id>` | fieldset/button/input | 动态 acompressor+alimiter 五字段(整对象替换单 Op) |
| `mix-loud-<id>` / `mix-loud-out-<id>` | button/span | 测源响度(audio_loudness 对该轨最后片段源;「源域,未含轨道 EQ/动态」标注) |
| `mix-bus` / `mix-target` / `mix-bus-src` / `mix-bus-latest` / `mix-bus-measure` | fieldset/input/button | 总线:响度目标 I[:TP](与导出面板双入口,写 prefs)/ 成片路径(render_probe 预填)/ 测量 |
| `mix-bus-out` / `mix-bus-meter` / `mix-bus-nums` / `mix-bus-dev` | div/span | 响度单:LUFS/TP/LRA + 偏差徽标(≤1LU 达标)+ 静态电平条(实时候播放链采样口,登记) |

### 复合片段(T5.4;panels/compound.js + menu.js + timeline-view.js)
| testid/锚点 | 元素 | 说明 |
|---|---|---|
| `clip-compound` | span(clip 内) | 「复合」徽标(title 给子片段数/总时长;`.clip.compound` 紫描边) |
| 片段右键「打包为复合片段」 | 菜单项 | compound_create(框选 ≥2 同轨相邻视频片段;前置校验失败禁用 + why) |
| 片段右键「解包复合片段」/「复合片段说明」 | 菜单项 | compound_unbind(单 Op)/ 说明卡入口 |
| `compound-card` / `cpd-summary` / `cpd-hint` / `cpd-unbind` / `cpd-close` | div | 双击复合片段弹说明卡:投影概要(clipCount/durationMs/canvas)+ 解包引导(内部编辑=解包流,诚实标注) |

### 多机位 + 场景检测(T5.4;panels/multicam.js + scenetool.js)
| testid | 元素 | 说明 |
|---|---|---|
| 素材右键「加入/移出多机位同步集」 | 菜单项 | 会话集维护(video/audio;angles[0]=基准) |
| `mc-angle-src` / `mc-angle-add` / `mc-angle-clear` / `mc-angles` / `mc-angle` | input/button/div/span | 手输加入 / 同步集 chips([0]=基准徽标) |
| `mc-sync` / `mc-window` / `mc-sync-out` / `mc-sync-summary` / `mc-sync-angles` / `mc-sync-angle` | button/input/div | multicam_sync:最小置信度 + 每角(src/offsetMs/confidence)+ engine=pcm-xcorr 如实标注 |
| `mc-switch-add` / `mc-switch-angle` / `mc-switches` / `mc-switch` | button/select/div/span | 播放头逐点打切换点(会话态;生成时以首点归一) |
| `mc-cut` / `mc-track` / `mc-duration` | button/select/input | multicam_cut 单 Op 生成序列(angles offsets 取最近同步结果) |
| `scene-tool` / `scene-src` / `scene-run` / `scene-sens` / `scene-auto` / `scene-track` | fieldset/input/button | scene_detect(frame-diff;勾选 autoSplit = 单 Op 自动切段) |
| `scene-out` / `scene-summary` / `scene-cuts` / `scene-cut` | div/span | 检测点列表(tMs/置信度;degraded 如实标注) |

### 导出面板升级(T5.6/T5.5;panels/export.js)
| testid | 元素 | 说明 |
|---|---|---|
| `export-encode`(组)/ `export-encoder` / `export-quality` | fieldset/select | 编码器(缺省 remux 零代损/auto/hw/sw)+ 质量预设三档 |
| `export-probe` / `export-probe-out` | button/span | encode_probe(本机 hw 可用名;如 h264_amf) |
| `export-crf` / `export-bitrate` / `export-gop` / `export-pixfmt` | input | 高级折叠(crf 0..51 覆盖预设/码率 kbps/GOP/像素格式) |
| `export-loudnorm` / `export-verbose` | input | 响度目标 I[:TP](混音台双入口,prefs 同源)/ 命令回显开关(缺省关,安全口径) |
| `export-otio` / `export-otio-format` / `export-otio-run` / `export-otio-import` | div/select/button | otio_export(otio|edl,落 06_成片输出)/ otio_import 对话框入口 |
| `otio-import-dialog` / `otio-src` / `otio-name` / `otio-run` / `otio-cancel` / `otio-progress` | div/input/button | 导入新工程(目标目录必须不存在;src 预填最近导出产物绝对拼接) |

### 渲染队列(T5.6;panels/queue.js)
| testid | 元素 | 说明 |
|---|---|---|
| `queue-head` / `queue-refresh` / `queue-count` | div/button/span | 头部:刷新 + 任务数/渲染中/并发上限 |
| `queue-rows` / `queue-row` / `queue-state` / `queue-runid` / `queue-empty` | div/span | 行:data-runId/data-state;状态徽标(queued/running/paused/ok/fail/canceled,色 + 文本双通道) |
| `queue-pause` / `queue-resume` / `queue-cancel` / `queue-retry` | button(行内) | 五 action 按态给/不给(禁用态 title 说明状态机;render.progress 事件联动刷新) |

## 九、册六 A6-F3 新增 testid(T6.1 工程库 / T6.3 导出矩阵 / T6.2 素材库)

### 静态骨架(index.html)
| testid | 元素 | 说明 |
|---|---|---|
| `project-menu` | button(topbar) | 「工程 ▾」菜单:新建向导 / 工程库 / 迁移到 v3(常驻入口;当前布局为只读信息项) |

### 工程库视图(panels/library.js + library-ops.js;openDialog #library-dialog)
| testid | 元素 | 说明 |
|---|---|---|
| `lib-root` / `lib-refresh` | input/button | 库根目录(缺省 = 当前工程旁,偏好记忆)/ library_list 重扫 |
| `lib-search` / `lib-archived` | input/button | 名称/slug 搜索(服务端 query)/ 并入归档区(aria-pressed) |
| `lib-new` / `lib-new-name` / `lib-new-canvas` / `lib-new-layout` / `lib-new-tracks` / `lib-new-run` / `lib-new-cancel` | — | 库内新建(library_manage new;布局 v2 缺省/v3;轨道模板三套) |
| `lib-grid` / `lib-card` / `lib-card-name` / `lib-thumb` / `lib-meta` / `lib-empty` | div | 卡片栅格;卡片 dataset.name/path/archived/locked;徽标:`lib-badge-current`(当前)/`lib-badge-archived`/`lib-badge-locked`/`lib-badge-invalid` |
| `lib-open` / `lib-rename` / `lib-copy` / `lib-archive`·`lib-unarchive` / `lib-delete` | button(卡内) | 七操作(锁定卡禁用移动类,title 给 why;无效卡禁用打开) |
| `lib-open-cmd` / `lib-open-copy` / `lib-open-close` | code/button | 打开=serve 命令一键复制(一个 serve 一个工程,诚实口径);数字键 1–9 开对应卡片 |
| `lib-rename-to`·`-run`·`-cancel` / `lib-copy-to`·`-run`·`-cancel` / `lib-delete-run`·`-cancel` | — | 重命名/复制/删除(确认框给 .trash 捞回提示)对话框 |
| `lib-recover-box` / `lib-recover-item` / `lib-recover-run` | div/div/button | 恢复清单(library_recover list→recover;pid 存活性/锁龄文本) |
| `banner-migrate` / `migrate-banner-run` / `migrate-banner-dismiss` | div/button(#banner 内) | v2/v1 工程迁移提示条(知道了 = 偏好记忆不再打扰) |
| `migrate-run` / `migrate-cancel` | button(#migrate-dialog) | migrate_layout to=v3 确认框(成功后页面重载) |
| `banner-recover` / `recover-banner-open` | div/button(#banner 内) | 启动静默扫描有可恢复项时的顶栏提示条 |

### 导出矩阵(panels/export-matrix.js;挂 #export 面板内 fieldset)
| testid | 元素 | 说明 |
|---|---|---|
| `export-matrix` | fieldset | 导出矩阵组(仅 cutforge 内核;ffmpeg 后端导出时 toast 如实提示被忽略) |
| `export-format` / `export-format-hint` | select/div | 七出口(mp4-h264 缺省/mp4-h265/mov/gif/m4a/mp3/png-seq;frame-png 走精确预览不入列)+ 按格式说明行(gif=12fps、m4a/mp3=仅音频、png-seq=序列帧) |
| `export-preset` / `export-quality-tier` / `export-bitrate-tier` | select | 画幅预设(缺省=工程画幅)/清晰度三档(短边)/码率三档(估算) |
| `export-in-ms` / `export-out-ms` / `export-range-fill` | input/button | 区域导出(留空=到片尾);回填 selectionStore 入出点(I/O 键),可手改 |
| `export-video-only` | input[checkbox] | 仅视频(音轨静音+BGM 摘除) |
| `export-preflight-run` / `export-preflight` / `export-pf-item` | button/fieldset/div | 手动跑检查;清单行 `data-warn`="0\|1"(色+属性双通道) |
| `export-pf-anyway` / `export-pf-cancel` | button | 自动门前的问题项裁决(仍要导出 / 先不导出;干净项自动放行) |
| `export-batch` / `export-batch-out` | button/span | export_all_variants 入队(ratios 缺省=工程 outputs);完成后切「渲染队列」页 |

### 素材面板双页签(panels/media-panel.js + media-lib.js)
| testid | 元素 | 说明 |
|---|---|---|
| `media-tabs` / `media-tab-project` / `media-tab-library` | div/button | 页签(工程素材=原面红线不动 / 素材库) |
| `media-import-src` / `media-import-run` / `media-import-pick` / `media-import-auto` | input/button | 绝对路径导入(media_import 拷贝入工程,不产 Op)/ 文件选择器(浏览器拿不到绝对路径时如实指引)/ 导入后自动插到播放头 |
| `media-lib-root` / `media-lib-refresh` | input/button | 素材库根(env CUTFORGE_MEDIA 缺省面壳读不到,首次手填偏好记忆) |
| `media-lib-kinds` / `media-lib-tag` / `media-lib-query` | div/input | kind chips / 标签过滤 / 名称搜索 |
| `media-lib-list` / `media-lib-item` / `media-lib-import` / `media-lib-tag-edit` / `media-lib-empty` | div/button | 条目行(ref=素材库相对引用)一键导入 / 标签整组替换(#media-lib-tags-dialog:tags-input/tags-run/tags-cancel) |

### 新建向导升级(wizard.js;旧锚点全保留)
| testid | 元素 | 说明 |
|---|---|---|
| `wiz-template` | select | 模板三套:竖屏单轨口播(9:16·V+A)/ 横屏双机位(16:9·V+V+A)/ 方形社媒(1:1·V+A+文本);选即预填画幅/帧率/轨道,手改即脱离 |
| `wiz-layout` | select | 布局 v2(缺省)/v3 扁平(project_new layout;ADR-0021 过渡期) |

