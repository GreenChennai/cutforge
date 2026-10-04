// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! zone 预览渲染(I1-M2,Kdenlive Timeline Preview 模式的内核侧):对时间线
//! 区间 [startMs, endMs) 以**半分辨率 + 加粗质量档(fast = crf28/veryfast)**
//! 走既有八步管线,产物落 `<工程>/.cutforge/preview-cache/<键>/zone.mp4`。
//!
//! 复用纪律(不建第二条管线):
//! - 时间窗 = `export::window_project`(区域导出同一纯函数,头部裁剪按速度
//!   分段积分折算源域);
//! - 分辨率覆盖 = plan 层画幅减半(段缓存键含 canvas,预渲与整片自然分键);
//! - 质量档/出口 = `RenderOptions`(quality=fast + preview_output 重定向),
//!   步骤循环、进度事件、段/混音/字幕缓存与 `render_with_opts` 完全同源。
//!
//! 缓存键 = 工作区指纹(fresh.rs disk_fingerprint,改一笔即 miss)+ 区间
//! (100ms 量化,与 render_frame 同网格)+ RENDERER_VERSION + 预览档常量。
//! 命中判定 = 产物文件存在(指纹键空间下文件在即产物在,无需清单);
//! 写入走临时件 + 原子改名,半途失败绝不留下可误命中的半成品。
//!
//! 已登记口径:
//! - 外部 ASS 不随时间窗平移(与区域导出同口径;文本轨 textass 片段随窗裁剪,
//!   无此问题);
//! - preview-cache 不在 render-cache 清单体系内,cache gc 不覆盖(键含指纹,
//!   生命周期由壳侧管理;积累治理留待 I9)。

use crate::plan::{RenderOptions, RenderPlan};
use cutforge_core::model::{Canvas, Project};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// 预览缓存根(相对工程目录;与 render-cache 平行的命名空间)。
pub const PREVIEW_CACHE_ROOT: &str = ".cutforge/preview-cache";
/// 预览降采样档:半分辨率(M2 硬性口径;入键,调档即整体失效)。
pub const PREVIEW_SCALE: u32 = 2;
/// zone 产物文件名(键目录内固定名;目录名 = 内容寻址键)。
pub const ZONE_FILE: &str = "zone.mp4";
/// 渲染中临时件名(同键目录内;成功后原子改名,绝不以半成品充当命中)。
pub const ZONE_BUILDING: &str = "zone-building.mp4";

/// zone 预渲结果。
#[derive(Debug, Clone)]
pub struct ZoneOutcome {
    /// 落盘绝对路径(`<工程>/.cutforge/preview-cache/<key>/zone.mp4`)。
    pub output: PathBuf,
    /// 实际渲染区间(100ms 量化后;与缓存键一致)。
    pub start_ms: u64,
    pub end_ms: u64,
    /// true = preview-cache 命中(零 ffmpeg)。
    pub cached: bool,
    /// 内容寻址键(调试/对账用)。
    pub key: String,
    /// 预览画幅(工程画幅减半后)。
    pub canvas: (u32, u32),
}

/// 半分辨率画幅:各边减半、取整到偶数(libx264 yuv420p 要求)、下限 2。
pub fn half_canvas(w: u32, h: u32) -> (u32, u32) {
    // v/4 取整 ×2 = 「减半后归偶」:320→160,321→160,3→2
    let even_half = |v: u32| ((v as f64 / 4.0).round() as u32).max(1) * 2;
    (even_half(w), even_half(h))
}

/// zone 输入 spec:工作区指纹 + 量化区间 + 预览档 + 渲染版本 + 外部 ASS 字节
/// 哈希(ass 是 CLI 入参不在盘面指纹内,给 ass 必须入键——否则换字幕陈旧复用)。
pub fn zone_spec(fp_key: &str, start_ms: u64, end_ms: u64, ass_bytes: Option<&[u8]>) -> Value {
    json!({
        "v": crate::RENDERER_VERSION,
        "fp": fp_key,
        "startMs": start_ms,
        "endMs": end_ms,
        "preview": {"scale": PREVIEW_SCALE, "quality": "fast"},
        "ass": ass_bytes.map(|b| format!("{:016x}", crate::cache::hash_text(&String::from_utf8_lossy(b)))),
    })
}

