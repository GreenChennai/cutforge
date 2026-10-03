// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 编排类工具:只封装既有脚本(子进程透传),不实现任何阶段逻辑
//! (T1.1 拆分自 lib.rs,纯移动)。册六 T6.2/ADR-0023:脚本定位独立化——
//! env CUTFLOW_REPO(显式,调试/对拍)→ 工程内 → 随包资产(<exe>/scripts/ 与
//! 开发树 tools/jianying/;剪映导出已收编)→ CutFlow 仓库回退(一个版本期)。

use crate::registry::envelope;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Python 启动器探测(py -3 → python3 → python;Windows 仅装 py-launcher 的机器不再全灭)。
pub(crate) fn py_launcher() -> Option<Vec<String>> {
    static CACHE: OnceLock<Option<Vec<String>>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            for cand in [["py", "-3"], ["python3", ""], ["python", ""]] {
                let args: Vec<&str> = cand[1..]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect();
                let ok = std::process::Command::new(cand[0])
                    .args(args)
                    .arg("-c")
                    .arg("print(1)")
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if ok {
                    // 过滤占位空串:空串若作为参数传回会给调用方埋雷(CI 实测 python3 "" -V 必败)
                    return Some(
                        cand.iter()
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect(),
                    );
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
        base.ancestors()
            .skip(1)
            .take(4)
            .map(|a| a.join("CutFlow"))
            .find(|c| c.join("skills/cutflow/scripts").is_dir())
    };
    if let Ok(exe) = std::env::current_exe()
        && let Some(p) = probe(&exe)
    {
        return Some(p);
    }
    probe(ws_root)
}

/// 脚本定位(册六 T6.2/ADR-0023 独立化关键):**env CUTFLOW_REPO(显式,调试/
/// 对拍)→ 工程内(rebuild.py 由 rs_run --init 生成)→ 随包资产(安装器落
/// <exe>/scripts/,开发树为 tools/jianying/;剪映导出已收编)→ CutFlow 仓库
/// (回退保留一个版本期)**。返回 (脚本路径, 来源标注;进响应 scriptSource)。
fn resolve_script(ws_root: &Path, script: &str) -> Option<(PathBuf, &'static str)> {
    // 1. env 显式(CUTFLOW_REPO 在位且脚本存在 = 开发/对拍显式覆盖,最高优先;
    //    只认 env——祖先发现的 CutFlow 归第 4 层回退,层级来源标注不混)
    if let Some(v) = std::env::var_os("CUTFLOW_REPO") {
        let cand = PathBuf::from(v).join("skills/cutflow/scripts").join(script);
        if cand.is_file() {
            return Some((cand, "cutflow-env"));
        }
    }
    // 2. 工程内
    let in_ws = ws_root.join(script);
    if in_ws.is_file() {
        return Some((in_ws, "workspace"));
    }
    // 3. 随包资产:exe 同目录 scripts/(T6.4 安装器落点)→ 开发树 tools/jianying/
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let cand = dir.join("scripts").join(script);
        if cand.is_file() {
            return Some((cand, "bundled"));
        }
    }
    let probe_bundled = |base: &Path| -> Option<PathBuf> {
        base.ancestors()
            .take(6)
            .map(|a| a.join("tools").join("jianying").join(script))
            .find(|c| c.is_file())
    };
    if let Ok(cwd) = std::env::current_dir()
        && let Some(p) = probe_bundled(&cwd)
    {
        return Some((p, "bundled"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(p) = probe_bundled(&exe)
    {
        return Some((p, "bundled"));
    }
    // 4. CutFlow 回退(一个版本期;独立机不依赖)
    if let Some(cutflow) = resolve_cutflow_dir(ws_root) {
        let cand = cutflow.join("skills/cutflow/scripts").join(script);
        if cand.is_file() {
            return Some((cand, "cutflow-fallback"));
        }
    }
    None
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

pub(crate) fn orchestrate(ws_root: &Path, script: &str, script_args: &[Value]) -> Value {
    let Some((script_path, script_source)) = resolve_script(ws_root, script) else {
        return envelope(
            false,
            "DEP_MISSING",
            "脚本不可达(定位序:env CUTFLOW_REPO → 工程内 → 随包资产 <exe>/scripts/ 与 tools/jianying/ → CutFlow 仓库): 请安装随包脚本或设 CUTFLOW_REPO",
            json!({}),
        );
    };
    let Some(py) = py_launcher() else {
        return envelope(
            false,
            "DEP_MISSING",
            "未找到可用的 Python(py -3/python3/python 均不可用)",
            json!({}),
        );
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
                // --json 白名单脚本:stdout 即 envelope,原样透传(脚本自持协议面,
                // 不注入编排面字段);解析失败回落编排面 envelope(附 scriptSource)
                let json_start = text.find('{').unwrap_or(text.len());
                match text[json_start..].parse::<Value>() {
                    Ok(v) => v,
                    Err(_) => envelope(
                        true,
                        "OK",
                        "编排完成",
                        json!({"stdout": text.trim(), "scriptSource": script_source}),
                    ),
                }
            } else {
                envelope(
                    true,
                    "OK",
                    "编排完成",
                    json!({"stdout": text.trim(), "scriptSource": script_source}),
                )
            }
        }
        Ok(o) => {
            let code = if o.status.code() == Some(3) {
                "DEP_MISSING"
            } else {
                "INTERNAL"
            };
            envelope(
                false,
                code,
                &String::from_utf8_lossy(&o.stderr)
                    .trim()
                    .chars()
                    .take(300)
                    .collect::<String>(),
                json!({}),
            )
        }
        Err(e) => envelope(
            false,
            "DEP_MISSING",
            &format!("{} 不可用: {e}", py.join(" ")),
            json!({}),
        ),
    }
}
