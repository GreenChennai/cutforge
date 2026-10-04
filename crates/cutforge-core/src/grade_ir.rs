// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 调色与轨道处理 IR(册五 T5.2/T5.3;自 model.rs 纯移动——行数红线 A1-3,
//! `model.rs` 经 `pub use` 保持 `crate::model::Grade` 等既有路径逐字不变):
//! - [`Grade`]/[`GradeCurves`]/[`GradeHsl`]:clip.grade 调色面(一级校色/曲线/LUT 引用/
//!   HSL 限定器登记降级);渲染映射与链序见 cutforge-render::grade 模块注释;
//! - [`EqBand`]/[`TrackDyn`]:track.eq / track.dyn 轨道处理面(多段 EQ + 压缩/限幅)。

use serde::{Deserialize, Serialize};

fn one_f() -> f64 {
    1.0
}

/// 曲线点集(册五 T5.2):[x,y] 归一域 0..1,升序;渲染端编译为 ffmpeg
/// `curves` 滤镜 `0/0 0.5/0.8 1/1` 点串(ADR-0020 bt709 域)。
pub type CurvePoint = [f64; 2];

/// 片段调色(册五 T5.2;clip.grade,整对象替换)。各字段 None = 不调整;
/// 值域界在 schema 层,模型从宽收数。渲染映射(链内序)见 render::grade。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grade {
    /// 色温 -100..100(暖+):colorbalance 红升蓝降。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// 色调 -100..100(品红+):colorbalance 绿反向。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<f64>,
    /// 曝光 -3..3 档:eq brightness。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f64>,
    /// 对比 -100..100:eq contrast。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f64>,
    /// 高光 -100..100:colorbalance highlights 三通道同调。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highlights: Option<f64>,
    /// 阴影 -100..100:colorbalance shadows 三通道同调。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadows: Option<f64>,
    /// 饱和度 0..3(1=不变):eq saturation。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saturation: Option<f64>,
    /// Lift 每轮 RGB -1..1(0=不变):colorbalance shadows 分通道。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lift: Option<[f64; 3]>,
    /// Gamma 每轮 RGB 0.2..5(1=不变):eq gamma_r/g/b。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gamma: Option<[f64; 3]>,
    /// Gain 每轮 RGB 0..4(1=不变):colorchannelmixer 对角增益。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<[f64; 3]>,
    /// RGB/亮度曲线:点集编译 curves(master=亮度近似,RGB 同步)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curves: Option<GradeCurves>,
    /// HSL 限定器(登记降级:ffmpeg 简单滤镜不达达芬奇级选色,渲染端 WARN 留痕,
    /// 见 render::grade 模块注释;IR 先行承载防丢,同 A4-L1 fx.in/out 槽位口径)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hsl: Option<GradeHsl>,
    /// LUT 引用:.cube 相对路径(工程根;经 lut_import 拷入 .cutforge/luts/ 并登记);
    /// 渲染端 lut3d 应用,文件内容哈希入段缓存键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<String>,
}

/// RGB/亮度曲线组(册五 T5.2):每通道独立点集;缺通道 = 该通道恒等。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeCurves {
    /// 亮度曲线(近似:curves master,RGB 同步)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master: Option<Vec<CurvePoint>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<Vec<CurvePoint>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub green: Option<Vec<CurvePoint>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blue: Option<Vec<CurvePoint>>,
}

/// HSL 限定器(册五 T5.2 二级;登记降级——渲染端不产滤镜,WARN 留痕)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeHsl {
    /// 选色中心 hue(度 0..360)。
    pub hue_center: f64,
    /// 选色 hue 半宽(度 0..180)。
    pub hue_width: f64,
    /// 饱和度下界 0..1。
    #[serde(default)]
    pub sat_min: f64,
    /// 饱和度上界 0..1(1=不限)。
    #[serde(default = "one_f")]
    pub sat_max: f64,
    /// 该范围内 hue 偏移(度)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue_shift: Option<f64>,
    /// 该范围内饱和度倍率(1=不变)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sat_gain: Option<f64>,
    /// 该范围内明度倍率(1=不变)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_gain: Option<f64>,
}

/// 轨道 EQ 频段(册五 T5.3):type=peaking/lowshelf/highshelf;biquad 族
/// (equalizer/lowshelf/highshelf 滤镜)链式应用,上限 8 段(schema 界)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    #[serde(rename = "type")]
    pub type_: String,
    /// 中心/拐点频率 Hz(20..20000)。
    pub freq: f64,
    /// 增益 dB(-24..24;shelf/peaking 同域)。
    pub gain: f64,
    /// Q 值(0.1..16;缺省 1.0,shelf 忽略)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<f64>,
}

/// 轨道动态(册五 T5.3):压缩(acompressor 参数子集)+ 限幅(alimiter)。
/// threshold/limit IR 以 dB 表达(混音 UI 语义),渲染端转线性域。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDyn {
    /// 压缩阈值 dB(-60..0;缺省不压缩)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_db: Option<f64>,
    /// 压缩比 1..20(缺省 4)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// 启动毫秒 1..500(缺省 25)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack_ms: Option<f64>,
    /// 释放毫秒 5..5000(缺省 250)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_ms: Option<f64>,
    /// 限幅电平 dB(-24..0;缺省不限幅)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_db: Option<f64>,
}

impl TrackDyn {
    /// 是否声明了任一动态处理(压缩或限幅)。
    pub fn is_empty(&self) -> bool {
        self.threshold_db.is_none() && self.limit_db.is_none()
    }
}
