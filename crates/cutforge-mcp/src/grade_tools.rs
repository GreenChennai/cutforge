// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 调色/示波器/响度/编码探测工具后端(册五 T5.2/T5.3/T5.6;免开工作区:
//! 派生物缓存与纯计算,不改工程 IR,不持排他锁——与 media_tools 同口径):
//! - `lut_import`:.cube 拷入 `.cutforge/luts/` + 结构合法性校验
//!   (3D 主格式,33³ 格主流;解析单一实现 = cutforge-render::grade::parse_cube);
//! - `scope_data`:示波器数据(帧像素 → 亮度波形列采样 / UV 矢量散点聚合 /
//!   RGB 直方图分桶;JSON 落 `.cutforge/scope-cache/`,内容寻址;
//!   壳 canvas 绘制是 FE 活,本工具只供数据);
//! - `audio_loudness`:响度计(ffmpeg loudnorm 测量 LUFS/TP/LRA/阈值 → JSON;
//!   导出响度单的测量面与 mix 双 pass 同源);
//! - `encode_probe`:硬件编码探测(ffmpeg -encoders 清单 + 试编可用性;
//!   渲染端 exec_encode 的降级判定同源 = cutforge-render::encode)。
//!
//! 纯计算核心(波形分桶/矢量聚合/直方图)在本模块内为纯函数,可不装 ffmpeg
//! 单测;ffmpeg/ffprobe 缺失 → DEP_MISSING(协议码如实)。

use crate::dispatch::resolve_within_root;
use crate::registry::envelope;
use serde_json::{json, Value};
use std::path::Path;

fn ff_bin() -> String {
    if let Some(v) = std::env::var_os("CUTFORGE_FFMPEG")
        && !v.is_empty()
    {
        return v.to_string_lossy().into_owned();
    }
    "ffmpeg".into()
}

fn ffmpeg_available() -> bool {
    std::process::Command::new(ff_bin()).arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// f64 → 定点串(去尾零;跨平台 golden 稳定的格式化口径)。
pub(crate) fn fmt_num(v: f64) -> String {
    let s = format!("{v:.6}").trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() || s == "-0" { "0".into() } else { s }
}

// ---------------- lut_import(T5.2) ----------------

/// lut_import 工具面:src(工程内相对路径).cube → 校验(parse_cube)→
/// 拷贝到 `.cutforge/luts/<消毒名>.cube`(同名覆盖 = 重复导入幂等)。
/// 返回登记信息:相对路径(lut 引用值)/标题/LUT_3D_SIZE/数据行数。
pub fn lut_import_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(.cube 工程内相对路径)", json!({}));
    };
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => return envelope(false, "PRECONDITION_FAILED", &format!("路径不合法({src}): {msg}"), json!({})),
    };
    if !abs.to_string_lossy().to_ascii_lowercase().ends_with(".cube") {
        return envelope(false, "PRECONDITION_FAILED", "仅支持 .cube(3D LUT 主格式)", json!({}));
    }
    let Ok(text) = std::fs::read_to_string(&abs) else {
        return envelope(false, "NO_CONFIG", &format!("LUT 不可读: {src}"), json!({}));
    };
    let (title, size, rows) = match cutforge_render::grade::parse_cube(&text) {
        Ok(v) => v,
        Err(e) => return envelope(false, "SCHEMA_INVALID", &format!(".cube 校验失败: {e}"), json!({})),
    };
    // 落库:消毒名(仅字母数字 -_ 与 CJK;重名覆盖 = 幂等导入)
    let stem = abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lut".into());
    let safe: String = stem
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | ' ' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let dir = root.join(".cutforge/luts");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return envelope(false, "INTERNAL", &format!("LUT 库目录创建失败: {e}"), json!({}));
    }
    let mut rel = format!(".cutforge/luts/{safe}.cube");
    let mut target = root.join(&rel);
    // 重名覆盖防混:同名但内容不同 → 追加序号(内容寻址精神;内容相同幂等覆盖)
    if target.is_file() && std::fs::read(&target).map(|b| b != text.as_bytes()).unwrap_or(true) {
        for n in 2..1000 {
            rel = format!(".cutforge/luts/{safe}-{n}.cube");
            target = root.join(&rel);
            if !target.is_file()
                || std::fs::read(&target).map(|b| b == text.as_bytes()).unwrap_or(false)
            {
                break;
            }
        }
    }
    if let Err(e) = std::fs::write(&target, text.as_bytes()) {
        return envelope(false, "INTERNAL", &format!("LUT 落库失败: {e}"), json!({}));
    }
    envelope(true, "OK", "LUT 已导入", json!({
        "lut": rel.replace('\\', "/"),
        "title": title,
        "size3d": size,
        "rows": rows,
    }))
}

