//! 桌面壳设计 token 体系(审查报告 v2 §9.4 + A-08;与 `apps/web/css/tokens.css`
//! **语义层逐值一致**——跨壳同源,用户跨壳零重学)。
//!
//! 三层结构(与 Web 同构):
//! - **原始层**:灰阶 GRAY_950..GRAY_100、色相档位(BLUE/RED/AMBER/GREEN/VIOLET)、
//!   Web 字面量档位(DANGER/MENU_BG/RULER_BG 等)。u32 为 `0xRRGGBBAA`;
//! - **语义层**:[`Semantic`](bg/panel/elevated/…/clip_image 20 槽) +
//!   瞬时叠加常量(MASK/SNAP_LINE 等);
//! - **组件层**:[`Components`](button/input/chrome/ruler/clip/lane/bubble/menu/
//!   toast/badge 12 槽)。
//!
//! **唯一定义点纪律(A-08)**:`apps/desktop/src` 内裸色值只允许出现在本文件
//! (门禁 `desktop-color-purity`/TC-GATE-003 豁免 `ui/`,其余文件一律引用
//! 语义/组件 token)。
//!
//! **sable 桥接(而非替换)**:[`color_tokens()`] 把语义层映射到 sable
//! `ColorTokens` 槽位,经 `sable::widgets::theme::inject` 注入全局——sable
//! widgets 组件与壳自绘面板同色,消除"两套主题并存"的分叉(A-08 根因)。
//! gpui-component(DockArea/tab chrome)固定 Dark 模式,见 main.rs。
//!
//! 数值对齐证据:`tests::tokens_css_parity`(TC-DESK-THEME-001,运行时解析
//! web tokens.css 逐值对拍);对比度全表:`docs/design/contrast-table-desktop.md`。

// 本目录为 token/图标/动效**基建层**:部分槽位与 helper 由第 4 波(逐面板
// 美化,报告 §9.6)按需消费;门禁 clippy -D warnings 下的 dead_code 豁免
// 仅限本目录(与裸色值/字形/散写 ms 的"唯一定义点"豁免同口径)。
#![allow(dead_code)]
use sable::gpui::{App, Hsla, rgba};
use sable::widgets::theme as sable_theme;
use sable::widgets::tokens::ColorTokens;

// ===========================================================================
// 一、原始层(仅此文件可写裸值;与 web tokens.css 原始层同源)
// ===========================================================================

/// 灰阶(类达芬奇蓝灰;950 最深 → 100 最浅;`--cf-gray-*`)。
pub const GRAY_950: u32 = 0x0B0D12FF;
pub const GRAY_900: u32 = 0x11141CFF;
pub const GRAY_850: u32 = 0x161A24FF;
pub const GRAY_800: u32 = 0x1C2130FF;
pub const GRAY_750: u32 = 0x232A3BFF;
pub const GRAY_700: u32 = 0x2B3244FF;
pub const GRAY_600: u32 = 0x39435CFF;
pub const GRAY_500: u32 = 0x4C5878FF;
pub const GRAY_400: u32 = 0x687591FF;
pub const GRAY_300: u32 = 0x98A2B8FF;
pub const GRAY_200: u32 = 0xC4CBDAFF;
pub const GRAY_100: u32 = 0xE9EDF5FF;

/// 色相档位(`--cf-blue-*` 等;强调/状态/轨型共用)。
pub const BLUE_500: u32 = 0x4DA3FFFF;
pub const BLUE_300: u32 = 0x9CC6FFFF;
pub const BLUE_700: u32 = 0x2C3650FF;
pub const RED_450: u32 = 0xFF8F85FF;
pub const RED_400: u32 = 0xFF8F8FFF;
pub const RED_600: u32 = 0xFF5F56FF;
pub const AMBER_400: u32 = 0xF0C674FF;
pub const AMBER_300: u32 = 0xFFD76AFF;
pub const GREEN_450: u32 = 0x8AD4A4FF;
pub const VIOLET_300: u32 = 0xC9A7FFFF;

