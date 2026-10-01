// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 关键帧引擎(IR v3,册五 T5.1):模型 + **求值器单源** + 白名单裁决。
//!
//! 单源纪律(ADR-0018 决策 3):同一份(关键帧, 插值, 区间)输入只有一个求值实现——
//! 本模块。预览面:Rust 求值后以采样序列进投影下发,壳只画样本零插值;渲染面:
//! cutforge-render 的编译器把本模块的求值语义编译为 ffmpeg 表达式/分段表,
//! 一致性由 parity 逐样本实渲对拍兜底。
//!
//! 属性白名单(ADR-0018 分级映射表):`position.x`/`position.y`/`scale`/`rotation`/
//! `opacity`/`volume`/`speed`/`fx.<fxId>.<param>`。未知 property 整组 SCHEMA_INVALID
//! (防静默丢);fx 参数键按 fx-catalog 注册表校验,`static` 三态打关键帧诚实拒绝;
//! speed 关键帧与 speedCurve 互斥(同给 SCHEMA_INVALID)。
//!
//! 本模块不启动进程、不做 IO;全部纯函数,同输入确定性(浮点全用 total_cmp/有限迭代)。

use crate::model::{Clip, SpeedPoint};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单条关键帧(对应 schema clip.keyframes 元素;timeMs 相对片段起点的播放域毫秒;
/// interp 描述本帧→下一帧的缓动;bezier 为 CSS cubic-bezier 同构四参数)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    /// 属性白名单键(白名单裁决见 [`validate_clip_keyframes`];枚举约束在 schema 层)。
    pub property: String,
    /// 关键帧时间(播放域毫秒;同属性序列严格递增)。
    pub time_ms: u64,
    /// 关键帧值。
    pub value: f64,
    /// 插值(缺省 linear;封闭枚举在 schema 层界)。
    #[serde(default = "default_interp")]
    pub interp: String,
    /// 贝塞尔四参数控制柄 [x1,y1,x2,y2](interp=bezier 时必给;校验见下)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bezier: Option<[f64; 4]>,
}

fn default_interp() -> String {
    "linear".into()
}

impl Keyframe {
    pub fn new(property: &str, time_ms: u64, value: f64) -> Self {
        Keyframe { property: property.into(), time_ms, value, interp: "linear".into(), bezier: None }
    }
}

/// 插值族(求值器内部形态;由 interp 字符串 + bezier 参数解出)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Interp {
    Linear,
    Hold,
    /// 缓动族与贝塞尔统一为 cubic-bezier 控制柄(CSS 标准预设):
    /// easeIn=(0.42,0,1,1) easeOut=(0,0,0.58,1) easeInOut=(0.42,0,0.58,1)。
    Bezier([f64; 4]),
}

pub const EASE_IN: [f64; 4] = [0.42, 0.0, 1.0, 1.0];
pub const EASE_OUT: [f64; 4] = [0.0, 0.0, 0.58, 1.0];
pub const EASE_IN_OUT: [f64; 4] = [0.42, 0.0, 0.58, 1.0];

/// interp 字符串 + bezier 参数 → 求值形态(未知串回落 Linear;枚举封闭性由 schema 层界)。
pub fn parse_interp(interp: &str, bezier: Option<[f64; 4]>) -> Interp {
    match interp {
        "hold" => Interp::Hold,
        "easeIn" => Interp::Bezier(EASE_IN),
        "easeOut" => Interp::Bezier(EASE_OUT),
        "easeInOut" => Interp::Bezier(EASE_IN_OUT),
        "bezier" => bezier.map_or(Interp::Linear, Interp::Bezier),
        _ => Interp::Linear,
    }
}

