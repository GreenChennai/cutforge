// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! `.cfpkg` 工程打包/解包(册七 T7.6):工程 + 素材 + OpLog(+ 可选产物)的单文件
//! zip 容器,"时间错开的多机搬运/分享"形态(ADR-0026:多实例协作暂缓的现实答案)。
//!
//! 容器布局(zip 条目名恒正斜杠):
//! - `manifest.json`  — 格式自描述(format/formatVersion/name/schemaVersion/rev/…);
//! - `project/…`      — 五真相源文件,一律按 v3 契约名存放(project.json /
//!   wordline.json/cutlist.json/cutlist.applied.json/notes.json;v1/v2 源工程上提
//!   到 v3 名,project.json 内容零字节改动);
//! - `oplog/…`        — `.cutforge/oplog/*.jsonl`(append-only 原样随包,撤销链保留);
//! - `media/<工程内相对路径>` — 打包按**引用收集**(project.json 全部 clips(含复合
//!   子时间线)与 bgm.src 引用到的文件),条目名保留工程内相对路径 → 解包原位还原,
//!   src 相对路径零改写;缺文件记入 manifest.missing 并 WARN(不阻断打包);
//! - `exports/…`      — 产物目录(includeExports,缺省 false)。
//!
//! 纪律:zip 原语走 `zipstore` 单一实现;全部落盘经 `atomic::atomic_write`(唯一
//! 落盘点);打包持工程锁(与 migrate 同先例,防撕裂快照);解包目标必须**不存在**
//! (拒绝覆盖,otio_import/project_new 同纪律);解包条目名防 zip-slip(拒绝对外
//! 穿越/绝对路径/反斜杠)。

use crate::atomic::atomic_write;
use crate::lock;
use crate::paths::{self, LayoutKind};
use crate::zipstore::{zip_read_all, zip_store, Entry};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

/// 容器格式标识与版本(manifest 校验锚)。
pub const FORMAT: &str = "cfpkg";
pub const FORMAT_VERSION: u64 = 1;

/// zip 内真相源文件名 → v3 契约位(解包映射;顺序即打包收集序)。
const TRUTH_FILES: [(&str, &str); 5] = [
    ("project.json", paths::V3_PROJECT_REL),
    ("wordline.json", paths::V3_WORDLINE_REL),
    ("cutlist.json", paths::V3_CUTLIST_REL),
    ("cutlist.applied.json", paths::V3_CUTLIST_APPLIED_REL),
    ("notes.json", paths::NOTES_REL),
];

/// 打包报告(逐项如实;missing = project.json 引用但盘上缺的素材)。
#[derive(Debug, Clone, PartialEq)]
pub struct PackReport {
    pub out: PathBuf,
    pub name: String,
    pub schema_version: String,
    pub rev: u64,
    pub source_layout: &'static str,
    pub include_media: bool,
    pub include_exports: bool,
    /// manifest 里 missing 的素材(引用在、文件缺)。
    pub missing: Vec<String>,
    /// 收集到的媒体文件数 / OpLog 文件数 / 产物文件数 / 真相源文件数。
    pub counts: (usize, usize, usize, usize),
    pub bytes: usize,
}

/// 解包报告。
#[derive(Debug, Clone, PartialEq)]
pub struct UnpackReport {
    pub dest: PathBuf,
    pub name: String,
    pub source_layout: String,
    pub files: usize,
    pub media: usize,
    /// 随包 manifest 里就缺失的素材(诚实转述;解包不重扫工程)。
    pub missing: Vec<String>,
}

#[derive(Debug)]
pub enum PkgError {
    /// root 不是可打开的工程(三态皆无 project.json)。
    NotAProject(PathBuf),
    /// 容器不合法(不是 zip / manifest 缺失或格式不符 / 缺 project 真相源)。
    InvalidPkg(String),
    /// 目标已存在(解包拒绝覆盖)。
    Conflict(String),
    /// 打包持锁失败(活进程持锁)。
    Locked(String),
    Io(io::Error),
}

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgError::NotAProject(p) => {
                write!(f, "不是可打开的工程(三态布局皆无 project.json): {}", p.display())
            }
            PkgError::InvalidPkg(m) => write!(f, "cfpkg 容器不合法: {m}"),
            PkgError::Conflict(m) => write!(f, "CONFLICT: {m}(整体拒绝,盘面未动)"),
            PkgError::Locked(m) => write!(f, "工程被其他进程锁定: {m}"),
            PkgError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for PkgError {
    fn from(e: io::Error) -> Self {
        PkgError::Io(e)
    }
}

