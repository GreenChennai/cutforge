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
  旧壳本体迁至 `/assets/legacy/`)

## 五、自测入口(T2.2 重建铁律 / AC-2.5)

```js
// 控制台(e2e 亦可驱动):
const r = await window.__cutforgeSelfTest();
// r = { ok: boolean, before: string, after: string }
// 语义:清空四个投影派生面(project/timeline/media/uiFields+conflicts)→ 全量重投影 →
// 逐字段 diff;ok=true 即「store 可由内核投影完全重建」。
```

补充:`window.__cfClipboard`(Ctrl+C 复制的片段 id,会话态)。
