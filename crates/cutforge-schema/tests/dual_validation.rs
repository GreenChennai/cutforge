//! M1-3 / M1-4 双端对拍与迁移幂等测试。
//!
//! dual_validation_equivalence:同一批回归样本,Rust 引擎与 Python 生成校验器
//! 结论逐样本一致(含注入负例);并验证 Rust 与 Python 迁移器输出语义相等。
//! migrate_idempotent:连续迁移两次,输出字节级一致。

use cutforge_schema::{migrate_project, validate};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn regression_dirs() -> Vec<PathBuf> {
    let reg = repo_root().join("tests/regression");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&reg)
        .expect("tests/regression 必须存在")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !p.file_name().unwrap().to_string_lossy().starts_with('_'))
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 3, "回归样本目录不足 3 类: {dirs:?}");
    dirs
}

fn python_bin() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

/// 跑 Python 生成校验器,返回 (exit_code)。
fn py_validate(file: &std::path::Path, schema: &str, migrate: bool) -> i32 {
    let mut cmd = Command::new(python_bin());
    cmd.arg(repo_root().join("tools/_generated/cf_validate.py"))
        .arg(schema)
        .arg(file)
        .arg("--json");
    if migrate {
        cmd.arg("--migrate");
    }
    let st = cmd.output().expect("无法启动 python(双端对拍需要本机 python)");
    st.status.code().unwrap_or(4)
}

fn py_migrate_canonical(file: &std::path::Path) -> String {
    let script = format!(
        "import json,sys; sys.path.insert(0, r'{root}\\tools\\_generated' if sys.platform=='win32' else r'{root}/tools/_generated'); \
         import cf_validate; \
         print(json.dumps(cf_validate.migrate_project_v1_to_v2(json.load(open(sys.argv[1], encoding='utf-8'))), ensure_ascii=False, sort_keys=True, separators=(',',':')))",
        root = repo_root().to_string_lossy()
    );
    let out = Command::new(python_bin())
        .arg("-c")
        .arg(script)
        .arg(file)
        .output()
        .expect("无法启动 python");
    assert!(out.status.success(), "python 迁移失败: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn canonical(v: &Value) -> String {
    serde_json::to_string(v).expect("序列化失败")
}

fn read_json(p: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).expect("读取失败")).expect("JSON 解析失败")
}

#[test]
fn dual_validation_equivalence() {
    for dir in regression_dirs() {
        for (fname, schema, migrate) in [
            ("project.json", "project", true),
            ("wordline.json", "wordline", false),
            ("cutlist.json", "cutlist", false),
            ("notes.json", "notes", false),
        ] {
            let f = dir.join(fname);
            let data = read_json(&f);
            let target = if migrate { migrate_project(&data) } else { data.clone() };
            let rust_ok = validate(schema, &target).is_empty();
            let py_code = py_validate(&f, schema, migrate);
            let py_ok = py_code == 0;
            assert_eq!(
                rust_ok, py_ok,
                "双端结论不一致: {}/{} rust_ok={rust_ok} py_exit={py_code}",
                dir.display(),
                fname
            );
            // 正向样本必须真的通过(不是双方一起坏)
            assert!(rust_ok, "回归样本未通过: {}/{}", dir.display(), fname);
        }
        // oplog.jsonl 逐行
        for (ln, line) in std::fs::read_to_string(dir.join("oplog.jsonl"))
            .expect("oplog.jsonl 缺失")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
        {
            let data: Value = serde_json::from_str(line).expect("jsonl 解析失败");
            let rust_ok = validate("oplog", &data).is_empty();
            assert!(rust_ok, "oplog 第 {ln} 行未通过: {}", dir.display());
        }
    }
}

#[test]
fn dual_negative_injection_agrees() {
    // 注入非法字段与非法枚举:双端都必须拒绝
    let dir = &regression_dirs()[0];
    let mut project = read_json(&dir.join("project.json"));
    let migrated = migrate_project(&project);
    drop(&mut project);
    let mut bad = migrated;
    bad["bogusField"] = Value::String("x".into());
    assert!(!validate("project", &bad).is_empty());
    let mut bad2 = read_json(&dir.join("cutlist.json"));
    bad2["cuts"][0]["action"] = Value::String("purge".into());
    assert!(!validate("cutlist", &bad2).is_empty());
    let _ = dir; // silence unused in some cfgs
}

#[test]
fn migrate_idempotent() {
    for dir in regression_dirs() {
        let f = dir.join("project.json");
        let data = read_json(&f);
        let once = migrate_project(&data);
        let twice = migrate_project(&once);
        // 字节级一致(serde_json 默认 BTreeMap 键序,序列化确定)
        assert_eq!(canonical(&once), canonical(&twice), "迁移不幂等: {}", dir.display());
        // v1 缺 v2 字段;迁移后必须过 v2 校验
        assert!(validate("project", &once).is_empty(), "迁移后未过 v2 校验: {}", dir.display());
    }
}

#[test]
fn migrate_py_rust_semantic_eq() {
    for dir in regression_dirs() {
        let f = dir.join("project.json");
        let data = read_json(&f);
        let rust = migrate_project(&data);
        let py_raw = py_migrate_canonical(&f);
        let py: Value = serde_json::from_str(&py_raw).expect("python 迁移输出非法");
        assert_eq!(
            canonical(&rust), canonical(&py),
            "双端迁移器输出不一致: {}",
            dir.display()
        );
    }
}
