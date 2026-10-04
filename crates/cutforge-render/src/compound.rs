// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 复合片段递归展开(册五 T5.4/ADR-0019):compound 片段在 segment 步先按
//! **同一管线**(exec_segment + exec_compose,共享内容寻址缓存)把子时间线
//! 渲染为中间段,再作为普通素材参与外层既有合成(变换/变速/转场全通路复用)。
//!
//! 缓存口径:中间段 = **compose 层**产物,键 = compose_key(inner seg keys)
//! —— seg 键逐段覆盖子 clip 全量 JSON + 画幅/帧率/尾帧/渲染版本/LUT 内容哈希,
//! 即「子内容寻址键 = 子 clips 内容指纹」;改子时间线任一字节 → seg 键变 →
//! compose 键变 → 外层 seg 键(含整 clip JSON,含 compound 字段)必 miss。
//!
//! 边界(诚实降级,登记遗留):子时间线当前为单轨 concat 语义(升序/首尾相接,
//! 模型层校验拒绝重叠与间隙);**子 clips 携带的音频暂不渲染**(WARN 留痕,
//! 能力矩阵登记),嵌套深度上限两级(模型层拒绝,此处防御性再查)。

use crate::cache::CacheIndex;
use crate::plan::{RenderOptions, RenderPlan};
use cutforge_core::model::{Backend, Canvas, Clip, CompoundSpec, Project, Role, Track, TrackKind};

/// 复合片段解析结果:中间段相对工程根的 src(喂给外层 segment 管线)+
/// WARN 列表(并入 segment 步进度事件)。
pub struct CompoundResolved {
    pub src_rel: String,
    pub warns: Vec<String>,
}

/// 解析复合片段为中间段素材(命中即零 ffmpeg;未命中先渲子时间线)。
/// 中间段落点 = `<工程>/.cutforge/render-cache/compose/<compose_key>.mp4`。
pub fn resolve(
    plan: &RenderPlan,
    clip: &Clip,
    spec: &CompoundSpec,
    idx: &mut CacheIndex,
) -> Result<CompoundResolved, String> {
    let mut warns = Vec::new();
    // 防御性再查深度上限(模型层已拒;渲染端兜底不炸)
    if spec.clips.iter().any(|c| c.compound.is_some()) {
        return Err("compound 嵌套超深(深度上限两级,拒绝三层)".into());
    }
    // 子 clips 排序(模型保证首尾相接;排序=确定性口径)
    let mut inner: Vec<&Clip> = spec.clips.iter().collect();
    inner.sort_by_key(|c| (c.start_ms, c.id.clone()));
    // 子时间线音频:诚实降级 WARN(渲染只取视频面)
    let audio_spec = CompoundSpec {
        canvas: spec.canvas,
        clips: inner.iter().map(|c| (*c).clone()).collect(),
    };
    warns.extend(audio_warns(&audio_spec));
    // 子时间线画幅:compound.canvas 声明优先,缺省 = 工程画幅(canvas 不缩放
    // 中间段——中间段在子管线内已按该画幅归一)
    let canvas = spec.canvas.unwrap_or(Canvas {
        width: plan.canvas_w,
        height: plan.canvas_h,
    });
    let mini = synthetic_project(&inner, canvas, plan.fps);
    let mini_plan = RenderPlan::build_full(
        &mini,
        &plan.project_dir,
        None,
        false,
        RenderOptions::default(),
    );
    if mini_plan.video_clips.is_empty() {
        return Err(format!("compound {} 子时间线为空", clip.id));
    }
    // 与整片渲染完全同源的子管线(共享 idx → 子段缓存跨渲染复用)
    let (_rep, seg_files, seg_keys, _h, _m) = crate::exec_segment(&mini_plan, idx)?;
    let (_rep, composed, _key, _cmds) =
        crate::exec_compose(&mini_plan, idx, &seg_files, &seg_keys)?;
    let rel = composed
        .strip_prefix(&plan.project_dir)
        .map_err(|_| format!("中间段不在工程根内: {}", composed.display()))?
        .to_string_lossy()
        .replace('\\', "/");
    Ok(CompoundResolved {
        src_rel: rel,
        warns,
    })
}