// ---------------- scope_data(T5.2 示波器数据后端) ----------------

/// 波形列采样(T5.2):rgb24 像素 → 列亮度统计。cols 列 × 每列 [min, max, avg]
/// (luma = bt709 加权;avg 保留 2 位)。纯函数,确定性。
pub fn waveform_columns(px: &[u8], w: usize, h: usize, cols: usize) -> Vec<[f64; 3]> {
    let mut out = Vec::with_capacity(cols);
    let per = w.max(1).div_ceil(cols);
    for c in 0..cols {
        let x0 = c * per;
        let x1 = ((c + 1) * per).min(w);
        let mut mn = 255.0f64;
        let mut mx = 0.0f64;
        let mut sum = 0.0f64;
        let mut n = 0u64;
        if x0 < x1 {
            for x in x0..x1 {
                for y in (0..h).step_by(2) {
                    let i = (y * w + x) * 3;
                    let l = (px[i] as u32 * 299 + px[i + 1] as u32 * 587 + px[i + 2] as u32 * 114) as f64 / 1000.0;
                    mn = mn.min(l);
                    mx = mx.max(l);
                    sum += l;
                    n += 1;
                }
            }
        }
        if n == 0 {
            out.push([0.0, 0.0, 0.0]);
        } else {
            out.push([mn, mx, (sum / n as f64 * 100.0).round() / 100.0]);
        }
    }
    out
}

/// UV 矢量聚合(T5.2):rgb24 像素 → BINS×BINS 网格计数(U/V ∈ [-0.5,0.5) 映射
/// 网格;行优先,行 0 = V=-0.5)。纯函数,确定性(采样步长 2 防超大帧)。
pub fn vectorscope_bins(px: &[u8], w: usize, h: usize, bins: usize) -> Vec<u64> {
    let mut grid = vec![0u64; bins * bins];
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let i = (y * w + x) * 3;
            let (r, g, b) = (px[i] as f64, px[i + 1] as f64, px[i + 2] as f64);
            // BT.601 UV(RGB→YUV 的标准系数;矢量示波器惯例)
            let u = (b - (r * 0.169 + g * 0.331 + b * 0.5)) * 0.492;
            let v = (r - (r * 0.5 + g * 0.419 + b * 0.081)) * 0.877;
            let cu = ((u + 0.5).clamp(0.0, 0.9999) * bins as f64) as usize;
            let cv = ((v + 0.5).clamp(0.0, 0.9999) * bins as f64) as usize;
            grid[cv * bins + cu] += 1;
        }
    }
    grid
}

/// RGB 直方图(T5.2):rgb24 像素 → 三通道 BINS 桶计数。纯函数,确定性。
pub fn rgb_histograms(px: &[u8], bins: usize) -> [Vec<u64>; 3] {
    let mut out = [vec![0u64; bins], vec![0u64; bins], vec![0u64; bins]];
    let scale = bins as f64 / 256.0;
    for [r, g, b] in px.as_chunks::<3>().0 {
        // bins 恒 ≥1(工具面固定 64);256 与 bins 对齐时无余桶
        out[0][((*r as f64 * scale) as usize).min(bins - 1)] += 1;
        out[1][((*g as f64 * scale) as usize).min(bins - 1)] += 1;
        out[2][((*b as f64 * scale) as usize).min(bins - 1)] += 1;
    }
    out
}

