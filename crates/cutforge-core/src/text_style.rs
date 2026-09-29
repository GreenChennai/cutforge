// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 文本样式 IR(册四 A4 T4.7,ADR-0016 路线的片段侧承载):
//!
//! - [`TextStyle`]:clip.textStyle 检查器文本组字段集——字体/字号/颜色/描边/
//!   底衬/阴影/对齐/行距/透明度/画布内位置 x,y/卡拉OK。位置为**画布像素坐标**
//!   (PlayRes = 画布,与 ASS `\pos` 逐一同域;百分比由壳换算后写入),缺省 =
//!   按 align 的边距网格(ASS 原生排版)。颜色统一 `#RRGGBB[AA]` 十六进制
//!   (schema pattern 界),ASS 生成端确定性转换为 `&HAABBGGRR`;
//! - [`Huazi`]:clip.huazi 花字挂载(schema 既有字段,此前模型不承接——
//!   "不加载 = 必丢"教训,本册补齐 roundtrip),template 引用
//!   schemas/huazi-catalog.json 的 `hz.<id>`,params 覆写模板默认;
//! - [`FontSpec`]:clip.font(CutFlow S7 兼容透传字段)承接,渲染消费以
//!   textStyle.fontFamily 优先(font 仅在无 textStyle 时兜底)。
//!
//! 全部结构 serde 形态即落盘形态;渲染端消费见 cutforge-render::textass(纯函数)。

use serde::{Deserialize, Serialize};

/// 文本样式(clip.textStyle;全字段可选,缺省由渲染端按 ASS 默认裁决——
/// 与全仓"缺省字段不臆造"纪律一致:旧文本片段无本字段时行为不变)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    /// 字号(PlayRes 像素;schema 界 8–500)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// 主色 `#RRGGBB` 或 `#RRGGBBAA`(AA 为不透明度,FF=不透明)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_color: Option<String>,
    /// 描边宽(schema 界 0–20;BorderStyle=3 时为底衬外边距)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline_width: Option<f64>,
    /// ASS BorderStyle:1=描边+阴影,3=底衬色块(schema enum)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_style: Option<u8>,
    /// 底衬色(BorderStyle=3 的底板 / 阴影色;`#RRGGBB[AA]`)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_color: Option<String>,
    /// 阴影深度(schema 界 0–20)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<f64>,
    /// 对齐(九宫格;schema enum:topLeft…bottomRight,缺省 bottomCenter)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    /// 行距(像素;0 = 字体默认)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_spacing: Option<f64>,
    /// 整体不透明度(schema 界 0–1;1=不透明)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// 画布内位置 x(PlayRes 像素;锚点由 align 决定,ASS \pos 语义;
    /// x/y 均缺省 = 按 align 边距网格排版)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// 卡拉OK(册四 T4.7):true 时按片段时长逐字均分生成 `\kf` 序列。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<bool>,
}

/// 花字挂载(clip.huazi;schema 既有字段,模型层首次承接)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Huazi {
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 片段级字体覆盖(clip.font;CutFlow S7 兼容字段,模型层首次承接防丢)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    pub family: String,
}

impl TextStyle {
    /// align 字面量 → ASS Alignment 数字(九宫格;缺省 bottomCenter = 2)。
    pub fn align_num(&self) -> u8 {
        match self.align.as_deref() {
            Some("topLeft") => 7,
            Some("topCenter") => 8,
            Some("topRight") => 9,
            Some("middleLeft") => 4,
            Some("center") => 5,
            Some("middleRight") => 6,
            Some("bottomLeft") => 1,
            Some("bottomRight") => 3,
            _ => 2,
        }
    }
}

