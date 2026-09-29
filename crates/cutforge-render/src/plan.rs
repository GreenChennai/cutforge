// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! RenderPlan(T1.4):渲染计划 = 工程快照 + 选项 → 收集好的段清单 + 显式步骤列表。
//! 计划构建是纯函数(不碰 ffmpeg、不碰文件系统写入);每个渲染步骤由
//! steps.rs 的纯函数生成命令行、lib.rs 的执行器调用,产出结构化 StepReport。

use cutforge_core::model::{Bgm, Clip, Project, Role, TrackKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// 混音段:一个需要进入总线的音频事件(人声段或音效),带源域裁剪与成片域落点。
/// 册四 A4 T4.4 起一个 clip 可按 speedCurve 拆为多个恒速子段(每子段一个本结构),
/// 段间时间线落点无缝(adelay 按子段起点);reverse 为子段级倒放(areverse)。
#[derive(Debug, Clone)]
pub struct AudioSeg {
    pub src: PathBuf,
    /// 成片域落点(ms)——adelay 的唯一来源(P0-4 修复前人声全部 0 秒起播)。
    pub start_ms: u64,
    pub duration_ms: u64,
    /// 源域入点(ms)——per-clip 裁剪,-ss 输入侧。
    pub source_in_ms: u64,
    pub volume: f64,
    pub speed: f64,
    /// 倒放(册四 A4 T4.4):areverse 先于 atempo(reverse 先于变速)。
    pub reverse: bool,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
}

/// 叠加段:品牌/花字位图,绝对像素 + opacity + 时间窗。
#[derive(Debug, Clone)]
pub struct OverlaySeg {
    pub src: PathBuf,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub spec: cutforge_core::model::Overlay,
}

/// 渲染步骤的显式名单(顺序即执行顺序;外部进度事件的 step 名以此为准,
/// 与既有字符串接口逐字对齐,不得擅改)。
pub const STEP_NAMES: [&str; 7] =
    ["probe", "segment", "compose-video", "overlay", "mix", "subtitle", "encode"];

/// 结构化步骤报告(T1.4):每步执行完产出一份;进度事件由它单源派生,
/// 保证 cutforge-render stdout 的 JSON 行接口向后兼容(键名与拆分前逐字一致)。
#[derive(Debug, Clone)]
pub struct StepReport {
    pub name: &'static str,
    pub ok: bool,
    /// 该步骤是否命中了内容寻址缓存(seg/compose/overlay/mix/sub;probe/encode 恒 false)。
    pub cache_hit: bool,
    /// 步骤专属结构化字段(cacheHits/segments/count/burned/output…),
    /// 原样并入进度事件顶层(与拆分前的事件形状一致)。
    pub detail: Value,
}

impl StepReport {
    pub fn new(name: &'static str, detail: Value) -> Self {
        StepReport { name, ok: true, cache_hit: false, detail }
    }

    pub fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache_hit = hit;
        self
    }

    /// 派生进度事件:{"step": name, "ok": ok, **detail}。
    /// 这是 stdout JSON 行的唯一来源(mcp progress.rs 透传本行,接口不变)。
    pub fn to_progress(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("step".into(), Value::String(self.name.to_string()));
        obj.insert("ok".into(), Value::Bool(self.ok));
        if let Value::Object(rest) = &self.detail {
            for (k, v) in rest {
                obj.insert(k.clone(), v.clone());
            }
        }
        Value::Object(obj)
    }
}

/// 渲染计划:一次 render() 的全部输入,经收集后显式成形。
/// 步骤函数(steps.rs)只消费本结构,不再回头翻工程模型。
#[derive(Debug, Clone)]
pub struct RenderPlan {
    pub project_dir: PathBuf,
    /// 成片输出目录(目录契约 0.5:06_成片输出;0.4.x 旧布局原地写 06_output,不迁移)。
    pub out_dir: PathBuf,
    /// 渲染缓存根(.cutforge/render-cache;分层见 cache.rs)。
    pub cache_dir: PathBuf,
    pub canvas_w: u32,
    pub canvas_h: u32,
    pub fps: u32,
    pub slug: String,
    /// 背景音乐(ducking/gain 在 mix 步消费;画幅无关)。
    pub bgm: Option<Bgm>,
    /// 烧录用 ASS 字幕(最后叠;None = 不烧录,仅转封装合流)。
    pub ass_path: Option<PathBuf>,
    /// concat/xfade 主时间线视频段(overlay 叠加层的 clip 不占此序列)。
    pub video_clips: Vec<Clip>,
    /// 进入混音总线的音频事件(人声段 + 音效;volume=0 不进)。
    pub audio_segs: Vec<AudioSeg>,
    pub overlay_segs: Vec<OverlaySeg>,
    /// 全片时长上界(ms)= max(startMs+durationMs),混音总长与 -t 的来源。
    pub total_ms: u64,
}

