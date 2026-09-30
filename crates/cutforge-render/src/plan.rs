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
    /// 降噪档(册四 T4.8):None/off = 不降噪;low/mid/high → afftdn 参数。
    pub denoise: Option<String>,
    /// 变调倍率(册四 T4.8):2^(半音/12);1.0 = 不变调。
    pub pitch: f64,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
    /// volume 关键帧表达式(IR v3,T5.1):`volume='…':eval=frame` 完整滤镜串
    /// (播放域子段局部 t,锚点已按子段起点平移;kf_expr 单源编译)。
    /// 有值时代替静态 volume(关键帧优先,ADR-0018);afade 仍在其后叠加。
    pub volume_expr: Option<String>,
    /// 所属视频片段下标(册四 T4.5 acrossfade 链的分组键;None = 音频轨事件,
    /// 恒走绝对落点 adelay,不经链)。
    pub clip_idx: Option<usize>,
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
    /// 主时间线相邻视频片段边界的有效转场时长(册四 T4.5;len = video_clips-1,
    /// 硬切边界 = 0)。非空即 acrossfade 音频链模式;与 mix 缓存键绑定。
    pub boundary_durs_ms: Vec<f64>,
    /// 全片时长上界(ms)= 过滤后视音轨的 max(startMs+durationMs),混音总长与
    /// -t 的来源(hidden/mute 片段不再撑长时间线;文本轨不计入)。
    pub total_ms: u64,
    /// 代理预览开关(册四 T4.1):true 时 build 已把存在代理的素材 src 换写为
    /// 代理路径(显式 opt-in,不悄悄降质;缺代理的片段回落原片)。
    pub use_proxy: bool,
}

impl RenderPlan {
    /// 从工程快照构建计划(纯函数:只读遍历,无 IO;代理关闭,整片/单帧共用)。
    pub fn build(project: &Project, project_dir: &Path, ass_path: Option<&Path>) -> RenderPlan {
        Self::build_opts(project, project_dir, ass_path, false)
    }