fn layout_label(k: LayoutKind) -> &'static str {
    match k {
        LayoutKind::Legacy => "v1",
        LayoutKind::V2 => "v2",
        LayoutKind::V3 => "v3",
    }
}

/// 工程内相对路径安全判定(zip 条目名与素材引用共用):非空、非绝对、
/// 无 `..` 段、无反斜杠、无盘符、无 URL 形(zip-slip 防线,解包侧强制、
/// 打包侧对素材引用同样口径)。
fn safe_rel(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.split('/').any(|seg| seg == ".." || seg.is_empty())
        && name != "." && !name.ends_with('/')
}

/// 收集 project.json 里引用的全部素材相对路径(clips 含复合子时间线 + bgm.src)。
fn collect_media_refs(project: &Value) -> Vec<String> {
    let mut refs = BTreeSet::new();
    let walk_clips = |clips: &Value, refs: &mut BTreeSet<String>| {
        if let Some(arr) = clips.as_array() {
            for c in arr {
                if let Some(src) = c["src"].as_str().filter(|s| !s.is_empty()) {
                    refs.insert(src.to_string());
                }
            }
        }
    };
    if let Some(tracks) = project["tracks"].as_array() {
        for t in tracks {
            walk_clips(&t["clips"], &mut refs);
        }
    }
    if let Some(src) = project["bgm"]["src"].as_str().filter(|s| !s.is_empty()) {
        refs.insert(src.to_string());
    }
    refs.into_iter().filter(|r| safe_rel(r) && !r.contains("://")).collect()
}

/// 读 .cutforge/oplog/*.jsonl 的最大 rev(Op.rev = 应用后修订号;无 OpLog/空 → 0)。
fn read_rev(root: &Path) -> u64 {
    let dir = root.join(".cutforge/oplog");
    let Ok(rd) = std::fs::read_dir(&dir) else { return 0 };
    let mut rev = 0u64;
    for f in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(f.path()) else { continue };
        for line in text.lines() {
            if let Ok(op) = serde_json::from_str::<Value>(line) {
                rev = rev.max(op["rev"].as_u64().unwrap_or(0));
            }
        }
    }
    rev
}

/// 收目录下全部文件(相对 dir 的安全相对路径,排序稳定;上限防御异常大目录)。
fn collect_dir_files(dir: &Path, cap: usize) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Ok(rel) = p.strip_prefix(dir) else { continue };
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if !safe_rel(&rel_s) || out.len() >= cap {
                continue;
            }
            out.push((rel_s, p));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 五真相源的**盘面在位**解析(布局感知;返回 zip 内标准名 → 盘上绝对路径)。