impl RenderPlan {
    /// 从工程快照构建计划(纯函数:只读遍历,无 IO)。
    pub fn build(project: &Project, project_dir: &Path, ass_path: Option<&Path>) -> RenderPlan {
        let out_dir = if cutforge_io::paths::is_legacy_layout(project_dir) {
            project_dir.join(cutforge_io::paths::LEGACY_OUTPUT)
        } else {
            project_dir.join(cutforge_io::paths::OUTPUT)
        };
        let mut plan = RenderPlan {
            project_dir: project_dir.to_path_buf(),
            out_dir,
            cache_dir: project_dir.join(crate::cache::CACHE_ROOT),
            canvas_w: project.canvas.width,
            canvas_h: project.canvas.height,
            fps: project.fps,
            slug: project.slug.clone(),
            bgm: project.bgm.clone(),
            ass_path: ass_path.map(Path::to_path_buf),
            video_clips: Vec::new(),
            audio_segs: Vec::new(),
            overlay_segs: Vec::new(),
            total_ms: 0,
        };
        for t in &project.tracks {
            for c in &t.clips {
                plan.total_ms = plan.total_ms.max(c.start_ms + c.duration_ms);
                match t.kind {
                    TrackKind::Video => {
                        // overlay 字段的 clip 是**叠加层**(rs_brand 变体轨口径),
                        // 不占用主时间线 concat 序列,由 overlay 步合成
                        if let Some(ov) = c.overlay {
                            if let Some(src) = &c.src {
                                plan.overlay_segs.push(OverlaySeg {
                                    src: project_dir.join(src),
                                    start_ms: c.start_ms,
                                    duration_ms: c.duration_ms,
                                    spec: ov,
                                });
                            }
                            continue;
                        }
                        plan.video_clips.push(c.clone());
                        if clip_gain(c) > 0.0 {
                            plan.audio_segs.extend(audio_segs_of(project_dir, c));
                        }
                    }
                    TrackKind::Audio => {
                        if clip_gain(c) > 0.0 {
                            plan.audio_segs.extend(audio_segs_of(project_dir, c));
                        }
                    }
                    // 文本轨:结构性锚点(字幕经 ASS 烧录链);渲染端不消费,与 CutFlow 同口径
                    TrackKind::Text => {}
                }
            }
        }
        plan
    }
}

/// 音量语义(BUGFIX E5 实测):volume=None = 未设置 = 自然音量(人声 1.0 / sfx 0.8),
/// **显式 0 才是静音**。此前 None 按 0.0 处理 → 真实 CutFlow IR(clip 不带 volume)
/// 整片无声,全静音混音又令 loudnorm 测得 -inf、linear=true 应用时 ffmpeg 崩
/// ("Result too large")。parity 夹具此前总是显式写 volume,故矩阵未暴露。
pub fn clip_gain(c: &Clip) -> f64 {
    c.volume.unwrap_or(if c.role == Some(Role::Sfx) { 0.8 } else { 1.0 })
}

/// 定格截断后的播放毫秒(册四 T4.4 组合语义):freezeMs 有值时源只播放到
/// min(freezeMs, durationMs),其后由 tpad 克隆尾帧补足到 durationMs(视频)/静音(音频)。
pub fn clip_play_ms(c: &Clip) -> u64 {
    match c.freeze_ms {
        Some(f) => c.duration_ms.min(f),
        None => c.duration_ms,
    }
}

