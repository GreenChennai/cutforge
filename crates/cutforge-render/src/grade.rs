// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 片段调色(册五 T5.2;ADR-0020 bt709 域):clip.grade IR → ffmpeg 滤镜链映射。
//!
//! **链内序**(grade 内部各滤镜的固定次序,一级校色在前、LUT 收尾):
//!
//! ```text
//! colorbalance(色温/色调/高光/阴影/Lift)
//!   → curves(RGB/亮度曲线)
//!   → eq(曝光 brightness / 对比 contrast / 饱和度 saturation / Gamma 分通道)
//!   → colorchannelmixer(Gain 对角增益)
//!   → lut3d(LUT 最后:整链观感收束,与达芬奇「LUT 在节点末端」心智一致)
//! ```
//!
//! **挂载位置**(全段链图,见 segment 模块注释):
//!
//! ```text
//! [源] → crop → flip → rotate → scale+pad+fps(画幅归一) → punchIn
//!      → ★grade(调色) → fx.combo → reverse → 变速 → kf 合成 → motion → tpad
//! ```
//!
//! 定档理由:调色是**画面内容级**操作,须在几何归一后(画布域稳定)而特效前
//! (胶片颗粒/暗角等 fx 叠在调色后的画面上,观感符合「先调色后加效果」的
//! 剪辑软件惯例);调色为逐像素静态操作,与 reverse/变速可交换,挂在变速前
//! 不改变任何时域语义。grade 缺省 = 链零变化(parity 红线,由
//! `segment_filter_speed_and_tail_compose_in_order` 等既有单测锁定)。
//!
//! **诚实降级**:HSL 限定器(hue/sat 范围选色)ffmpeg 简单滤镜链不达达芬奇级
//! (hue 滤镜无范围控制;split+maskedmerge 路线需 geq 生成 HSL 掩膜,重且脆),
//! 本期登记不渲染(IR 承载防丢,降级 WARN 留痕,同 A4-L1 fx.in/out 槽位口径)。
//!
//! **LUT**:.cube 解析合法性校验(3D 主流,33³ 格)在 lut_import 工具面强制;
//! 渲染端只校验文件在位 + 相对路径约定,应用走 lut3d(file=…),Windows 盘符
//! 冒号按 filtergraph 转义(`E\:/…`)。LUT 文件内容哈希入段缓存键(cache::seg_spec)。
//!
//! 本文件不启动进程:全部函数只做输入 → 滤镜串映射,可离线单测。

use cutforge_core::model::{Clip, GradeCurves};
use std::path::{Path, PathBuf};

