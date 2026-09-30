// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 领域模型(计划书 1.2/1.3):Project/Timeline/Track/Clip 与 schema v2 逐字段对齐。
//! 时间统一毫秒(`*Ms`);剪映微秒只存在于适配层,不得进入本层。

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Voice,
    Sfx,
    Music,
    Ambient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Ffmpeg,
    Jianying,
    Cutforge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ratio {
    #[serde(rename = "9x16")]
    NineBySixteen,
    #[serde(rename = "3x4")]
    ThreeByFour,
    #[serde(rename = "16x9")]
    SixteenByNine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bgm {
    pub src: String,
    #[serde(default = "default_gain")]
    pub gain_db: f64,
    #[serde(default = "yes")]
    pub ducking: bool,
    #[serde(default = "yes", rename = "loop")]
    pub loop_: bool,
}

fn default_gain() -> f64 {
    -18.0
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub ms: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Subtitle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ass: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// 片段(轨道上占据时间区间的元素)。稳定 `id`(如 V1-001)是锚点与 Op target 的引用基础,
/// 禁止依赖数组下标(计划书 1.1 原则 4)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    pub start_ms: u64,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_in_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// 分段速度曲线(册四 A4 T4.4):有值时渲染/投影以其为准,speed 仅作兼容回退;
    /// 段语义见 [`Clip::speed_segments`](分段恒速,左点区间)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_curve: Option<Vec<SpeedPoint>>,
    /// 倒放(册四 A4 T4.4):视频 reverse / 音频 areverse;reverse 先于变速。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverse: Option<bool>,
    /// 旋转角度(度;册四 A4 T4.9):任意值,渲染端 mod 360 归一。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<f64>,
    /// 源域裁剪(册四 A4 T4.9):源素材像素矩形,画幅归一前执行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Crop>,
    /// 翻转(册四 A4 T4.9):none/h/v(枚举约束在 schema 层,模型从宽收 String)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 文本样式(册四 A4 T4.7):字体/字号/颜色/描边/底衬/阴影/对齐/行距/
    /// 透明度/画布内位置 x,y/卡拉OK;渲染端按 ADR-0016 生成临时 ASS 消费。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<crate::text_style::TextStyle>,
    /// 花字挂载(册四 T4.7):hz.<id> 引用 huazi-catalog.json;模型层承接防丢。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub huazi: Option<crate::text_style::Huazi>,
    /// 片段级字体覆盖(CutFlow S7 兼容字段;模型层承接防丢,渲染以
    /// textStyle.fontFamily 优先,无 textStyle 时本字段兜底)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<crate::text_style::FontSpec>,
    /// 降噪档(册四 A4 T4.8):off/low/mid/high → afftdn 参数映射(混音链)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denoise: Option<String>,
    /// 保速变调半音档(册四 A4 T4.8):±12;asetrate+aresample 补偿 atempo,
    /// 与 speed 组合的链序见 across::event_body(降噪→倒放→变调→变速)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reframe: Option<Reframe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<Motion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Overlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<Fade>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "loop")]
    pub loop_: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub punch_in: Option<PunchIn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze_ms: Option<u64>,
    /// 片段特效(册四 A4 T4.6):combo 叠加栈(上限 3,顺序即应用顺序)+ in/out 槽位;
    /// 渲染端按 fx 目录(schemas/fx-catalog.json)解析为段滤镜链,未注册降级 WARN。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<FxSpec>,
    /// 关键帧(IR v3,册五 T5.1):白名单属性动画载体,求值单源见
    /// [`crate::keyframes`](模块级纪律);语义校验在 to_validated_value/from_value 钩子。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyframes: Option<Vec<crate::keyframes::Keyframe>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// 速度曲线点(册四 A4 T4.4):`atMs` 为相对片段 startMs 的时间线毫秒,
/// `speed` ∈ [0.25,4](界在 schema 层);相邻点之间不内插——左点区间恒速。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedPoint {
    pub at_ms: u64,
    pub speed: f64,
}

/// 源域裁剪矩形(册四 A4 T4.9):源素材像素坐标;x/y 缺省 0,w/h 必填(>0 由 schema 界)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Crop {
    #[serde(default)]
    pub x: u64,
    #[serde(default)]
    pub y: u64,
    pub w: u64,
    pub h: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reframe {
    pub anchor_y: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Motion {
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub in_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_ms: Option<f64>,
    /// 入场直通别名(册四 T4.6):mo.<id>(fx-catalog motion.in 目录);与 in 枚举
    /// 并存,声明时优先于枚举;未注册渲染端降级枚举并 WARN。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_fx: Option<String>,
    /// 出场直通别名(册四 T4.6):mo.<id>(fx-catalog motion.out 目录)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_fx: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub type_: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dur_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 转场 fxId(CutFlow 分册04 §3.3;与 type 并存,同给以 fx 为准)。
    /// 册四 T4.5:tr.<id>/裸 id 经转场目录(schemas/transition-catalog.json,
    /// ffmpeg xfade 全集)直通;未注册渲染端降级 type(缺省 fade)并 WARN。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<String>,
}

/// 单条特效声明(册四 T4.6):fxId + 透传参数(未声明的键由渲染端注册表按默认值裁决)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FxEntry {
    pub fx: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Map<String, Value>>,
}

/// 片段特效(册四 T4.6,对应 schema clip.fx):`in`/`out` 为入场/出场槽位
/// (暂登记不渲染,渲染端 WARN 留痕);`combo` 为整段特效叠加栈——数组上限 3
/// (schema maxItems),数组顺序即应用顺序,整组替换。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxSpec {
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub in_: Option<FxEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out: Option<FxEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub combo: Option<Vec<FxEntry>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    pub x: u64,
    pub y: u64,
    pub w: u64,
    pub h: u64,
    #[serde(default = "one_f", skip_serializing_if = "is_one_f")]
    pub opacity: f64,
}