/// cubic-bezier 缓动进度:进度 x∈[0,1] → 输出 y∈R(控制点 (x1,y1),(x2,y2);
/// 端点固定 (0,0),(1,1))。de Casteljau 展开 + 牛顿迭代(初值 x)辅以二分兜底,
/// 固定迭代上限 → 同输入逐位确定(无随机性无平台差异)。
pub fn bezier_progress(cp: [f64; 4], x: f64) -> f64 {
    let [x1, y1, x2, y2] = cp;
    let clamp01 = |v: f64| v.clamp(0.0, 1.0);
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // Bx(u) = 3u(1-u)²x1 + 3u²(1-u)x2 + u³;牛顿法求 Bx(u)=x
    let bx = |u: f64| 3.0 * u * (1.0 - u) * (1.0 - u) * x1 + 3.0 * u * u * (1.0 - u) * x2 + u * u * u;
    let dbx = |u: f64| {
        3.0 * (1.0 - u) * (1.0 - u) * x1 + 6.0 * u * (1.0 - u) * (x2 - x1) + 3.0 * u * u * (1.0 - x2)
    };
    let mut u = clamp01(x);
    for _ in 0..12 {
        let e = bx(u) - x;
        if e.abs() < 1e-9 {
            break;
        }
        let d = dbx(u);
        if d.abs() < 1e-12 {
            break;
        }
        let nu = u - e / d;
        if !(0.0..=1.0).contains(&nu) {
            break; // 越界转二分兜底
        }
        u = nu;
    }
    if (bx(u) - x).abs() > 1e-6 {
        // 二分兜底(Bx 在 [0,1] 单调当 x1,x2∈[0,1],必收敛)
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        for _ in 0..64 {
            let mid = (lo + hi) / 2.0;
            if bx(mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        u = (lo + hi) / 2.0;
    }
    // By(u) 同构展开
    3.0 * u * (1.0 - u) * (1.0 - u) * y1 + 3.0 * u * u * (1.0 - u) * y2 + u * u * u
}

/// 同属性关键帧序列的求值(单源):t ≤ 首帧取首值,t ≥ 末帧取末值(端点外延);
/// 帧间按该帧 interp 插值;hold 恒左值;bezier 按进度精确求值。
/// `kfs` 必须已按 time_ms 升序(装配经 [`group_by_property`])。
pub fn eval_group(kfs: &[&Keyframe], interp_of: impl Fn(&Keyframe) -> Interp, t_ms: f64) -> f64 {
    if kfs.is_empty() {
        return 0.0;
    }
    if t_ms <= kfs[0].time_ms as f64 {
        return kfs[0].value;
    }
    let last = kfs[kfs.len() - 1];
    if t_ms >= last.time_ms as f64 {
        return last.value;
    }
    for w in kfs.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (t0, t1) = (a.time_ms as f64, b.time_ms as f64);
        if t_ms >= t0 && t_ms <= t1 {
            let progress = if t1 > t0 { (t_ms - t0) / (t1 - t0) } else { 1.0 };
            return match interp_of(a) {
                Interp::Hold => a.value,
                Interp::Linear => a.value + (b.value - a.value) * progress,
                Interp::Bezier(cp) => a.value + (b.value - a.value) * bezier_progress(cp, progress),
            };
        }
    }
    last.value // 不可达(t_ms 已夹在末帧内)
}

/// clip.keyframes → 按属性分组(保持数组内相对次序;组内按 time_ms 升序稳定排序)。
pub fn group_by_property(kfs: &[Keyframe]) -> Vec<(String, Vec<Keyframe>)> {
    let mut order: Vec<String> = Vec::new();
    for k in kfs {
        if !order.iter().any(|p| p == &k.property) {
            order.push(k.property.clone());
        }
    }
    order
        .into_iter()
        .map(|p| {
            let mut group: Vec<Keyframe> = kfs.iter().filter(|k| k.property == p).cloned().collect();
            group.sort_by(|a, b| a.time_ms.cmp(&b.time_ms).then(a.value.total_cmp(&b.value)));
            (p, group)
        })
        .collect()
}

/// clip 的某属性求值入口(单源门面):无关键帧/属性不存在 → None。
pub fn eval_property(clip: &Clip, property: &str, t_ms: f64) -> Option<f64> {
    let kfs = clip.keyframes.as_ref()?;
    let groups = group_by_property(kfs);
    let (_, group) = groups.into_iter().find(|(p, _)| p == property)?;
    let refs: Vec<&Keyframe> = group.iter().collect();
    Some(eval_group(&refs, |k| parse_interp(&k.interp, k.bezier), t_ms))
}

/// 采样投影(壳曲线编辑器绘制用;壳零插值——只画本函数产出的点集):
/// 网格 100ms ∪ 关键帧时刻(保证极值/拐点可见),t ∈ [0, durationMs]。
/// 返回 [(timeMs, value)];无该属性关键帧 → 空集。
pub fn sample_property(clip: &Clip, property: &str, step_ms: u64) -> Vec<(u64, f64)> {
    let kfs = clip.keyframes.as_ref();
    let Some(kfs) = kfs else { return Vec::new() };
    let groups = group_by_property(kfs);
    let Some((_, group)) = groups.iter().find(|(p, _)| p == property) else { return Vec::new() };
    let mut times: Vec<u64> = Vec::new();
    let mut t = 0u64;
    while t <= clip.duration_ms {
        times.push(t);
        t += step_ms.max(1);
    }
    for k in group {
        times.push(k.time_ms.min(clip.duration_ms));
    }
    times.sort_unstable();
    times.dedup();
    let refs: Vec<&Keyframe> = group.iter().collect();
    times
        .into_iter()
        .map(|tt| (tt, eval_group(&refs, |k| parse_interp(&k.interp, k.bezier), tt as f64)))
        .collect()
}

// ---------------- 白名单裁决(ADR-0018 决策 4:fx 三态注册表) ----------------

/// fx 参数时间轴能力三态(ADR-0018;注册表未标注即视为 static)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxTimeline {
    /// T 标记滤镜:sendcmd 命令序列直达。
    Sendcmd,
    /// 分段子段重建滤镜串(分段边界 = 关键帧)。
    Segment,
    /// 不可动画:打点诚实拒绝。
    Static,
}