/// project.json 的位置随布局(V3 根 / V2 时间线目录 / V1 05_ir);其余四个经
/// truth_rel_on_disk(布局感知)+ notes.json(恒工程根)。
fn truth_sources(root: &Path) -> Vec<(&'static str, PathBuf)> {
    let project_rel = match paths::detect_layout(root) {
        LayoutKind::V3 => paths::V3_PROJECT_REL,
        LayoutKind::V2 => paths::PROJECT_REL,
        LayoutKind::Legacy => paths::LEGACY_PROJECT_REL,
    };
    let mut v: Vec<(&'static str, PathBuf)> = vec![("project.json", root.join(project_rel))];
    for fname in ["wordline.json", "cutlist.json", "cutlist.applied.json"] {
        if let Some(rel) = paths::truth_rel_on_disk(root, fname) {
            v.push((fname, root.join(rel)));
        }
    }
    v.push(("notes.json", root.join(paths::NOTES_REL)));
    v
}

/// 打包工程为 `.cfpkg`(zip store 形)。out 缺省 = `<root>/<slug>.cfpkg`;
/// includeMedia 缺省 true;includeExports 缺省 false。持工程锁防撕裂快照。
pub fn pack(
    root: &Path,
    out: Option<&Path>,
    include_media: bool,
    include_exports: bool,
) -> Result<PackReport, PkgError> {
    let layout = paths::detect_layout(root);
    if !paths::has_project(root) {
        return Err(PkgError::NotAProject(root.to_path_buf()));
    }
    let project_bytes = std::fs::read(paths::project_path(root))?;
    let project: Value = serde_json::from_slice(&project_bytes)
        .map_err(|e| PkgError::InvalidPkg(format!("project.json 不是合法 JSON: {e}")))?;
    let name = project["slug"].as_str().unwrap_or("cutforge-project").to_string();
    let schema_version = project["schemaVersion"].as_str().unwrap_or("3.0.0").to_string();

    // 持锁执行(锁内复验工程仍在;活进程持锁 → 拒绝,残留锁由 library_recover 接管)
    let _guard = lock::acquire(root, 30_000, 2).map_err(|e| PkgError::Locked(e.to_string()))?;
    if !paths::has_project(root) {
        return Err(PkgError::NotAProject(root.to_path_buf()));
    }

    let mut entries: Vec<Entry> = Vec::new();
    // 1) 真相源:按盘面布局解析在位文件,一律以 v3 契约名入包
    let mut project_count = 0usize;
    for (fname, disk_path) in truth_sources(root) {
        let Ok(bytes) = std::fs::read(&disk_path) else { continue };
        entries.push(Entry { name: format!("project/{fname}"), data: bytes });
        project_count += 1;
    }
    // 2) OpLog(append-only 原样随包)
    let oplog_files = collect_dir_files(&root.join(".cutforge/oplog"), 4096);
    for (rel, p) in &oplog_files {
        entries.push(Entry { name: format!("oplog/{rel}"), data: std::fs::read(p)? });
    }
    // 3) 素材:按引用收集,条目名 = 工程内相对路径原样(解包原位还原,src 零改写)
    let mut missing: Vec<String> = Vec::new();
    let mut media_count = 0usize;
    if include_media {
        for rel in collect_media_refs(&project) {
            let p = root.join(&rel);
            if p.is_file() {
                entries.push(Entry { name: format!("media/{rel}"), data: std::fs::read(&p)? });
                media_count += 1;
            } else {
                missing.push(rel);
            }
        }
        missing.sort();
    }
    // 4) 产物(可选):源布局产物目录整体入包
    let mut exports_count = 0usize;
    if include_exports {
        for (rel, p) in collect_dir_files(&paths::output_dir(root), 4096) {
            entries.push(Entry { name: format!("exports/{rel}"), data: std::fs::read(p)? });
            exports_count += 1;
        }
    }
    // 5) manifest(最后组;createdAt 记录打包时刻)
    let manifest = json!({
        "format": FORMAT,
        "formatVersion": FORMAT_VERSION,
        "name": name,
        "schemaVersion": schema_version,
        "rev": read_rev(root),
        "createdAt": cutforge_core::timeutil::now_rfc3339(),
        "sourceLayout": layout_label(layout),
        "generator": format!("cutforge {}", env!("CARGO_PKG_VERSION")),
        "includeMedia": include_media,
        "includeExports": include_exports,
        "counts": {"project": project_count, "oplog": oplog_files.len(),
                   "media": media_count, "exports": exports_count},
        "missing": missing,
    });
    entries.push(Entry {
        name: "manifest.json".into(),
        data: serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    });

    let out_path = match out {
        Some(p) => p.to_path_buf(),
        None => root.join(format!("{name}.cfpkg")),
    };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let zip = zip_store(&entries, secs);
    let bytes = zip.len();
    atomic_write(&out_path, &zip)?;
    Ok(PackReport {
        out: out_path,
        name,
        schema_version,
        rev: manifest["rev"].as_u64().unwrap_or(0),
        source_layout: layout_label(layout),
        include_media,
        include_exports,
        missing,
        counts: (project_count, oplog_files.len(), media_count, exports_count),
        bytes,
    })
}

/// 解包 `.cfpkg` 到**新工程目录**(目标必须不存在;真相源落 v3 契约位,media/oplog
/// 原位还原)。清单校验:format/formatVersion/project 真相源齐备;条目名防 zip-slip。
pub fn unpack(src: &Path, dest: &Path) -> Result<UnpackReport, PkgError> {
    let zip = std::fs::read(src).map_err(|e| {
        PkgError::InvalidPkg(format!("读取失败({}): {e}", src.display()))
    })?;
    let items = zip_read_all(&zip).map_err(|e| PkgError::InvalidPkg(e.to_string()))?;
    // 清单校验:先 manifest 后 project 真相源
    let manifest_raw = items.iter().find(|e| e.name == "manifest.json")
        .ok_or_else(|| PkgError::InvalidPkg("缺 manifest.json".into()))?;
    let manifest: Value = serde_json::from_slice(&manifest_raw.data)
        .map_err(|e| PkgError::InvalidPkg(format!("manifest.json 不是合法 JSON: {e}")))?;
    if manifest["format"].as_str() != Some(FORMAT) {
        return Err(PkgError::InvalidPkg(format!(
            "format 必须为 \"{FORMAT}\"(实得 {:?})", manifest["format"].as_str())));
    }
    if manifest["formatVersion"].as_u64() != Some(FORMAT_VERSION) {
        return Err(PkgError::InvalidPkg(format!(
            "formatVersion 必须为 {FORMAT_VERSION}(实得 {:?});高版本容器由新版工具解",
            manifest["formatVersion"].as_u64())));
    }
    if !items.iter().any(|e| e.name == "project/project.json") {
        return Err(PkgError::InvalidPkg("缺 project/project.json(容器必须含工程真相源)".into()));
    }
    // 目标:不存在,或存在且为空目录(拒绝覆盖,otio_import/project_new 同纪律)
    if dest.exists() {
        let empty = dest.is_dir()
            && std::fs::read_dir(dest).map(|mut rd| rd.next().is_none()).unwrap_or(false);
        if !empty {
            return Err(PkgError::Conflict(format!(
                "解包目标已存在: {}(拒绝覆盖;换目标或先清理)", dest.display())));
        }
    }
    // 条目落位(project/ → v3 契约位;media/ 原位还原;oplog/ → .cutforge/oplog/;
    // exports/ → exports/;manifest 自身跳过;其余未知前缀拒收——零幻觉面)
    let mut files = 0usize;
    let mut media = 0usize;
    for e in &items {
        if e.name == "manifest.json" {
            continue;
        }
        let Some((prefix, rest)) = e.name.split_once('/') else {
            return Err(PkgError::InvalidPkg(format!("条目 {} 不在已知前缀下", e.name)));
        };
        if !safe_rel(rest) {
            return Err(PkgError::InvalidPkg(format!("条目 {} 路径不合法(拒绝对外穿越)", e.name)));
        }
        let dest_file = match prefix {
            "project" => match TRUTH_FILES.iter().find(|(f, _)| *f == rest) {
                Some((_, v3_rel)) => dest.join(v3_rel),
                None => dest.join(rest), // 未知 project/ 子文件原位保留(前向兼容)
            },
            "media" => {
                media += 1;
                dest.join(rest)
            }
            "oplog" => dest.join(".cutforge/oplog").join(rest),
            "exports" => dest.join(paths::V3_EXPORTS).join(rest),
            other => {
                return Err(PkgError::InvalidPkg(format!(
                    "条目前缀 {other:?} 不在容器契约内(project/media/oplog/exports)")))
            }
        };
        if let Some(parent) = dest_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&dest_file, &e.data)?;
        files += 1;
    }
    std::fs::create_dir_all(dest.join(".cutforge"))?;
    Ok(UnpackReport {
        dest: dest.to_path_buf(),
        name: manifest["name"].as_str().unwrap_or_default().to_string(),
        source_layout: manifest["sourceLayout"].as_str().unwrap_or_default().to_string(),
        files,
        media,
        missing: manifest["missing"].as_array().map(|a| {
            a.iter().filter_map(|v| v.as_str().map(String::from)).collect()
        }).unwrap_or_default(),
    })
}

