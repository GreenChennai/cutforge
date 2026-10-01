// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 渲染缓存体系(T1.5,修 R2):中间产物全部内容寻址,消灭固定文件名陈旧复用。
//!
//! 目录契约:`<工程>/.cutforge/render-cache/{seg,mix,compose,overlay,sub}/tmp`
//! + 清单 `cache-index.json`。
//!
//! 键设计(键 = hash(输入 spec + RENDERER_VERSION + 相关画幅/帧率)):
//! - seg:clip JSON + 尾帧扩展毫秒 + 画幅 + fps(尾帧来自下一 clip 的转场,
//!   必须入键——旧键漏掉它,改转场会陈旧复用前一段的 tpad);
//! - compose:seg 键序列(传递性覆盖 clip 内容/画幅/转场);
//! - overlay:compose 键 + overlay 段清单;
//! - mix:音频段清单 + BGM + 总长(画幅无关 → 多画幅变体共享,真分叉的判据);
//! - sub:video 键(compose/overlay)+ mix 键 + ASS 字节哈希。
//!
//! 最终成片(06_成片输出)不缓存:输出路径与格式保持不变。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 渲染缓存根(相对工程目录)。
pub const CACHE_ROOT: &str = ".cutforge/render-cache";
/// 分层目录:seg(段)/mix(混音)/compose(合成)/overlay(叠加)/sub(字幕合流)
/// /frame(单帧,T2.4 精确预览;键含工作区指纹,改一笔即 miss)。
/// 缓存层(册五 T5.4 增 adjust:调整层时间窗处理产物;复合中间段挂 compose 层,
/// 键 = 子内容指纹——见 compound.rs 模块注释,不另设层)。
pub const LAYERS: [&str; 7] = ["seg", "mix", "compose", "overlay", "adjust", "sub", "frame"];
/// 临时文件目录(concat 清单、burn 用 ASS 副本;不入索引,gc 按超龄清理)。
pub const TMP_DIR: &str = "tmp";
/// 清单文件名(相对缓存根)。
pub const INDEX_FILE: &str = "cache-index.json";
/// `cache gc` 缺省容量上限:10GB。
pub const DEFAULT_CAPACITY_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// 孤儿文件(磁盘有、清单无)与 tmp 件的最长保留时长(gc 时超龄即清),秒。
/// 新于此时长的孤儿可能是并发渲染的在写文件,不动。
pub const ORPHAN_TTL_SECS: u64 = 24 * 3600;

/// cache-index.json 单条目:输入键、大小、创建时间、最近使用时间与命中数(供 gc 与调试)。
#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub layer: String,
    pub key: String,
    /// 相对缓存根的文件路径(如 "seg/1a2b….mp4")。
    pub file: String,
    pub size: u64,
    /// Unix 秒。
    pub created_at: u64,
    /// Unix 秒;LRU 的排序依据。
    pub last_used_at: u64,
    pub hits: u64,
    /// 输入 spec 摘要(调试用;键即其哈希)。
    pub input: Value,
}

impl CacheEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "layer": self.layer, "key": self.key, "file": self.file,
            "size": self.size, "createdAt": self.created_at,
            "lastUsedAt": self.last_used_at, "hits": self.hits, "input": self.input,
        })
    }

    pub fn from_json(v: &Value) -> Option<CacheEntry> {
        Some(CacheEntry {
            layer: v.get("layer")?.as_str()?.to_string(),
            key: v.get("key")?.as_str()?.to_string(),
            file: v.get("file")?.as_str()?.to_string(),
            size: v.get("size")?.as_u64()?,
            created_at: v.get("createdAt")?.as_u64()?,
            last_used_at: v.get("lastUsedAt").and_then(|x| x.as_u64()).unwrap_or(0),
            hits: v.get("hits").and_then(|x| x.as_u64()).unwrap_or(0),
            input: v.get("input").cloned().unwrap_or(Value::Null),
        })
    }
}

/// 缓存清单(渲染期内存态;落盘即 cache-index.json)。
#[derive(Debug, Default, Clone)]
pub struct CacheIndex {
    pub entries: Vec<CacheEntry>,
}

