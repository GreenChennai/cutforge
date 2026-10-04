# V2-W3-SHELLB 桌面壳视觉基础层(§9.4/9.5/9.7 + A-08/A-09)

范围:apps/desktop/src/ui/**(新建 theme.rs/icon.rs/fx.rs)+ apps/desktop/assets/icons/** +
panels 字形/裸色值清理 + tools/gates 观察项转阻断(与 GATES 收口轮协调)。

## 条目(报告 v2 §9 专章)
1. **§9.4 + A-08 ui/theme.rs**:三层 token(原始/语义/组件),数值与 apps/web/css/tokens.css
   语义层逐值一致(石墨灰阶 #16181B/#1C1F23/#23272C + 暖橙 #E8833A + 轨型四色);
   桥接 sable ColorTokens;唯一裸值定义点;WCAG AA 对比校验表入 docs/design/;
   TC-DESK-THEME-001 跨壳平价;
2. **§9.5 + A-09 ui/icon.rs + assets/icons/**:复用 web assets/icons.js 的 SVG symbol 集
   扩到 NLE 全集(40~50 个,16/20 两档,currentColor),Icon 枚举 + svg() helper;
   清理全部文本字形(timeline.rs/preview.rs/library.rs,含"卑"乱码字形);
   TC-DESK-ICON-002 静态门禁(字形字面量清零);
3. **§9.7 ui/fx.rs**:FX_PRESS 80/FX_PANEL 160/FX_VIEW 240 + easing 三族 + spring;
   reduced-motion 设置页总控;纪律四条(状态变化才动效/拖拽零动画/transform-opacity/
   同屏入场≤3);TC-GATE-004 散写 ms 扫描(报告模式);
4. 交付口径:ui-taste-checklist 自评 ≥8/10;录屏留档(webm ≤8MB);
   门禁:纯度扫描器(TC-GATE-002/003)在本轮收口后由第 4 波转阻断。

## 纪律
环境同前;报告 v2 §9.3 视觉语言(精密仪器定位)为准绳;
SHELLA 刚落的 app/ 六模块与键位注册表**不得破坏**(消费,不改结构)。
状态:待第 2 波收口后开工