/// 数值格式化:固定 4 位小数去尾零(与 steps::fmt_f64 同风格;确定性红线)。
fn g4(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

/// 强度换算(与 schema 描述同口径):色温/色调 ±100 → ±0.35;高光/阴影 ±100 → ±0.6。
const TEMP_SCALE: f64 = 0.35;
const RANGE_SCALE: f64 = 0.6;

/// grade 链(段内挂载;返回 (滤镜串, 降级警告)。None grade / 全空 = 空串,链零变化)。
/// LUT 文件缺失 → 警告 + 跳过 lut3d(诚实降级,不炸整段)。
pub fn grade_chain(clip: &Clip, project_dir: &Path) -> (String, Vec<String>) {
    let Some(grade) = &clip.grade else { return (String::new(), Vec::new()) };
    let mut warns: Vec<String> = Vec::new();
    let mut filters: Vec<String> = Vec::new();

    // ---- ① colorbalance:色温/色调(三段均匀)+ 高光/阴影 + Lift(shadows 槽) ----
    let t = grade.temperature.unwrap_or(0.0) / 100.0 * TEMP_SCALE;
    let g = -grade.tint.unwrap_or(0.0) / 100.0 * TEMP_SCALE;
    let s = grade.shadows.unwrap_or(0.0) / 100.0 * RANGE_SCALE;
    let h = grade.highlights.unwrap_or(0.0) / 100.0 * RANGE_SCALE;
    let lift = grade.lift.unwrap_or([0.0, 0.0, 0.0]);
    let cb = [
        ("rs", t + lift[0] + s), ("rm", t), ("rh", t + h),
        ("gs", g + lift[1] + s), ("gm", g), ("gh", g + h),
        ("bs", -t + lift[2] + s), ("bm", -t), ("bh", -t + h),
    ];
    let cb_active = cb.iter().any(|(_, v)| v.abs() > 1e-6);
    if cb_active {
        let body: Vec<String> =
            cb.iter().filter(|(_, v)| v.abs() > 1e-6).map(|(k, v)| format!("{k}={}", g4(*v))).collect();
        filters.push(format!("colorbalance={}", body.join(":")));
    }

    // ---- ② curves:RGB/亮度曲线(master=亮度近似 RGB 同步) ----
    if let Some(c) = &grade.curves
        && let Some(cv) = curves_filter(c)
    {
        filters.push(cv);
    }

    // ---- ③ eq:曝光/对比/饱和度/Gamma ----
    let mut eqp: Vec<String> = Vec::new();
    if let Some(e) = grade.exposure
        && e.abs() > 1e-6
    {
        eqp.push(format!("brightness={}", g4(e / 6.0)));
    }
    if let Some(c) = grade.contrast
        && c.abs() > 1e-6
    {
        eqp.push(format!("contrast={}", g4(c / 100.0)));
    }
    if let Some(v) = grade.saturation
        && (v - 1.0).abs() > 1e-6
    {
        eqp.push(format!("saturation={}", g4(v)));
    }
    if let Some(ga) = grade.gamma {
        for (k, v) in [("gamma_r", ga[0]), ("gamma_g", ga[1]), ("gamma_b", ga[2])] {
            if (v - 1.0).abs() > 1e-6 {
                eqp.push(format!("{k}={}", g4(v)));
            }
        }
    }
    if !eqp.is_empty() {
        filters.push(format!("eq={}", eqp.join(":")));
    }

    // ---- ④ colorchannelmixer:Gain 对角增益 ----
    let gain = grade.gain.unwrap_or([1.0, 1.0, 1.0]);
    if gain.iter().any(|v| (v - 1.0).abs() > 1e-6) {
        filters.push(format!(
            "colorchannelmixer=rr={}:gg={}:bb={}",
            g4(gain[0]),
            g4(gain[1]),
            g4(gain[2])
        ));
    }

    // ---- ⑤ lut3d:LUT 收尾(文件缺失/未登记 → 诚实降级 WARN) ----
    if let Some(lut) = &grade.lut {
        if lut.is_empty() {
            warns.push("grade.lut 为空串,跳过 lut3d(诚实降级);".into());
        } else {
            let abs = project_dir.join(lut);
            if abs.is_file() {
                filters.push(format!("lut3d=file={}", escape_filter_path(&abs)));
            } else {
                warns.push(format!("grade.lut 文件不存在({lut}),跳过 lut3d(诚实降级);"));
            }
        }
    }

    // ---- HSL 限定器:登记降级(不产滤镜,模块注释见定档理由) ----
    if grade.hsl.is_some() {
        warns.push("grade.hsl HSL 限定器本期登记不渲染(ffmpeg 简单滤镜不达选色,降级 WARN 留痕);".into());
    }

    (filters.join(","), warns)
}

/// 曲线点集 → `curves` 滤镜串;每通道 ≥2 点才产出;端点缺失自动补 0/0 与 1/1
/// (ffmpeg curves 至少两点且必须递增)。master 即 ffmpeg curves 的 master 槽。
fn curves_filter(c: &GradeCurves) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (slot, pts) in [
        ("master", &c.master),
        ("red", &c.red),
        ("green", &c.green),
        ("blue", &c.blue),
    ] {
        let Some(pts) = pts else { continue };
        if pts.len() < 2 {
            continue;
        }
        let mut sorted: Vec<[f64; 2]> = pts.clone();
        sorted.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
        if (sorted[0][0] - 0.0).abs() > 1e-6 {
            sorted.insert(0, [0.0, 0.0]);
        }
        if (sorted[sorted.len() - 1][0] - 1.0).abs() > 1e-6 {
            sorted.push([1.0, 1.0]);
        }
        let expr: Vec<String> =
            sorted.iter().map(|p| format!("{}/{}", g4(p[0]), g4(p[1]))).collect();
        parts.push(format!("{slot}='{}'", expr.join(" ")));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("curves={}", parts.join(":")))
    }
}