/// zone 内容寻址键(16 位十六进制;区间经 [`crate::quantize_ms`] 量化,
/// 壳播放头毫秒级抖动不至于击穿缓存——量化后的区间才是实际渲染与键输入)。
pub fn zone_key(fp_key: &str, start_ms: u64, end_ms: u64, ass_bytes: Option<&[u8]>) -> String {
    crate::cache::key_hex(&zone_spec(fp_key, start_ms, end_ms, ass_bytes))
}

/// 预览缓存根绝对路径。
pub fn preview_cache_dir(project_dir: &Path) -> PathBuf {
    project_dir.join(PREVIEW_CACHE_ROOT)
}

/// zone 产物目录(内容寻址:`preview-cache/<key>/`)。
pub fn zone_dir(project_dir: &Path, key: &str) -> PathBuf {
    preview_cache_dir(project_dir).join(key)
}

/// zone 完成事件(stdout JSON 行;CLI 面唯一进度收口——同步执行,一区一报)。
pub fn zone_done_event(o: &ZoneOutcome) -> Value {
    json!({
        "done": true,
        "mode": "zone",
        "file": o.output.to_string_lossy(),
        "startMs": o.start_ms,
        "endMs": o.end_ms,
        "cached": o.cached,
        "key": o.key,
        "rendererVersion": crate::RENDERER_VERSION,
        "canvas": [o.canvas.0, o.canvas.1],
    })
}