/// scope_data 工具面:src(工程内相对路径,图片或视频)+ atMs(视频抽帧)+
/// 分辨率档 width(缺省 256;64..512)→ 三类数据 JSON。
/// 缓存:`.cutforge/scope-cache/<key>.json`(内容寻址 = 路径+mtime+size+参数)。
pub fn scope_data_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(帧/视频工程内相对路径)", json!({}));
    };
    let at_ms = args["atMs"].as_u64().unwrap_or(0);
    let width = args["width"].as_u64().unwrap_or(256).clamp(64, 512) as u32;
    let cols = args["waveCols"].as_u64().unwrap_or(128).clamp(16, 512) as usize;
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => return envelope(false, "PRECONDITION_FAILED", &format!("路径不合法({src}): {msg}"), json!({})),
    };
    let Some((mtime, size)) = cutforge_io::mediacache::source_stamp(&abs) else {
        return envelope(false, "NO_CONFIG", &format!("素材不可读: {src}"), json!({}));
    };
    let key = cutforge_io::mediacache::media_key(
        "scope", src, mtime, size, &[&at_ms.to_string(), &width.to_string(), &cols.to_string()],
    );
    let rel = format!(".cutforge/scope-cache/{key}.json");
    let out_path = root.join(&rel);
    if out_path.is_file()
        && let Ok(text) = std::fs::read_to_string(&out_path)
        && let Ok(scope) = serde_json::from_str::<Value>(&text)
    {
        return envelope(true, "OK", "示波器数据(缓存命中)", json!({"cached": true, "key": key, "scope": scope}));
    }
    if !ffmpeg_available() {
        return envelope(false, "DEP_MISSING", "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)", json!({}));
    }
    let height = if width >= 2 { width / 2 * 3 } else { width }; // 3:2 采样面(数据面,非显示面)
    let mut dec = std::process::Command::new(ff_bin());
    dec.args(["-v", "error"]);
    if at_ms > 0 {
        dec.args(["-ss", &format!("{:.3}", at_ms as f64 / 1000.0)]);
    }
    dec.args([
        "-i", &abs.to_string_lossy(),
        "-frames:v", "1",
        "-vf", &format!("scale={width}:{height}"),
        "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
    ]);
    let Ok(dec) = dec.output() else {
        return envelope(false, "DEP_MISSING", "ffmpeg 启动失败", json!({}));
    };
    if !dec.status.success() {
        return envelope(false, "DEP_MISSING",
            &format!("像素解码失败: {}", String::from_utf8_lossy(&dec.stderr).chars().take(200).collect::<String>()), json!({}));
    }
    let px = dec.stdout;
    let expect = width as usize * height as usize * 3;
    if px.len() < expect {
        return envelope(false, "NO_CONFIG",
            &format!("帧解码不完整({}/{} 字节;视频可能短于 atMs)", px.len(), expect), json!({}));
    }
    let (w, h) = (width as usize, height as usize);
    let wave = waveform_columns(&px, w, h, cols);
    let vec_bins = vectorscope_bins(&px, w, h, 64);
    let hist = rgb_histograms(&px, 64);
    let doc = json!({
        "src": src, "atMs": at_ms, "width": width, "height": height,
        "waveform": {"cols": cols, "columns": wave.iter()
            .map(|c| [fmt_num(c[0]), fmt_num(c[1]), fmt_num(c[2])]).collect::<Vec<_>>()},
        "vectorscope": {"bins": 64, "grid": vec_bins},
        "histogram": {"bins": 64, "r": hist[0], "g": hist[1], "b": hist[2]},
    });
    let _ = std::fs::create_dir_all(root.join(".cutforge/scope-cache"));
    let _ = cutforge_io::atomic::atomic_write(&out_path, serde_json::to_string(&doc).unwrap_or_default().as_bytes());
    envelope(true, "OK", "示波器数据", json!({"cached": false, "key": key, "scope": doc}))
}

// ---------------- audio_loudness(T5.3 响度计) ----------------

