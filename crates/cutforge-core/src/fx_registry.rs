// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! fx 目录注册表(ADR-0018 决策 4;自 keyframes.rs 纯移动拆出——行数红线 A1-3,
//! `crate::keyframes` 经 `pub use` 桥保持原路径)。core 只读 params 的 timeline
//! 标注做打点裁决;渲染面消费目录的滤镜编译在 cutforge-render 侧(同源文件)。

use serde_json::Value;

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

/// fx 目录(schemas/fx-catalog.json;编译期嵌入,与 cutforge-render 同源文件)。
pub const FX_CATALOG_SOURCE: &str = include_str!("../../../schemas/fx-catalog.json");

fn fx_catalog() -> &'static Value {
    use std::sync::OnceLock;
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(FX_CATALOG_SOURCE).expect("fx-catalog.json 必须合法"))
}

/// fx.<fxId>.<param> 键裁决:(fxId 是否注册, 参数是否声明, 时间轴三态)。
pub fn fx_param_timeline(fx_id: &str, param: &str) -> (bool, bool, FxTimeline) {
    let doc = fx_catalog();
    let Some(entry) = doc["fx"]
        .as_array()
        .and_then(|a| a.iter().find(|f| f["id"].as_str() == Some(fx_id)))
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