impl FxTimeline {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("sendcmd") => FxTimeline::Sendcmd,
            Some("segment") => FxTimeline::Segment,
            _ => FxTimeline::Static,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            FxTimeline::Sendcmd => "sendcmd",
            FxTimeline::Segment => "segment",
            FxTimeline::Static => "static",
        }
    }
}

/// fx 目录(schemas/fx-catalog.json;编译期嵌入,与 cutforge-render 同源文件——
/// core 只读 params 的 timeline 标注做裁决,渲染面在 render 侧)。
pub const FX_CATALOG_SOURCE: &str = include_str!("../../../schemas/fx-catalog.json");

fn fx_catalog() -> &'static Value {
    use std::sync::OnceLock;
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(FX_CATALOG_SOURCE).expect("fx-catalog.json 必须合法"))
}

/// fx.<fxId>.<param> 键裁决:(fxId 是否注册, 参数是否声明, 时间轴三态)。
pub fn fx_param_timeline(fx_id: &str, param: &str) -> (bool, bool, FxTimeline) {
    let doc = fx_catalog();
    let Some(entry) = doc["fx"].as_array().and_then(|a| a.iter().find(|f| f["id"].as_str() == Some(fx_id)))
    else {
        return (false, false, FxTimeline::Static);
    };
    let Some(p) = entry["params"]
        .as_array()
        .and_then(|a| a.iter().find(|p| p["name"].as_str() == Some(param)))
    else {
        return (true, false, FxTimeline::Static);
    };
    (true, true, FxTimeline::parse(p["timeline"].as_str()))
}

/// 固定白名单属性(fx 键除外)。
pub const FIXED_PROPERTIES: [&str; 7] =
    ["position.x", "position.y", "scale", "rotation", "opacity", "volume", "speed"];

/// 单 clip 关键帧语义校验(SCHEMA_INVALID 裁决;错误列表非空 = 拒):
/// 1. property 白名单(未知整组拒;fx 键按注册表,未注册/static 诚实拒);
/// 2. interp=bezier 必须携带四参数;3. 同属性 timeMs 严格递增;
/// 4. speed 关键帧与 speedCurve 互斥(报错信息说明,ADR-0018 B 级单机制)。
pub fn validate_clip_keyframes(clip: &Clip) -> Vec<String> {
    let mut errs = Vec::new();
    let Some(kfs) = &clip.keyframes else { return errs };
    for k in kfs {
        if let Some(fx) = k.property.strip_prefix("fx.") {
            // fxId 本身含点(catalog id = "fx.grain" 等)→ 以最后一个点分隔参数键
            let (fx_id, param) = match fx.rsplit_once('.') {
                Some(v) => v,
                None => {
                    errs.push(format!("keyframes.property({}) 非法 fx 键(fx.<fxId>.<param>)", k.property));
                    continue;
                }
            };
            let (reg, has_p, state) = fx_param_timeline(fx_id, param);
            if !reg {
                errs.push(format!(
                    "keyframes.property({}): fx({fx_id}) 未注册,打点拒绝(防静默丢;目录见 GET /catalogs)",
                    k.property
                ));
            } else if !has_p {
                errs.push(format!(
                    "keyframes.property({}): fx({fx_id}) 无参数 {param},打点拒绝",
                    k.property
                ));
            } else if state == FxTimeline::Static {
                errs.push(format!(
                    "keyframes.property({}): fx 参数时间轴能力为 static(不可动画),打关键帧诚实拒绝(ADR-0018 三态)",
                    k.property
                ));
            }
        } else if !FIXED_PROPERTIES.contains(&k.property.as_str()) {
            errs.push(format!(
                "keyframes.property({}) 不在白名单(position.x/position.y/scale/rotation/opacity/volume/speed/fx.*),整组拒绝",
                k.property
            ));
        }
        if k.interp == "bezier" && k.bezier.is_none() {
            errs.push(format!(
                "keyframes[{}/{}]: interp=bezier 必须携带 bezier 四参数控制柄",
                k.property, k.time_ms
            ));
        }
    }
    // 同属性时间严格递增(按**书写次序**裁决;求值端 group_by_property 的防御性
    // 排序只对已过本闸的工程生效,两处口径一致:合法工程排序前后同序)
    {
        let mut last: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
        for k in kfs {
            if let Some(prev) = last.get(k.property.as_str())
                && k.time_ms <= *prev
            {
                errs.push(format!(
                    "keyframes 属性 {}: timeMs 必须严格递增(书写序 {} → {})",
                    k.property, prev, k.time_ms
                ));
            }
            last.insert(k.property.as_str(), k.time_ms);
        }
    }
    // speed 关键帧与 speedCurve 互斥(同给 SCHEMA_INVALID)
    if kfs.iter().any(|k| k.property == "speed") && clip.speed_curve.is_some() {
        errs.push(
            "speed 关键帧与 speedCurve 互斥:两者同为变速的单一真相源候选(ADR-0018 B 级 \
             speed_segments 单机制),请二选一——删除 speedCurve 或改用 speed 关键帧"
                .to_string(),
        );
    }
    errs
}