impl CacheIndex {
    /// 读取清单;缺失/损坏 → 空清单(缓存退化为全 miss,绝不误报命中)。
    pub fn load(root: &Path) -> CacheIndex {
        std::fs::read_to_string(root.join(INDEX_FILE))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| {
                let entries = v
                    .get("entries")
                    .and_then(|e| e.as_array())
                    .map(|a| a.iter().filter_map(CacheEntry::from_json).collect::<Vec<_>>())
                    .unwrap_or_default();
                let mut idx = CacheIndex { entries };
                idx.prune_missing(root);
                idx
            })
            .unwrap_or_default()
    }

    /// 原子写清单(唯一落盘点纪律:走 cutforge_io::atomic)。
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let doc = json!({
            "version": 1,
            "entries": self.entries.iter().map(|e| e.to_json()).collect::<Vec<_>>(),
        });
        cutforge_io::atomic::atomic_write(&root.join(INDEX_FILE), doc.to_string().as_bytes())
            .map_err(|e| e.to_string())
    }

    pub fn find(&self, layer: &str, key: &str) -> Option<&CacheEntry> {
        self.entries.iter().find(|e| e.layer == layer && e.key == key)
    }

    /// 命中:刷新 LRU 时间与命中数,返回相对缓存根的文件路径。
    pub fn touch(&mut self, layer: &str, key: &str, now: u64) -> Option<PathBuf> {
        let e = self.entries.iter_mut().find(|e| e.layer == layer && e.key == key)?;
        e.hits += 1;
        e.last_used_at = now;
        Some(PathBuf::from(&e.file))
    }

    /// 登记新条目(同键覆盖);返回相对缓存根的落盘路径(调用方随后写文件并回填 size)。
    /// 清单内路径统一**正斜杠**约定(跨平台一致;Windows 的 join/rename 均兼容)。
    pub fn record(&mut self, layer: &str, key: &str, input: Value, now: u64) -> PathBuf {
        self.record_with_ext(layer, key, input, now, layer_ext(layer))
    }

    /// 同 [`CacheIndex::record`],扩展名显式给定(frame 层单帧格式 png/jpeg 共用一层,
    /// 扩展名不随 layer 固定;其余层不受影响)。
    pub fn record_with_ext(&mut self, layer: &str, key: &str, input: Value, now: u64, ext: &str) -> PathBuf {
        let rel = Path::new(layer).join(format!("{key}{ext}"));
        self.entries.retain(|e| !(e.layer == layer && e.key == key));
        self.entries.push(CacheEntry {
            layer: layer.to_string(),
            key: key.to_string(),
            file: rel.to_string_lossy().replace('\\', "/"),
            size: 0,
            created_at: now,
            last_used_at: now,
            hits: 0,
            input,
        });
        rel
    }

    /// 回填实际文件大小(ffmpeg/拷贝落盘后、清单保存前调用)。
    pub fn set_size(&mut self, layer: &str, key: &str, size: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.layer == layer && e.key == key) {
            e.size = size;
        }
    }

    /// 清掉文件已不存在的条目(清单与磁盘对账)。
    pub fn prune_missing(&mut self, root: &Path) {
        self.entries.retain(|e| root.join(&e.file).is_file());
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().map(|e| e.size).sum()
    }
}

/// 各层产物扩展名。
fn layer_ext(layer: &str) -> &'static str {
    match layer {
        "mix" => ".m4a",
        _ => ".mp4",
    }
}