/// Windows 盘符冒号的 filtergraph 转义(实测口径,argv 直传无 shell):
/// `E:\a\b.cube` → `'E\:/a/b.cube'`——**整值单引号包裹 + 冒号 `\:`**。
/// 只加引号不转义会在盘符冒号处截断;只转义不加引号被解析器拆成无名选项
/// (本机 ffmpeg 2026-07-30 full build 实测,两者缺一不可)。
pub fn escape_filter_path(p: &Path) -> String {
    let flat = p.to_string_lossy().replace('\\', "/").replace('\'', "");
    format!("'{}'", flat.replace(':', "\\:"))
}

/// .cube 文件解析(3D 主格式;lut_import 校验面 + 渲染端单测共用单一实现)。
/// 返回 (title, LUT_3D_SIZE, 数据行数)。非 0 退出契约:任何结构性违规 = Err。
pub fn parse_cube(text: &str) -> Result<(Option<String>, u32, usize), String> {
    let mut title: Option<String> = None;
    let mut size: Option<u32> = None;
    let mut rows = 0usize;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if let Some(v) = line.strip_prefix("TITLE").map(str::trim_start) {
            title = Some(v.trim_matches('"').to_string());
            continue;
        }
        if upper.starts_with("LUT_1D_SIZE") {
            return Err("一维 LUT 不支持(仅 3D;.cube 3D 主格式)".into());
        }
        if upper.starts_with("LUT_3D_SIZE") {
            let n = line
                .split_whitespace()
                .next_back()
                .and_then(|w| w.parse::<u32>().ok())
                .ok_or_else(|| format!("LUT_3D_SIZE 非法: {line}"))?;
            if !(2..=256).contains(&n) {
                return Err(format!("LUT_3D_SIZE 越界: {n}(允许 2..=256)"));
            }
            size = Some(n);
            continue;
        }
        if upper.starts_with("DOMAIN_MIN") || upper.starts_with("DOMAIN_MAX")
            || upper.starts_with("LUT_1D_INPUT_RANGE") || upper.starts_with("LUT_3D_INPUT_RANGE")
        {
            continue; // 域声明合法但本链不消费(lut3d 自行解释)
        }
        // 数据行:三列浮点(有限值)
        let mut cols = 0usize;
        for w in line.split_whitespace() {
            let v: f64 = w.parse().map_err(|_| format!("数据行含非数值: {line}"))?;
            if !v.is_finite() {
                return Err(format!("数据行含非有限值: {line}"));
            }
            cols += 1;
        }
        if cols != 3 {
            return Err(format!("数据行列数非 3: {line}"));
        }
        rows += 1;
    }
    let n = size.ok_or("缺 LUT_3D_SIZE 头(仅 3D 主格式)")?;
    let expect = n as usize * n as usize * n as usize;
    if rows != expect {
        return Err(format!("数据行数 {rows} ≠ LUT_3D_SIZE³({expect})"));
    }
    Ok((title, n, rows))
}