/// audio_loudness 工具面:src(成片/段落,工程内相对路径)→ ffmpeg loudnorm
/// 测量(与 mix 双 pass 同源参数)→ {inputI, inputTp, inputLra, inputThresh}。
/// 可选 target 校验:deviation = |inputI - target|(AC-5.3 的 ≤1LU 判定依据)。
pub fn audio_loudness_tool(root: &Path, args: &Value) -> Value {
    let Some(src) = args["src"].as_str() else {
        return envelope(false, "PRECONDITION_FAILED", "缺 src(音频/成片工程内相对路径)", json!({}));
    };
    let target = args["target"].as_f64();
    let abs = match resolve_within_root(root, src) {
        Ok(p) => p,
        Err(msg) => return envelope(false, "PRECONDITION_FAILED", &format!("路径不合法({src}): {msg}"), json!({})),
    };
    if !ffmpeg_available() {
        return envelope(false, "DEP_MISSING", "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)", json!({}));
    }
    let ti = target.unwrap_or(-14.0);
    let out = std::process::Command::new(ff_bin())
        .args([
            "-hide_banner", "-nostats",
            "-i", &abs.to_string_lossy(),
            "-filter_complex", &format!("loudnorm=I={}:TP=-1.0:print_format=json", fmt_num(ti)),
            "-f", "null", "-",
        ])
        .output();
    let Ok(out) = out else {
        return envelope(false, "DEP_MISSING", "ffmpeg 启动失败", json!({}));
    };
    if !out.status.success() {
        return envelope(false, "DEP_MISSING",
            &format!("响度测量失败: {}", String::from_utf8_lossy(&out.stderr).chars().take(200).collect::<String>()), json!({}));
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let (Some(s), Some(e)) = (err.rfind('{'), err.rfind('}')) else {
        return envelope(false, "INTERNAL", "loudnorm 测量输出无 JSON", json!({}));
    };
    let Ok(m) = serde_json::from_str::<Value>(&err[s..=e]) else {
        return envelope(false, "INTERNAL", "loudnorm 测量 JSON 解析失败", json!({}));
    };
    let input_i = m["input_i"].as_str().and_then(|v| v.parse::<f64>().ok());
    let mut doc = json!({
        "src": src,
        "inputI": m["input_i"].as_str().map(fmt_str_num),
        "inputTp": m["input_tp"].as_str().map(fmt_str_num),
        "inputLra": m["input_lra"].as_str().map(fmt_str_num),
        "inputThresh": m["input_thresh"].as_str().map(fmt_str_num),
    });
    if let Some(i) = input_i {
        let dev = (i - ti).abs();
        doc["target"] = json!(ti);
        doc["deviation"] = json!((dev * 100.0).round() / 100.0);
        doc["within1LU"] = json!(dev <= 1.0);
    }
    envelope(true, "OK", "响度测量", doc)
}

/// 字符串数值规整("−14.32" → "-14.32";非数值原样,如 "-inf")。
fn fmt_str_num(s: &str) -> String {
    match s.parse::<f64>() {
        Ok(v) => fmt_num(v),
        Err(_) => s.to_string(),
    }
}

// ---------------- encode_probe(T5.6 硬件编码探测) ----------------

/// encode_probe 工具面:ffmpeg -encoders 清单(nvenc/qsv/amf 编译期在位)+
/// 试编可用性(驱动/会话真可用)+ auto 缺省结论(libx264 确定性基线)。
/// 渲染端降级判定同源(cutforge-render::encode::hw_encoder_usable)。
pub fn encode_probe_tool(root: &Path, args: &Value) -> Value {
    let _ = root;
    let no_trial = args["trial"] == json!(false);
    if !ffmpeg_available() {
        return envelope(false, "DEP_MISSING", "ffmpeg 不可用(安装 ffmpeg 或设 CUTFORGE_FFMPEG)", json!({}));
    }
    let Ok(out) = std::process::Command::new(ff_bin()).args(["-hide_banner", "-encoders"]).output() else {
        return envelope(false, "DEP_MISSING", "ffmpeg 启动失败", json!({}));
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let listed = |name: &str| text.lines().any(|l| l.contains(name));
    let mut encoders = serde_json::Map::new();
    for name in cutforge_render::encode::HW_CANDIDATES {
        let l = listed(name);
        let usable = if !no_trial && l { cutforge_render::encode::hw_encoder_usable(name) } else { false };
        encoders.insert(
            name.trim_start_matches("h264_").to_string(),
            json!({"listed": l, "usable": usable}),
        );
    }
    envelope(true, "OK", "编码器探测", json!({
        "encoders": Value::Object(encoders),
        "autoDefault": "libx264",
        "note": "auto/缺省 = libx264 确定性基线;hw = 优先硬件,试编失败优雅降级(AC-5.6)",
    }))
}