    /// 同 [`RenderPlan::build`],代理预览开关显式给定(册四 T4.1)。
    /// 代理替换在收集前完成:src 换写进 clip 副本 → seg/mix 键自动分叉
    /// (代理渲染与原片渲染不共享缓存条目)。
    pub fn build_opts(project: &Project, project_dir: &Path, ass_path: Option<&Path>, use_proxy: bool) -> RenderPlan {
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
            boundary_durs_ms: Vec::new(),
            total_ms: 0,
            use_proxy,
        };
        // 代理替换(use_proxy 且代理文件在位):对片段换 src(声画同源)。
        // 代理缺失回落原片——opt-in 预览语义下回落是升格而非降质,不告警。
        let mut owned = project.clone();
        if use_proxy {
            swap_to_proxies(&mut owned, project_dir);
        }
        let project = &owned;
        let solo_active = any_solo(project);
        for t in &project.tracks {
            // 轨道级渲染联动(册四 BE3b 收口,BE1 欠账):
            // - solo 只作用**音频面**:任一轨 solo 活跃时,非 solo 轨的音频不进混音;
            // - mute 只作用**音频面**:静音轨的音频不进混音;
            // - hidden 只作用**视觉面**:视频轨 hidden → 整轨(含叠加层)不进合成;
            //   文本轨的 mute/hidden(文本静默)由 textass::text_tracks 过滤。
            // 缓存正确性:过滤发生在 plan 收集前 → video_clips/audio_segs 派生的
            // seg/compose/mix 键自动变化,无需另设键维度。
            let track_muted = t.mute.unwrap_or(false);
            let track_solo = t.solo.unwrap_or(false);
            let audio_ok = !track_muted && (!solo_active || track_solo);
            let track_hidden = t.hidden.unwrap_or(false);
            for c in &t.clips {
                match t.kind {
                    TrackKind::Video => {
                        // overlay 字段的 clip 是**叠加层**(rs_brand 变体轨口径),
                        // 不占用主时间线 concat 序列,由 overlay 步合成(不产音频事件)
                        if let Some(ov) = c.overlay {
                            if !track_hidden
                                && let Some(src) = &c.src
                            {
                                plan.overlay_segs.push(OverlaySeg {
                                    src: project_dir.join(src),
                                    start_ms: c.start_ms,
                                    duration_ms: c.duration_ms,
                                    spec: ov,
                                });
                            }
                            continue;
                        }
                        if !track_hidden {
                            let idx = plan.video_clips.len();
                            plan.video_clips.push(c.clone());
                            if audio_ok && clip_gain(c) > 0.0 {
                                plan.audio_segs
                                    .extend(audio_segs_of(project_dir, c).into_iter().map(|mut s| {
                                        s.clip_idx = Some(idx);
                                        s
                                    }));
                            }
                        } else if audio_ok && clip_gain(c) > 0.0 {
                            // hidden 只作用视觉面(册四 BE3b 定义):画面不进合成,声音仍在
                            plan.audio_segs.extend(audio_segs_of(project_dir, c));
                        }
                    }
                    TrackKind::Audio => {
                        if audio_ok && clip_gain(c) > 0.0 {
                            plan.audio_segs.extend(audio_segs_of(project_dir, c));
                        }
                    }
                    // 文本轨:结构性锚点(字幕经 textass 生成 ASS 烧录链,ADR-0016);
                    // mute/hidden(文本静默)在 textass::text_tracks 过滤
                    TrackKind::Text => {}
                }
            }
        }
        // 时长上界按**过滤后**的视音轨重算(hidden/mute 的片段不再撑长时间线;
        // 无轨道标志的工程与既有行为一致——文本轨此前计入上界的病态口径一并修正)
        plan.total_ms = plan
            .video_clips
            .iter()
            .map(|c| c.start_ms + c.duration_ms)
            .chain(plan.audio_segs.iter().map(|s| s.start_ms + s.duration_ms))
            .max()
            .unwrap_or(0);
        // 边界有效转场时长(册四 T4.5 acrossfade 口径;与 segment 尾帧同一钳制函数)
        plan.boundary_durs_ms = (1..plan.video_clips.len())
            .map(|i| crate::catalog::effective_transition_ms(&plan.video_clips, i))
            .collect();
        plan
    }
}

/// solo 语义的判定前提:任一音频承载轨(video/audio)声明了 solo。
fn any_solo(project: &Project) -> bool {
    project.tracks.iter().any(|t| {
        (t.kind == TrackKind::Video || t.kind == TrackKind::Audio) && t.solo == Some(true)
    })
}