/// Web 侧字面量档位(tokens.css 语义层直接写死的 hex,非 var 链派生)。
pub const DANGER: u32 = 0xEF7C74FF;
pub const DANGER_BG: u32 = 0x3A1D21FF;
pub const WARN_BG: u32 = 0x39311DFF;
pub const OK_BG: u32 = 0x16301FFF;
pub const FG_BRIGHT: u32 = 0xEEF4FFFF;
pub const CLIP_FG_AUDIO: u32 = 0xEAFFF2FF;
pub const CLIP_FG_TEXT: u32 = 0xFFF8E0FF;
pub const BTN_FG: u32 = 0xDCE6FFFF;
pub const INPUT_FG: u32 = 0xE6ECFFFF;
pub const CHROME_GRAD_A: u32 = 0x1A2030FF;
pub const CHROME_GRAD_B: u32 = 0x151A26FF;
pub const LANE_ALT_BG: u32 = 0x141826FF;
pub const LANE_LABEL_BG: u32 = 0x202636FF;
pub const LANE_FG: u32 = 0xA9B7D6FF;
pub const LANE_FG_OFF: u32 = 0x66738FFF;
pub const RULER_BG: u32 = 0x131826FF;
pub const RULER_MINOR: u32 = 0x262D3FFF;
pub const RULER_MAJOR: u32 = 0x3A4358FF;
pub const RULER_TEXT: u32 = 0x7A8AA8FF;
pub const MENU_BG: u32 = 0x202838FF;
pub const TIP_BG: u32 = 0x0E1118FF;
pub const TOAST_BG: u32 = 0x22303CFF;
pub const TOAST_ERR_BG: u32 = 0x3C2222FF;
pub const VIDEO_1: u32 = 0x38659FFF;
pub const VIDEO_2: u32 = 0x27497CFF;
pub const AUDIO_1: u32 = 0x2F7D52FF;
pub const AUDIO_2: u32 = 0x1F5C3CFF;
pub const TEXTK_1: u32 = 0x82691FFF;
pub const TEXTK_2: u32 = 0x5F4D16FF;

/// 瞬时叠加(半透明;`rgba(r,g,b,a)` → alpha 字节 = a×255 四舍五入)。
pub const MASK: u32 = 0x05070C9E;
pub const SNAP_LINE: u32 = 0x4DA3FFD9;
pub const WAVE_LINE: u32 = 0xFFFFFF59;
pub const SELECT_GLOW: u32 = 0xFFD76A66;
pub const MARQUEE_BG: u32 = 0x4DA3FF1A;
pub const GHOST: u32 = 0x4DA3FF59;
pub const GHOST_INVALID_BG: u32 = 0xEF7C7447;
pub const DOT_GLOW: u32 = 0x8AD4A480;
pub const CLIP_BORDER: u32 = 0xFFFFFF24;
pub const CLIP_INSET: u32 = 0xFFFFFF1A;
pub const CLIP_EDGE: u32 = 0xFFFFFF29;
pub const CLIP_EDGE_HOVER: u32 = 0xFFFFFF61;

/// 原始层 → [`Hsla`]。
pub fn h(hex: u32) -> Hsla {
    rgba(hex).into()
}

// ===========================================================================
// 二、语义层(槽位与 web 语义层同名同值;`--cf-*` 注释即对拍目标)
// ===========================================================================

/// 语义层(§9.4 定义的 20 槽;每字段注释标注对拍的 web token)。
#[derive(Clone, Copy, Debug)]
pub struct Semantic {
    /// `--cf-bg`(全局底色)
    pub bg: Hsla,
    /// `--cf-panel`(面板底)
    pub panel: Hsla,
    /// `--cf-elevated`(浮层/hover 面)
    pub elevated: Hsla,
    /// `--cf-sunken`(凹陷面:舞台/气泡)
    pub sunken: Hsla,
    /// `--cf-line`(分隔线)
    pub line: Hsla,
    /// `--cf-line-strong`(强描边:输入/按钮)
    pub line_strong: Hsla,
    /// `--cf-fg`(主文本)
    pub fg: Hsla,
    /// `--cf-dim`(次级文本)
    pub fg_dim: Hsla,
    /// 禁用/占位文本(`--cf-gray-400`;web 无专名,取灰阶档)
    pub fg_faint: Hsla,
    /// `--cf-accent`(强调:选中/链接/吸附)
    pub accent: Hsla,
    /// `--cf-btn-hover`(强调 hover/按下;web 以蓝 700 承担)
    pub accent_hover: Hsla,
    /// `--cf-danger`(危险:徽标/非法落点)
    pub danger: Hsla,
    /// `--cf-warn-fg`(警告)
    pub warn: Hsla,
    /// `--cf-ok-fg`(成功)
    pub ok: Hsla,
    /// `--cf-playhead`(播放头;不可与选中混淆)
    pub playhead: Hsla,
    /// `--cf-select`(选中描边)
    pub select: Hsla,
    /// `--cf-video-1`(视频片段面)
    pub clip_video: Hsla,
    /// `--cf-audio-1`(音频片段面)
    pub clip_audio: Hsla,
    /// `--cf-textk-1`(字幕/文本片段面)
    pub clip_subtitle: Hsla,
    /// `--cf-badge-image`(图片片段;web 无 imagek 专名,与徽标同源)
    pub clip_image: Hsla,
}

