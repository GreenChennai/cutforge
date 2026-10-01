// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 效果目录注册表(册四 A4 T4.5/T4.6):转场目录 + 特效(fx)目录 + 动效(motion)目录。
//!
//! 真相源 = `schemas/transition-catalog.json` + `schemas/fx-catalog.json`(编译期嵌入,
//! 与 mcp-tools.json 同模式);本模块是其在渲染端的唯一消费者:
//!
//! - 转场:xfade 名直通(id 即 ffmpeg transition 名,方向展开为独立 id);fx=tr.* 优先于
//!   type;未注册降级 fade + WARN(诚实降级,进度事件 warnings 留痕);ADR-0023 口径的
//!   有效转场时长(钳到两侧片段时长,零漂移的钳制前提)也在此定义;
//! - fx:clip.fx.combo 叠加栈(上限 3,顺序即应用顺序)→ 段滤镜链;参数 = 条目值覆写
//!   目录默认,越界钳制;未知 fxId 逐项降级 WARN,不整链回退;
//! - motion:入场/出场动画(只收真实渲染项)→ 段滤镜链(时域:入场 t=0 起,出场挂在
//!   播放域末尾);mo.* 直通别名优先于枚举,未注册降级枚举 + WARN。
//!
//! 全部函数为纯函数(字符串进、字符串出),可在不装 ffmpeg 的环境单测断言。

use cutforge_core::model::Clip;
use serde_json::Value;
use std::sync::OnceLock;

/// 转场目录(编译期嵌入;集合 = 本机 ffmpeg `-h filter=xfade` 实测 58 项)。
pub const TRANSITION_CATALOG_JSON: &str = include_str!("../../../schemas/transition-catalog.json");
/// 特效 + 动效目录(编译期嵌入)。
pub const FX_CATALOG_JSON: &str = include_str!("../../../schemas/fx-catalog.json");
/// 花字目录(册四 A4 T4.7;12 模板,吸收旧版 rs_subtitle/assets/huazi 标签语法)。
pub const HUAZI_CATALOG_JSON: &str = include_str!("../../../schemas/huazi-catalog.json");

fn catalog_doc() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(TRANSITION_CATALOG_JSON).expect("transition-catalog.json 必须合法"))
}

fn fx_doc() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(FX_CATALOG_JSON).expect("fx-catalog.json 必须合法"))
}

// ---------------- 转场目录(T4.5) ----------------

/// 目录单条转场(id = xfade 名;方向展开为独立 id,不做方向参数化)。
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub directional: bool,
}

/// 全部转场目录项(按 schema 文件顺序)。
pub fn transitions() -> &'static [TransitionDef] {
    static LIST: OnceLock<Vec<TransitionDef>> = OnceLock::new();
    LIST.get_or_init(|| {
        catalog_doc()["transitions"]
            .as_array()
            .expect("transitions 数组必须存在")
            .iter()
            .map(|t| TransitionDef {
                id: t["id"].as_str().unwrap_or_default().to_string(),
                name: t["name"].as_str().unwrap_or_default().to_string(),
                category: t["category"].as_str().unwrap_or_default().to_string(),
                directional: t["directional"].as_bool().unwrap_or(false),
            })
            .collect()
    })
}

fn find_transition(id: &str) -> Option<&'static TransitionDef> {
    transitions().iter().find(|t| t.id == id)
}

/// 解析片段出向转场的 xfade 名(T4.5 直通):`transition.fx`(tr.<id> 或裸 id)
/// 优先于 `type`;两者同给以 fx 为准并 WARN;fx 未注册回退 type;type 未注册
/// 降级 fade 并 WARN(返回 (xfade 名, WARN))。cut/none 由调用方先行排除。
pub fn resolve_transition(clip: &Clip) -> (String, Option<String>) {
    let Some(t) = &clip.transition else { return ("fade".into(), None) };
    let mut warns: Option<String> = None;
    if let Some(fx) = &t.fx {
        let id = fx.strip_prefix("tr.").unwrap_or(fx);
        if let Some(def) = find_transition(id) {
            if t.type_.is_some() {
                warns = Some(format!(
                    "transition.fx({fx}) 与 type 同给,以 fx 为准({});",
                    def.id
                ));
            }
            return (def.id.clone(), warns);
        }
        warns = Some(format!("transition.fx({fx}) 未注册,回退 type;"));
    }
    let ty = t.type_.clone().unwrap_or_else(|| "fade".into());
    match find_transition(&ty) {
        Some(def) => (def.id.clone(), warns),
        None => {
            warns = Some(format!(
                "{warns_part}transition.type({ty}) 未注册,降级 fade",
                warns_part = warns.as_deref().unwrap_or("")
            ));
            ("fade".into(), Some(warns.unwrap()))
        }
    }
}