/// 代理替换(册四 T4.1):素材存在内容寻址代理(.cutforge/proxy/<key>.mp4,
/// key = 路径+mtime+size)时把 clip.src 换写为代理相对路径;改素材 → 键变 →
/// miss → 回落原片(自愈,不告警)。返回被换写的片段 id 清单(调试用)。
pub fn swap_to_proxies(project: &mut Project, project_dir: &Path) -> Vec<String> {
    let mut swapped = Vec::new();
    for t in &mut project.tracks {
        if t.kind != TrackKind::Video {
            continue;
        }
        for c in &mut t.clips {
            let Some(src) = &c.src else { continue };
            if c.overlay.is_some() {
                continue; // 叠加层(品牌位图等)通常非视频素材,保持原样
            }
            let Some(proxy_rel) =
                cutforge_io::mediacache::proxy_lookup(project_dir, src)
            else {
                continue;
            };
            c.src = Some(proxy_rel);
            swapped.push(c.id.clone());
        }
    }
    swapped
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
            denoise: c.denoise.clone().filter(|d| d != "off"),
            pitch: crate::across::pitch_factor(c.pitch.unwrap_or(0.0)),
            fade_in_ms: if first { fade_in } else { 0.0 },
            fade_out_ms: if end >= play_end { fade_out } else { 0.0 },
            // volume 关键帧(IR v3):表达式按子段起点平移(a = 片段播放域偏移);
            // 无 volume 关键帧 → None(静态 volume 生效,与既有语义逐位一致)
            volume_expr: crate::kf_expr::kf_volume_filter(c, a),
            clip_idx: None, // 归属由 RenderPlan::build 按轨道回填
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

    // ---- 册四 BE3b:track mute/solo/hidden 渲染联动收口 ----

    /// mute 只作用音频面:静音轨的片段不进混音;视觉与时长不受影响。
    /// 平台一致的路径期望值(join 分隔符随平台变化)。
    fn pj(base: &str, rel: &str) -> String {
        Path::new(base).join(rel).to_string_lossy().into_owned()
    }

    #[test]
    fn track_mute_excludes_audio_from_mix() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice", "volume": 1.0}
                ]},
                {"id": "A1", "kind": "audio", "mute": true, "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 500, "durationMs": 500, "volume": 0.9}
                ]}
            ]
        }));
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        assert_eq!(plan.audio_segs.len(), 1, "mute 轨的音频事件不进混音(仅 V1 的人声)");
        assert_eq!(plan.audio_segs[0].src.to_string_lossy(), pj("/w", "a.mp4"));
        assert_eq!(plan.video_clips.len(), 1, "mute 不影响视觉面");
        assert_eq!(plan.total_ms, 2000);
        // mix 缓存键随 mute 变化(改 flag 即 miss 的机械保证)
        let unmuted = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "m", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice", "volume": 1.0}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 500, "durationMs": 500, "volume": 0.9}
                ]}
            ]
        }));
        assert_ne!(
            crate::cache::mix_key(&plan),
            crate::cache::mix_key(&RenderPlan::build(&unmuted, Path::new("/w"), None)),
            "mute 状态必须入 mix 键"
        );
    }

    /// solo 只作用音频面:任一轨 solo 活跃时,非 solo 轨静音;solo 轨正常。
    #[test]
    fn track_solo_silences_non_solo_tracks() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "s", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice", "volume": 1.0}
                ]},
                {"id": "A1", "kind": "audio", "solo": true, "clips": [
                    {"id": "A1-001", "src": "sfx.mp3", "startMs": 500, "durationMs": 500, "volume": 0.9}
                ]},
                {"id": "A2", "kind": "audio", "clips": [
                    {"id": "A2-001", "src": "bgm.mp3", "startMs": 0, "durationMs": 1000, "volume": 0.5}
                ]}
            ]
        }));
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        let srcs: Vec<&std::path::Path> = plan.audio_segs.iter().map(|s| s.src.as_path()).collect();
        assert!(srcs.contains(&std::path::Path::new(&pj("/w", "sfx.mp3"))), "solo 轨出声: {srcs:?}");
        assert!(!srcs.contains(&std::path::Path::new(&pj("/w", "bgm.mp3"))), "非 solo 轨静音: {srcs:?}");
        assert!(!srcs.contains(&std::path::Path::new(&pj("/w", "a.mp4"))), "非 solo 视频轨的音频同样静音");
        assert_eq!(plan.video_clips.len(), 1, "solo 不影响视觉面");
    }

    /// hidden 只作用视觉面:隐藏视频轨整轨不进合成;其音频仍进混音。
    #[test]
    fn track_hidden_excludes_video_not_audio() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "h", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "hidden": true, "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "role": "voice", "volume": 1.0}
                ]},
                {"id": "V2", "kind": "video", "clips": [
                    {"id": "V2-001", "src": "b.mp4", "startMs": 0, "durationMs": 1500, "volume": 0}
                ]}
            ]
        }));
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        assert_eq!(plan.video_clips.len(), 1, "hidden 轨不进合成");
        assert_eq!(plan.video_clips[0].id, "V2-001");
        assert_eq!(plan.audio_segs.len(), 1, "hidden 轨的音频仍进混音(hidden 只作用视觉)");
        assert_eq!(plan.audio_segs[0].src.to_string_lossy(), pj("/w", "a.mp4"));
        assert_eq!(plan.total_ms, 2000, "时长上界按过滤后重算(隐藏片段不撑长)");
    }

    /// 册四 T4.8:clip 级 denoise/pitch 进入混音段(链消费入口)并入 mix 键。
    #[test]
    fn denoise_and_pitch_flow_into_audio_segs() {
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "d", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                 "role": "voice", "volume": 1.0, "denoise": "mid", "pitch": 3}
            ]}]
        }));
        let plan = RenderPlan::build(&p, Path::new("/w"), None);
        let s = &plan.audio_segs[0];
        assert_eq!(s.denoise.as_deref(), Some("mid"));
        assert!((s.pitch - 2f64.powf(3.0 / 12.0)).abs() < 1e-9, "pitch = 2^(3/12)");
        // off 档 → None(不产滤镜)
        let p2 = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "d", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [{"id": "V1", "kind": "video", "clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000,
                 "role": "voice", "volume": 1.0, "denoise": "off"}
            ]}]
        }));
        assert!(RenderPlan::build(&p2, Path::new("/w"), None).audio_segs[0].denoise.is_none());
    }

    /// 册四 T4.1:use_proxy 时存在代理的片段换 src;缺失回落原片。
    #[test]
    fn build_opts_swaps_existing_proxies_only() {
        let dir = std::env::temp_dir().join(format!("cf-plan-proxy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mp4"), b"fake").unwrap();
        let p = project(json!({
            "version": 1, "schemaVersion": "2.0.0", "slug": "p", "fps": 30,
            "canvas": {"width": 1080, "height": 1920},
            "tracks": [
                {"id": "V1", "kind": "video", "clips": [
                    {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 2000, "volume": 0},
                    {"id": "V1-002", "src": "缺代理.mp4", "startMs": 2000, "durationMs": 1000, "volume": 0}
                ]},
                {"id": "A1", "kind": "audio", "clips": [
                    {"id": "A1-001", "src": "a.mp4", "startMs": 0, "durationMs": 100, "volume": 0}
                ]}
            ]
        }));
        // 预生成 a.mp4 的代理(mediacache 同一实现)
        let (mtime, size) = cutforge_io::mediacache::source_stamp(&dir.join("a.mp4")).unwrap();
        let proxy_rel = cutforge_io::mediacache::proxy_rel("a.mp4", mtime, size);
        std::fs::create_dir_all(dir.join(".cutforge/proxy")).unwrap();
        std::fs::write(dir.join(&proxy_rel), b"fakeproxy").unwrap();
        // 关:零换写(缺省行为不变)
        let plan_off = RenderPlan::build(&p, &dir, None);
        assert_eq!(plan_off.video_clips[0].src.as_deref(), Some("a.mp4"));
        assert!(!plan_off.use_proxy);
        // 开:有代理的换,缺代理的回落;音频轨不换(声画同源仅视频主轨)
        let plan_on = RenderPlan::build_opts(&p, &dir, None, true);
        assert!(plan_on.use_proxy);
        assert_eq!(
            plan_on.video_clips[0].src.as_deref().unwrap(),
            proxy_rel,
            "有代理 → 换写(相对路径)"
        );
        assert_eq!(plan_on.video_clips[1].src.as_deref(), Some("缺代理.mp4"), "缺代理回落原片");
        assert_eq!(plan_off.video_clips[0].start_ms, plan_on.video_clips[0].start_ms, "时域不变");
        // 换写进 clip JSON → seg 键分叉(代理渲染不与原片共享缓存)
        let tail = 0.0;
        assert_ne!(
            crate::cache::seg_key(&plan_off, &plan_off.video_clips[0], tail),
            crate::cache::seg_key(&plan_on, &plan_on.video_clips[0], tail),
            "代理/原片必须分键"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