/// 语义层取值(每次构造,零全局态;与 [`theme`] 同为渲染期廉价调用)。
pub fn semantic() -> Semantic {
    Semantic {
        bg: h(GRAY_900),
        panel: h(GRAY_800),
        elevated: h(GRAY_750),
        sunken: h(GRAY_950),
        line: h(GRAY_700),
        line_strong: h(GRAY_600),
        fg: h(GRAY_100),
        fg_dim: h(GRAY_300),
        fg_faint: h(GRAY_400),
        accent: h(BLUE_500),
        accent_hover: h(BLUE_700),
        danger: h(DANGER),
        warn: h(AMBER_400),
        ok: h(GREEN_450),
        playhead: h(RED_600),
        select: h(AMBER_300),
        clip_video: h(VIDEO_1),
        clip_audio: h(AUDIO_1),
        clip_subtitle: h(TEXTK_1),
        clip_image: h(VIOLET_300),
    }
}

// ===========================================================================
// 三、组件层(组件 css 一律引用组件 token——web `--cf-btn-*` 等)
// ===========================================================================

/// 组件层(§9.4 定义的 12 槽)。
#[derive(Clone, Copy, Debug)]
pub struct Components {
    /// `--cf-btn-bg`
    pub button_bg: Hsla,
    /// `--cf-btn-hover`
    pub button_hover: Hsla,
    /// `--cf-panel-deep`(输入/内嵌区底)
    pub input_bg: Hsla,
    /// 输入聚焦环(web = accent 一色承担)
    pub input_focus_ring: Hsla,
    /// 顶栏 chrome 底(`--cf-chrome-grad-b` 深端;单色承担渐变)
    pub chrome_bg: Hsla,
    /// `--cf-ruler-bg`
    pub ruler_bg: Hsla,
    /// clip 面(`--cf-video-1`;轨型分色见 [`Semantic`] 的 clip_*)
    pub clip_bg: Hsla,
    /// `--cf-lane-bg`
    pub lane_bg: Hsla,
    /// `--cf-bubble-bg`
    pub bubble_bg: Hsla,
    /// `--cf-menu-bg`
    pub menu_bg: Hsla,
    /// `--cf-toast-bg`
    pub toast_bg: Hsla,
    /// `--cf-badge-bg`
    pub badge_bg: Hsla,
}

/// 组件层取值。
pub fn components() -> Components {
    Components {
        button_bg: h(GRAY_750),
        button_hover: h(BLUE_700),
        input_bg: h(GRAY_850),
        input_focus_ring: h(BLUE_500),
        chrome_bg: h(CHROME_GRAD_B),
        ruler_bg: h(RULER_BG),
        clip_bg: h(VIDEO_1),
        lane_bg: h(GRAY_850),
        bubble_bg: h(GRAY_950),
        menu_bg: h(MENU_BG),
        toast_bg: h(TOAST_BG),
        badge_bg: h(GRAY_700),
    }
}

// ===========================================================================
// 四、sable 桥接(A-08:桥接而非替换)
// ===========================================================================

/// 语义层 → sable `ColorTokens` 槽位映射。
///
/// 对应关系(桌面语义 ↔ sable 槽):bg→surface_0,panel→surface_1,
/// elevated→surface_2(卡片/按钮底,web `--cf-btn-bg` 同值),line→surface_3
/// (hover 提亮一档),line_strong→surface_4(按下再亮一档),line→border_subtle,
/// line_strong→border_strong,fg/fg_dim/fg_faint→text_* 三级,
/// GHOST→accent_muted(选中背景,web 拖拽 ghost 同源 35% 蓝)。
pub fn color_tokens() -> ColorTokens {
    let s = semantic();
    ColorTokens {
        surface_0: s.bg,
        surface_1: s.panel,
        surface_2: s.elevated,
        surface_3: s.line,
        surface_4: s.line_strong,
        border_subtle: s.line,
        border_strong: s.line_strong,
        text_primary: s.fg,
        text_secondary: s.fg_dim,
        text_disabled: s.fg_faint,
        accent: s.accent,
        accent_muted: h(GHOST),
        danger: s.danger,
        warning: s.warn,
        success: s.ok,
    }
}