/// 边界 i(转场挂在 clip i 上,表示 i-1→i)的**有效转场时长**(ADR-0023 零漂移口径):
/// type=cut/none → 0(显式硬切,与 transition_out_ms 同口径);durMs 钳到两侧片段
/// 时长内——转场窗必须完整落在前段(含尾帧)与后段内,否则 xfade/acrossfade 坍缩
/// 截断(册四 BE3a 实测:offset 含尾帧即触发截断虫)。尾帧扩展(segment_tail_ms)
/// 与 compose/acrossfade 的 d 必须同取本值。
pub fn effective_transition_ms(clips: &[Clip], boundary: usize) -> f64 {
    if boundary == 0 || boundary >= clips.len() {
        return 0.0;
    }
    let Some(t) = &clips[boundary].transition else { return 0.0 };
    match t.type_.as_deref() {
        Some("cut") | Some("none") => return 0.0,
        _ => {}
    }
    let raw = t.dur_ms.unwrap_or(0.0);
    if raw <= 0.0 {
        return 0.0;
    }
    raw.min(clips[boundary - 1].duration_ms as f64).min(clips[boundary].duration_ms as f64)
}

// ---------------- 花字目录(T4.7;ADR-0016 ASS 路线的模板面) ----------------

/// 花字目录单条:参数 schema + 样式级覆写 + override 标签模板(或逐字动画 kind)。
#[derive(Debug, Clone, PartialEq)]
pub struct HuaziDef {
    pub id: String,
    pub name: String,
    pub category: String,
    /// 参数定义(min/max/default;type: number|color——color 值为 ASS 字面量直取)。
    pub params: Vec<HuaziParamDef>,
    /// 样式级覆写(borderStyle/backColor/outlineWidth/primaryColor/secondaryColor/
    /// outlineColor;BorderStyle 无法用 override 表达,由逐片段 Style 行承载)。
    pub style: serde_json::Map<String, Value>,
    /// override 模板({TEXT}=转义正文,参数名直取,{DUR}=片段时长毫秒)。
    pub body: Option<String>,
    /// 逐字动画生成器实现名(perchar-pop/perchar-typewriter/perchar-wave)。
    pub body_kind: Option<String>,
    /// 卡拉OK模板:生成器按片段时长逐字均分 \kf。
    pub karaoke: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HuaziParamDef {
    pub name: String,
    pub kind: String,
    pub min: f64,
    pub max: f64,
    pub default: Value,
}

pub fn huazi_list() -> &'static [HuaziDef] {
    static LIST: OnceLock<Vec<HuaziDef>> = OnceLock::new();
    LIST.get_or_init(|| {
        serde_json::from_str::<Value>(HUAZI_CATALOG_JSON)
            .expect("huazi-catalog.json 必须合法")["huazi"]
            .as_array()
            .expect("huazi 数组必须存在")
            .iter()
            .map(|h| HuaziDef {
                id: h["id"].as_str().unwrap_or_default().to_string(),
                name: h["name"].as_str().unwrap_or_default().to_string(),
                category: h["category"].as_str().unwrap_or_default().to_string(),
                params: h["params"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|p| HuaziParamDef {
                                name: p["name"].as_str().unwrap_or_default().to_string(),
                                kind: p["type"].as_str().unwrap_or("number").to_string(),
                                min: p["min"].as_f64().unwrap_or(0.0),
                                max: p["max"].as_f64().unwrap_or(1.0),
                                default: p["default"].clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                style: h["style"].as_object().cloned().unwrap_or_default(),
                body: h["body"].as_str().map(String::from),
                body_kind: h["bodyKind"].as_str().map(String::from),
                karaoke: h["karaoke"].as_bool().unwrap_or(false),
            })
            .collect()
    })
}

/// 花字模板解析:`hz.<id>` 或裸 id;未注册 → None(调用方诚实降级纯文本)。
pub fn find_huazi(id: &str) -> Option<&'static HuaziDef> {
    let id = id.strip_prefix("hz.").unwrap_or(id);
    huazi_list().iter().find(|h| h.id.strip_prefix("hz.").unwrap_or(&h.id) == id)
}

/// 花字单参数取值:clip 覆写 → 目录默认;数值钳到 [min,max];未知键按默认裁决。
/// 返回 (字符串化取值, 越界/未知告警)。
pub fn huazi_param_value(
    clip_params: Option<&serde_json::Map<String, Value>>,
    p: &HuaziParamDef,
) -> (String, Vec<String>) {
    let mut warns = Vec::new();
    let raw = clip_params.and_then(|m| m.get(&p.name));
    let v: Value = match raw {
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(0.0).clamp(p.min, p.max);
            Value::from(f)
        }
        Some(Value::String(s)) if p.kind == "color" => Value::String(s.clone()),
        Some(_) => {
            warns.push(format!("huazi 参数 {} 非法,按默认值裁决;", p.name));
            p.default.clone()
        }
        None => p.default.clone(),
    };
    // 字符串化:color 直取字面量;number 走 fmt_f64(去尾零,确定性)
    let s = match &v {
        Value::String(s) => s.clone(),
        Value::Number(n) => crate::steps::fmt_f64(n.as_f64().unwrap_or(0.0)),
        _ => String::new(),
    };
    (s, warns)
}