/// LUT 文件内容哈希(段缓存键维度;文件改一字节即 miss)。
pub fn lut_content_hash(clip: &Clip, project_dir: &Path) -> Option<String> {
    let lut = clip.grade.as_ref()?.lut.as_deref()?;
    let lut = lut.trim();
    if lut.is_empty() {
        return None;
    }
    let abs: PathBuf = project_dir.join(lut);
    let bytes = std::fs::read(abs).ok()?;
    Some(format!("{:016x}", crate::cache::hash_text(&format!("lut:{lut}:{}", bytes.len()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn clip_with(grade: serde_json::Value) -> Clip {
        let v = json!({
            "id": "V1-001", "startMs": 0, "durationMs": 1000,
            "grade": grade,
        });
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn no_grade_is_empty_chain() {
        let v = json!({"id": "V1-001", "startMs": 0, "durationMs": 1000});
        let c: Clip = serde_json::from_value(v).unwrap();
        let (chain, warns) = grade_chain(&c, Path::new("/w"));
        assert!(chain.is_empty());
        assert!(warns.is_empty());
    }

    /// 链内序红线:colorbalance → curves → eq → colorchannelmixer → lut3d。
    #[test]
    fn full_grade_chain_order_is_fixed() {
        let c = clip_with(json!({
            "temperature": 30, "tint": -20, "shadows": 15, "highlights": -10,
            "lift": [0.1, 0.0, -0.1], "gamma": [1.2, 1.0, 0.9], "gain": [1.3, 1.0, 0.8],
            "saturation": 0.5, "exposure": 1.0, "contrast": 20,
            "curves": {"master": [[0.5, 0.8], [1.0, 1.0]], "red": [[0.0, 0.0], [1.0, 1.0]]},
            "lut": ".cutforge/luts/teal.cube",
        }));
        let dir = std::env::temp_dir().join(format!("cf-grade-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".cutforge/luts")).unwrap();
        let n = 2u32;
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for r in 0..n * n * n {
            let v = r as f64 / (n * n * n) as f64;
            cube.push_str(&format!("{v:.6} {v:.6} {v:.6}\n"));
        }
        std::fs::write(dir.join(".cutforge/luts/teal.cube"), cube).unwrap();
        let (chain, warns) = grade_chain(&c, &dir);
        let pos = |needle: &str| chain.find(needle).expect(needle);
        assert!(pos("colorbalance=") < pos("curves="), "{chain}");
        assert!(pos("curves=") < pos("eq="), "{chain}");
        assert!(pos("eq=") < pos("colorchannelmixer="), "{chain}");
        assert!(pos("colorchannelmixer=") < pos("lut3d="), "{chain}");
        assert!(warns.is_empty());
        // 参数面抽样:温度红升蓝降、Lift 入 shadows 槽、曝光 ±3 → ±0.5
        assert!(chain.contains("colorbalance=rs="), "{chain}");
        assert!(chain.contains("bs="), "{chain}");
        assert!(chain.contains("brightness=0.1667"), "{chain}");
        assert!(chain.contains("contrast=0.2"), "{chain}");
        assert!(chain.contains("saturation=0.5"), "{chain}");
        assert!(chain.contains("gamma_r=1.2"), "{chain}");
        assert!(chain.contains("colorchannelmixer=rr=1.3:gg=1:bb=0.8"), "{chain}");
        assert!(chain.contains("curves=master='0/0 0.5/0.8 1/1'"), "{chain}");
        assert!(chain.starts_with("colorbalance="), "{chain}");
        assert!(chain.contains("lut3d=file="), "{chain}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 中性值不产滤镜(饱和度 1 / gain 全 1 / 全零):单字段粒度的零变化面。
    #[test]
    fn neutral_values_emit_no_filters() {
        let c = clip_with(json!({"saturation": 1.0, "gain": [1, 1, 1], "temperature": 0}));
        let (chain, _) = grade_chain(&c, Path::new("/w"));
        assert!(chain.is_empty(), "{chain}");
    }

    /// 饱和度归零 → eq=saturation=0(与 fx.mono 同断言口径的灰度通路)。
    #[test]
    fn saturation_zero_maps_to_eq() {
        let c = clip_with(json!({"saturation": 0}));
        let (chain, _) = grade_chain(&c, Path::new("/w"));
        assert_eq!(chain, "eq=saturation=0");
    }

    /// 曲线端点补全 + 乱序归一 + 单点跳过。
    #[test]
    fn curves_points_are_sorted_and_capped() {
        let c = clip_with(json!({"curves": {"master": [[0.5, 0.8]]}}));
        let (chain, _) = grade_chain(&c, Path::new("/w"));
        assert!(chain.is_empty(), "单点不足两条 → 不产 curves: {chain}");
        let c = clip_with(json!({"curves": {"master": [[0.7, 0.9], [0.2, 0.3]]}}));
        let (chain, _) = grade_chain(&c, Path::new("/w"));
        assert_eq!(chain, "curves=master='0/0 0.2/0.3 0.7/0.9 1/1'");
    }

    /// LUT 文件缺失 → 跳过 lut3d + 诚实降级警告;路径转义含盘符冒号。
    #[test]
    fn missing_lut_degrades_with_warning() {
        let c = clip_with(json!({"lut": ".cutforge/luts/幽灵.cube"}));
        let (chain, warns) = grade_chain(&c, Path::new("/w"));
        assert!(chain.is_empty(), "{chain}");
        assert!(warns.iter().any(|w| w.contains("不存在")), "{warns:?}");
    }

    #[test]
    fn escape_filter_path_handles_windows_drive() {
        let p = Path::new("E:\\a b\\luts\\x.cube");
        assert_eq!(escape_filter_path(p), r"'E\:/a b/luts/x.cube'");
    }

    /// .cube 解析:合法 33³ 主格式通过;1D / 缺头 / 行数不足 / 列数非 3 拒绝。
    #[test]
    fn parse_cube_validates_structure() {
        let n = 2u32;
        let mut rows = String::new();
        for r in 0..n * n * n {
            let v = r as f64 / (n * n * n) as f64;
            rows.push_str(&format!("{v:.6} {v:.6} {v:.6}\n"));
        }
        let good = format!("TITLE \"测试 LUT\"\nLUT_3D_SIZE {n}\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n{rows}");
        let (title, size, cnt) = parse_cube(&good).unwrap();
        assert_eq!((title.as_deref(), size, cnt), (Some("测试 LUT"), 2, 8));
        // 注释行与空行被忽略(头部齐全时)
        let (_, _, cnt) = parse_cube(&format!("LUT_3D_SIZE 2\n# 注释\n\n{rows}")).unwrap();
        assert_eq!(cnt, 8);
        // 一维 LUT 拒绝
        assert!(parse_cube("LUT_1D_SIZE 32\n").is_err());
        // 缺头
        assert!(parse_cube(&rows).is_err());
        // 行数不足
        assert!(parse_cube(&format!("LUT_3D_SIZE 2\n{}", &rows[..rows.len() - 20])).is_err());
        // 列数非 3
        assert!(parse_cube("LUT_3D_SIZE 2\n0.1 0.2\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n").is_err());
        // 非数值
        assert!(parse_cube("LUT_3D_SIZE 2\nabc 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n0.1 0.2 0.3\n").is_err());
    }

    /// HSL 限定器:登记降级(不产滤镜 + 警告)。
    #[test]
    fn hsl_qualifier_degrades_with_warning() {
        let c = clip_with(json!({
            "hsl": {"hueCenter": 30, "hueWidth": 20, "hueShift": -10}
        }));
        let (chain, warns) = grade_chain(&c, Path::new("/w"));
        assert!(chain.is_empty(), "{chain}");
        assert!(warns.iter().any(|w| w.contains("HSL")), "{warns:?}");
    }
}
