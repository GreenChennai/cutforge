//! 播放源纯函数层(A-02 拆分的可测核心;无 GPUI / 无 IO):
//! - 源类型([`EngineSource`]/[`DirectSource`]/[`ZoneReady`])与时间域映射;
//! - `preview_zone_render` 响应宽容解析;
//! - 直解码源解析(播放头所在片段 → 媒体内区间;快照 + 工程根进,参数出);
//! - JKL 变速挡与展示格式化。
//!
//! 全部纯函数:A-07 单测直接对拍,不依赖引擎/网络/窗口。

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::state::{Snapshot, ZoneRendered};

/// 引擎当前播放源(M1 直解码片段 / M2 zone 预渲段)。
#[derive(Clone, Debug)]
pub enum EngineSource {
    /// 直解码:播放头所在视频片段的原素材(工程内绝对路径)
    Direct {
        clip_id: String,
        src: PathBuf,
        clip_start_ms: u64,
        clip_end_ms: u64,
        /// 源内入点(媒体绝对域;时间映射基准 t = clip_start + (m − source_in)/s)
        source_in: f64,
        /// 本次流覆盖的媒体内区间
        media_in: f64,
        media_out: f64,
        /// 片段恒速(speed 字段;speedCurve 暂按 1.0,已知局限)
        clip_speed: f64,
    },
    /// zone 预渲段(preview_zone_render 产物;1:1 工程时间)
    Zone {
        file: PathBuf,
        start_ms: u64,
        end_ms: u64,
    },
}

/// 直解码源解析结果(播放头所在视频片段;媒体域为源内绝对时间)。
pub(crate) struct DirectSource {
    pub clip_id: String,
    pub src: PathBuf,
    pub clip_start_ms: u64,
    pub clip_end_ms: u64,
    /// 源内入点(sourceInMs;时间映射基准)
    pub source_in: f64,
    /// 本次流覆盖的媒体内区间
    pub media_in: f64,
    pub media_out: f64,
    /// 片段恒速(speed 字段;speedCurve 暂按 1.0,已知局限)
    pub clip_speed: f64,
}

impl DirectSource {
    /// 与当前引擎源逐字段对账(播放中编辑重同步:变了才重启流)。
    pub(crate) fn matches_engine(&self, engine_source: &EngineSource) -> bool {
        match engine_source {
            EngineSource::Direct {
                clip_id,
                src,
                clip_start_ms,
                clip_end_ms,
                source_in,
                media_in,
                media_out,
                ..
            } => {
                self.clip_id == *clip_id
                    && self.src == *src
                    && self.clip_start_ms == *clip_start_ms
                    && self.clip_end_ms == *clip_end_ms
                    && self.source_in == *source_in
                    && self.media_in == *media_in
                    && self.media_out == *media_out
            }
            _ => false,
        }
    }
}

/// 已渲染可用的 zone(暂停后原地重播免重渲;rev/marker 失配即作废)。
#[derive(Clone)]
pub(crate) struct ZoneReady {
    pub file: PathBuf,
    pub start_ms: u64,
    pub end_ms: u64,
    /// 内核内容寻址键(指纹 marker 判定用)
    pub key: String,
    pub rev: u64,
}

/// Direct 源内工程时刻 → 媒体内时刻(seek 换算;越界 None)。
pub(crate) fn direct_params_at(src: &EngineSource, t: u64) -> Option<f64> {
    match src {
        EngineSource::Direct {
            clip_start_ms,
            clip_end_ms,
            source_in,
            clip_speed,
            ..
        } => {
            if t < *clip_start_ms || t >= *clip_end_ms {
                return None;
            }
            Some(source_in + t.saturating_sub(*clip_start_ms) as f64 * clip_speed)
        }
        _ => None,
    }
}

/// 引擎媒体位置 → 工程播放头(ms;Direct 经片段速度折算,zone 1:1)。
pub(crate) fn project_time_of(src: &EngineSource, media_pos: f64) -> u64 {
    match src {
        EngineSource::Direct {
            clip_start_ms,
            source_in,
            clip_speed,
            ..
        } => {
            // pos 是媒体绝对域(引擎时钟锚在 in_ms),映射与载入点无关
            let dt = ((media_pos - source_in) / clip_speed).max(0.0);
            clip_start_ms + dt as u64
        }
        EngineSource::Zone { start_ms, .. } => start_ms + media_pos.max(0.0) as u64,
    }
}

/// 引擎流终点(媒体域;到尾由引擎自动 pause)。
pub(crate) fn source_end_ms(src: &EngineSource) -> f64 {
    match src {
        EngineSource::Direct { media_out, .. } => *media_out,
        EngineSource::Zone {
            start_ms, end_ms, ..
        } => end_ms.saturating_sub(*start_ms) as f64,
    }
}