/// speed 关键帧 → 合成速度曲线(与 speedCurve 同机制;ADR-0018 B 级"分段常速逼近"):
/// 线性区间直接成点;hold 区间落**常速点对**(t,va)+(tb,va),变速积分取常速精确;
/// 非线性缓动区间按求值器(单源)等分采样成 N 段折线逼近。
/// 由 [`crate::model::speed_segments`] 在 speedCurve 缺席时消费(单一真相源不破)。
pub fn speed_keyframes_to_curve(kfs: &[Keyframe]) -> Vec<SpeedPoint> {
    const SUBDIV: u64 = 4; // 非线性段逼近密度(与对拍阈值同源,实现期定)
    let mut pts: Vec<SpeedPoint> = Vec::new();
    for w in kfs.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        pts.push(SpeedPoint { at_ms: a.time_ms, speed: a.value });
        match parse_interp(&a.interp, a.bezier) {
            // hold:常速点对,步进量化到 1ms(同刻双点会被 speed_segments 的
            // (at,speed) 排序打乱方向;1ms 远小于帧粒度,分段积分仍精确)
            Interp::Hold => pts.push(SpeedPoint { at_ms: b.time_ms.saturating_sub(1), speed: a.value }),
            Interp::Linear => {}
            Interp::Bezier(_) => {
                // 缓动/贝塞尔:区间内等分采样(不含端点;端点由各自帧次推入)
                for i in 1..SUBDIV {
                    let t = a.time_ms as f64 + (b.time_ms - a.time_ms) as f64 * (i as f64 / SUBDIV as f64);
                    let v = eval_group(&[a, b], |k| parse_interp(&k.interp, k.bezier), t);
                    pts.push(SpeedPoint { at_ms: t.round() as u64, speed: v });
                }
            }
        }
    }
    if let Some(last) = kfs.last() {
        pts.push(SpeedPoint { at_ms: last.time_ms, speed: last.value });
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kf(prop: &str, t: u64, v: f64) -> Keyframe {
        Keyframe::new(prop, t, v)
    }

    fn clip_with(kfs: Value) -> Clip {
        serde_json::from_value(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 4000, "keyframes": kfs
        }))
        .unwrap()
    }

    // ---- interp 解析与贝塞尔精确求值 ----

    #[test]
    fn interp_presets_match_css_standards() {
        assert_eq!(parse_interp("linear", None), Interp::Linear);
        assert_eq!(parse_interp("hold", None), Interp::Hold);
        assert_eq!(parse_interp("easeIn", None), Interp::Bezier(EASE_IN));
        assert_eq!(parse_interp("easeOut", None), Interp::Bezier(EASE_OUT));
        assert_eq!(parse_interp("easeInOut", None), Interp::Bezier(EASE_IN_OUT));
        assert_eq!(parse_interp("bezier", Some([0.25, 0.1, 0.25, 1.0])), Interp::Bezier([0.25, 0.1, 0.25, 1.0]));
        assert_eq!(parse_interp("bezier", None), Interp::Linear, "bezier 无参数回落 linear(校验层另行拒)");
    }

    #[test]
    fn bezier_progress_is_exact_and_deterministic() {
        // 端点精确
        assert_eq!(bezier_progress([0.42, 0.0, 0.58, 1.0], 0.0), 0.0);
        assert_eq!(bezier_progress([0.42, 0.0, 0.58, 1.0], 1.0), 1.0);
        // CSS ease-in-out 中点对称:进度 0.5 → 输出 0.5(控制点中心对称)
        let mid = bezier_progress(EASE_IN_OUT, 0.5);
        assert!((mid - 0.5).abs() < 1e-9, "对称控制柄中点必 0.5: {mid}");
        // easeIn 前慢后快:进度 0.5 输出 < 0.5
        assert!(bezier_progress(EASE_IN, 0.5) < 0.5);
        // easeOut 前快后慢
        assert!(bezier_progress(EASE_OUT, 0.5) > 0.5);
        // 单调(取样点全序)
        let mut prev = -1.0;
        for i in 0..=20 {
            let v = bezier_progress(EASE_IN, i as f64 / 20.0);
            assert!(v > prev, "easeIn 必须单调递增 {prev}→{v}");
            prev = v;
        }
        // 确定性:同输入两次逐位一致
        assert_eq!(bezier_progress([0.1, 0.7, 0.3, 0.9], 0.37), bezier_progress([0.1, 0.7, 0.3, 0.9], 0.37));
        // 直线控制柄 (x1=y1, x2=y2) ≡ 线性
        let lin = bezier_progress([0.25, 0.25, 0.75, 0.75], 0.3);
        assert!((lin - 0.3).abs() < 1e-6, "对角控制柄 ≡ 线性: {lin}");
    }

    // ---- 求值器:线性/缓动/hold/端点外延/多属性 ----

    #[test]
    fn eval_linear_two_point_midpoint() {
        let c = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.0},
            {"property": "opacity", "timeMs": 1000, "value": 1.0}
        ]));
        assert_eq!(eval_property(&c, "opacity", 0.0), Some(0.0));
        assert_eq!(eval_property(&c, "opacity", 500.0), Some(0.5));
        assert_eq!(eval_property(&c, "opacity", 1000.0), Some(1.0));
    }

    #[test]
    fn eval_extrapolates_beyond_keyframe_range() {
        let c = clip_with(json!([
            {"property": "scale", "timeMs": 1000, "value": 1.5},
            {"property": "scale", "timeMs": 2000, "value": 2.0}
        ]));
        // 首帧前延 = 首值;末帧后延 = 末值(关键帧外延语义)
        assert_eq!(eval_property(&c, "scale", 0.0), Some(1.5));
        assert_eq!(eval_property(&c, "scale", 999.0), Some(1.5));
        assert_eq!(eval_property(&c, "scale", 3999.0), Some(2.0));
    }

    #[test]
    fn eval_hold_keeps_left_value_across_interval() {
        let c = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.2},
            {"property": "opacity", "timeMs": 1000, "value": 0.8, "interp": "hold"},
            {"property": "opacity", "timeMs": 2000, "value": 1.0}
        ]));
        // [0,1000) 线性;[1000,2000) hold = 0.8;2000 后外延 = 1.0
        assert_eq!(eval_property(&c, "opacity", 500.0), Some(0.5));
        assert_eq!(eval_property(&c, "opacity", 1000.0), Some(0.8));
        assert_eq!(eval_property(&c, "opacity", 1999.0), Some(0.8));
        assert_eq!(eval_property(&c, "opacity", 2500.0), Some(1.0));
    }

    #[test]
    fn eval_ease_beats_linear_midpoint_gap() {
        // 同区间 easeIn vs linear:中点值必可区分(缓动语义成立性)。
        // interp 语义 = 本关键帧→下一帧区间的缓动 → 缓动标注打在区间首帧。
        let mk = |interp: &str| {
            let mut first = json!({"property": "opacity", "timeMs": 0, "value": 0.0});
            first["interp"] = json!(interp);
            let c = clip_with(json!([first, {"property": "opacity", "timeMs": 1000, "value": 1.0}]));
            eval_property(&c, "opacity", 500.0).unwrap()
        };
        let lin = mk("linear");
        let ein = mk("easeIn");
        let eout = mk("easeOut");
        assert!((lin - 0.5).abs() < 1e-9);
        assert!(ein < 0.45, "easeIn 中点应慢: {ein}");
        assert!(eout > 0.55, "easeOut 中点应快: {eout}");
        // bezier 显式四参数 = 等价预设时同值(easeInOut 复用)
        let c1 = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.0, "interp": "bezier", "bezier": [0.42, 0.0, 0.58, 1.0]},
            {"property": "opacity", "timeMs": 1000, "value": 1.0}
        ]));
        let c2 = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.0, "interp": "easeInOut"},
            {"property": "opacity", "timeMs": 1000, "value": 1.0}
        ]));
        let b = eval_property(&c1, "opacity", 300.0).unwrap();
        let e = eval_property(&c2, "opacity", 300.0).unwrap();
        assert!((b - e).abs() < 1e-12, "bezier 预设同柄必须同值: {b} vs {e}");
    }

    #[test]
    fn eval_multi_property_isolated_groups() {
        let c = clip_with(json!([
            {"property": "position.x", "timeMs": 0, "value": 0.5},
            {"property": "position.x", "timeMs": 2000, "value": 0.7},
            {"property": "rotation", "timeMs": 0, "value": 0.0},
            {"property": "rotation", "timeMs": 2000, "value": 90.0},
            {"property": "fx.fx.grain.strength", "timeMs": 0, "value": 10.0},
            {"property": "fx.fx.grain.strength", "timeMs": 2000, "value": 40.0}
        ]));
        assert_eq!(eval_property(&c, "position.x", 1000.0), Some(0.6));
        assert_eq!(eval_property(&c, "rotation", 1000.0), Some(45.0));
        assert_eq!(eval_property(&c, "fx.fx.grain.strength", 1000.0), Some(25.0));
        assert_eq!(eval_property(&c, "volume", 0.0), None, "未打点属性为 None(不臆造)");    }

    #[test]
    fn eval_no_keyframes_is_none() {
        let c = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 1.0}
        ]));
        assert_eq!(eval_property(&c, "opacity", 0.0), Some(1.0));
        let none: Clip = serde_json::from_value(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000})).unwrap();
        assert_eq!(eval_property(&none, "opacity", 0.0), None);
    }

    /// 同输入确定性:乱序输入先排序,求值结果稳定(浮点 total_cmp 消 NaN 歧义)。
    #[test]
    fn eval_is_deterministic_under_unsorted_input() {
        let a = clip_with(json!([
            {"property": "opacity", "timeMs": 1000, "value": 1.0},
            {"property": "opacity", "timeMs": 0, "value": 0.0}
        ]));
        let b = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.0},
            {"property": "opacity", "timeMs": 1000, "value": 1.0}
        ]));
        assert_eq!(eval_property(&a, "opacity", 400.0), eval_property(&b, "opacity", 400.0));
    }

    // ---- 采样投影 ----

    #[test]
    fn sampling_grid_union_keyframe_times() {
        let c = clip_with(json!([
            {"property": "position.x", "timeMs": 0, "value": 0.5},
            {"property": "position.x", "timeMs": 150, "value": 0.6},
            {"property": "position.x", "timeMs": 2000, "value": 0.8}
        ]));
        let s = sample_property(&c, "position.x", 100);
        // 网格 0,100,200,… ∪ 关键帧 0,150,2000;150 在网格点之间必被含
        assert!(s.iter().any(|(t, _)| *t == 150), "关键帧时刻必须入样本: {s:?}");
        assert!(s.iter().any(|(t, _)| *t == 2000), "末关键帧(=时长)入样本");
        assert_eq!(s.first().unwrap().0, 0);
        // 单调且首尾对齐时长
        assert!(s.windows(2).all(|w| w[0].0 < w[1].0), "样本时间严格递增");
        // 值 = 求值器同点输出(单源)
        let at150 = s.iter().find(|(t, _)| *t == 150).unwrap().1;
        assert!((at150 - 0.6).abs() < 1e-12);
        let at100 = s.iter().find(|(t, _)| *t == 100).unwrap().1;
        let direct = eval_property(&c, "position.x", 100.0).unwrap();
        assert!((at100 - direct).abs() < 1e-12, "网格样本值必须 = 求值器同点输出");
    }

    #[test]
    fn sampling_absent_property_is_empty() {
        let c = clip_with(json!([{"property": "opacity", "timeMs": 0, "value": 1.0}]));
        assert!(sample_property(&c, "scale", 100).is_empty());
    }

    // ---- 白名单裁决与互斥 ----

    #[test]
    fn validate_rejects_unknown_property_and_missing_bezier() {
        let c = clip_with(json!([
            {"property": "crop", "timeMs": 0, "value": 1.0}
        ]));
        let errs = validate_clip_keyframes(&c);
        assert!(errs.iter().any(|e| e.contains("crop") && e.contains("白名单")), "{errs:?}");
        let c = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.0},
            {"property": "opacity", "timeMs": 1000, "value": 1.0, "interp": "bezier"}
        ]));
        assert!(validate_clip_keyframes(&c).iter().any(|e| e.contains("bezier 必须携带")), "缺控制柄必拒");
        let ok = clip_with(json!([
            {"property": "opacity", "timeMs": 0, "value": 0.0},
            {"property": "opacity", "timeMs": 1000, "value": 1.0, "interp": "bezier", "bezier": [0.3, 0.0, 0.7, 1.0]}
        ]));
        assert!(validate_clip_keyframes(&ok).is_empty());
    }

    #[test]
    fn validate_rejects_unregistered_or_static_fx_param() {
        // fx.grain.strength 注册为 sendcmd(见 fx-catalog)→ 合法
        let ok = clip_with(json!([
            {"property": "fx.fx.grain.strength", "timeMs": 0, "value": 10.0},
            {"property": "fx.fx.grain.strength", "timeMs": 1000, "value": 40.0}
        ]));
        let errs = validate_clip_keyframes(&ok);
        assert!(errs.is_empty(), "sendcmd 态参数可打点: {errs:?}");
        // 未注册 fxId
        let c = clip_with(json!([{"property": "fx.fx.幽灵.strength", "timeMs": 0, "value": 1.0}]));
        assert!(validate_clip_keyframes(&c).iter().any(|e| e.contains("未注册")), "{:?}", validate_clip_keyframes(&c));
        // 未声明参数
        let c = clip_with(json!([{"property": "fx.fx.grain.无此参", "timeMs": 0, "value": 1.0}]));
        assert!(validate_clip_keyframes(&c).iter().any(|e| e.contains("无参数")));
        // static 态(shake.amplitude 注册表标注 static)诚实拒绝
        let c = clip_with(json!([{"property": "fx.fx.shake.amplitude", "timeMs": 0, "value": 6.0}]));
        assert!(
            validate_clip_keyframes(&c).iter().any(|e| e.contains("static")),
            "static 参数打点必须诚实拒绝: {:?}",
            validate_clip_keyframes(&c)
        );
    }

    #[test]
    fn validate_rejects_non_monotonic_time() {
        let c = clip_with(json!([
            {"property": "opacity", "timeMs": 1000, "value": 0.5},
            {"property": "opacity", "timeMs": 1000, "value": 0.8}
        ]));
        assert!(
            validate_clip_keyframes(&c).iter().any(|e| e.contains("严格递增")),
            "重复 timeMs 必拒: {:?}",
            validate_clip_keyframes(&c)
        );
        // 乱序但可排序:不同属性各自单调即可;同属性乱序由递增校验拒
        let c = clip_with(json!([
            {"property": "opacity", "timeMs": 2000, "value": 0.8},
            {"property": "opacity", "timeMs": 1000, "value": 0.5}
        ]));
        assert!(validate_clip_keyframes(&c).iter().any(|e| e.contains("严格递增")));
    }

    #[test]
    fn validate_speed_keyframes_mutually_exclusive_with_speed_curve() {
        let mut c = clip_with(json!([
            {"property": "speed", "timeMs": 0, "value": 1.0},
            {"property": "speed", "timeMs": 1000, "value": 2.0}
        ]));
        assert!(validate_clip_keyframes(&c).is_empty());
        c.speed_curve = Some(vec![SpeedPoint { at_ms: 0, speed: 1.0 }]);
        let errs = validate_clip_keyframes(&c);
        assert!(errs.iter().any(|e| e.contains("互斥") && e.contains("speedCurve")), "{errs:?}");
    }

    // ---- speed 关键帧 → 曲线展开(B 级分段) ----

    #[test]
    fn speed_keyframes_expand_linear_directly() {
        let pts = speed_keyframes_to_curve(&[kf("speed", 0, 1.0), kf("speed", 1000, 3.0)]);
        assert_eq!(pts, vec![SpeedPoint { at_ms: 0, speed: 1.0 }, SpeedPoint { at_ms: 1000, speed: 3.0 }]);
    }

    #[test]
    fn speed_keyframes_hold_becomes_constant_pair() {
        // hold → (0,2.0)+(1000,2.0) 常速点对 → speed_segments 区间均值 = 2.0(精确)
        let mut a = kf("speed", 0, 2.0);
        a.interp = "hold".into();
        let pts = speed_keyframes_to_curve(&[a, kf("speed", 1000, 1.0)]);
        assert_eq!(pts.len(), 3, "hold 区间落常速点对: {pts:?}");
        assert_eq!(pts[1], SpeedPoint { at_ms: 999, speed: 2.0 }, "步进量化到 1ms");
        // 经单一真相源:主体 [0,999)@2.0 + [1000,外延)@1.0;中间 1ms 步进段
        // (均值 1.5)为 hold 阶跃的量化边界(远小于帧粒度,积分影响 ~0)
        let c = clip_with(json!([
            {"property": "speed", "timeMs": 0, "value": 2.0, "interp": "hold"},
            {"property": "speed", "timeMs": 1000, "value": 1.0}
        ]));
        let segs = crate::model::speed_segments(&c);
        assert_eq!(segs[0], (0, 999, 2.0), "hold 主体恒速 2.0");
        assert_eq!(*segs.last().unwrap(), (1000, 4000, 1.0), "步进后恒速 1.0");
        assert_eq!(segs.len(), 3, "仅 1ms 量化段");
    }

    #[test]
    fn speed_keyframes_eased_interval_subdivides_via_evaluator() {
        let mut a = kf("speed", 0, 1.0);
        a.interp = "easeIn".into();
        let b = kf("speed", 1000, 2.0);
        let pts = speed_keyframes_to_curve(&[a.clone(), b.clone()]);
        // 4 段逼近:区间内 3 个采样点 + 两端点 = 5 点;首段速度 < 线性均值(慢启动)
        assert!(pts.len() >= 5, "缓动段必须细分逼近: {pts:?}");
        let seg1_mean = (pts[0].speed + pts[1].speed) / 2.0;
        assert!(seg1_mean < 1.125 + 1e-9, "easeIn 首段均值应低于线性 1.125: {seg1_mean}");
        // 采样值 = 求值器同点输出(单源纪律)
        let direct = eval_group(&[&a, &b], |k| parse_interp(&k.interp, k.bezier), 250.0);
        assert!((pts[1].speed - direct).abs() < 1e-9, "细分点必须来自求值器");
    }

    // ---- 模型承载(roundtrip 防静默丢) ----

    #[test]
    fn keyframes_roundtrip_no_loss() {
        let c = clip_with(json!([
            {"property": "position.x", "timeMs": 0, "value": 0.5},
            {"property": "position.x", "timeMs": 1000, "value": 0.7, "interp": "easeInOut"},
            {"property": "opacity", "timeMs": 0, "value": 0.0},
            {"property": "opacity", "timeMs": 800, "value": 1.0, "interp": "bezier", "bezier": [0.25, 0.1, 0.25, 1.0]}
        ]));
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["keyframes"].as_array().unwrap().len(), 4);
        assert_eq!(back["keyframes"][3]["interp"], json!("bezier"));
        assert_eq!(back["keyframes"][3]["bezier"][1], json!(0.1));
        let c2: Clip = serde_json::from_value(back).unwrap();
        assert_eq!(c, c2, "serde 往返无静默丢弃");
        // 无关键帧 clip:字段不臆造
        let none: Clip = serde_json::from_value(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000})).unwrap();
        let back = serde_json::to_value(&none).unwrap();
        assert!(back.get("keyframes").is_none(), "缺省字段不得臆造");
    }

    /// undo/replay:keyframes 变更单 Op 回滚(T5.1)——clip_update 携带 keyframes
    /// 产一条字段级 Op(/keyframes before/after),undo 恢复原状,redo 复现;
    /// 白名单外值由 schema+语义双闸拒绝并回滚(不升 rev 不产 Op)。
    #[test]
    fn keyframes_patch_undo_redo_single_op() {
        use crate::command::{ClipPatch, Command};
        use crate::engine::{ApplyOpts, Engine, sample_project};
        use crate::oplog::Actor;
        let mut eng = Engine::new(sample_project()).unwrap();
        let kfs = vec![
            Keyframe::new("position.x", 0, 0.5),
            Keyframe::new("position.x", 1000, 0.7),
        ];
        let rec = eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { keyframes: Some(kfs.clone()), ..Default::default() },
            },
            Actor::agent("kf-test"),
            ApplyOpts::default(),
        )
        .unwrap();
        assert_eq!(rec.op_ids.len(), 1, "单 Op");
        let op = &eng.oplog().ops()[0];
        assert_eq!(op.target.path, "/tracks/0/clips/0");
        assert_eq!(op.after["keyframes"].as_array().unwrap().len(), 2, "after 携带整组");
        assert_eq!(op.before["keyframes"], serde_json::Value::Null, "before = 原缺席");
        // undo 恢复
        eng.undo(Actor::agent("kf-test")).unwrap();
        let clip = eng.project().tracks[0].clips[0].clone();
        assert!(clip.keyframes.is_none(), "undo 后 keyframes 恢复缺席");
        // redo 复现
        eng.redo(Actor::agent("kf-test")).unwrap();
        let clip = eng.project().tracks[0].clips[0].clone();
        assert_eq!(clip.keyframes.as_ref().unwrap(), &kfs, "redo 复现整组");
        // 非法值:静态 fx 参数打点 → SCHEMA_INVALID 回滚
        let bad = vec![Keyframe::new("fx.fx.shake.amplitude", 0, 6.0)];
        let before = eng.project().clone();
        let r = eng.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { keyframes: Some(bad), ..Default::default() },
            },
            Actor::agent("kf-test"),
            ApplyOpts::default(),
        );
        assert!(matches!(r, Err(crate::engine::Reject::SchemaInvalid(_))), "static 参数必须拒: {r:?}");
        assert_eq!(eng.project(), &before, "拒绝必须回滚");
        assert_eq!(eng.rev(), 3, "拒绝不得升 rev(apply 1 + undo 2 + redo 3)");
        // 互斥:speed 关键帧与 speedCurve 同给 → 拒
        let mut eng2 = Engine::new(sample_project()).unwrap();
        eng2.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch {
                    speed_curve: Some(vec![crate::model::SpeedPoint { at_ms: 0, speed: 1.0 }]),
                    ..Default::default()
                },
            },
            Actor::agent("kf-test"),
            ApplyOpts::default(),
        )
        .unwrap();
        let r = eng2.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch {
                    keyframes: Some(vec![Keyframe::new("speed", 0, 1.0), Keyframe::new("speed", 500, 2.0)]),
                    ..Default::default()
                },
            },
            Actor::agent("kf-test"),
            ApplyOpts::default(),
        );
        assert!(matches!(r, Err(crate::engine::Reject::SchemaInvalid(_))), "互斥必须拒: {r:?}");
    }
}
