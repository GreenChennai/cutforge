# 对比度自查表(册三 T3.1 / AC-3.1 / ADR-0014)

计算方法:WCAG 2.1 相对亮度(L = 0.2126R + 0.7152G + 0.0722B,线性化后)对比比率
(CR = (L亮+0.05)/(L暗+0.05))。全部前景/背景组合以脚本逐对计算(计算脚本口径见下),
判定基线:**正文 ≥ 4.5:1(WCAG AA),大字(≥18.66px bold 或 24px)与图形/UI 部件 ≥ 3:1**。

色值定义点唯一:`apps/web/css/tokens.css`(原始层)。本表为深色主题(ADR-0014,本册仅深色)。

## 正文组合(基线 4.5:1)

| 前景 token | 背景 token | 组合 | 实测 | 判定 |
|---|---|---|---|---|
| `--cf-fg` #e9edf5 | `--cf-panel` #1c2130 | 主文本/面板 | 13.67 | AA |
| `--cf-fg` #e9edf5 | `--cf-bg` #11141c | 主文本/底色 | 15.69 | AA |
| `--cf-dim` #98a2b8 | `--cf-panel` #1c2130 | 次级文本/面板 | 6.26 | AA |
| `--cf-dim` #98a2b8 | `--cf-bg` #11141c | 次级文本/底色 | 7.18 | AA |
| `--cf-dim` #98a2b8 | `--cf-elevated` #232a3b | 次级文本/浮层 | 5.59 | AA |
| `--cf-accent` #4da3ff | `--cf-panel` #1c2130 | 强调字/面板 | 6.11 | AA |
| `--cf-accent` #4da3ff | `--cf-bg` #11141c | 强调字/底色 | 7.01 | AA |
| `--cf-accent-soft` #9cc6ff | `--cf-panel` #1c2130 | 面板标题/面板 | 9.11 | AA |
| `--cf-btn-fg` #dce6ff | `--cf-btn-bg` #232a3b | 按钮字/按钮底 | 11.47 | AA |
| `--cf-btn-fg` #dce6ff | `--cf-btn-hover` #2c3650 | 按钮字/hover | 9.60 | AA |
| `--cf-danger-fg` #ff8f8f | `--cf-danger-bg` #3a1d21 | 危险横幅字/底 | 6.95 | AA |
| `--cf-warn-fg` #f0c674 | `--cf-warn-bg` #39311d | 警告横幅字/底 | 7.99 | AA |
| `--cf-ok-fg` #8ad4a4 | `--cf-ok-bg` #16301f | 成功字/底 | 8.15 | AA |
| `--cf-lane-fg` #a9b7d6 | `--cf-lane-label-bg` #202636 | 轨头字/轨头底 | 7.49 | AA |
| `--cf-fg-bright` #eef4ff | `--cf-video-1` #38659f | clip 字/视频面 | 5.39 | AA |
| `--cf-clip-fg-audio` #eafff2 | `--cf-audio-1` #2f7d52 | clip 字/音频面 | 4.81 | AA |
| `--cf-clip-fg-text` #fff8e0 | `--cf-textk-1` #82691f | clip 字/文本面 | 4.95 | AA |
| `--cf-bubble-fg` #c4cbda | `--cf-bubble-bg` #0d1017 | 气泡字/气泡底 | 11.69 | AA |
| `--cf-danger` #ef7c74 | `--cf-badge-bg` #2b3244 | 危险徽标/徽标底 | 4.75 | AA |

## 大字/图形组合(基线 3:1)

| 前景 token | 背景 token | 组合 | 实测 | 判定 |
|---|---|---|---|---|
| `--cf-playhead` #ff5f56 | `--cf-ruler-bg` #131826 | 播放头/标尺底 | 5.92 | AA(图形) |
| `--cf-select` #ffd76a | `--cf-video-1` #38659f | 选中描边/视频面 | 4.30 | AA(图形) |
| `--cf-select` #ffd76a | `--cf-audio-1` #2f7d52 | 选中描边/音频面 | 3.63 | AA(图形) |
| `--cf-lane-fg-off` #66738f | `--cf-lane-label-bg` #202636 | 轨头关态图标/轨头底 | 3.17 | AA(图形;且有关态文字 aria-pressed 非颜色线索) |

## 计算脚本(复核用)

```js
const L = (hex) => {
  const c = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map(i => parseInt(c.slice(i, i + 2), 16) / 255)
    .map(v => v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a, b) => {
  const [l1, l2] = [L(a), L(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
};
```

## 维护约定

- 新增/调整 token 必须重跑上表组合并追加行;低于基线的组合不得上线(R5 门禁管色值来源,
  本表管可读性,两道闸共同构成 AC-3.1);
- 已知余量最小项:`clip 字/音频面 4.81`——若未来音频轨面再提亮,优先压暗 `--cf-audio-1` 而非提字色;
- 深色假设:上表全部按深色面计算;浅色主题(册七可选)须整表重算并重开 ADR。