fn one_f() -> f64 {
    1.0
}
fn is_one_f(v: &f64) -> bool {
    *v == 1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fade {
    #[serde(default)]
    pub in_ms: f64,
    #[serde(default)]
    pub out_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PunchIn {
    #[serde(default = "default_factor")]
    pub factor: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn default_factor() -> f64 {
    1.4
}

/// 轨道:同类型元素的容器;`id`(如 V1)首次生成后写回并不再变。
/// 轨道级属性字段(册四 A4 T4.2):Option + skip_serializing_if,旧工程缺省即
/// 不落盘(零迁移);静音/独奏/隐藏的渲染混音联动候 BE3,此处先保证读写不丢。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

/// 工程(Project):IR 的序列化形式即 05_ir/project.json;`version` 恒为 1 兼容旧读法,
/// 契约演进由 `schemaVersion` 表达(计划书 3.3 第 5 项)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    #[serde(default = "one")]
    pub version: u8,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub slug: String,
    pub fps: u32,
    pub canvas: Canvas,
    #[serde(default = "default_backends")]
    pub backends: Vec<Backend>,
    #[serde(default = "default_notes_path")]
    pub notes: String,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bgm: Option<Bgm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Ratio>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markers: Option<Vec<Marker>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<Subtitle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_crossfade_ms: Option<f64>,
}

fn default_schema_version() -> String {
    "3.0.0".into()
}
fn default_backends() -> Vec<Backend> {
    vec![Backend::Ffmpeg]
}
fn default_notes_path() -> String {
    "notes.json".into()
}

impl Project {
    /// 反序列化 + schema v3 校验 + 关键帧语义裁决(契约优先:没有 schema 支撑的
    /// 字段不存在;关键帧白名单/互斥裁决见 [`crate::keyframes::validate_clip_keyframes`])。
    pub fn from_value(v: &Value) -> Result<Self, Vec<String>> {
        let p: Project = serde_json::from_value(v.clone())
            .map_err(|e| vec![format!("反序列化失败: {e}")])?;
        let errors = cutforge_schema::validate("project", v);
        if !errors.is_empty() {
            return Err(errors);
        }
        let errors = p.validate_keyframes();
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(p)
    }

    /// 序列化后必须仍然通过 schema(命令通道在每次变更后强制走这一步)。
    pub fn to_validated_value(&self) -> Result<Value, Vec<String>> {
        let v = serde_json::to_value(self).map_err(|e| vec![format!("序列化失败: {e}")])?;
        let errors = cutforge_schema::validate("project", &v);
        if !errors.is_empty() {
            return Err(errors);
        }
        let errors = self.validate_keyframes();
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(v)
    }

    /// 全工程关键帧语义校验(IR v3):任一 clip 违例整组 SCHEMA_INVALID。
    pub fn validate_keyframes(&self) -> Vec<String> {
        let mut errs = Vec::new();
        for t in &self.tracks {
            for c in &t.clips {
                for e in crate::keyframes::validate_clip_keyframes(c) {
                    errs.push(format!("clip {}: {e}", c.id));
                }
            }
        }
        errs
    }

    /// 按 id 找片段,返回 (轨道下标, 片段下标)。
    pub fn find_clip(&self, clip_id: &str) -> Option<(usize, usize)> {
        for (ti, t) in self.tracks.iter().enumerate() {
            for (ci, c) in t.clips.iter().enumerate() {
                if c.id == clip_id {
                    return Some((ti, ci));
                }
            }
        }
        None
    }

    pub fn find_track(&self, track_id: &str) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == track_id)
    }

    /// 片段在时间轴上的终点(ms)。
    pub fn clip_end(p: &Clip) -> u64 {
        p.start_ms.saturating_add(p.duration_ms)
    }

    /// 同轨道时间重叠检测(剪辑不变量;破坏即 CF-004 的前提)。
    pub fn overlaps(track: &Track) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for i in 0..track.clips.len() {
            for j in (i + 1)..track.clips.len() {
                let a = &track.clips[i];
                let b = &track.clips[j];
                if a.start_ms < Self::clip_end(b) && b.start_ms < Self::clip_end(a) {
                    pairs.push((a.id.clone(), b.id.clone()));
                }
            }
        }
        pairs
    }

    /// 下一个可用片段 id:<轨道id>-<序号三位零填>。
    pub fn next_clip_id(track: &Track) -> String {
        let mut n = track.clips.len() + 1;
        loop {
            let id = format!("{}-{n:03}", track.id);
            if !track.clips.iter().any(|c| c.id == id) {
                return id;
            }
            n += 1;
        }
    }

    /// 轨道确定性 id:<kind 首字母大写><序号>。
    pub fn track_letter(kind: TrackKind) -> char {
        match kind {
            TrackKind::Video => 'V',
            TrackKind::Audio => 'A',
            TrackKind::Text => 'T',
        }
    }

    pub fn next_track_id(&self, kind: TrackKind) -> String {
        let letter = Self::track_letter(kind);
        let mut n = 1;
        loop {
            let id = format!("{letter}{n}");
            if !self.tracks.iter().any(|t| t.id == id) {
                return id;
            }
            n += 1;
        }
    }
}