// ---------------- fx 目录(T4.6) ----------------

/// fx 参数定义(渲染端钳制 + 默认值裁决)。
#[derive(Debug, Clone, PartialEq)]
pub struct FxParamDef {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

/// fx 目录单条(fxId → ffmpeg 滤镜映射模板)。
#[derive(Debug, Clone, PartialEq)]
pub struct FxDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub params: Vec<FxParamDef>,
    pub filter: String,
}

/// 全部 fx 目录项(按 schema 文件顺序;首批 11 项)。
pub fn fx_list() -> &'static [FxDef] {
    static LIST: OnceLock<Vec<FxDef>> = OnceLock::new();
    LIST.get_or_init(|| {
        fx_doc()["fx"]
            .as_array()
            .expect("fx 数组必须存在")
            .iter()
            .map(|f| FxDef {
                id: f["id"].as_str().unwrap_or_default().to_string(),
                name: f["name"].as_str().unwrap_or_default().to_string(),
                category: f["category"].as_str().unwrap_or_default().to_string(),
                params: f["params"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|p| FxParamDef {
                                name: p["name"].as_str().unwrap_or_default().to_string(),
                                min: p["min"].as_f64().unwrap_or(0.0),
                                max: p["max"].as_f64().unwrap_or(1.0),
                                default: p["default"].as_f64().unwrap_or(0.0),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                filter: f["filter"].as_str().unwrap_or_default().to_string(),
            })
            .collect()
    })
}

fn find_fx(id: &str) -> Option<&'static FxDef> {
    fx_list().iter().find(|f| f.id == id)
}

/// combo 叠加上限(schema maxItems 同源;越界渲染端钳制 + WARN)。
pub const FX_COMBO_CAP: usize = 3;

/// 单条 fx 声明 → 滤镜串(参数覆写默认、越界钳制、未声明键按默认裁决 WARN;
/// 上下文 = 画布宽高帧率 + 实例标签)。公开给关键帧 fx 通路复用(kf_expr:
/// sendcmd 打标签/segment 分支重建均需逐条渲染,单源本函数,册五 T5.1)。
pub fn fx_entry_text(
    def: &FxDef,
    entry: &cutforge_core::model::FxEntry,
    w: u32,
    h: u32,
    fps: u32,
    label: &str,
) -> (String, Vec<String>) {
    let mut warns: Vec<String> = Vec::new();
    let mut text = def.filter.clone();
    for p in &def.params {
        let (val, mut w2) = param_value(entry, p);
        warns.append(&mut w2);
        text = text.replace(&format!("{{{}}}", p.name), &crate::steps::fmt_f64(val));
    }
    for key in entry.params.as_ref().map(|m| m.keys()).into_iter().flatten() {
        if !def.params.iter().any(|p| &p.name == key) {
            warns.push(format!("fx({}) 参数 {key} 未声明,按默认值裁决;", def.id));
        }
    }
    let text = text
        .replace("{W}", &w.to_string())
        .replace("{H}", &h.to_string())
        .replace("{FPS}", &fps.to_string())
        .replace("{L}", label);
    (text, warns)
}