/// 子时间线 → 单轨合成工程(纯构造;不经 from_value,渲染计划只消费字段面)。
fn synthetic_project(inner: &[&Clip], canvas: Canvas, fps: u32) -> Project {
    Project {
        version: 1,
        schema_version: "3.0.0".into(),
        slug: "compound-inner".into(),
        fps,
        canvas,
        backends: vec![Backend::Ffmpeg],
        notes: "notes.json".into(),
        tracks: vec![Track {
            id: "V1".into(),
            kind: TrackKind::Video,
            name: None,
            locked: None,
            mute: None,
            solo: None,
            hidden: None,
            height_px: None,
            color: None,
            eq: None,
            dyn_: None,
            clips: inner.iter().map(|c| (*c).clone()).collect(),
        }],
        bgm: None,
        outputs: None,
        markers: None,
        subtitle: None,
        join_crossfade_ms: None,
    }
}

/// 外层 segment 用的有效片段:壳克隆 + src 换写为中间段相对路径
/// (其余变换字段原样——外层通路对中间段再施 scale/fx/变速/转场等)。
pub fn effective_clip(clip: &Clip, resolved: &CompoundResolved) -> Clip {
    let mut c = clip.clone();
    c.src = Some(resolved.src_rel.clone());
    c
}

/// 进度事件警告聚合用:解析产生的 WARN(纯函数面,便于单测直查)。
pub fn audio_warns(spec: &CompoundSpec) -> Vec<String> {
    spec.clips
        .iter()
        .filter(|c| {
            c.volume.map(|v| v > 0.0).unwrap_or(true)
                || c.role == Some(Role::Voice)
                || c.denoise.is_some()
                || c.pitch.is_some()
        })
        .map(|c| {
            format!(
                "compound 内音频暂不渲染(clip {c};单轨中间段只取视频面,登记遗留);",
                c = c.id
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clip(v: serde_json::Value) -> Clip {
        serde_json::from_value(v).unwrap()
    }

    /// 有效片段:src 换写为中间段相对路径,其余字段(变换/速度)原样保留。
    #[test]
    fn effective_clip_swaps_src_keeps_fields() {
        let shell = clip(json!({
            "id": "V1-001", "startMs": 0, "durationMs": 2000, "speed": 2.0,
            "opacity": 0.5,
            "compound": {"clips": [
                {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000}
            ]}
        }));
        let resolved = CompoundResolved {
            src_rel: ".cutforge/render-cache/compose/abc.mp4".into(),
            warns: vec![],
        };
        let eff = effective_clip(&shell, &resolved);
        assert_eq!(
            eff.src.as_deref(),
            Some(".cutforge/render-cache/compose/abc.mp4")
        );
        assert_eq!(eff.speed, Some(2.0), "外层变速通路保留");
        assert_eq!(eff.opacity, Some(0.5));
        assert!(
            eff.compound.is_some(),
            "compound 字段保留(seg 键含整 clip JSON)"
        );
    }

    /// 子时间线音频诚实降级 WARN:volume>0 / voice / denoise / pitch 命中即留痕。
    #[test]
    fn audio_warns_flags_audio_bearing_inner_clips() {
        let spec: CompoundSpec = serde_json::from_value(json!({"clips": [
            {"id": "V1-001", "src": "a.mp4", "startMs": 0, "durationMs": 1000, "volume": 1.0},
            {"id": "V1-002", "src": "b.mp4", "startMs": 1000, "durationMs": 1000, "volume": 0}
        ]}))
        .unwrap();
        let w = audio_warns(&spec);
        assert_eq!(w.len(), 1, "volume=0 不告警: {w:?}");
        assert!(w[0].contains("V1-001"));
        // 深度上限兜底:子 clip 带 compound → resolve 前置拒绝(不进 ffmpeg 面)
        let bad: CompoundSpec = serde_json::from_value(json!({"clips": [
            {"id": "V1-001", "startMs": 0, "durationMs": 1000,
             "compound": {"clips": [{"id": "V1-002", "startMs": 0, "durationMs": 500}]}}
        ]}))
        .unwrap();
        assert!(bad.clips.iter().any(|c| c.compound.is_some()));
    }
}