/// preview_zone_render 响应 → ZoneRendered(宽容解析;file/media 任一)。
pub(crate) fn parse_zone_data(
    data: &Value,
    req_start: u64,
    req_end: u64,
    rev: u64,
) -> Result<ZoneRendered, String> {
    let file = data
        .get("file")
        .and_then(Value::as_str)
        .or_else(|| data.get("media").and_then(Value::as_str))
        .ok_or_else(|| "响应缺 file".to_string())?
        .to_string();
    if file.is_empty() {
        return Err("响应 file 为空".into());
    }
    // startMs/endMs 为内核量化后实际区间(时间映射基准;缺失回落请求值)
    let start_ms = data
        .get("startMs")
        .and_then(Value::as_u64)
        .unwrap_or(req_start);
    let end_ms = data.get("endMs").and_then(Value::as_u64).unwrap_or(req_end);
    Ok(ZoneRendered {
        file,
        start_ms,
        end_ms,
        key: data
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        rev,
    })
}

/// 工程内相对路径 → 绝对(与 `Rpc::absolutize` 同口径;纯函数版供解析用)。
fn absolutize(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

/// 解析播放头所在视频片段为直解码参数:
/// 媒体偏移 = sourceInMs + (播放头 − 片头) × 片段速度(含 trim 修正)。
pub(crate) fn resolve_direct_source(snap: &Snapshot, root: &Path, t: u64) -> Option<DirectSource> {
    let clip = snap.video_clip_at(t)?;
    let clip_id = clip.get("id").and_then(Value::as_str)?.to_string();
    let src_rel = clip.get("src").and_then(Value::as_str)?;
    let cs = clip.get("startMs").and_then(Value::as_u64)?;
    let ce = clip.get("endMs").and_then(Value::as_u64)?;
    let src = absolutize(root, src_rel);
    let clip_speed = crate::state::clip_speed(&clip);
    let source_in = clip.get("sourceInMs").and_then(Value::as_u64).unwrap_or(0) as f64;
    let media_in = source_in + t.saturating_sub(cs) as f64 * clip_speed;
    let media_out = source_in + ce.saturating_sub(cs) as f64 * clip_speed;
    if media_out - media_in < 1.0 {
        return None;
    }
    Some(DirectSource {
        clip_id,
        src,
        clip_start_ms: cs,
        clip_end_ms: ce,
        source_in,
        media_in,
        media_out,
        clip_speed,
    })
}

// ---- JKL 变速挡(L 正向 / Shift+L 反向 / J 全梯减速) ----

pub(crate) const SPEED_LADDER_UP: [f32; 3] = [1.0, 2.0, 4.0];
pub(crate) const SPEED_LADDER_DOWN: [f32; 3] = [1.0, 0.5, 0.25];
pub(crate) const SPEED_LADDER_ALL: [f32; 5] = [4.0, 2.0, 1.0, 0.5, 0.25];

/// 在挡位序列内步进:命中 → 下一挡(循环由调用方语义决定,见各挡说明);
/// 未命中 → 回落值。`stop_at_end = true` 时到尾停在末挡(J 挡;不到尾循环)。
pub(crate) fn ladder_next(ladder: &[f32], cur: f32, cycle: bool, fallback: f32) -> f32 {
    let idx = ladder.iter().position(|s| (*s - cur).abs() < 1e-3);
    match idx {
        Some(i) => {
            let next = i + 1;
            if next < ladder.len() {
                ladder[next]
            } else if cycle {
                ladder[0]
            } else {
                ladder[ladder.len() - 1]
            }
        }
        None => fallback,
    }
}

/// 变速挡展示(整数不带小数)。
pub fn fmt_speed(s: f32) -> String {
    if (s - s.round()).abs() < 1e-3 {
        format!("{}x", s.round() as i32)
    } else {
        format!("{s}x")
    }
}

// ---------------------------------------------------------------------------
// 纯函数单测(A-07):时间域映射 / zone 解析 / 源解析 golden / 变速挡
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 平台原生的绝对路径样例(Windows 盘符 / Unix 根;全部正斜杠书写,
    /// Windows Path 解析同样接受,避免测试内出现分隔符字面量耦合)。
    fn abs_sample() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:/cutforge-abs/a.mp4")
        } else {
            PathBuf::from("/tmp/cutforge-abs/a.mp4")
        }
    }

    fn direct_src() -> EngineSource {
        EngineSource::Direct {
            clip_id: "c1".into(),
            src: abs_sample(),
            clip_start_ms: 1000,
            clip_end_ms: 5000,
            source_in: 2000.0,
            media_in: 2000.0,
            media_out: 6000.0,
            clip_speed: 2.0,
        }
    }

    #[test]
    fn direct_params_inside_range_maps_with_speed() {
        // t = 1000(片头)→ source_in;t = 3000 → +2000ms × 2 = +4000ms 媒体
        let p = direct_params_at(&direct_src(), 3000);
        assert!((p.unwrap() - 6000.0).abs() < 1e-9);
    }

    #[test]
    fn direct_params_out_of_range_is_none() {
        assert!(direct_params_at(&direct_src(), 999).is_none());
        assert!(direct_params_at(&direct_src(), 5000).is_none()); // 半开区间
    }

    #[test]
    fn direct_params_on_zone_source_is_none() {
        let zone = EngineSource::Zone {
            file: PathBuf::from("z.mp4"),
            start_ms: 0,
            end_ms: 100,
        };
        assert!(direct_params_at(&zone, 50).is_none());
    }

    #[test]
    fn project_time_of_direct_divides_by_speed() {
        // 媒体 6000 − source_in 2000 = 4000;÷速度2 = 2000ms + 片头1000 = 3000
        assert_eq!(project_time_of(&direct_src(), 6000.0), 3000);
    }

    #[test]
    fn project_time_of_direct_negative_dt_clamps_to_clip_start() {
        assert_eq!(project_time_of(&direct_src(), 0.0), 1000);
    }

    #[test]
    fn project_time_of_zone_is_one_to_one() {
        let zone = EngineSource::Zone {
            file: PathBuf::from("z.mp4"),
            start_ms: 7000,
            end_ms: 15000,
        };
        assert_eq!(project_time_of(&zone, 500.0), 7500);
    }

    #[test]
    fn source_end_ms_direct_and_zone() {
        assert!((source_end_ms(&direct_src()) - 6000.0).abs() < 1e-9);
        let zone = EngineSource::Zone {
            file: PathBuf::from("z.mp4"),
            start_ms: 7000,
            end_ms: 15000,
        };
        assert!((source_end_ms(&zone) - 8000.0).abs() < 1e-9);
    }

    #[test]
    fn parse_zone_data_quantized_window_wins_over_request() {
        let zr = parse_zone_data(
            &json!({"file": "cache/z.mp4", "startMs": 800, "endMs": 8800, "key": "abc"}),
            800,
            8800,
            7,
        )
        .unwrap();
        assert_eq!(zr.file, "cache/z.mp4");
        assert_eq!((zr.start_ms, zr.end_ms, zr.rev), (800, 8800, 7));
        assert_eq!(zr.key, "abc");
    }

    #[test]
    fn parse_zone_data_media_fallback_and_request_defaults() {
        let zr = parse_zone_data(&json!({"media": "m.mp4"}), 100, 200, 3).unwrap();
        assert_eq!(zr.file, "m.mp4");
        assert_eq!((zr.start_ms, zr.end_ms), (100, 200));
        assert!(zr.key.is_empty());
    }

    #[test]
    fn parse_zone_data_rejects_missing_and_empty_file() {
        assert!(parse_zone_data(&json!({}), 0, 1, 0).is_err());
        assert!(parse_zone_data(&json!({"file": ""}), 0, 1, 0).is_err());
    }

    fn snap_with_clips(clips: serde_json::Value) -> Snapshot {
        Snapshot {
            clips: clips.as_array().cloned().unwrap_or_default(),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_direct_source_golden_with_trim_and_speed() {
        let snap = snap_with_clips(json!([{
            "id": "c9", "track": "V1", "trackKind": "video", "src": "media/a.mp4",
            "startMs": 1000, "endMs": 5000, "sourceInMs": 2000, "speed": 2.0
        }]));
        // 工程根用平台原生绝对路径(temp_dir 各平台恒绝对)
        let root = std::env::temp_dir().join("cutforge-srcmap-golden");
        let ds = resolve_direct_source(&snap, &root, 3000).expect("命中片段");
        assert_eq!(ds.clip_id, "c9");
        // 相对 src 挂根:词法 join 后即绝对路径
        assert_eq!(ds.src, root.join("media/a.mp4"));
        assert!(ds.src.is_absolute());
        assert_eq!((ds.clip_start_ms, ds.clip_end_ms), (1000, 5000));
        assert!((ds.source_in - 2000.0).abs() < 1e-9);
        // media_in = 2000 + (3000−1000)×2 = 6000;media_out = 2000 + 4000×2 = 10000
        assert!((ds.media_in - 6000.0).abs() < 1e-9);
        assert!((ds.media_out - 10000.0).abs() < 1e-9);
    }

    #[test]
    fn resolve_direct_source_absolute_src_stays_absolute() {
        let src = abs_sample();
        let snap = snap_with_clips(json!([{
            "id": "c1", "trackKind": "video", "src": src.to_string_lossy(),
            "startMs": 0, "endMs": 4000
        }]));
        let ds =
            resolve_direct_source(&snap, std::path::Path::new("/cutforge-any-root"), 0).unwrap();
        // 绝对 src 原样保绝对(不挂工程根)
        assert_eq!(ds.src, src);
        assert!(ds.src.is_absolute());
        // sourceInMs 缺省 0:media_out = 4000
        assert!((ds.media_out - 4000.0).abs() < 1e-9);
    }

    #[test]
    fn resolve_direct_source_rejects_gap_and_micro_segment() {
        // 空窗:无命中
        let snap = snap_with_clips(json!([{
            "id": "c1", "trackKind": "video", "src": "a.mp4",
            "startMs": 0, "endMs": 100
        }]));
        assert!(resolve_direct_source(&snap, Path::new("."), 500).is_none());
        // <1ms 媒体区间(0.5x 速度下 1ms 片段)→ None
        let micro = snap_with_clips(json!([{
            "id": "c2", "trackKind": "video", "src": "a.mp4",
            "startMs": 0, "endMs": 1, "speed": 0.5
        }]));
        assert!(resolve_direct_source(&micro, Path::new("."), 0).is_none());
    }

    #[test]
    fn matches_engine_fieldwise_and_zone_mismatch() {
        // live 源从一次真实解析反构(src 来自 resolve 产物):
        // 测试聚焦「逐字段对账」语义,不耦合路径拼写
        let snap = snap_with_clips(json!([{
            "id": "c9", "trackKind": "video", "src": "media/a.mp4",
            "startMs": 1000, "endMs": 5000, "sourceInMs": 2000, "speed": 2.0
        }]));
        let root = std::env::temp_dir().join("cutforge-srcmap-matches");
        let ds = resolve_direct_source(&snap, &root, 3000).unwrap();
        let live = EngineSource::Direct {
            clip_id: ds.clip_id.clone(),
            src: ds.src.clone(),
            clip_start_ms: ds.clip_start_ms,
            clip_end_ms: ds.clip_end_ms,
            source_in: ds.source_in,
            media_in: ds.media_in,
            media_out: ds.media_out,
            clip_speed: ds.clip_speed,
        };
        assert!(ds.matches_engine(&live));
        // 任一字段漂移(片段被 trim/移动)→ 失配
        let mut drifted = live.clone();
        if let EngineSource::Direct { clip_start_ms, .. } = &mut drifted {
            *clip_start_ms += 1;
        }
        assert!(!ds.matches_engine(&drifted));
        // zone 源恒与 Direct 失配
        let zone = EngineSource::Zone {
            file: root.join("z.mp4"),
            start_ms: 0,
            end_ms: 1,
        };
        assert!(!ds.matches_engine(&zone));
    }

    #[test]
    fn speed_ladders_up_cycles_and_reverse_ends_at_slow() {
        assert!((ladder_next(&SPEED_LADDER_UP, 1.0, true, 1.0) - 2.0).abs() < 1e-3);
        assert!((ladder_next(&SPEED_LADDER_UP, 4.0, true, 1.0) - 1.0).abs() < 1e-3);
        assert!((ladder_next(&SPEED_LADDER_UP, 3.0, true, 1.0) - 1.0).abs() < 1e-3); // 脱挡回落
        assert!((ladder_next(&SPEED_LADDER_DOWN, 1.0, false, 1.0) - 0.5).abs() < 1e-3);
        assert!((ladder_next(&SPEED_LADDER_DOWN, 0.25, false, 1.0) - 0.25).abs() < 1e-3);
    }

    #[test]
    fn speed_ladder_all_steps_down_and_stops() {
        assert!((ladder_next(&SPEED_LADDER_ALL, 4.0, false, 0.25) - 2.0).abs() < 1e-3);
        assert!((ladder_next(&SPEED_LADDER_ALL, 0.5, false, 0.25) - 0.25).abs() < 1e-3);
        assert!((ladder_next(&SPEED_LADDER_ALL, 0.25, false, 0.25) - 0.25).abs() < 1e-3);
    }

    #[test]
    fn fmt_speed_integer_and_fraction() {
        assert_eq!(fmt_speed(1.0), "1x");
        assert_eq!(fmt_speed(4.0), "4x");
        assert_eq!(fmt_speed(0.5), "0.5x");
    }
}