/// v1→v2 迁移入口(委托 schema crate 的迁移器,再过一遍类型化模型)。
pub fn migrate_from_value(v: &Value) -> Result<Project, Vec<String>> {
    let migrated = cutforge_schema::migrate_project(v);
    Project::from_value(&migrated)
}

/// 时间线恒速段(timeline 恒速段;册四 A4 T4.4 的**单一真相源**,册五 T5.1 扩
/// speed 关键帧入口):
/// `(start_ms, end_ms, mean_speed)` 三元组,`mean_speed` 为该段渲染用的常速
/// (区间两端点速度的算术平均 = 线性插值 speed 函数在区间上的积分均值)。
///
/// 口径(投影与渲染共用本函数,红线 = 两边时长严格一致):
/// - **speed 关键帧优先**(IR v3):clip.keyframes 含 speed 属性时,先经
///   [`crate::keyframes::speed_keyframes_to_curve`] 合成为曲线(非线性缓动区间
///   按求值器细分逼近),此后与本条 speedCurve 口径完全一致(与 speedCurve 互斥
///   由校验器保证,此处不再判);
/// - 无 speedCurve → 单段 `(0, duration_ms, speed.unwrap_or(1.0))`,与既有线性 speed 完全同形;
/// - 有 speedCurve → 点按 atMs 升序(防御性排序),首点速度前延到 0、末点速度后延到
///   durationMs(端点常速外延);相邻点之间 speed 函数线性插值,渲染按区间
///   **均值常速**执行(每段一个 setpts,总时长与总源消耗都是分段积分的精确值);
/// - 单点曲线 ≡ 常速;atMs 超出 [0,durationMs] 的点被钳到边界后并段。
pub fn speed_segments(clip: &Clip) -> Vec<(u64, u64, f64)> {
    let dur = clip.duration_ms;
    if dur == 0 {
        return Vec::new();
    }
    // speed 关键帧(IR v3):合成曲线后与 speedCurve 同路径(B 级分段常速逼近)
    let speed_kf_curve = clip.keyframes.as_ref().and_then(|kfs| {
        let pts: Vec<crate::keyframes::Keyframe> =
            kfs.iter().filter(|k| k.property == "speed").cloned().collect();
        if pts.is_empty() {
            None
        } else {
            Some(crate::keyframes::speed_keyframes_to_curve(&pts))
        }
    });
    let points = speed_kf_curve.as_ref().or(clip.speed_curve.as_ref());
    let Some(points) = points else {
        return vec![(0, dur, clip.speed.unwrap_or(1.0))];
    };
    // 防御性归一:排序(乱序输入),钳到 [0, dur](越界点钳边)
    let mut pts: Vec<(u64, f64)> = points.iter().map(|p| (p.at_ms.min(dur), p.speed)).collect();
    pts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    if pts.is_empty() {
        return vec![(0, dur, clip.speed.unwrap_or(1.0))];
    }
    // 端点外延成完整覆盖 [0, dur] 的节点序列:0→首点速度,dur→末点速度
    let mut nodes: Vec<(u64, f64)> = Vec::with_capacity(pts.len() + 2);
    if pts[0].0 > 0 {
        nodes.push((0, pts[0].1));
    }
    nodes.extend(pts);
    if nodes.last().map(|(t, _)| *t).unwrap_or(0) < dur {
        let s = nodes.last().map(|(_, s)| *s).unwrap_or(1.0);
        nodes.push((dur, s));
    }
    // 相邻节点成段;区间速度 = 线性插值 → 常速渲染取区间均值(积分精确)
    let mut segs: Vec<(u64, u64, f64)> = Vec::new();
    for w in nodes.windows(2) {
        let (a, sa) = w[0];
        let (b, sb) = w[1];
        if b <= a {
            continue; // 零长段(重复 atMs)跳过
        }
        let mean = (sa + sb) / 2.0;
        match segs.last_mut() {
            // 均值相等的相邻段并段(等速点不产生多余 setpts)
            Some(last) if (last.2 - mean).abs() < f64::EPSILON => last.1 = b,
            _ => segs.push((a, b, mean)),
        }
    }
    segs
}

