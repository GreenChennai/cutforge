// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 编排类工具:只封装 CutFlow 既有脚本(子进程透传),不实现任何阶段逻辑
//! (T1.1 拆分自 lib.rs,纯移动)。

use crate::registry::envelope;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Python 启动器探测(py -3 → python3 → python;Windows 仅装 py-launcher 的机器不再全灭)。
pub(crate) fn py_launcher() -> Option<Vec<String>> {
    static CACHE: OnceLock<Option<Vec<String>>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
                for cand in [["py", "-3"], ["python3", ""], ["python", ""]] {
                    let args: Vec<&str> = cand[1..].iter().filter(|s| !s.is_empty()).copied().collect();
                    let ok = std::process::Command::new(cand[0])
                        .args(args)
                    .arg("-c")
                    .arg("print(1)")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if ok {
                    // 过滤占位空串:空串若作为参数传回会给调用方埋雷(CI 实测 python3 "" -V 必败)
                    return Some(cand.iter().filter(|s| !s.is_empty()).map(|s| s.to_string()).collect());
                }
            }
            None
        })
        .clone()
}

/// CutFlow 仓库定位(去硬编码):CUTFLOW_REPO → 可执行文件祖先目录 → 工程目录祖先。
fn resolve_cutflow_dir(ws_root: &Path) -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CUTFLOW_REPO") {
        let p = PathBuf::from(v);
        if p.join("skills/cutflow/scripts").is_dir() {
            return Some(p);
        }
    }
    let probe = |base: &Path| -> Option<PathBuf> {
        base.ancestors().skip(1).take(4).map(|a| a.join("CutFlow"))
            .find(|c| c.join("skills/cutflow/scripts").is_dir())
    };
    if let Ok(exe) = std::env::current_exe()
        && let Some(p) = probe(&exe) {
            return Some(p);
        }
    probe(ws_root)
}

/// scriptArgs 序列化:字符串原样,数字/布尔转字符串;复杂对象如实拒绝(不再静默丢弃)。
pub(crate) fn script_arg_to_string(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        other => Err(format!("不支持scriptArgs 元素类型: {other}")),
    }
}

pub(crate) fn orchestrate(ws_root: &Path, script: &str, script_args: &[Value]) -> Value {    let Some(cutflow) = resolve_cutflow_dir(ws_root) else {
        return envelope(false, "DEP_MISSING",
            "未找到 CutFlow 仓库:设 CUTFLOW_REPO 指向仓库根(skills/cutflow/scripts 需存在)", json!({}));
    };
    let mut script_path = cutflow.join("skills/cutflow/scripts").join(script);
    // stage_rebuild 的脚本在工程目录内(rebuild.py 由 rs_run --init 生成)
    if !script_path.is_file() {
        script_path = ws_root.join(script);
    }
    if !script_path.is_file() {
        return envelope(false, "DEP_MISSING", &format!("脚本不存在: {}", script_path.display()), json!({}));
    }
    let Some(py) = py_launcher() else {
        return envelope(false, "DEP_MISSING", "未找到可用的 Python(py -3/python3/python 均不可用)", json!({}));
    };
    // --json 白名单:仅契约声明支持该旗标的脚本(rs_verify);其余追加 --json 会被
    // argparse 以退出码 2 拒绝——这正是 M4 三个编排工具必崩的根因(P1-5)。
    let supports_json = matches!(script, "rs_verify.py");
    let mut str_args: Vec<String> = Vec::new();
    for v in script_args {
        match script_arg_to_string(v) {
            Ok(s) => str_args.push(s),
            Err(m) => return envelope(false, "PRECONDITION_FAILED", &m, json!({})),
        }
    }
    let mut cmd = std::process::Command::new(&py[0]);
    cmd.args(&py[1..]).arg(&script_path);
    for a in &str_args {
        cmd.arg(a);
    }
    if supports_json {
        cmd.arg("--json");
    }
    cmd.current_dir(ws_root);
    let out = cmd.output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            if supports_json {
                let json_start = text.find('{').unwrap_or(text.len());
                match text[json_start..].parse::<Value>() {
                    Ok(v) => v,
                    Err(_) => envelope(true, "OK", "编排完成", json!({"stdout": text.trim()})),
                }
            } else {
                envelope(true, "OK", "编排完成", json!({"stdout": text.trim()}))
            }
        }
        Ok(o) => {
            let code = if o.status.code() == Some(3) { "DEP_MISSING" } else { "INTERNAL" };
            envelope(false, code, &String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect::<String>(), json!({}))
        }
        Err(e) => envelope(false, "DEP_MISSING", &format!("{} 不可用: {e}", py.join(" ")), json!({})),
    }
}