/// 注入 sable 全局主题(应用启动时调用一次,须在 `sable::dock::init` 之后)。
///
/// 此后 `sable::widgets::theme::theme(cx).colors` 即本文件语义层——sable
/// widgets 与壳自绘面板同色,单一真相落地。
pub fn inject(cx: &mut App) {
    sable_theme::inject(cx, color_tokens(), sable_theme::ThemeMode::Dark);
}

// ===========================================================================
// 测试:TC-DESK-THEME-001 跨壳平价 + WCAG AA 对比度
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析 web `tokens.css`:`--cf-x: value;` → 原始值表(var 链未解析)。
    /// 以 `;` 切声明(容一行多声明),再以首个 `:` 切名值;去块注释。
    fn parse_tokens_css() -> std::collections::HashMap<String, String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/css/tokens.css");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读 web tokens.css 失败({path:?}):{e}"));
        let mut map = std::collections::HashMap::new();
        let stripped = text.split("/*").map(|part| {
            // 简易块注释剥离:每段丢弃到 */ 之前的内容(首段无前置 /* 除外)
            match part.split_once("*/") {
                Some((_, rest)) => rest,
                None => part,
            }
        });
        let joined: String = stripped.collect::<Vec<_>>().join("");
        for decl in joined.split(';') {
            // 段内定位 --cf-(容忍 `:root {` 等前缀与首个声明同段)
            let Some(pos) = decl.find("--cf-") else {
                continue;
            };
            let rest = &decl[pos + "--cf-".len()..];
            let Some((name, value)) = rest.split_once(':') else {
                continue;
            };
            map.insert(name.trim().to_string(), value.trim().to_string());
        }
        map
    }

    /// 解析 token 值为 `0xRRGGBBAA`:#hex6/#hex8、rgba(r,g,b,a)、var(--cf-x)。
    fn resolve(map: &std::collections::HashMap<String, String>, name: &str) -> u32 {
        let raw = map
            .get(name)
            .unwrap_or_else(|| panic!("web tokens.css 缺 --cf-{name}"));
        let raw = raw.trim();
        if let Some(inner) = raw.strip_prefix("var(--cf-") {
            let inner = inner.trim_end_matches(')');
            return resolve(map, inner);
        }
        if let Some(hex) = raw.strip_prefix('#') {
            return match hex.len() {
                6 => u32::from_str_radix(hex, 16).expect("hex6") << 8 | 0xFF,
                8 => u32::from_str_radix(hex, 16).expect("hex8"),
                other => panic!("--cf-{name}: 不支持的 hex 位数 {other}"),
            };
        }
        if let Some(rest) = raw.strip_prefix("rgba(") {
            let rest = rest.trim_end_matches(')');
            let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
            let (r, g, b) = (
                parts[0].parse::<u32>().expect("rgba r"),
                parts[1].parse::<u32>().expect("rgba g"),
                parts[2].parse::<u32>().expect("rgba b"),
            );
            let a: f64 = parts[3].parse().expect("rgba a");
            let alpha = (a * 255.0 + 0.5).floor() as u32;
            return r << 24 | g << 16 | b << 8 | alpha;
        }
        panic!("--cf-{name}: 不支持的值格式 {raw:?}")
    }

    /// 断言桌面原始层常量 == web token 解析值(逐值平价证据)。
    fn assert_parity(map: &std::collections::HashMap<String, String>, pairs: &[(&str, u32)]) {
        for (css_name, desktop) in pairs {
            let web = resolve(map, css_name);
            assert_eq!(
                web, *desktop,
                "跨壳 token 漂移:--cf-{css_name} = #{web:08X} ≠ 桌面 #{desktop:08X}"
            );
        }
    }

    /// TC-DESK-THEME-001(§9.4/A-08):桌面原始层与 web tokens.css 逐值一致。
    ///
    /// 同一 token 概念在两壳必须同值:灰阶/色相档全档对拍,Web 字面量档
    /// (danger/menu/ruler/lane/toast 等)逐一对拍,瞬时叠加对拍。
    #[test]
    fn tokens_css_parity() {
        let map = parse_tokens_css();
        assert_parity(
            &map,
            &[
                ("gray-950", GRAY_950),
                ("gray-900", GRAY_900),
                ("gray-850", GRAY_850),
                ("gray-800", GRAY_800),
                ("gray-750", GRAY_750),
                ("gray-700", GRAY_700),
                ("gray-600", GRAY_600),
                ("gray-500", GRAY_500),
                ("gray-400", GRAY_400),
                ("gray-300", GRAY_300),
                ("gray-200", GRAY_200),
                ("gray-100", GRAY_100),
                ("blue-500", BLUE_500),
                ("blue-300", BLUE_300),
                ("blue-700", BLUE_700),
                ("red-450", RED_450),
                ("red-400", RED_400),
                ("red-600", RED_600),
                ("amber-400", AMBER_400),
                ("amber-300", AMBER_300),
                ("green-450", GREEN_450),
                ("violet-300", VIOLET_300),
                // Web 字面量档
                ("danger", DANGER),
                ("danger-bg", DANGER_BG),
                ("warn-bg", WARN_BG),
                ("ok-bg", OK_BG),
                ("fg-bright", FG_BRIGHT),
                ("clip-fg-audio", CLIP_FG_AUDIO),
                ("clip-fg-text", CLIP_FG_TEXT),
                ("btn-fg", BTN_FG),
                ("input-fg", INPUT_FG),
                ("chrome-grad-a", CHROME_GRAD_A),
                ("chrome-grad-b", CHROME_GRAD_B),
                ("lane-alt-bg", LANE_ALT_BG),
                ("lane-label-bg", LANE_LABEL_BG),
                ("lane-fg", LANE_FG),
                ("lane-fg-off", LANE_FG_OFF),
                ("ruler-bg", RULER_BG),
                ("ruler-minor", RULER_MINOR),
                ("ruler-major", RULER_MAJOR),
                ("ruler-text", RULER_TEXT),
                ("menu-bg", MENU_BG),
                ("tip-bg", TIP_BG),
                ("toast-bg", TOAST_BG),
                ("toast-err-bg", TOAST_ERR_BG),
                ("video-1", VIDEO_1),
                ("video-2", VIDEO_2),
                ("audio-1", AUDIO_1),
                ("audio-2", AUDIO_2),
                ("textk-1", TEXTK_1),
                ("textk-2", TEXTK_2),
                // 瞬时叠加
                ("mask", MASK),
                ("snap-line", SNAP_LINE),
                ("wave-line", WAVE_LINE),
                ("select-glow", SELECT_GLOW),
                ("marquee-bg", MARQUEE_BG),
                ("ghost", GHOST),
                ("ghost-invalid-bg", GHOST_INVALID_BG),
                ("dot-glow", DOT_GLOW),
                ("clip-border", CLIP_BORDER),
                ("clip-inset", CLIP_INSET),
                ("clip-edge", CLIP_EDGE),
                ("clip-edge-hover", CLIP_EDGE_HOVER),
            ],
        );
    }

    /// TC-DESK-THEME-001(续):语义/组件层槽位与 web 语义层逐值一致。
    #[test]
    fn semantic_components_parity() {
        let map = parse_tokens_css();
        let s = semantic();
        let c = components();
        let pairs = [
            ("bg", s.bg, "bg"),
            ("panel", s.panel, "panel"),
            ("elevated", s.elevated, "elevated"),
            ("sunken", s.sunken, "sunken"),
            ("line", s.line, "line"),
            ("line-strong", s.line_strong, "line_strong"),
            ("fg", s.fg, "fg"),
            ("dim", s.fg_dim, "fg_dim"),
            ("gray-400", s.fg_faint, "fg_faint(灰阶档)"),
            ("accent", s.accent, "accent"),
            ("btn-hover", s.accent_hover, "accent_hover"),
            ("danger", s.danger, "danger"),
            ("warn-fg", s.warn, "warn"),
            ("ok-fg", s.ok, "ok"),
            ("playhead", s.playhead, "playhead"),
            ("select", s.select, "select"),
            ("video-1", s.clip_video, "clip_video"),
            ("audio-1", s.clip_audio, "clip_audio"),
            ("textk-1", s.clip_subtitle, "clip_subtitle"),
            ("badge-image", s.clip_image, "clip_image"),
            // 组件层
            ("btn-bg", c.button_bg, "button_bg"),
            ("btn-hover", c.button_hover, "button_hover"),
            ("panel-deep", c.input_bg, "input_bg"),
            ("accent", c.input_focus_ring, "input_focus_ring"),
            ("chrome-grad-b", c.chrome_bg, "chrome_bg"),
            ("ruler-bg", c.ruler_bg, "ruler_bg"),
            ("video-1", c.clip_bg, "clip_bg"),
            ("lane-bg", c.lane_bg, "lane_bg"),
            ("bubble-bg", c.bubble_bg, "bubble_bg"),
            ("menu-bg", c.menu_bg, "menu_bg"),
            ("toast-bg", c.toast_bg, "toast_bg"),
            ("badge-bg", c.badge_bg, "badge_bg"),
        ];
        for (css_name, desktop_hsla, slot) in pairs {
            let web = resolve(&map, css_name);
            let desktop = rgba_of(web);
            assert_eq!(
                desktop_hsla, desktop,
                "语义/组件槽 {slot} 与 --cf-{css_name} 漂移"
            );
        }
    }

    /// `0xRRGGBBAA` → Hsla(与 gpui `rgba(..).into()` 同路)。
    fn rgba_of(hex: u32) -> Hsla {
        rgba(hex).into()
    }

    /// WCAG 2.1 相对亮度(sRGB 线性化;与 web contrast-table.md 同式)。
    fn luminance(hex: u32) -> f64 {
        fn channel(v: f64) -> f64 {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        let r = channel(((hex >> 24) & 0xFF) as f64 / 255.0);
        let g = channel(((hex >> 16) & 0xFF) as f64 / 255.0);
        let b = channel(((hex >> 8) & 0xFF) as f64 / 255.0);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// 对比比率(CR = (L亮+0.05)/(L暗+0.05))。
    fn contrast(a: u32, b: u32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// WCAG AA 门(§9.4-4):桌面实际用到的正文组合 ≥4.5:1,
    /// 大字/图形组合 ≥3:1(全表见 docs/design/contrast-table-desktop.md)。
    #[test]
    fn wcag_aa_contrast() {
        // 正文组合(≥4.5:1)
        let body = [
            ("主文本/面板", GRAY_100, GRAY_800),
            ("主文本/底色", GRAY_100, GRAY_900),
            ("次级文本/面板", GRAY_300, GRAY_800),
            ("次级文本/浮层", GRAY_300, GRAY_750),
            ("强调字/面板", BLUE_500, GRAY_800),
            ("按钮字/按钮底", BTN_FG, GRAY_750),
            ("按钮字/hover", BTN_FG, BLUE_700),
            ("clip 字/视频面", FG_BRIGHT, VIDEO_1),
            ("clip 字/音频面", CLIP_FG_AUDIO, AUDIO_1),
            ("clip 字/文本面", CLIP_FG_TEXT, TEXTK_1),
            ("轨头字/轨头底", LANE_FG, LANE_LABEL_BG),
            ("危险徽标/徽标底", DANGER, GRAY_700),
        ];
        for (name, fg, bg) in body {
            let cr = contrast(fg, bg);
            assert!(cr >= 4.5, "{name}: 对比度 {cr:.2} < 4.5(AA)");
        }
        // 大字/图形组合(≥3:1;与 web contrast-table「大字/图形」同基线)
        let graphics = [
            ("播放头/标尺底", RED_600, RULER_BG),
            ("选中描边/视频面", AMBER_300, VIDEO_1),
            ("选中描边/音频面", AMBER_300, AUDIO_1),
            ("轨头关态字/轨头底", LANE_FG_OFF, LANE_LABEL_BG),
            ("标尺字/标尺底", RULER_TEXT, RULER_BG),
            ("禁用字/面板", GRAY_400, GRAY_800),
        ];
        for (name, fg, bg) in graphics {
            let cr = contrast(fg, bg);
            assert!(cr >= 3.0, "{name}: 对比度 {cr:.2} < 3.0(AA 图形)");
        }
    }

    /// 品牌色块(hero/主按钮)上深色文字可读(桌面壳 surface_0 字 on accent)。
    #[test]
    fn accent_block_text_contrast() {
        // hero 品牌色块:深底字(#11141c)on accent(#4da3ff)
        let cr = contrast(GRAY_900, BLUE_500);
        assert!(cr >= 4.5, "accent 块深字:对比度 {cr:.2} < 4.5");
    }

    /// sable 桥映射非退化(每槽可解析且与语义层来源一致)。
    #[test]
    fn sable_bridge_mapping() {
        let t = color_tokens();
        let s = semantic();
        assert_eq!(t.surface_0, s.bg);
        assert_eq!(t.surface_1, s.panel);
        assert_eq!(t.surface_2, s.elevated);
        assert_eq!(t.accent, s.accent);
        assert_eq!(t.danger, s.danger);
        assert_eq!(t.success, s.ok);
        assert_eq!(t.accent_muted, h(GHOST));
    }
}