/// 统一哈希入口(复用仓库既有 DefaultHasher 文本哈希,与拆分前同源)。
pub fn hash_text(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// 键 = 输入 spec 的 16 位十六进制哈希(spec 必须已含 RENDERER_VERSION 等全部输入)。
pub fn key_hex(spec: &Value) -> String {
    format!("{:016x}", hash_text(&spec.to_string()))
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 确保分层目录齐全(渲染开工前调用)。
pub fn ensure_dirs(cache_root: &Path) -> Result<(), String> {
    for layer in LAYERS {
        std::fs::create_dir_all(cache_root.join(layer)).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(cache_root.join(TMP_DIR)).map_err(|e| e.to_string())
}

// ---------------- 各层缓存键(T1.5:键 = hash(输入 spec + RENDERER_VERSION + 画幅/帧率)) ----------------

use crate::plan::{OverlaySeg, RenderPlan};
use cutforge_core::model::Clip;

/// seg 层输入 spec:clip JSON + 尾帧扩展 + 画幅 + fps(+ LUT 内容哈希,册五 T5.2:
/// clip JSON 只含 grade.lut **路径**,文件内容被替换时必须 miss)。
/// 尾帧来自**下一 clip 的转场**,必须入键——旧键(仅 clip 自身 JSON)漏掉它,
/// 改转场时长会陈旧复用前一段的 tpad(R2 的实体案例)。
pub fn seg_spec(plan: &RenderPlan, clip: &Clip, tail_ms: f64) -> Value {
    json!({
        "v": crate::RENDERER_VERSION,
        "canvas": [plan.canvas_w, plan.canvas_h],
        "fps": plan.fps,
        "clip": serde_json::to_string(clip).unwrap_or_default(),
        "tailMs": tail_ms,
        "lutHash": crate::grade::lut_content_hash(clip, &plan.project_dir),
    })
}

/// seg 键(带画幅调试后缀;哈希已保证唯一)。
pub fn seg_key(plan: &RenderPlan, clip: &Clip, tail_ms: f64) -> String {
    let spec = seg_spec(plan, clip, tail_ms);
    format!("{}-{}x{}f{}", key_hex(&spec), plan.canvas_w, plan.canvas_h, plan.fps)
}

/// compose 输入 spec:seg 键序列(传递性覆盖 clip 内容/画幅/转场)。
pub fn compose_spec(seg_keys: &[String]) -> Value {
    json!({"v": crate::RENDERER_VERSION, "segKeys": seg_keys})
}

pub fn compose_key(seg_keys: &[String]) -> String {
    key_hex(&compose_spec(seg_keys))
}

/// overlay 输入 spec:基片键 + overlay 段清单(源/落点/时间窗/透明度)。
pub fn overlay_spec(base_key: &str, overlays: &[OverlaySeg]) -> Value {
    json!({
        "v": crate::RENDERER_VERSION,
        "base": base_key,
        "overlays": overlays.iter().map(|o| json!({
            "src": o.src.to_string_lossy(),
            "startMs": o.start_ms,
            "durationMs": o.duration_ms,
            "spec": o.spec,
        })).collect::<Vec<_>>(),
    })
}

pub fn overlay_key(base_key: &str, overlays: &[OverlaySeg]) -> String {
    key_hex(&overlay_spec(base_key, overlays))
}

/// adjust 层输入 spec(册五 T5.4 调整层):基片键 + adjust 片段清单
/// (整 clip JSON:fx/grade 链 + 时间窗)+ 画幅/帧率。基片变 → 键变;
/// 改调整层任一片段 → 键变(真分叉)。
pub fn adjust_spec(base_key: &str, clips: &[Clip], w: u32, h: u32, fps: u32) -> Value {
    json!({
        "v": crate::RENDERER_VERSION,
        "base": base_key,
        "canvas": [w, h],
        "fps": fps,
        "clips": clips.iter().map(|c| serde_json::to_string(c).unwrap_or_default()).collect::<Vec<_>>(),
    })
}

/// mix 输入 spec:音频段清单 + BGM + 总长 + 边界转场时长(册四 T4.5:acrossfade
/// 链由边界决定,改转场时长必须换键)。**画幅无关** → 多画幅变体共享一份
/// (真分叉判据,与拆分前同构)。册四 T4.4:段元组并入 reverse(倒放改变混音产物)。
/// 册四 T4.8:段元组并入 denoise/pitch(降噪/变调改变混音产物)。
/// 册五 T5.3:段元组并入 track_id + 全局 trackProc(轨道 EQ/动态改变混音产物,
/// 且事件跨轨移动时同键复用会陈旧——track_id 必须逐段入键)。
pub fn mix_spec(plan: &RenderPlan) -> Value {
    json!({
        "segs": plan.audio_segs.iter().map(|s| (
            s.src.to_string_lossy(), s.start_ms, s.duration_ms, s.source_in_ms,
            s.volume, s.speed, s.reverse, s.denoise.clone(), s.pitch, s.fade_in_ms, s.fade_out_ms,
            s.volume_expr.clone(), // volume 关键帧表达式入键(IR v3;改关键帧必换键)
            s.track_id.clone(),    // 轨道归属入键(T5.3 分组建流;跨轨移动必换键)
        )).collect::<Vec<_>>(),
        "bgm": plan.bgm,
        "total": plan.total_ms,
        "trn": plan.boundary_durs_ms.iter().map(|d| crate::steps::fmt_f64(*d)).collect::<Vec<_>>(),
        "trackProc": plan.track_proc.iter().map(|p| json!({
            "trackId": p.track_id, "eq": p.eq, "dyn": p.dyn_,
        })).collect::<Vec<_>>(),
        // 响度目标入键(册五 T5.6 loudnormTarget:改目标必换键,pass B 产物不同)
        "lnTarget": [plan.opts.loudnorm_i, plan.opts.loudnorm_tp],
        "v": crate::RENDERER_VERSION,
    })
}

pub fn mix_key(plan: &RenderPlan) -> String {
    key_hex(&mix_spec(plan))
}

/// sub 输入 spec:video 键(compose/overlay)+ mix 键 + ASS 字节哈希。
/// mix 键必须入键:subbed = video+audio 合流,漏掉会陈旧复用旧音频的成片。
pub fn sub_spec(video_key: &str, mix_key: &str, ass_bytes: Option<&[u8]>) -> Value {
    json!({
        "v": crate::RENDERER_VERSION,
        "video": video_key,
        "mix": mix_key,
        "ass": ass_bytes.map(|b| format!("{:016x}", hash_text(&String::from_utf8_lossy(b)))),
    })
}

pub fn sub_key(video_key: &str, mix_key: &str, ass_bytes: Option<&[u8]>) -> String {
    key_hex(&sub_spec(video_key, mix_key, ass_bytes))
}

/// frame 输入 spec(T2.4 单帧):工作区指纹(fresh.rs,改一笔即 miss 的根基)+
/// atMs(100ms 量化)+ 画幅 + 渲染版本 + ASS 字节哈希 + 代理开关(册四 T4.1,
/// 代理帧与原片帧不共享条目)。与管线层(键=渲染输入)不同,帧键直接以
/// **工程盘面指纹**为输入:预览语义是"当前工程这一刻的样子",任何盘面变化
/// (哪怕不影响画面的 oplog 追加)都宁可重渲一帧,绝不给陈旧帧。
pub fn frame_spec(fp_key: &str, at_ms: u64, canvas: (u32, u32), fmt: &str, ass_bytes: Option<&[u8]>) -> Value {
    frame_spec_proxy(fp_key, at_ms, canvas, fmt, ass_bytes, false)
}

/// 同 [`frame_spec`],代理开关显式给定(册四 T4.1)。
pub fn frame_spec_proxy(fp_key: &str, at_ms: u64, canvas: (u32, u32), fmt: &str, ass_bytes: Option<&[u8]>, use_proxy: bool) -> Value {
    json!({
        "v": crate::RENDERER_VERSION,
        "fp": fp_key,
        "atMs": at_ms,
        "canvas": [canvas.0, canvas.1],
        "fmt": fmt,
        "ass": ass_bytes.map(|b| format!("{:016x}", hash_text(&String::from_utf8_lossy(b)))),
        "px": use_proxy,
    })
}

pub fn frame_key(fp_key: &str, at_ms: u64, canvas: (u32, u32), fmt: &str, ass_bytes: Option<&[u8]>) -> String {
    key_hex(&frame_spec(fp_key, at_ms, canvas, fmt, ass_bytes))
}

pub fn frame_key_proxy(fp_key: &str, at_ms: u64, canvas: (u32, u32), fmt: &str, ass_bytes: Option<&[u8]>, use_proxy: bool) -> String {
    key_hex(&frame_spec_proxy(fp_key, at_ms, canvas, fmt, ass_bytes, use_proxy))
}

/// tmp 文件相对路径(内容寻址命名,避免并发互踩)。
pub fn tmp_rel(name: &str) -> PathBuf {
    Path::new(TMP_DIR).join(name)
}

// --------------- info / gc(治理面;cutforge-cli cache 子命令按同一文件契约实现) ---------------

/// 单层统计(cache info)。
#[derive(Debug, Clone, PartialEq)]
pub struct LayerStat {
    pub layer: String,
    pub entries: usize,
    pub bytes: u64,
}

/// cache info 汇总:分层统计 + 清单外孤儿(旧版固定文件名等遗留)。
#[derive(Debug, Clone, PartialEq)]
pub struct CacheInfoReport {
    pub layers: Vec<LayerStat>,
    pub orphan_files: usize,
    pub orphan_bytes: u64,
    pub total_indexed_bytes: u64,
}

/// cache gc 执行报告。
#[derive(Debug, Clone, PartialEq)]
pub struct GcReport {
    pub removed: usize,
    pub freed_bytes: u64,
    pub remaining_bytes: u64,
    pub capacity_bytes: u64,
}

/// 遍历缓存根下全部文件(返回绝对路径 + 大小;跳过清单自身)。
fn walk_files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n.to_string_lossy() != INDEX_FILE) {
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                out.push((p, size));
            }
        }
    }
    out
}