/// 片段的源域读取时长(ms,f64;调用方决定取整)= ∫ speed dt 的分段积分。
/// 无曲线 = durationMs × speed(与既有语义逐位一致);freezeMs 定格在调用方裁剪。
pub fn source_read_ms(clip: &Clip) -> f64 {
    speed_segments(clip)
        .iter()
        .map(|(a, b, s)| (*b - *a) as f64 * s)
        .sum()
}

#[cfg(test)]
#[path = "fx_roundtrip_tests.rs"]
mod fx_roundtrip_tests; // 册四 T4.6 fx 字段 roundtrip(纯移动拆分,行数红线 A1-3)

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "version": 1, "schemaVersion": "2.0.0",
            "slug": "测试工程", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "backends": ["ffmpeg", "cutforge"],
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 8400,
                     "sourceInMs": 12000, "role": "voice"},
                    {"id": "V1-002", "src": "a.mp4", "startMs": 8400, "durationMs": 6200,
                     "transition": {"type": "fade", "durMs": 300, "reason": "topic"}}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 8400, "durationMs": 400,
                     "role": "sfx", "volume": 0.8}
                ]}
            ]
        })
    }

    #[test]
    fn parse_and_validate_sample() {
        let v = sample();
        let p = Project::from_value(&v).expect("样本必须通过 v2 校验");
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(p.find_clip("V1-002"), Some((0, 1)));
        assert_eq!(p.find_clip("不存在"), None);
        assert_eq!(p.find_track("A1"), Some(1));
    }

    #[test]
    fn roundtrip_value_preserves_semantics() {
        let v = sample();
        let p = Project::from_value(&v).unwrap();
        let back = p.to_validated_value().unwrap();
        let a: Project = serde_json::from_value(back).unwrap();
        assert_eq!(p, a);
    }

    #[test]
    fn rejects_schema_violation() {
        let mut v = sample();
        v["fps"] = json!(90); // 不在允许集
        assert!(Project::from_value(&v).is_err());
        v["fps"] = json!(30);
        v["幽灵字段"] = json!(1); // additionalProperties=false
        let errs = Project::from_value(&v).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("幽灵字段")));
    }

    #[test]
    fn migrate_v1_sample() {
        let mut v = sample();
        // 去掉 v2 字段并剥掉 id,构造 v1 形态
        v.as_object_mut().unwrap().remove("schemaVersion");
        v.as_object_mut().unwrap().remove("backends");
        v["tracks"][0].as_object_mut().unwrap().remove("id");
        v["tracks"][0]["clips"][0].as_object_mut().unwrap().remove("id");
        let p = migrate_from_value(&v).expect("迁移后必须合法");
        assert_eq!(p.schema_version, "3.0.0", "v1 迁移目标随 IR v3(T5.1)");
        assert_eq!(p.tracks[0].id, "V1");
        assert_eq!(p.tracks[0].clips[0].id, "V1-001");
        assert_eq!(p.backends, vec![Backend::Ffmpeg]);
        // 迁移幂等
        let v2 = p.to_validated_value().unwrap();
        let p2 = migrate_from_value(&v2).unwrap();
        assert_eq!(p, p2);
    }

    /// speed_segments:投影与渲染共用的段划分单一真相源(T4.4 红线的根基)。
    #[test]
    fn speed_segments_piecewise_semantics() {
        let mk = |v: Value| -> Clip { serde_json::from_value(v).unwrap() };
        // 无曲线:单段,线性 speed 回退
        let c = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 4000, "speed": 2.0}));
        assert_eq!(speed_segments(&c), vec![(0, 4000, 2.0)]);
        assert_eq!(source_read_ms(&c), 8000.0, "无曲线 = durationMs × speed(既有语义逐位一致)");
        // 无曲线无 speed:1.0
        let c = mk(json!({"id": "V1-001", "startMs": 0, "durationMs": 4000}));
        assert_eq!(speed_segments(&c), vec![(0, 4000, 1.0)]);
        // 两段曲线:乱序输入被排序;0 点在集 → 无前延;末点速度后延到 durationMs
        // [0,1s) 为 0.5→2.0 的线性插值区间 → 均值 1.25;[1s,2s) 后延恒速 2.0
        let c = mk(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000, "src": "a.mp4",
            "speedCurve": [{"atMs": 1000, "speed": 2.0}, {"atMs": 0, "speed": 0.5}]
        }));
        assert_eq!(speed_segments(&c), vec![(0, 1000, 1.25), (1000, 2000, 2.0)]);
        assert_eq!(source_read_ms(&c), 1250.0 + 2000.0, "分段积分 ΣΔt×均值速度");
        // 线性插值区间:0→1s 速度 0.5→1.5,区间均值 1.0(积分均值=常速渲染值)
        let c = mk(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000, "src": "a.mp4",
            "speedCurve": [{"atMs": 0, "speed": 0.5}, {"atMs": 1000, "speed": 1.5}]
        }));
        assert_eq!(speed_segments(&c), vec![(0, 1000, 1.0), (1000, 2000, 1.5)],
            "区间渲染速度 = 两端点均值(线性插值的精确积分)");
        assert_eq!(source_read_ms(&c), 1000.0 * 1.0 + 1000.0 * 1.5);
        // 等速相邻点并段;首点不在 0 → 首速前延到 0
        let c = mk(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 3000, "src": "a.mp4",
            "speedCurve": [{"atMs": 1000, "speed": 2.0}]
        }));
        assert_eq!(speed_segments(&c), vec![(0, 3000, 2.0)], "等速段并段+端速外延");
        // 越界 atMs 钳到 durationMs
        let c = mk(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000, "src": "a.mp4",
            "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 9999, "speed": 4.0}]
        }));
        assert_eq!(speed_segments(&c), vec![(0, 2000, 2.5)], "越界点钳边,区间均值");
    }
}