/// `#RRGGBB[AA]` → ASS `&HAABBGGRR&`(确定性;非法输入回落 default 并带 alpha FF)。
/// 颜色在 schema 层经 pattern 界定,此处防御性回落仅保证生成端纯函数永不 panic。
pub fn color_to_ass(input: &str, default: &str) -> String {
    let parse = |s: &str| -> Option<String> {
        let hex = s.strip_prefix('#')?;
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let (rgb, aa) = if hex.len() == 8 { (&hex[..6], &hex[6..]) } else { (hex, "FF") };
        let rr = &rgb[0..2];
        let gg = &rgb[2..4];
        let bb = &rgb[4..6];
        // IR 语义:AA = 不透明度(CSS 风,FF=不透明)→ ASS alpha 取反(00=不透明);
        // 输出形态 &H{alpha}{blue}{green}{red}(BGR 序,无尾 &;确定性)。
        let ass_alpha = format!("{:02X}", 0xFF - u8::from_str_radix(aa, 16).unwrap_or(0xFF));
        let mut out = format!("&H{ass_alpha}{bb}{gg}{rr}").to_ascii_uppercase();
        out.truncate(10); // &H + 8 位十六进制(防极端输入拖尾)
        Some(out)
    };
    parse(input).unwrap_or_else(|| parse(default).unwrap_or_else(|| "&H00FFFFFF".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ts(v: serde_json::Value) -> TextStyle {
        serde_json::from_value(v).unwrap()
    }

    /// 册四 A4 T4.7:textStyle/huazi/font 三字段 roundtrip 读写不丢
    /// (model.rs 的逐字段 roundtrip 纪律;缺省不臆造)。
    #[test]
    fn text_style_roundtrip_no_loss() {
        let v = json!({
            "fontFamily": "思源黑体", "fontSize": 72, "color": "#FFCC00",
            "outlineColor": "#000000", "outlineWidth": 3, "borderStyle": 1,
            "backColor": "#00000080", "shadow": 2, "align": "topCenter",
            "lineSpacing": 8, "opacity": 0.9, "x": 540, "y": 240, "karaoke": true
        });
        let t = ts(v.clone());
        assert_eq!(t.font_family.as_deref(), Some("思源黑体"));
        assert_eq!(t.font_size, Some(72.0));
        assert_eq!(t.align_num(), 8);
        let back = serde_json::to_value(&t).unwrap();
        assert_eq!(back["fontFamily"], json!("思源黑体"));
        assert_eq!(back["karaoke"], json!(true));
        assert!(back.get("bold").is_none(), "缺省字段不得臆造");
        let t2: TextStyle = serde_json::from_value(back).unwrap();
        assert_eq!(t, t2, "serde 往返语义相等");
        // 空对象合法(全缺省,渲染端按默认裁决)
        let empty = ts(json!({}));
        assert_eq!(empty.align_num(), 2, "缺省 bottomCenter");
        assert_eq!(empty, TextStyle::default());
    }

    #[test]
    fn align_nine_grid_maps_to_ass_numbers() {
        for (s, n) in [
            ("topLeft", 7), ("topCenter", 8), ("topRight", 9),
            ("middleLeft", 4), ("center", 5), ("middleRight", 6),
            ("bottomLeft", 1), ("bottomCenter", 2), ("bottomRight", 3),
        ] {
            assert_eq!(ts(json!({"align": s})).align_num(), n, "{s}");
        }
        // 未知值/缺省 → bottomCenter(渲染端诚实回落,schema 层 enum 先拒)
        assert_eq!(ts(json!({})).align_num(), 2);
    }

    #[test]
    fn color_conversion_is_deterministic_ass_bgr() {
        assert_eq!(color_to_ass("#FF0000", "#FFFFFF"), "&H000000FF", "RRGGBB→BGR: 红 #FF0000(alpha 00 不透明)");
        assert_eq!(color_to_ass("#00FF00", "#FFFFFF"), "&H0000FF00");
        assert_eq!(color_to_ass("#0000FF", "#FFFFFF"), "&H00FF0000");
        // 8 位带 alpha:AA = 不透明度(CSS 风)→ ASS alpha 取反:80 → 7F
        assert_eq!(color_to_ass("#00000080", "#FFFFFF"), "&H7F000000");
        assert_eq!(color_to_ass("#000000FF", "#FFFFFF"), "&H00000000", "FF 不透明 → ASS 00");
        assert_eq!(color_to_ass("#00000000", "#FFFFFF"), "&HFF000000", "00 全透 → ASS FF");
        // 大小写归一为大写(确定性)
        assert_eq!(color_to_ass("#ff8800", "#FFFFFF"), "&H000088FF");
        // 非法回落 default(防御;schema pattern 先拒)
        assert_eq!(color_to_ass("红色", "#123456"), "&H00563412");
        assert_eq!(color_to_ass("#XYZ", "#FFFFFF"), "&H00FFFFFF");
    }

    /// huazi/font 字段 serde 形态与 schema 逐字对齐(template 必填,params 可选)。
    #[test]
    fn huazi_and_font_spec_serde() {
        let h: Huazi = serde_json::from_value(json!({
            "template": "hz.pop", "params": {"stepMs": 90}
        })).unwrap();
        assert_eq!(h.template, "hz.pop");
        assert_eq!(h.params.as_ref().unwrap()["stepMs"], json!(90));
        let f: FontSpec = serde_json::from_value(json!({"family": "Arial"})).unwrap();
        assert_eq!(f.family, "Arial");
        let back = serde_json::to_value(&h).unwrap();
        assert_eq!(back["template"], json!("hz.pop"));
    }

    /// 册四 A4 T4.7/T4.8:clip 五新字段(textStyle/huazi/font/denoise/pitch)在
    /// Clip 级 roundtrip 读写不丢;旧工程(无这些字段)不臆造落盘。
    #[test]
    fn clip_text_audio_fields_roundtrip_no_loss() {
        let base = json!({"id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "你好"});
        let c: crate::model::Clip = serde_json::from_value(base.clone()).unwrap();
        assert!(c.text_style.is_none() && c.huazi.is_none() && c.font.is_none()
            && c.denoise.is_none() && c.pitch.is_none());
        let back = serde_json::to_value(&c).unwrap();
        assert!(back.get("textStyle").is_none() && back.get("denoise").is_none(), "缺省不得臆造");
        let full = json!({
            "id": "T1-001", "startMs": 0, "durationMs": 2000, "text": "你好",
            "textStyle": {"fontFamily": "黑体", "fontSize": 64, "color": "#FFFFFF",
                          "outlineWidth": 2, "align": "bottomCenter", "x": 540, "y": 1700},
            "huazi": {"template": "hz.pop"},
            "font": {"family": "黑体"},
            "denoise": "mid", "pitch": -4
        });
        let c: crate::model::Clip = serde_json::from_value(full).unwrap();
        assert_eq!(c.text_style.as_ref().unwrap().font_size, Some(64.0));
        assert_eq!(c.huazi.as_ref().unwrap().template, "hz.pop");
        assert_eq!(c.font.as_ref().unwrap().family, "黑体");
        assert_eq!(c.denoise.as_deref(), Some("mid"));
        assert_eq!(c.pitch, Some(-4.0));
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["textStyle"]["x"], json!(540.0));
        assert_eq!(back["huazi"]["template"], json!("hz.pop"));
        assert_eq!(back["denoise"], json!("mid"));
        assert_eq!(back["pitch"], json!(-4.0));
        let c2: crate::model::Clip = serde_json::from_value(back).unwrap();
        assert_eq!(c, c2, "serde 往返语义相等");
    }
}