fn file_mtime_secs(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 相对路径归一化为正斜杠(与清单 `file` 字段约定一致;Windows strip_prefix 产反斜杠)。
fn normalize_rel(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// cache info:分层条目/体积 + 孤儿统计(只读,不改清单)。
pub fn cache_info(root: &Path) -> Result<CacheInfoReport, String> {
    let idx = CacheIndex::load(root);
    let mut layers = Vec::new();
    for l in LAYERS {
        let es: Vec<&CacheEntry> = idx.entries.iter().filter(|e| e.layer == *l).collect();
        layers.push(LayerStat {
            layer: l.to_string(),
            entries: es.len(),
            bytes: es.iter().map(|e| e.size).sum(),
        });
    }
    // 孤儿 = 磁盘上有、清单里没有的文件(旧版固定文件名 / 半途崩溃产物)
    let mut orphan_files = 0usize;
    let mut orphan_bytes = 0u64;
    for (p, size) in walk_files(root) {
        let rel = normalize_rel(&p, root);
        if !idx.entries.iter().any(|e| e.file == rel) {
            orphan_files += 1;
            orphan_bytes += size;
        }
    }
    Ok(CacheInfoReport {
        total_indexed_bytes: idx.total_bytes(),
        orphan_files,
        orphan_bytes,
        layers,
    })
}

/// cache gc:先清超龄孤儿(旧版遗留 / tmp 超龄件),再按 LRU + 容量上限淘汰
/// 清单条目。删文件走 atomic::remove(唯一落盘点纪律);`now` 入参便于测试控时。
pub fn cache_gc(root: &Path, capacity_bytes: u64, now: u64) -> Result<GcReport, String> {
    let mut idx = CacheIndex::load(root);
    idx.prune_missing(root);
    let indexed: std::collections::HashSet<String> =
        idx.entries.iter().map(|e| e.file.clone()).collect();
    let mut removed = 0usize;
    let mut freed = 0u64;
    // 第一刀:超龄孤儿(TTL 外的旧版固定文件名、tmp 件)无条件清——新键空间永不复用它们
    for (p, size) in walk_files(root) {
        let rel = normalize_rel(&p, root);
        if indexed.contains(&rel) {
            continue;
        }
        if now.saturating_sub(file_mtime_secs(&p)) <= ORPHAN_TTL_SECS {
            continue; // 可能是并发渲染的在写文件,不动
        }
        cutforge_io::atomic::remove(&p).map_err(|e| e.to_string())?;
        removed += 1;
        freed += size;
    }
    // 第二刀:容量压力下按 LRU 淘汰(清单条目按 last_used_at;新孤儿按 mtime 一并参评)
    let mut candidates: Vec<(u64, String, u64)> = Vec::new();
    for e in &idx.entries {
        candidates.push((e.last_used_at, e.file.clone(), e.size));
    }
    for (p, size) in walk_files(root) {
        let rel = normalize_rel(&p, root);
        if !indexed.contains(&rel) {
            candidates.push((file_mtime_secs(&p), rel, size));
        }
    }
    candidates.sort_by_key(|(rank, rel, _)| (*rank, rel.clone()));
    let mut total: u64 = candidates.iter().map(|(_, _, s)| *s).sum();
    for (_, rel, size) in &candidates {
        if total <= capacity_bytes {
            break;
        }
        cutforge_io::atomic::remove(&root.join(rel)).map_err(|e| e.to_string())?;
        idx.entries.retain(|e| e.file != *rel);
        total = total.saturating_sub(*size);
        removed += 1;
        freed += size;
    }
    idx.entries.retain(|e| root.join(&e.file).is_file());
    idx.save(root)?;
    Ok(GcReport { removed, freed_bytes: freed, remaining_bytes: total, capacity_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(tag: &str) -> TempRoot {
            let dir = std::env::temp_dir().join(format!("cf-cache-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempRoot(dir)
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 写一个 size 字节的假缓存文件并登记(测试用;不走 ffmpeg)。
    fn put(root: &Path, idx: &mut CacheIndex, layer: &str, key: &str, size: usize, used: u64) -> PathBuf {
        let rel = idx.record(layer, key, serde_json::json!({"k": key}), used);
        let full = root.join(&rel);
        let data = vec![0u8; size];
        cutforge_io::atomic::atomic_write(&full, &data).unwrap();
        idx.set_size(layer, key, size as u64);
        full
    }

    #[test]
    fn key_changes_when_any_input_changes() {
        let v = |x: u64| json!({"v": "cutforge-render-4.0", "clip": x});
        assert_eq!(key_hex(&v(1)), key_hex(&v(1)), "同输入同键(确定性)");
        assert_ne!(key_hex(&v(1)), key_hex(&v(2)), "输入变 → 键变(零陈旧复用的根基)");
    }

    /// 册四 A4 T4.4/T4.9 验证:seg 键对 clip JSON 全量哈希——新字段(曲线/倒放/
    /// 变换)入 clip JSON 后自动改变键;本测试用真 Clip 走 seg_spec 证明逐字段敏感。
    #[test]
    fn seg_key_is_sensitive_to_new_time_transform_fields() {
        let mk = |extra: &str| -> (RenderPlan, Clip) {
            let v: Value = serde_json::from_str(&format!(
                r#"{{"version":1,"schemaVersion":"2.0.0","slug":"k","fps":30,
                    "canvas":{{"width":1080,"height":1920}},
                    "tracks":[{{"id":"V1","kind":"video","clips":[
                        {{"id":"V1-001","src":"a.mp4","startMs":0,"durationMs":2000{extra}}} ]}}]}}"#
            ))
            .unwrap();
            let p: cutforge_core::model::Project = serde_json::from_value(v).unwrap();
            let plan = RenderPlan::build(&p, Path::new("/w"), None);
            let clip = p.tracks[0].clips[0].clone();
            (plan, clip)
        };
        let key_of = |extra: &str| {
            let (plan, clip) = mk(extra);
            seg_key(&plan, &clip, 0.0)
        };
        let base = key_of("");
        assert_eq!(base, key_of(""), "同输入同键");
        for extra in [
            r#", "speedCurve":[{"atMs":0,"speed":2.0}]"#,
            r#", "reverse":true"#,
            r#", "rotation":90"#,
            r#", "crop":{"x":0,"y":0,"w":100,"h":100}"#,
            r#", "flip":"h""#,
            r#", "freezeMs":500"#,
        ] {
            assert_ne!(base, key_of(extra), "新字段必须改变 seg 键: {extra}");
        }
    }

    /// IR v3(T5.1):seg 键对 clip JSON 全量哈希——keyframes 自动入键,且
    /// **逐字段改键**:动任一关键帧的 property/timeMs/value/interp/bezier 或
    /// 增删一条都必换键(陈旧复用零容忍;keyframed clip 渲染产物随曲线变)。
    #[test]
    fn seg_key_is_sensitive_to_keyframes_field_by_field() {
        let mk = |kfs: &str| -> (RenderPlan, Clip) {
            let raw = r#"{"version":1,"schemaVersion":"3.0.0","slug":"kf","fps":30,
                    "canvas":{"width":1080,"height":1920},
                    "tracks":[{"id":"V1","kind":"video","clips":[
                        {"id":"V1-001","src":"a.mp4","startMs":0,"durationMs":2000@@KFS@@} ]}]}"#
                .replace("@@KFS@@", kfs);
            let v: Value = serde_json::from_str(&raw).unwrap();
            let p: cutforge_core::model::Project = serde_json::from_value(v).unwrap();
            let plan = RenderPlan::build(&p, Path::new("/w"), None);
            (plan, p.tracks[0].clips[0].clone())
        };
        let key_of = |kfs: &str| {
            let (plan, clip) = mk(kfs);
            seg_key(&plan, &clip, 0.0)
        };
        let base = key_of(r#", "keyframes":[
            {"property":"position.x","timeMs":0,"value":0.5},
            {"property":"position.x","timeMs":1000,"value":0.7,"interp":"linear"}]"#);
        // property 变
        assert_ne!(base, key_of(r#", "keyframes":[
            {"property":"position.y","timeMs":0,"value":0.5},
            {"property":"position.x","timeMs":1000,"value":0.7}]"#), "property 变必换键");
        // timeMs 变
        assert_ne!(base, key_of(r#", "keyframes":[
            {"property":"position.x","timeMs":100,"value":0.5},
            {"property":"position.x","timeMs":1000,"value":0.7}]"#), "timeMs 变必换键");
        // value 变
        assert_ne!(base, key_of(r#", "keyframes":[
            {"property":"position.x","timeMs":0,"value":0.6},
            {"property":"position.x","timeMs":1000,"value":0.7}]"#), "value 变必换键");
        // interp 变
        assert_ne!(base, key_of(r#", "keyframes":[
            {"property":"position.x","timeMs":0,"value":0.5,"interp":"hold"},
            {"property":"position.x","timeMs":1000,"value":0.7}]"#), "interp 变必换键");
        // bezier 控制柄变
        assert_ne!(
            key_of(r#", "keyframes":[
                {"property":"position.x","timeMs":0,"value":0.5,"interp":"bezier","bezier":[0.3,0,0.7,1]},
                {"property":"position.x","timeMs":1000,"value":0.7}]"#),
            key_of(r#", "keyframes":[
                {"property":"position.x","timeMs":0,"value":0.5,"interp":"bezier","bezier":[0.1,0,0.9,1]},
                {"property":"position.x","timeMs":1000,"value":0.7}]"#),
            "bezier 控制柄变必换键"
        );
        // 增删一条
        assert_ne!(base, key_of(r#", "keyframes":[
            {"property":"position.x","timeMs":0,"value":0.5}]"#), "删一条必换键");
        // 无关键帧与空差异照常(无 keyframes 字段的 clip 键与 v2 时点同形语义)
        assert_ne!(base, key_of(""), "有关键帧 vs 无关键帧必不同键");
    }

    /// mix 键对 volume 关键帧敏感(IR v3:volume_expr 入键)。
    #[test]
    fn mix_key_is_sensitive_to_volume_keyframes() {
        let mk = |kfs: &str| -> RenderPlan {
            let raw = r#"{"version":1,"schemaVersion":"3.0.0","slug":"m","fps":30,
                    "canvas":{"width":1080,"height":1920},
                    "tracks":[{"id":"V1","kind":"video","clips":[
                        {"id":"V1-001","src":"a.mp4","startMs":0,"durationMs":2000,
                          "role":"voice","volume":1.0@@KFS@@} ]}]}"#
                .replace("@@KFS@@", kfs);
            let p: cutforge_core::model::Project = serde_json::from_str(&raw).unwrap();
            RenderPlan::build(&p, Path::new("/w"), None)
        };
        assert_ne!(
            mix_key(&mk(r#", "keyframes":[{"property":"volume","timeMs":0,"value":1.0},
                {"property":"volume","timeMs":1000,"value":0.0}]"#)),
            mix_key(&mk("")),
            "volume 关键帧必须改变 mix 键"
        );
    }

    /// mix 键对 reverse 敏感(倒放改变混音产物;T4.4)。
    #[test]
    fn mix_key_is_sensitive_to_reverse() {
        let mk = |reverse: &str| -> RenderPlan {
            let v: Value = serde_json::from_str(&format!(
                r#"{{"version":1,"schemaVersion":"2.0.0","slug":"m","fps":30,
                    "canvas":{{"width":1080,"height":1920}},
                    "tracks":[{{"id":"A1","kind":"audio","clips":[
                        {{"id":"A1-001","src":"v.mp3","startMs":0,"durationMs":1000,
                          "volume":1.0{reverse}}} ]}}]}}"#
            ))
            .unwrap();
            RenderPlan::build(&mk_project(v), Path::new("/w"), None)
        };
        fn mk_project(v: Value) -> cutforge_core::model::Project {
            serde_json::from_value(v).unwrap()
        }
        assert_ne!(mix_key(&mk("")), mix_key(&mk(r#", "reverse":true"#)));
    }

    #[test]
    fn frame_key_is_fingerprint_sensitive_and_quantized() {
        let base = ("fp-aaa", 1500u64, (1080u32, 1920u32), "png");
        let k = frame_key(base.0, base.1, base.2, base.3, None);
        // 指纹 / 时间点 / 画幅 / 格式任一变化 → 键变
        assert_ne!(k, frame_key("fp-bbb", base.1, base.2, base.3, None), "指纹入键");
        assert_ne!(k, frame_key(base.0, 1600, base.2, base.3, None), "atMs 入键(量化后)");
        assert_ne!(k, frame_key(base.0, base.1, (1920, 1080), base.3, None), "画幅入键");
        assert_ne!(k, frame_key(base.0, base.1, base.2, "jpeg", None), "格式入键");
        assert_ne!(k, frame_key(base.0, base.1, base.2, base.3, Some(b"[Script]")), "ASS 入键");
        assert_eq!(k, frame_key(base.0, base.1, base.2, base.3, None), "同输入同键(确定性)");
    }

    #[test]
    fn record_with_ext_keeps_explicit_extension() {
        let mut idx = CacheIndex::default();
        let rel = idx.record_with_ext("frame", "k1", json!(1), 10, ".jpg");
        assert!(rel.starts_with("frame/") && rel.to_string_lossy().ends_with("k1.jpg"));
        assert!(idx.touch("frame", "k1", 20).is_some(), "frame 层与其他层同机制命中");
    }

    #[test]
    fn index_touch_increments_hits_and_lru() {
        let mut idx = CacheIndex::default();
        idx.record("seg", "k1", json!(1), 100);
        idx.record("seg", "k2", json!(2), 200);
        let rel = idx.touch("seg", "k1", 300).expect("k1 应命中");
        assert!(rel.starts_with("seg/"));
        assert_eq!(idx.find("seg", "k1").unwrap().hits, 1);
        assert_eq!(idx.find("seg", "k1").unwrap().last_used_at, 300);
        assert!(idx.touch("seg", "nope", 1).is_none());
    }

    #[test]
    fn index_roundtrip_through_disk() {
        let root = TempRoot::new("roundtrip");
        let mut idx = CacheIndex::default();
        put(&root.0, &mut idx, "mix", "abc", 42, 7);
        idx.save(&root.0).unwrap();
        let loaded = CacheIndex::load(&root.0);
        assert_eq!(loaded.entries.len(), 1);
        let e = &loaded.entries[0];
        assert_eq!((e.layer.as_str(), e.key.as_str(), e.size), ("mix", "abc", 42));
        assert_eq!(loaded.total_bytes(), 42);
    }

    #[test]
    fn index_load_missing_or_corrupt_is_empty() {
        let root = TempRoot::new("corrupt");
        cutforge_io::atomic::atomic_write(&root.0.join(INDEX_FILE), b"{not json").unwrap();
        assert!(CacheIndex::load(&root.0).entries.is_empty(), "损坏清单不得误报命中");
        assert!(CacheIndex::load(&TempRoot::new("missing").0).entries.is_empty());
    }

    #[test]
    fn index_prunes_entries_whose_file_vanished() {
        let root = TempRoot::new("prune");
        let mut idx = CacheIndex::default();
        let gone = put(&root.0, &mut idx, "seg", "gone", 10, 1);
        put(&root.0, &mut idx, "seg", "here", 10, 2);
        // 删除亦守唯一落盘点纪律(M2-4):走 atomic::remove
        cutforge_io::atomic::remove(&gone).unwrap();
        idx.prune_missing(&root.0);
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].key, "here");
    }

    #[test]
    fn gc_respects_capacity_and_lru_order() {
        let root = TempRoot::new("gc");
        let mut idx = CacheIndex::default();
        // 三条:used 越小越老 → 先删
        put(&root.0, &mut idx, "seg", "old", 100, 10);
        put(&root.0, &mut idx, "mix", "mid", 100, 20);
        put(&root.0, &mut idx, "sub", "new", 100, 30);
        idx.save(&root.0).unwrap();
        let report = cache_gc(&root.0, 250, now_secs()).unwrap();
        assert_eq!(report.removed, 1, "300 字节压到 250:只删最老的 1 条");
        assert_eq!(report.freed_bytes, 100);
        assert_eq!(report.remaining_bytes, 200);
        let idx = CacheIndex::load(&root.0);
        assert!(idx.find("seg", "old").is_none(), "LRU 最老者被删");
        assert!(idx.find("mix", "mid").is_some());
        assert!(idx.find("sub", "new").is_some());
        // 容量内不动
        let report2 = cache_gc(&root.0, 250, now_secs()).unwrap();
        assert_eq!(report2.removed, 0);
    }

    #[test]
    fn gc_sweeps_stale_orphans_and_tmp() {
        let root = TempRoot::new("orphan");
        let mut idx = CacheIndex::default();
        put(&root.0, &mut idx, "seg", "tracked", 50, 999);
        idx.save(&root.0).unwrap();
        // 旧版固定文件名遗留(无清单条目)
        let junk = vec![0u8; 500];
        cutforge_io::atomic::atomic_write(&root.0.join("composed.mp4"), &junk).unwrap();
        // tmp 件(刚落盘;用 now 前移 TTL 验证超龄判定)
        let tmp = root.0.join(TMP_DIR);
        std::fs::create_dir_all(&tmp).unwrap();
        let junk = vec![0u8; 10];
        cutforge_io::atomic::atomic_write(&tmp.join("concat-x.txt"), &junk).unwrap();
        let future = now_secs() + ORPHAN_TTL_SECS + 10;
        let report = cache_gc(&root.0, DEFAULT_CAPACITY_BYTES, future).unwrap();
        assert!(!root.0.join("composed.mp4").exists(), "超龄孤儿必须被 gc 清理");
        assert!(!tmp.join("concat-x.txt").exists(), "tmp 超龄件必须被 gc 清理");
        assert_eq!(report.capacity_bytes, DEFAULT_CAPACITY_BYTES);
        // 清单内条目容量充足 → 保留
        assert!(CacheIndex::load(&root.0).find("seg", "tracked").is_some());
        // 新孤儿(可能在写)在真实 now 下不清理
        cutforge_io::atomic::atomic_write(&root.0.join("half-written.mp4"), b"x").unwrap();
        cache_gc(&root.0, DEFAULT_CAPACITY_BYTES, now_secs()).unwrap();
        assert!(root.0.join("half-written.mp4").exists(), "新孤儿不得误删");
    }

    #[test]
    fn cache_info_reports_layers_and_orphans() {
        let root = TempRoot::new("info");
        let mut idx = CacheIndex::default();
        put(&root.0, &mut idx, "seg", "s1", 10, 1);
        put(&root.0, &mut idx, "seg", "s2", 20, 2);
        put(&root.0, &mut idx, "mix", "m1", 30, 3);
        idx.save(&root.0).unwrap();
        let junk = vec![0u8; 5];
        cutforge_io::atomic::atomic_write(&root.0.join("subbed.mp4"), &junk).unwrap();
        let r = cache_info(&root.0).unwrap();
        let seg = r.layers.iter().find(|l| l.layer == "seg").unwrap();
        assert_eq!((seg.entries, seg.bytes), (2, 30));
        let mix = r.layers.iter().find(|l| l.layer == "mix").unwrap();
        assert_eq!((mix.entries, mix.bytes), (1, 30));
        assert_eq!((r.orphan_files, r.orphan_bytes), (1, 5), "旧版遗留应记为孤儿");
        assert_eq!(r.total_indexed_bytes, 60);
    }
}
