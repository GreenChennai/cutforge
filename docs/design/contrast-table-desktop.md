# 桌面壳对比度自查表(V2-W3-SHELLB / 审查报告 v2 §9.4-4 / A-08)

计算方法:WCAG 2.1 相对亮度(L = 0.2126R + 0.7152G + 0.0722B,线性化后)对比比率
(CR = (L亮+0.05)/(L暗+0.05))。判定基线:**正文 ≥ 4.5:1(WCAG AA),大字
(≥18.66px bold 或 24px)与图形/UI 部件 ≥ 3:1**。与 Web 壳
`docs/design/contrast-table.md` 同法同基线。

色值定义点唯一:`apps/desktop/src/ui/theme.rs`(原始层),数值与
`apps/web/css/tokens.css` 语义层逐值一致(跨壳同源,平价测试
`ui::theme::tests::tokens_css_parity` / `semantic_components_parity` = TC-DESK-THEME-001)。
本表为深色主题(ADR-0014)。

## 正文组合(基线 4.5:1)

| 前景 token | 背景 token | 组合 | 实测 | 判定 |
|---|---|---|---|---|
| `fg` #e9edf5 | `panel` #1c2130 | 主文本/面板 | 13.67 | AA |
| `fg` #e9edf5 | `bg` #11141c | 主文本/底色 | 15.69 | AA |
| `dim` #98a2b8 | `panel` #1c2130 | 次级文本/面板 | 6.26 | AA |
| `dim` #98a2b8 | `elevated` #232a3b | 次级文本/浮层(工具钮字) | 5.59 | AA |
| `dim` #98a2b8 | `line` #2b3244 | 次级文本/按钮 hover 面 | 4.99 | AA |
| `accent` #4da3ff | `panel` #1c2130 | 强调字/面板 | 6.11 | AA |
| `accent` #4da3ff | `bg` #11141c | 强调字/底色(品牌栏) | 7.01 | AA |
| `btn-fg` #dce6ff | `btn-bg` #232a3b | 按钮字/按钮底 | 11.47 | AA |
| `btn-fg` #dce6ff | `btn-hover` #2c3650 | 按钮字/hover | 9.60 | AA |
| `fg-bright` #eef4ff | `video-1` #38659f | clip 字/视频面 | 5.39 | AA |
| `clip-fg-audio` #eafff2 | `audio-1` #2f7d52 | clip 字/音频面 | 4.81 | AA |
| `clip-fg-text` #fff8e0 | `textk-1` #82691f | clip 字/文本面 | 4.95 | AA |
| `lane-fg` #a9b7d6 | `lane-label-bg` #202636 | 轨头字/轨头底 | 7.49 | AA |
| `danger` #ef7c74 | `badge-bg` #2b3244 | 危险徽标/徽标底 | 4.75 | AA |
| `bg`(深字)#11141c | `accent`(品牌色块)#4da3ff | hero/主按钮 深字/accent 底 | 7.01 | AA |
| `fg` #e9edf5 | `panel-deep`(输入底)#161a24 | 输入字/输入底 | 14.82 | AA |

## 大字/图形组合(基线 3:1)

| 前景 token | 背景 token | 组合 | 实测 | 判定 |
|---|---|---|---|---|
| `playhead` #ff5f56 | `ruler-bg` #131826 | 播放头/标尺底 | 5.92 | AA |
| `select` #ffd76a | `video-1` #38659f | 选中描边/视频面 | 4.30 | AA(图形) |
| `select` #ffd76a | `audio-1` #2f7d52 | 选中描边/音频面 | 3.63 | AA(图形) |
| `lane-fg-off` #66738f | `lane-label-bg` #202636 | 轨头关态图标/轨头底 | 3.17 | AA(图形;关态另有形态差,非仅颜色线索) |
| `ruler-text` #7a8aa8 | `ruler-bg` #131826 | 标尺刻度字/标尺底 | 5.08 | AA |
| `faint` #687591 | `panel` #1c2130 | 禁用图标/面板 | 3.47 | AA(图形) |
| `dim` #98a2b8 | `elevated` #232a3b | 工具行图标/按钮底 | 5.59 | AA |

## 与桌面壳实际用色的对应(抽样即用即查)

- 工具行/传输按钮:图标与文字同色(dim/fg on elevated,5.59/13+);激活态
  (吸附开)accent 底 + bg 深字(7.01);
- 轨头锁/静音徽标:激活 warn/danger 底 + bg 深字(同 hero 组合,7.01);
- 时长角标:MASK 遮罩(rgba(5,7,12,.62))上的 fg(等价 #11141c 深底,≥7);
- 播放头/选中/轨型色与 Web 完全同值——跨壳无需重学(§9.4 目标)。

守门:`ui::theme::tests::wcag_aa_contrast`(桌面高频组合 ≥4.5 / 图形 ≥3.0,
回归即红);本表为人工核对快照,色值变更须先过平价测试再更新本表。