/// 一个 clip 的混音事件序列(册四 A4 T4.4):无曲线 = 单事件(与既有语义逐位一致);
/// 有 speedCurve = 按恒速段拆分为多事件,各自 -ss/-t/atempo/adelay,时间线无缝拼接;
/// 倒放逐子段 areverse(全局倒放 = 分窗倒放的逐窗内容,两种切法内容逐帧一致,
/// 故音频按子段独立 -ss/-t+areverse 与视频整段 reverse+分段消费完全同构);
/// freezeMs 定格:定格点之后的曲线段不产生音频事件(画面冻结、声音静默)。
fn audio_segs_of(project_dir: &Path, c: &Clip) -> Vec<AudioSeg> {
    let play_end = clip_play_ms(c);
    let segs = cutforge_core::model::speed_segments(c);
    let mut out: Vec<AudioSeg> = Vec::new();
    // 源域累计偏移(ms,f64 累计、落点取整;整数速度与整毫秒段下逐位精确)
    let mut x = 0f64;
    let gain = clip_gain(c);
    let fade_in = c.fade.as_ref().map(|f| f.in_ms).unwrap_or(0.0);
    let fade_out = c.fade.as_ref().map(|f| f.out_ms).unwrap_or(0.0);
    for (a, b, s) in segs {
        if a >= play_end {
            break; // 定格点之后的段:源不播放
        }
        let end = b.min(play_end);
        let span = (end - a) as f64;
        let first = out.is_empty();
        out.push(AudioSeg {
            src: project_dir.join(c.src.clone().unwrap_or_default()),
            start_ms: c.start_ms + a,
            duration_ms: end - a,
            source_in_ms: c.source_in_ms.unwrap_or(0).saturating_add(x.round() as u64),
            volume: gain,
            speed: s,
            reverse: c.reverse.unwrap_or(false),
            fade_in_ms: if first { fade_in } else { 0.0 },
            fade_out_ms: if end >= play_end { fade_out } else { 0.0 },
        });
        x += span * s;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn project(v: Value) -> Project {
        serde_json::from_value(v).unwrap()
    }

    fn base_project() -> Value {
        json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "p", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                     "sourceInMs": 100, "role": "voice", "volume": 1.0},
                    {"id": "V1-002", "src": "a.mp4", "startMs": 2000, "durationMs": 1000,
                     "role": "voice"},
                    {"id": "V2-001", "src": "logo.png", "startMs": 0, "durationMs": 2000,
                     "volume": 0, "overlay": {"x": 1, "y": 2, "w": 3, "h": 4, "opacity": 1.0}}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 500, "durationMs": 500,
                     "role": "sfx", "volume": 0.9}
                ]},
                {"id": "T1", "kind": "text", "clips": [
                    {"id": "T1-001", "startMs": 0, "durationMs": 1000, "text": "x"}
                ]}
            ]
        })
    }

    #[test]
    fn plan_collects_video_audio_overlay_disjointly() {
        let p = project(base_project());
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        // overlay 叠加层不占主时间线
        assert_eq!(plan.video_clips.len(), 2);
        assert_eq!(plan.overlay_segs.len(), 1);
        // 主线两段(音量>0)+ 音频轨一段
        assert_eq!(plan.audio_segs.len(), 3);
        assert_eq!(plan.total_ms, 3000);
        assert_eq!(plan.canvas_w, 1080);
        assert_eq!(plan.fps, 30);
        assert_eq!(plan.out_dir, Path::new("/w").join("06_成片输出"));
        assert_eq!(plan.cache_dir, Path::new("/w").join(".cutforge/render-cache"));
    }

    #[test]
    fn clip_gain_none_means_natural_not_silent() {
        let p = project(base_project());
        let c0 = &p.tracks[0].clips[0]; // volume=1.0
        let c1 = &p.tracks[0].clips[1]; // volume=None,role=voice
        assert_eq!(clip_gain(c0), 1.0);
        assert_eq!(clip_gain(c1), 1.0, "None=自然音量,不是静音");
        let sfx = &p.tracks[1].clips[0];
        assert_eq!(clip_gain(sfx), 0.9, "显式值优先于角色缺省");
    }

    #[test]
    fn audio_seg_defaults_from_clip() {
        let p = project(base_project());
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        let seg = &plan.audio_segs[1]; // V1-002:volume/speed/fade 全缺省
        assert_eq!(seg.volume, 1.0);
        assert_eq!(seg.speed, 1.0);
        assert_eq!(seg.source_in_ms, 0);
        assert_eq!(seg.fade_in_ms, 0.0);
        assert!(!seg.reverse, "无 reverse 字段 = 不倒放");
        assert_eq!(plan.overlay_segs[0].spec.x, 1);
    }

    /// 册四 A4 T4.4:speedCurve 把一个 clip 的混音拆为多子段(时间线落点无缝,
    /// 源域累计偏移 = 分段积分);reverse 逐子段继承;既有无曲线工程逐位同形。
    #[test]
    fn speed_curve_splits_audio_segs_and_reverse_inherits() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "curve", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 500, "durationMs": 2000,
                 "sourceInMs": 100, "role": "voice", "volume": 1.0,
                 "speedCurve": [
                    {"atMs": 0, "speed": 1.0}, {"atMs": 1000, "speed": 1.0},
                    {"atMs": 2000, "speed": 3.0}
                 ],
                 "reverse": true, "fade": {"inMs": 200, "outMs": 400}}
            ]}]
        }));
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        assert_eq!(plan.audio_segs.len(), 2, "两段曲线 → 两个混音子段");
        let (s0, s1) = (&plan.audio_segs[0], &plan.audio_segs[1]);
        // 段 1:[0,1000) 均值 (1+1)/2=1.0,源累计 [0,1000)
        assert_eq!((s0.start_ms, s0.duration_ms, s0.speed), (500, 1000, 1.0));
        assert_eq!(s0.source_in_ms, 100);
        assert!(s0.reverse, "倒放逐子段继承");
        assert_eq!(s0.fade_in_ms, 200.0, "淡入只在首子段");
        assert_eq!(s0.fade_out_ms, 0.0);
        // 段 2:[1000,2000) 均值 (1+3)/2=2.0,源累计 [1000,1000+1000×2)
        assert_eq!((s1.start_ms, s1.duration_ms, s1.speed), (1500, 1000, 2.0));
        assert_eq!(s1.source_in_ms, 100 + 1000, "源域偏移 = 前段积分 ΣΔt×speed");
        assert_eq!(s1.fade_out_ms, 400.0, "淡出只在末子段");
        assert!(s1.reverse, "末子段同样继承倒放");
    }

    /// 册四 T4.4 组合语义:freezeMs 定格点之后的曲线段不产音频事件(画面冻结/声音静默)。
    #[test]
    fn freeze_clips_audio_segs_at_freeze_point() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "freeze", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 3000,
                 "sourceInMs": 0, "role": "voice", "volume": 1.0, "freezeMs": 1200,
                 "fade": {"inMs": 100, "outMs": 300},
                 "speedCurve": [{"atMs": 0, "speed": 1.0}, {"atMs": 2000, "speed": 2.0}]
                }
            ]}]
        }));
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        // play=[0,1200):曲线在 2000ms 才拐,段 [0,3000) 均速 1.5 → 截断到 [0,1200)
        assert_eq!(plan.audio_segs.len(), 1);
        let s = &plan.audio_segs[0];
        assert_eq!((s.start_ms, s.duration_ms), (0, 1200));
        assert_eq!(s.fade_out_ms, 300.0, "截断段即播放末段 → 淡出挂定格点");
    }

    #[test]
    fn step_report_progress_shape_is_backward_compatible() {
        let r = StepReport::new(
            "segment",
            json!({"cacheHits": 2, "segments": 5}),
        )
        .with_cache_hit(false);
        assert_eq!(
            r.to_progress(),
            json!({"step": "segment", "ok": true, "cacheHits": 2, "segments": 5})
        );
        let probe = StepReport::new("probe", json!({}));
        assert_eq!(probe.to_progress(), json!({"step": "probe", "ok": true}));
    }

    #[test]
    fn step_names_are_the_seven_stage_pipeline() {
        assert_eq!(
            STEP_NAMES,
            ["probe", "segment", "compose-video", "overlay", "mix", "subtitle", "encode"]
        );
    }
}