/// 片段 fx.combo → 段滤镜链(册四 T4.6):数组顺序即应用顺序;参数覆写默认、
/// 越界钳制、未声明键按默认裁决(WARN);未知 fxId 逐项降级 WARN。上下文 =
/// 画布宽高 + 帧率(马赛克/抖镜模板需要);{L} 标签前缀逐实例唯一化。
pub fn fx_chain(clip: &Clip, w: u32, h: u32, fps: u32) -> (String, Vec<String>) {
    let mut warns: Vec<String> = Vec::new();
    let Some(fx_spec) = &clip.fx else { return (String::new(), warns) };
    let Some(combo) = &fx_spec.combo else { return (String::new(), warns) };
    if combo.len() > FX_COMBO_CAP {
        warns.push(format!("fx.combo {} 条超上限 {FX_COMBO_CAP},截断;", combo.len()));
    }
    let mut parts: Vec<String> = Vec::new();
    for (i, entry) in combo.iter().take(FX_COMBO_CAP).enumerate() {
        let Some(def) = find_fx(&entry.fx) else {
            warns.push(format!("fx({}) 未注册,该项降级跳过(fxDegraded);", entry.fx));
            continue;
        };
        let (text, mut w2) = fx_entry_text(def, entry, w, h, fps, &format!("c{i}"));
        warns.append(&mut w2);
        parts.push(text);
    }
    (parts.join(","), warns)
}

/// 单参数取值:条目覆写 → 默认;数值钳到 [min,max];非数值按默认裁决。
fn param_value(entry: &cutforge_core::model::FxEntry, p: &FxParamDef) -> (f64, Vec<String>) {
    let mut warns = Vec::new();
    let raw = entry.params.as_ref().and_then(|m| m.get(&p.name));
    let v = match raw {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(p.default).clamp(p.min, p.max),
        Some(_) => {
            warns.push(format!("fx 参数 {} 非数值,按默认值裁决;", p.name));
            p.default
        }
        None => p.default,
    };
    (v, warns)
}

// ---------------- motion 目录(T4.6;只收真实渲染项) ----------------

/// motion 目录单条(id → 滤镜模板;kind 标注实现族)。
#[derive(Debug, Clone, PartialEq)]
pub struct MotionDef {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub filter: String,
}

fn motion_list(dir: &str) -> &'static [MotionDef] {
    static IN: OnceLock<Vec<MotionDef>> = OnceLock::new();
    static OUT: OnceLock<Vec<MotionDef>> = OnceLock::new();
    match dir {
        "in" => IN.get_or_init(|| parse_motion("in")),
        _ => OUT.get_or_init(|| parse_motion("out")),
    }
}