/// zone 预渲主入口(同步;命中缓存时零 ffmpeg,未命中复用整片管线后原子落盘)。
/// 错误串带 `NO_CONFIG:` / `PRECONDITION:` 前缀的,调用方(MCP 面)按 5.4 码映射。
pub fn render_zone(
    project: &Project,
    project_dir: &Path,
    ass_path: Option<&Path>,
    start_ms: u64,
    end_ms: u64,
    progress: &mut dyn FnMut(Value),
) -> Result<ZoneOutcome, String> {
    if end_ms <= start_ms {
        return Err(format!(
            "PRECONDITION: 区间非法(endMs({end_ms}) 必须 > startMs({start_ms}))"
        ));
    }
    let q_start = crate::quantize_ms(start_ms);
    let q_end = crate::quantize_ms(end_ms).max(q_start + 100);
    // 工作区指纹:与 render_frame 同一真相源(改一笔即 miss,绝不给陈旧 zone)
    let fp = cutforge_io::fresh::disk_fingerprint(project_dir)
        .ok_or_else(|| "NO_CONFIG: 工程不可读,无法计算工作区指纹".to_string())?;
    let ass_bytes = match ass_path {
        Some(p) => Some(std::fs::read(p).map_err(|e| format!("NO_CONFIG: ass 不可读: {e}"))?),
        None => None,
    };
    let key = zone_key(&fp.cache_key(), q_start, q_end, ass_bytes.as_deref());
    let output = zone_dir(project_dir, &key).join(ZONE_FILE);
    let canvas = half_canvas(project.canvas.width, project.canvas.height);
    // 命中即返回(键含指纹;文件在 = 产物在,零 ffmpeg)
    if output.is_file() {
        return Ok(ZoneOutcome {
            output,
            start_ms: q_start,
            end_ms: q_end,
            cached: true,
            key,
            canvas,
        });
    }
    // 时间窗(区域导出同一纯函数)+ 半分辨率画幅(plan 层覆盖:键自动分叉)
    let mut prepared = crate::export::window_project(project, q_start, Some(q_end));
    prepared.canvas = Canvas {
        width: canvas.0,
        height: canvas.1,
    };
    // 守卫先于 ffmpeg:窗口内无视频片段(纯空隙/越出内容末端)→ PRECONDITION
    let guard_plan = RenderPlan::build(&prepared, project_dir, None);
    if guard_plan.video_clips.is_empty() {
        return Err(format!(
            "PRECONDITION: 区间 [{q_start},{q_end}) 内无视频片段(时间线标称 {}ms)",
            RenderPlan::build(project, project_dir, None).total_ms
        ));
    }
    // 预览质量档固定 fast(crf28/veryfast,M2「CRF 加粗」);出口重定向到
    // 临时件,成功后原子改名——半途失败绝不留下可误命中的半成品。
    let building = zone_dir(project_dir, &key).join(ZONE_BUILDING);
    std::fs::create_dir_all(zone_dir(project_dir, &key)).map_err(|e| e.to_string())?;
    let opts = RenderOptions {
        quality: Some("fast".into()),
        preview_output: Some(building.clone()),
        ..Default::default()
    };
    let outcome = crate::render_with_opts(&prepared, project_dir, ass_path, false, opts, progress)?;
    debug_assert_eq!(outcome.output, building, "preview_output 重定向必须生效");
    cutforge_io::atomic::rename(&building, &output).map_err(|e| e.to_string())?;
    Ok(ZoneOutcome {
        output,
        start_ms: q_start,
        end_ms: q_end,
        cached: false,
        key,
        canvas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_canvas_is_even_and_halved() {
        assert_eq!(half_canvas(1080, 1920), (540, 960));
        assert_eq!(half_canvas(1920, 1080), (960, 540));
        assert_eq!(half_canvas(320, 240), (160, 120));
        assert_eq!(half_canvas(321, 241), (160, 120), "奇数边减半归偶");
        assert_eq!(half_canvas(3, 2), (2, 2), "下限 2(libx264 偶数锁)");
        assert_eq!(half_canvas(1, 1), (2, 2));
    }

    #[test]
    fn zone_key_is_sensitive_to_every_input() {
        let base = zone_key("fp-aaa", 0, 2000, None);
        assert_eq!(
            base,
            zone_key("fp-aaa", 0, 2000, None),
            "同输入同键(确定性)"
        );
        assert_ne!(base, zone_key("fp-bbb", 0, 2000, None), "指纹入键");
        assert_ne!(base, zone_key("fp-aaa", 500, 2000, None), "入点入键");
        assert_ne!(base, zone_key("fp-aaa", 0, 3000, None), "出点入键");
        assert_ne!(
            base,
            zone_key("fp-aaa", 0, 2000, Some(b"[Script]")),
            "ASS 字节入键"
        );
        // 版本位:RENDERER_VERSION 升版 → 键空间整体迁移
        let spec = zone_spec("fp-aaa", 0, 2000, None);
        assert_eq!(spec["v"], json!(crate::RENDERER_VERSION));
        assert_eq!(spec["preview"], json!({"scale": 2, "quality": "fast"}));
    }

    #[test]
    fn zone_paths_are_under_preview_cache() {
        let dir = zone_dir(Path::new("/w"), "abc123");
        assert!(dir.starts_with("/w/.cutforge/preview-cache"));
        assert_eq!(dir, Path::new("/w").join(".cutforge/preview-cache/abc123"));
        assert_eq!(
            zone_dir(Path::new("/w"), "abc123").join(ZONE_FILE),
            Path::new("/w").join(".cutforge/preview-cache/abc123/zone.mp4")
        );
    }

    #[test]
    fn zone_done_event_carries_contract_fields() {
        let o = ZoneOutcome {
            output: PathBuf::from("/w/.cutforge/preview-cache/k/zone.mp4"),
            start_ms: 0,
            end_ms: 2000,
            cached: true,
            key: "k".into(),
            canvas: (540, 960),
        };
        let v = zone_done_event(&o);
        assert_eq!(v["done"], json!(true));
        assert_eq!(v["mode"], json!("zone"));
        assert_eq!(v["startMs"], json!(0));
        assert_eq!(v["endMs"], json!(2000));
        assert_eq!(v["cached"], json!(true));
        assert_eq!(v["rendererVersion"], json!(crate::RENDERER_VERSION));
        assert_eq!(v["canvas"], json!([540, 960]));
        assert!(v["file"].as_str().unwrap().ends_with("zone.mp4"));
    }
}