/// 打包 → 解包 → 再打包的 **byte 语义等价**比较键(测试/工具共用):条目名 → 字节,
/// manifest 的 createdAt 归一(时间戳是唯一合法差异)。
pub fn semantic_entries(entries: &[crate::zipstore::ZipEntry]) -> BTreeSet<(String, Vec<u8>)> {
    let mut set = BTreeSet::new();
    for e in entries {
        let data = if e.name == "manifest.json" {
            match serde_json::from_slice::<Value>(&e.data) {
                Ok(mut m) => {
                    if let Some(o) = m.as_object_mut() {
                        o.insert("createdAt".into(), json!("<TS>"));
                    }
                    serde_json::to_vec(&m).unwrap_or_default()
                }
                Err(_) => e.data.clone(),
            }
        } else {
            e.data.clone()
        };
        set.insert((e.name.clone(), data));
    }
    set
}

/// 便捷:读一个 zip 文件的全部条目(测试与冒烟用)。
pub fn read_pkg(path: &Path) -> Result<Vec<crate::zipstore::ZipEntry>, PkgError> {
    let zip = std::fs::read(path)
        .map_err(|e| PkgError::InvalidPkg(format!("读取失败({}): {e}", path.display())))?;
    zip_read_all(&zip).map_err(|e| PkgError::InvalidPkg(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atomic;
    use crate::fsutil;
    use crate::zipstore::zip_read_all;

    /// 手工造一个 v3 工程(真相源平铺 + media/ + oplog 一笔 + 引用素材)。
    fn make_v3(root: &Path) {
        let project = "{\"version\":1,\"schemaVersion\":\"3.0.0\",\"slug\":\"打包体\",\"fps\":30,\
            \"canvas\":{\"width\":320,\"height\":240},\"backends\":[\"ffmpeg\"],\
            \"bgm\":{\"src\":\"media/bgm.mp3\",\"gainDb\":-18},\
            \"tracks\":[{\"id\":\"V1\",\"kind\":\"video\",\"clips\":[\
            {\"id\":\"V1-001\",\"src\":\"media/a.mp4\",\"startMs\":0,\"durationMs\":1000}]}]}";
        fsutil::ensure(root).unwrap();
        atomic::atomic_write(&root.join("project.json"), project.as_bytes()).unwrap();
        atomic::atomic_write(&root.join("media/a.mp4"), b"AAA").unwrap();
        atomic::atomic_write(&root.join("media/bgm.mp3"), b"BBB").unwrap();
        atomic::atomic_write(&root.join(".cutforge/oplog/2026-09-30.jsonl"),
            b"{\"op_id\":\"op1\",\"rev\":3}\n").unwrap();
        atomic::atomic_write(&root.join("exports/final.mp4"), b"OUT").unwrap();
    }

    /// v3 工程打包:manifest/真相源/oplog/media 齐备;rev 取 oplog 最大值;
    /// exports 缺省不入包。
    #[test]
    fn pack_v3_container_layout() {
        let root = fsutil::temp_dir("cfpkg-pack-v3");
        make_v3(&root);
        let r = pack(&root, None, true, false).unwrap();
        assert_eq!(r.name, "打包体");
        assert_eq!(r.schema_version, "3.0.0");
        assert_eq!(r.rev, 3);
        assert_eq!(r.source_layout, "v3");
        assert_eq!(r.counts, (1, 1, 2, 0), "真相源1 + oplog1 + media2 + exports0");
        assert!(r.missing.is_empty());
        assert!(r.out.is_file(), "缺省落点 <root>/<slug>.cfpkg");
        let items = read_pkg(&r.out).unwrap();
        let names: Vec<&str> = items.iter().map(|e| e.name.as_str()).collect();
        for want in ["manifest.json", "project/project.json", "oplog/2026-09-30.jsonl",
                     "media/media/a.mp4", "media/media/bgm.mp3"] {
            assert!(names.contains(&want), "缺条目 {want}: {names:?}");
        }
        assert!(!names.iter().any(|n| n.starts_with("exports/")), "exports 缺省不入包");
        let m: Value = serde_json::from_slice(
            &items.iter().find(|e| e.name == "manifest.json").unwrap().data).unwrap();
        assert_eq!(m["format"], json!("cfpkg"));
        assert_eq!(m["formatVersion"], json!(1));
        assert_eq!(m["includeExports"], json!(false));
        fsutil::cleanup(&root);
    }

    /// v2 源工程:真相源上提到 v3 契约名;素材按工程内相对路径原样入包;
    /// 解包后 = v3 布局 + 素材原位还原(src 相对路径零改写)。
    #[test]
    fn pack_v2_source_and_unpack_restores() {
        let root = fsutil::temp_dir("cfpkg-pack-v2");
        fsutil::ensure(&root).unwrap();
        let project = "{\"version\":1,\"schemaVersion\":\"3.0.0\",\"slug\":\"旧布局\",\"fps\":25,\
            \"canvas\":{\"width\":1080,\"height\":1920},\"backends\":[\"ffmpeg\"],\
            \"tracks\":[{\"id\":\"V1\",\"kind\":\"video\",\"clips\":[\
            {\"id\":\"V1-001\",\"src\":\"01_原始素材/x.mp4\",\"startMs\":0,\"durationMs\":500}]}]}";
        atomic::atomic_write(&root.join(paths::PROJECT_REL), project.as_bytes()).unwrap();
        atomic::atomic_write(&root.join(paths::WORDLINE_REL), b"{}").unwrap();
        atomic::atomic_write(&root.join("01_原始素材/x.mp4"), b"X").unwrap();
        let pkg = root.join("share.cfpkg");
        let r = pack(&root, Some(&pkg), true, false).unwrap();
        assert_eq!(r.source_layout, "v2");
        let items = read_pkg(&pkg).unwrap();
        let names: Vec<&str> = items.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"project/project.json"), "真相源上提到 v3 名:{names:?}");
        assert!(names.contains(&"project/wordline.json"));
        assert!(names.contains(&"media/01_原始素材/x.mp4"), "素材按工程内相对路径入包");
        // 解包到新目录:v3 布局 + 素材原位
        let dest = fsutil::temp_dir("cfpkg-unpack-v2");
        let u = unpack(&pkg, &dest).unwrap();
        assert_eq!(u.name, "旧布局");
        assert_eq!(u.source_layout, "v2");
        assert_eq!(u.media, 1);
        assert!(dest.join("project.json").is_file(), "真相源落 v3 契约位(根)");
        assert!(dest.join("wordline.json").is_file());
        assert!(dest.join("01_原始素材/x.mp4").is_file(), "素材原位还原(src 路径成立)");
        assert_eq!(paths::detect_layout(&dest), LayoutKind::V3, "解包产物即 v3 布局");
        // 解包工程可被 Workspace 打开(可独立起步)
        let ws = crate::Workspace::open(&dest).unwrap();
        assert_eq!(ws.rev(), 0);
        fsutil::cleanup(&root);
        fsutil::cleanup(&dest);
    }

    /// 打包 → 解包 → 再打包 byte 语义等价(manifest 时间戳归一后逐条目字节相等)。
    #[test]
    fn pack_unpack_repack_byte_semantic_eq() {
        let root = fsutil::temp_dir("cfpkg-repack-src");
        make_v3(&root);
        let pkg1 = root.join("p1.cfpkg");
        pack(&root, Some(&pkg1), true, true).unwrap();
        let dest = fsutil::temp_dir("cfpkg-repack-dst");
        unpack(&pkg1, &dest).unwrap();
        let pkg2 = dest.join("p2.cfpkg");
        pack(&dest, Some(&pkg2), true, true).unwrap();
        let a = semantic_entries(&read_pkg(&pkg1).unwrap());
        let b = semantic_entries(&read_pkg(&pkg2).unwrap());
        assert_eq!(a, b, "打包→解包→再打包必须语义等价(manifest 仅 createdAt 归一)");
        fsutil::cleanup(&root);
        fsutil::cleanup(&dest);
    }

    /// 缺素材:引用在、文件缺 → 打包成功 + manifest.missing 如实登记(不阻断)。
    #[test]
    fn pack_missing_media_warns_not_fails() {
        let root = fsutil::temp_dir("cfpkg-missing");
        fsutil::ensure(&root).unwrap();
        let project = "{\"slug\":\"缺素材\",\"fps\":30,\"canvas\":{\"width\":10,\"height\":10},\
            \"backends\":[\"ffmpeg\"],\"tracks\":[{\"id\":\"V1\",\"kind\":\"video\",\
            \"clips\":[{\"id\":\"V1-001\",\"src\":\"media/gone.mp4\",\"startMs\":0,\
            \"durationMs\":1}]}]}";
        atomic::atomic_write(&root.join("project.json"), project.as_bytes()).unwrap();
        let pkg = root.join("m.cfpkg");
        let r = pack(&root, Some(&pkg), true, false).unwrap();
        assert_eq!(r.missing, vec!["media/gone.mp4".to_string()]);
        assert_eq!(r.counts.2, 0);
        let items = read_pkg(&pkg).unwrap();
        let m: Value = serde_json::from_slice(
            &items.iter().find(|e| e.name == "manifest.json").unwrap().data).unwrap();
        assert_eq!(m["missing"], json!(["media/gone.mp4"]));
        fsutil::cleanup(&root);
    }

    /// 解包拒绝覆盖既有非空目标;坏 manifest(格式不符/缺真相源)拒收;
    /// zip-slip 条目(穿越/绝对路径/反斜杠)拒绝且盘面零落点。
    #[test]
    fn unpack_guards_and_zip_slip() {
        let root = fsutil::temp_dir("cfpkg-guard");
        make_v3(&root);
        let pkg = root.join("g.cfpkg");
        pack(&root, Some(&pkg), true, false).unwrap();
        // 目标已存在且非空 → CONFLICT
        let dest = fsutil::temp_dir("cfpkg-guard-dst");
        fsutil::ensure(&dest).unwrap();
        std::fs::write(dest.join("占位.txt"), b"x").unwrap();
        assert!(matches!(unpack(&pkg, &dest), Err(PkgError::Conflict(_))));
        // 空 dir 目标允许
        let empty = dest.join("空");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(unpack(&pkg, &empty).is_ok(), "存在但为空的目标放行");
        // 坏 manifest:格式错
        let bad = zip_store(&[
            Entry { name: "manifest.json".into(),
                    data: b"{\"format\":\"other\",\"formatVersion\":1}".to_vec() },
            Entry { name: "project/project.json".into(), data: b"{}".to_vec() },
        ], 0);
        let bad_path = root.join("bad.cfpkg");
        atomic::atomic_write(&bad_path, &bad).unwrap();
        assert!(matches!(unpack(&bad_path, &dest.join("b1")), Err(PkgError::InvalidPkg(_))));
        // 缺 project 真相源
        let no_proj = zip_store(&[
            Entry { name: "manifest.json".into(),
                    data: b"{\"format\":\"cfpkg\",\"formatVersion\":1}".to_vec() },
        ], 0);
        atomic::atomic_write(&root.join("np.cfpkg"), &no_proj).unwrap();
        assert!(matches!(unpack(&root.join("np.cfpkg"), &dest.join("b2")),
                Err(PkgError::InvalidPkg(_))));
        // zip-slip:media/../evil.txt 与绝对/反斜杠条目名全部拒绝,盘面零落点
        for evil in ["media/../evil.txt", "media/C:/evil.txt", "media/\\evil.txt"] {
            let slip = zip_store(&[
                Entry { name: "manifest.json".into(),
                        data: b"{\"format\":\"cfpkg\",\"formatVersion\":1}".to_vec() },
                Entry { name: "project/project.json".into(), data: b"{}".to_vec() },
                Entry { name: evil.into(), data: b"EVIL".to_vec() },
            ], 0);
            atomic::atomic_write(&root.join("slip.cfpkg"), &slip).unwrap();
            let target = dest.join(format!("slip-{}", evil.replace([':', '\\', '/', '.'], "_")));
            let err = unpack(&root.join("slip.cfpkg"), &target);
            assert!(matches!(err, Err(PkgError::InvalidPkg(_))), "{evil} 必须拒绝:{err:?}");
            assert!(!target.parent().unwrap().join("evil.txt").exists(), "穿越落点不得存在");
        }
        // 非 zip 输入拒收
        atomic::atomic_write(&root.join("x.bin"), b"not zip").unwrap();
        assert!(matches!(unpack(&root.join("x.bin"), &dest.join("b3")),
                Err(PkgError::InvalidPkg(_))));
        fsutil::cleanup(&root);
        fsutil::cleanup(&dest);
    }

    /// 打包结果可被标准解析器回读(结构自证);首条目签名与 EOCD 在位。
    #[test]
    fn pack_produces_parseable_zip() {
        let root = fsutil::temp_dir("cfpkg-zip-shape");
        make_v3(&root);
        let pkg = root.join("s.cfpkg");
        pack(&root, Some(&pkg), false, false).unwrap();
        let zip = std::fs::read(&pkg).unwrap();
        assert_eq!(&zip[0..4], &[0x50, 0x4B, 0x03, 0x04]);
        assert_eq!(&zip[zip.len() - 22..zip.len() - 18], &[0x50, 0x4B, 0x05, 0x06]);
        let items = zip_read_all(&zip).unwrap();
        assert!(items.iter().any(|e| e.name == "manifest.json"));
        assert!(items.iter().any(|e| e.name == "project/project.json"));
        assert!(items.iter().all(|e| !e.name.starts_with("media/")), "includeMedia=false 不收素材");
        fsutil::cleanup(&root);
    }
}