fn parse_motion(dir: &str) -> Vec<MotionDef> {
    fx_doc()["motion"][dir]
        .as_array()
        .expect("motion 数组必须存在")
        .iter()
        .map(|m| MotionDef {
            id: m["id"].as_str().unwrap_or_default().to_string(),
            name: m["name"].as_str().unwrap_or_default().to_string(),
            kind: m["kind"].as_str().unwrap_or_default().to_string(),
            filter: m["filter"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

fn find_motion(dir: &str, id: &str) -> Option<&'static MotionDef> {
    motion_list(dir).iter().find(|m| m.id == id)
}

/// 片段 motion → (入场链, 出场链, WARN)。时窗:D = min(inMs/outMs, durationMs)
/// (秒);出场 ST = (durationMs − outMs)/1000 钳非负;N = D×FPS(zoompan 帧数)。
/// 直通别名:mo.<id> 优先于枚举(inFx/outFx);未注册降级枚举 + WARN;
/// 枚举值不在目录(理论不可达:schema 枚举与目录同源)→ 空 + WARN。
pub fn motion_chains(clip: &Clip, w: u32, h: u32, fps: u32) -> (String, String, Vec<String>) {
    let mut warns: Vec<String> = Vec::new();
    let Some(m) = &clip.motion else { return (String::new(), String::new(), warns) };
    let dur_ms = clip.duration_ms as f64;
    let mut mk = |dir: &str, enum_id: Option<&str>, fx_id: Option<&String>, ms: f64, label: &str| -> String {
        let mut id_owned: Option<String> = None;
        if let Some(fx) = fx_id {
            let id = fx.strip_prefix("mo.").unwrap_or(fx);
            if find_motion(dir, id).is_some() {
                id_owned = Some(id.to_string());
                if enum_id.is_some() {
                    warns.push(format!("motion.{dir}Fx({fx}) 与枚举同给,以 fx 为准;"));
                }
            } else {
                warns.push(format!("motion.{dir}Fx({fx}) 未注册,降级枚举 { };", enum_id.unwrap_or("none")));
            }
        }
        let id = id_owned.or_else(|| enum_id.map(String::from));
        let Some(id) = id else { return String::new() };
        if id == "none" {
            return String::new();
        }
        let Some(def) = find_motion(dir, &id) else {
            warns.push(format!("motion.{dir}({id}) 不在目录,降级 none;"));
            return String::new();
        };
        let d = ms.min(dur_ms).max(0.0) / 1000.0;
        let st = if dir == "out" { ((dur_ms - ms).max(0.0)) / 1000.0 } else { 0.0 };
        let n = (d * fps as f64).round().max(1.0);
        def.filter
            .replace("{D}", &crate::steps::fmt_f64(d))
            .replace("{ST}", &crate::steps::fmt_f64(st))
            .replace("{N}", &crate::steps::fmt_f64(n))
            .replace("{W}", &w.to_string())
            .replace("{H}", &h.to_string())
            .replace("{FPS}", &fps.to_string())
            .replace("{L}", label)
    };
    let in_chain = mk("in", m.in_.as_deref(), m.in_fx.as_ref(), m.in_ms.unwrap_or(400.0), "mi");
    let out_chain = mk("out", m.out.as_deref(), m.out_fx.as_ref(), m.out_ms.unwrap_or(400.0), "mo");
    (in_chain, out_chain, warns)
}

/// 片段级降级清单(fx + motion;exec_segment 并入进度事件 warnings,fxDegraded 留痕)。
pub fn clip_degradations(clip: &Clip, w: u32, h: u32, fps: u32) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = &clip.transition
        && (t.fx.is_some() || t.type_.as_deref().is_some_and(|ty| find_transition(ty).is_none()))
        && let Some(warn) = resolve_transition(clip).1
    {
        out.push(format!("clip {} 转场: {warn}", clip.id));
    }
    let (_, fx_warns) = fx_chain(clip, w, h, fps);
    out.extend(fx_warns.into_iter().map(|x| format!("clip {} fx: {x}", clip.id)));
    let (_, _, mo_warns) = motion_chains(clip, w, h, fps);
    out.extend(mo_warns.into_iter().map(|x| format!("clip {} motion: {x}", clip.id)));
    if let Some(fx) = &clip.fx
        && (fx.in_.is_some() || fx.out.is_some())
    {
        out.push(format!("clip {} fx: in/out 槽位暂不渲染(册五遗留),仅 combo 生效;", clip.id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clip(v: Value) -> Clip {
        serde_json::from_value(v).unwrap()
    }

    // ---- 转场目录 ----

    #[test]
    fn transition_catalog_has_58_entries_measured_from_ffmpeg() {
        // 集合来自本机 `ffmpeg -h filter=xfade` 实测(0..57,58 项;custom 不入目录)
        assert_eq!(transitions().len(), 58, "xfade 全集 58 项(实测口径)");
        let ids: Vec<&str> = transitions().iter().map(|t| t.id.as_str()).collect();
        for must in [
            "fade", "dissolve", "pixelize", "slideleft", "slideright", "slideup", "slidedown",
            "wipeleft", "wiperight", "wipeup", "wipedown", "radial", "circleopen", "circleclose",
            "smoothleft", "smoothright", "smoothup", "smoothdown", "hlslice", "hrslice",
            "vuslice", "vdslice", "zoomin", "hblur", "fadeblack", "fadewhite", "distance",
            "squeezev", "squeezeh", "coverleft", "revealright", "hlwind", "diagtl",
        ] {
            assert!(ids.contains(&must), "计划书点名族缺 {must}");
        }
        // 每分类 ≥2(AC-4.3 实渲口径的前提)
        for cat in ["基础", "滑动", "擦除", "图形", "模糊"] {
            let n = transitions().iter().filter(|t| t.category == cat).count();
            assert!(n >= 2, "分类 {cat} 仅 {n} 项");
        }
    }

    #[test]
    fn legacy_seven_enum_maps_verbatim() {
        for ty in ["fade", "wipeleft", "wipeup", "slideleft", "circleopen"] {
            let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000,
                "transition": {"type": ty, "durMs": 300}}));
            assert_eq!(resolve_transition(&c), (ty.to_string(), None), "旧枚举 {ty} 直通");
        }
    }

    #[test]
    fn transition_fx_takes_precedence_and_degrades_honestly() {
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000,
            "transition": {"type": "fade", "fx": "tr.circleclose", "durMs": 300}}));
        let (name, warn) = resolve_transition(&c);
        assert_eq!(name, "circleclose", "fx 以 tr.<id> 直通");
        assert!(warn.unwrap().contains("以 fx 为准"), "同给 WARN");
        // 裸 id 同样直通
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000,
            "transition": {"type": "fade", "fx": "pixelize", "durMs": 300}}));
        assert_eq!(resolve_transition(&c).0, "pixelize");
        // 未注册 → 回退 type
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000,
            "transition": {"type": "fade", "fx": "tr.不存在", "durMs": 300}}));
        let (name, warn) = resolve_transition(&c);
        assert_eq!(name, "fade");
        assert!(warn.unwrap().contains("未注册"));
        // 未注册 type → 降级 fade
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 1000,
            "transition": {"type": "爆闪", "durMs": 300}}));
        let (name, warn) = resolve_transition(&c);
        assert_eq!(name, "fade", "未知 type 诚实降级 + WARN");
        assert!(warn.unwrap().contains("降级 fade"));
    }

    #[test]
    fn effective_transition_clamps_to_both_sides() {
        let mk = |durs: Vec<u64>, dur_ms: u64| -> Vec<Clip> {
            durs.iter()
                .enumerate()
                .map(|(i, d)| {
                    let mut v = json!({"id": format!("V1-00{i}"), "startMs": 0, "durationMs": d});
                    if i == 1 {
                        v["transition"] = json!({"type": "fade", "durMs": dur_ms});
                    }
                    clip(v)
                })
                .collect()
        };
        assert_eq!(effective_transition_ms(&mk(vec![2000, 2000], 500), 1), 500.0);
        assert_eq!(
            effective_transition_ms(&mk(vec![300, 2000], 500), 1),
            300.0,
            "超过前段时长 → 钳到前段"
        );
        assert_eq!(
            effective_transition_ms(&mk(vec![2000, 400], 500), 1),
            400.0,
            "超过后段时长 → 钳到后段"
        );
        assert_eq!(effective_transition_ms(&mk(vec![2000, 2000], 0), 1), 0.0, "durMs=0 硬切");
        assert_eq!(effective_transition_ms(&mk(vec![2000, 2000], 500), 0), 0.0, "边界 0 无效");
    }

    // ---- 花字目录(册四 T4.7) ----

    /// AC-4.4:花字目录 ≥10 模板;分类覆盖描边/发光/立体/底衬/渐变/动画/卡拉OK。
    #[test]
    fn huazi_catalog_has_12_templates_across_categories() {
        assert!(huazi_list().len() >= 10, "花字 ≥10(AC-4.4),实得 {}", huazi_list().len());
        let ids: Vec<&str> = huazi_list().iter().map(|h| h.id.as_str()).collect();
        for must in [
            "hz.outline", "hz.neon", "hz.glow", "hz.emboss", "hz.extrude", "hz.box",
            "hz.brush", "hz.gradient", "hz.pop", "hz.typewriter", "hz.wave", "hz.karaoke",
        ] {
            assert!(ids.contains(&must), "花字模板缺 {must}");
        }
        for cat in ["描边", "发光", "立体", "底衬", "渐变", "动画", "卡拉OK"] {
            assert!(huazi_list().iter().any(|h| h.category == cat), "分类 {cat} 空");
        }
        // 每个模板:body 与 bodyKind 至少其一;karaoké 模板必须声明 karaoke
        for h in huazi_list() {
            assert!(h.body.is_some() || h.body_kind.is_some(), "{} 无产物模板", h.id);
            assert_eq!(h.karaoke, h.id == "hz.karaoke");
        }
    }

    #[test]
    fn find_huazi_resolves_prefix_and_degrades_none() {
        assert_eq!(find_huazi("hz.pop").unwrap().id, "hz.pop");
        assert_eq!(find_huazi("pop").unwrap().id, "hz.pop", "裸 id 直通");
        assert!(find_huazi("hz.不存在").is_none(), "未注册 → 调用方诚实降级");
    }

    #[test]
    fn huazi_param_override_clamps_and_defaults() {
        let def = find_huazi("hz.pop").unwrap();
        let step = def.params.iter().find(|p| p.name == "stepMs").unwrap();
        let mut clip_params = serde_json::Map::new();
        clip_params.insert("stepMs".into(), json!(999));
        let (v, warns) = huazi_param_value(Some(&clip_params), step);
        assert_eq!(v, "200", "越界钳到 max");
        assert!(warns.is_empty(), "钳制不算告警(与 fx 同口径)");
        // 未知键不进这里(逐参数查询);缺省走默认
        let (v, _) = huazi_param_value(None, step);
        assert_eq!(v, "70");
        // color 参数:字面量直取(确定性)
        let ndef = find_huazi("hz.karaoke").unwrap();
        let hl = ndef.params.iter().find(|p| p.name == "highlight").unwrap();
        let (v, _) = huazi_param_value(None, hl);
        assert_eq!(v, "&H0040FF");
        // 样式级覆写在位(box:borderStyle=3)
        let box_def = find_huazi("hz.box").unwrap();
        assert_eq!(box_def.style["borderStyle"], json!(3));
    }

    // ---- fx 目录 ----

    #[test]
    fn fx_catalog_has_first_batch_11() {
        assert!(fx_list().len() >= 11, "首批特效 ≥11,实得 {}", fx_list().len());
        let ids: Vec<&str> = fx_list().iter().map(|f| f.id.as_str()).collect();
        for must in [
            "fx.blur", "fx.gaussian", "fx.mosaic", "fx.sharpen", "fx.glow", "fx.shake",
            "fx.glitch", "fx.grain", "fx.vignette", "fx.mono", "fx.vintage",
        ] {
            assert!(ids.contains(&must), "首批特效缺 {must}");
        }
    }

    #[test]
    fn fx_chain_orders_combo_and_clamps_params() {
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [
                {"fx": "fx.mono"},
                {"fx": "fx.gaussian", "params": {"sigma": 999}},
                {"fx": "fx.grain", "params": {"strength": 24, "未知键": 1}}
            ]}}));
        let (chain, warns) = fx_chain(&c, 1080, 1920, 30);
        assert!(chain.starts_with("hue=s=0,"), "数组顺序即应用顺序: {chain}");
        assert!(chain.contains("gblur=sigma=50"), "越界钳到 max: {chain}");
        assert!(chain.contains("noise=alls=24:allf=t+u"), "{chain}");
        assert_eq!(fx_list().iter().find(|f| f.id == "fx.gaussian").unwrap().params[0].max, 50.0);
        assert!(warns.iter().any(|w| w.contains("未声明")), "未知参数键 WARN: {warns:?}");
    }

    #[test]
    fn fx_chain_caps_at_three_and_skips_unknown_with_warn() {
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [
                {"fx": "fx.mono"}, {"fx": "fx.blur"}, {"fx": "fx.不存在"}, {"fx": "fx.vignette"}
            ]}}));
        let (chain, warns) = fx_chain(&c, 1080, 1920, 30);
        assert_eq!(chain.matches("vignette").count(), 0, "超上限第 4 条截断");
        assert!(warns.iter().any(|w| w.contains("超上限")));
        assert!(warns.iter().any(|w| w.contains("fx.不存在") && w.contains("未注册")), "{warns:?}");
        // glow 多实例标签唯一化(split/blend 图内标签不得互撞)
        let c2 = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [{"fx": "fx.glow"}, {"fx": "fx.glow"}]}}));
        let (chain2, _) = fx_chain(&c2, 1080, 1920, 30);
        assert!(chain2.contains("[c0a]") && chain2.contains("[c1a]"), "标签逐实例唯一: {chain2}");
    }

    // ---- motion 目录 ----

    #[test]
    fn motion_catalog_covers_legacy_and_new() {
        let in_ids: Vec<&str> = motion_list("in").iter().map(|m| m.id.as_str()).collect();
        let out_ids: Vec<&str> = motion_list("out").iter().map(|m| m.id.as_str()).collect();
        // 既有 6+4 枚举(除 none)首次全部落地
        for must in ["fadeIn", "slideInLeft", "slideInRight", "scaleIn", "zoomIn"] {
            assert!(in_ids.contains(&must), "旧枚举 {must} 必须真实渲染");
        }
        for must in ["fadeOut", "slideOutLeft", "slideOutRight"] {
            assert!(out_ids.contains(&must), "旧枚举 {must} 必须真实渲染");
        }
        // 册四新增
        for must in ["slideInUp", "slideInDown", "popIn", "bounceIn", "spinIn"] {
            assert!(in_ids.contains(&must), "新增 {must}");
        }
        for must in ["slideOutUp", "slideOutDown", "zoomOut", "popOut", "bounceOut", "spinOut"] {
            assert!(out_ids.contains(&must), "新增 {must}");
        }
        assert!(in_ids.len() + out_ids.len() >= 19, "目录规模(不含 none): {}", in_ids.len() + out_ids.len());
    }

    #[test]
    fn motion_chains_render_in_and_out_with_windows() {
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "motion": {"in": "fadeIn", "inMs": 400, "out": "fadeOut", "outMs": 600}}));
        let (i, o, warns) = motion_chains(&c, 1080, 1920, 30);
        assert!(warns.is_empty());
        assert_eq!(i, "fade=t=in:st=0:d=0.4", "{i}");
        assert_eq!(o, "fade=t=out:st=1.4:d=0.6", "出场窗挂播放域末尾: {o}");
        // zoompan 模板上下文(N = D×FPS)
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "motion": {"in": "zoomIn", "inMs": 500}}));
        let (i, _, _) = motion_chains(&c, 1080, 1920, 30);
        assert!(i.contains("z='1.5-0.5*min(in/15,1)'"), "N=0.5s×30fps: {i}");
        assert!(i.contains("s=1080x1920:fps=30"), "{i}");
        // inMs 超 durationMs 钳到时长
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 800,
            "motion": {"in": "fadeIn", "inMs": 2000}}));
        let (i, _, _) = motion_chains(&c, 1080, 1920, 30);
        assert!(i.ends_with("d=0.8"), "{i}");
    }

    #[test]
    fn motion_fx_alias_takes_precedence_and_degrades() {
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "motion": {"in": "fadeIn", "inFx": "mo.spinIn", "inMs": 400}}));
        let (i, _, warns) = motion_chains(&c, 1080, 1920, 30);
        assert!(i.starts_with("rotate="), "mo.* 直通优先于枚举: {i}");
        assert!(warns.iter().any(|w| w.contains("以 fx 为准")));
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "motion": {"in": "fadeIn", "inFx": "mo.不存在", "inMs": 400}}));
        let (i, _, warns) = motion_chains(&c, 1080, 1920, 30);
        assert!(i.starts_with("fade="), "未注册别名降级枚举: {i}");
        assert!(warns.iter().any(|w| w.contains("未注册")));
        // none 恒为空链
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "motion": {"in": "none"}}));
        assert_eq!(motion_chains(&c, 1080, 1920, 30).0, "");
    }

    #[test]
    fn clip_degradations_reports_fx_in_out_deferred() {
        let c = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"in": {"fx": "fx.mono"}, "combo": [{"fx": "fx.blur"}]}}));
        let ds = clip_degradations(&c, 1080, 1920, 30);
        assert!(ds.iter().any(|d| d.contains("in/out 槽位暂不渲染")), "{ds:?}");
        // 无 fx/motion/转场问题的片段零降级报告
        let clean = clip(json!({"id": "V1-001", "startMs": 0, "durationMs": 2000,
            "fx": {"combo": [{"fx": "fx.blur"}]}}));
        assert!(clip_degradations(&clean, 1080, 1920, 30).is_empty());
    }
}
