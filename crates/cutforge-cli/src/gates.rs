// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 门禁判定器(册二 T2.6:`check-shell-purity` v2 与 `check-write-paths` v2,自 lib.rs 迁入并升级)。
//!
//! == check-shell-purity v2(T2.6 / D-B3 / ADR-0013) ==
//! 扫描面:apps/web/**(.js/.html)全量 + crates/cutforge-wasm/src/**(.rs,沿 v1 不变)。
//! - R1 持久化语义计算禁令(沿 v1 字面量,扫描面扩为新 js 目录):node:fs / require("fs") /
//!   child_process /「startMs +」「startMs+」「endMs  =」「durationMs  +」——壳不自算时间线
//!   语义(总纲 R6:视图与盘面不得两套真相);
//! - R2 投影只读:projectStore/timelineStore 是内核投影只读面,唯一写入方 js/core/projector.js
//!   (.set/.reset);其他文件出现写入调用、或 `projectStore as`/`timelineStore as` 改名逃逸
//!   即违规;ephemeralStore 与 `ephemeral.` 前缀键豁免(ADR-0013 三原则:不进 IR/不落盘/
//!   不参与撤销),ui/selection/playback/media 等会话态 store 不在本条约束内;
//! - R3 禁裸 fetch:`fetch(` 只准出现在 js/core/api.js(唯一网络收口,T2.3);词边界判定
//!   (prefetch( 等标识符不误报;window.fetch( 同样算违规),注释行不计;
//! - R4 legacy 豁免(已移除):apps/web/legacy 旧壳回退期豁免于册三收尾随 legacy/ 整树
//!   删除一并收口(A2-L2 了断),扫描面即 apps/web 全量,不再有目录级豁免;
//! - R5 色值纪律(册三 T3.1/AC-3.1/ADR-0014):apps/web 的 css/js/html 零硬编码色值
//!   (hex 3/4/6/8 位与 rgb/rgba/hsl/hsla 函数形态;注释同样计违——色值即使只在注释
//!   里也会被复制回代码)。唯一色值定义点 css/tokens.css(三层 token);canvas 侧经
//!   js/render/theme.js 读 var(--cf-*)。豁免登记(逐项可审计):css/tokens.css(定义层)、
//!   assets/icons.js(SVG 现有 fill 保留)。判定器可单测:注入样例必抓。
//!
//! == check-write-paths v2(册二 T2.6 存量收口) ==
//! 盘面获取/落盘 API 家族(std 文件写的 写/建/改名/删/复制 五族,加 Open+Options 与
//! File 的 options 两个等价获取面——为免自匹配,本注释不逐字拼写 API 名,全表见 disk_pats)
//! 只允许 crates/cutforge-io/src/atomic.rs(v2 把 v1 的「任意同名 atomic.rs」收紧为该唯一路径)。
//! `write_all` 是 Write trait 方法、盘/网两用,v2 按接收者类型定性收窄(不开大豁免口):
//! - 接收者为 TcpStream 变量(类型标注 / TcpStream::connect 绑定 / incoming() 循环变量 /
//!   已知流变量 .try_clone() 派生)→ socket/SSE 网络流,合法(events.rs 的 SSE 帧推送);
//! - 行内经 stdin/stdout/stderr → 进程 stdio/控制台,合法;
//! - 其余仍记违规——磁盘旁路必先取得 File 句柄,而句柄获取面(上一段 API 表)全在扫描面内,
//!   `write_all` 本身无法凭空造出磁盘句柄,故该收窄不弱化真实防护。
//! 测试上下文豁免(按上下文识别,非对 crates 一刀切):
//! - 路径含 tests 目录段(integration 测试,如 http_hardening.rs)——夹具在临时目录搭假工程;
//! - src 内 `#[cfg(test)] mod` 区块(单元测试夹具,如 resident.rs/static_files.rs/doctor.rs);
//! 生产代码不受豁免;以上规则即判定器口径,与本文件头注释一致。

use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 一处违规:文件(相对仓库根)/ 行摘要 / 命中规则。
#[derive(Debug)]
struct Violation {
    file: String,
    line: String,
    rule: &'static str,
}

fn rel_of(p: &Path, repo: &Path) -> String {
    p.strip_prefix(repo)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/"))
}

fn vio(p: &Path, repo: &Path, line: &str, rule: &'static str) -> Violation {
    Violation {
        file: rel_of(p, repo),
        line: line.trim().chars().take(80).collect(),
        rule,
    }
}

// ---------------- check-shell-purity v2 ----------------

/// R2:project/timeline 两 store 的写入调用或 import 改名逃逸(投影只读面,ADR-0013)。
fn store_write_hit(line: &str) -> bool {
    ["projectStore", "timelineStore"].iter().any(|n| {
        line.contains(&format!("{n}.set("))
            || line.contains(&format!("{n}.reset("))
            || line.contains(&format!("{n} as"))
    })
}

/// R3:裸 fetch(词边界;`x.fetch(`/`window.fetch(` 均算,`prefetch(` 不算);注释行不计。
fn bare_fetch_hit(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
        return false;
    }
    let mut from = 0;
    while let Some(pos) = line[from..].find("fetch(") {
        let abs = from + pos;
        let word_before = line[..abs]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !word_before {
            return true;
        }
        from = abs + "fetch(".len();
    }
    false
}

/// R5:硬编码色值判定(hex 3/4/6/8 位 + rgb/rgba/hsl/hsla 函数形态)。
/// 无正则依赖:手扫 `#` 后的十六进制段;函数形态按子串。注释行不豁免(防复制回潮)。
fn color_literal_hit(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_hexdigit() {
                j += 1;
            }
            if matches!(j - i - 1, 3 | 4 | 6 | 8) {
                return true;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    ["rgb(", "rgba(", "hsl(", "hsla("]
        .iter()
        .any(|f| line.contains(f))
}

/// R5 豁免面登记(相对 apps/web 的路径):token 定义层 + 图标 SVG 登记点。
fn color_exempt(rel: &str) -> bool {
    rel == "css/tokens.css" || rel == "assets/icons.js"
}

/// v2 判定核心(独立于 repo_root,可单测):扫 apps_web 全量(无目录豁免)与 wasm 绑定面。
fn shell_purity_scan(repo: &Path, apps_web: &Path, wasm_src: &Path) -> Vec<Violation> {
    // R1 字面量沿用 v1 的拼接构造与字节(含历史双空格形态),旧判定面零漂移;
    // 扫描面不含 cutforge-cli 自身,判定器/测试文件不会自匹配。
    let r1: Vec<String> = vec![
        ["node", "fs"].join(":"),
        ["requ", "ire(\"fs\")"].join(""),
        ["child_process"].join(""),
        ["startMs", "+"].join(" "),
        ["startMs", "+"].join(""),
        ["endMs", " ="].join(" "),
        ["durationMs", " +"].join(" "),
    ];
    let mut violations: Vec<Violation> = Vec::new();
    // JS 壳面:apps/web 全量(R4 legacy 豁免已随 legacy/ 删除收口,无目录豁免)
    let mut stack = vec![apps_web.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x == "js" || x == "html" || x == "css") {
                continue;
            }
            if p.file_name().is_some_and(|n| n.to_string_lossy().contains("min.")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let rel = p
                .strip_prefix(apps_web)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let is_projector = rel == "js/core/projector.js";
            let is_api = rel == "js/core/api.js";
            let r5_exempt = color_exempt(&rel);
            for line in text.lines() {
                if r1.iter().any(|pat| line.contains(pat.as_str())) {
                    violations.push(vio(&p, repo, line, "R1 持久化语义计算/壳能力面"));
                    continue;
                }
                if !is_projector && store_write_hit(line) {
                    violations.push(vio(
                        &p,
                        repo,
                        line,
                        "R2 投影只读违规(project/timeline 唯一写入方 js/core/projector.js)",
                    ));
                    continue;
                }
                if !is_api && bare_fetch_hit(line) {
                    violations.push(vio(&p, repo, line, "R3 裸 fetch(唯一收口 js/core/api.js)"));
                    continue;
                }
                if !r5_exempt && color_literal_hit(line) {
                    violations.push(vio(
                        &p,
                        repo,
                        line,
                        "R5 硬编码色值(唯一色值定义点 css/tokens.css;canvas 经 js/render/theme.js)",
                    ));
                }
            }
        }
    }
    // wasm 绑定:禁文件系统(RS 侧,沿 v1 原样)
    let rs_pats: Vec<String> = vec![["std", "fs"].join("::"), ["fs", "read_to_string"].join("::")];
    let mut stack = vec![wasm_src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x == "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            for line in text.lines() {
                if rs_pats.iter().any(|pat| line.contains(pat.as_str())) {
                    violations.push(vio(&p, repo, line, "R0 wasm 侧禁文件系统(一切 IO 经宿主桥)"));
                    break;
                }
            }
        }
    }
    violations
}

/// M5-5/A2 判定器入口(v2):壳不持有真相——语义禁令 + 投影只读 + 禁裸 fetch。
pub fn check_shell_purity(json: bool) -> i32 {
    let root = crate::repo_root();
    let violations = shell_purity_scan(&root, &root.join("apps/web"), &root.join("crates/cutforge-wasm/src"));
    let items: Vec<serde_json::Value> = violations
        .iter()
        .map(|v| json!({"file": v.file, "line": v.line, "rule": v.rule}))
        .collect();
    let data = json!({
        "violations": items,
        "version": "v3(T2.6/ADR-0013/T3.1 R5 色值;册三收尾 R4 legacy 豁免已收口)",
        "rule": "壳禁文件系统/子进程/持久化语义计算;project/timeline store 投影只读(唯一写入方 \
                 js/core/projector.js);裸 fetch 唯一收口 js/core/api.js;ephemeral.* 豁免(ADR-0013 三原则);\
                 css/js/html 零硬编码色值(唯一定义点 css/tokens.css;canvas 经 js/render/theme.js 读 token)",
        "colorExempt": "R5 豁免登记:css/tokens.css(三层 token 唯一定义点)、assets/icons.js(SVG 现有 fill 保留)",
    });
    if violations.is_empty() {
        crate::emit(json, true, "OK", "壳纯度合规(v3):违规点 = 0", data)
    } else {
        crate::emit(
            json,
            false,
            "SHELL_PURITY_VIOLATION",
            &format!("违规 {} 处", violations.len()),
            data,
        )
    }
}

// ---------------- check-write-paths v2 ----------------

/// 行内流式写调用(Write trait 方法)的接收者是否网络流/进程 stdio(类型定性,见模块头规则)。
/// needle 由调用方以拼接构造传入(沿 v1 惯例):判定器源码内不出现完整调用字面量,防自匹配。
fn is_stream_write(line: &str, stream_call: &str, streams: &BTreeSet<String>) -> bool {
    let Some(pos) = line.find(stream_call) else { return false };
    let before = &line[..pos];
    let recv: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if !recv.is_empty() && streams.contains(recv.as_str()) {
        return true;
    }
    ["stdin", "stdout", "stderr"]
        .iter()
        .any(|s| before.contains(&format!(".{s}")) || before.contains(&format!("{s}(")))
}

/// `let (mut) NAME = …` 绑定名。
fn let_binding_name(line: &str) -> Option<String> {
    let pos = line.find("let ")?;
    let rest = line[pos + 4..].trim_start();
    let rest = rest.strip_prefix("mut ").unwrap_or(rest).trim_start();
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

/// `NAME: (mut) TcpStream` 形态的类型标注名(fn 参数/let)。
fn typed_stream_name(line: &str) -> Option<String> {
    let idx = line.find("TcpStream")?;
    let mut head = line[..idx].trim_end();
    loop {
        let mut changed = false;
        for suf in ["mut", "&", ":"] {
            while let Some(t) = head.strip_suffix(suf) {
                head = t.trim_end();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let name: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// `for NAME in listener.incoming()` 循环变量名。
fn for_incoming_name(line: &str) -> Option<String> {
    if !line.contains(".incoming()") {
        return None;
    }
    let pos = line.find("for ")?;
    let rest = line[pos + 4..].trim_start();
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if name.is_empty() { None } else { Some(name) }
}

/// 收集文件内的 TcpStream 变量名(轻量类型定性;两轮收敛覆盖 try_clone 派生乱序)。
fn stream_var_names(text: &str) -> BTreeSet<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    for _ in 0..2 {
        for line in text.lines() {
            if line.contains("TcpStream") {
                if let Some(n) = typed_stream_name(line) {
                    set.insert(n);
                }
                if line.contains("TcpStream::")
                    && let Some(n) = let_binding_name(line)
                {
                    set.insert(n);
                }
            }
            if let Some(n) = for_incoming_name(line) {
                set.insert(n);
            }
            if let Some(pos) = line.find(".try_clone()") {
                let recv: String = line[..pos]
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                if set.contains(recv.as_str())
                    && let Some(n) = let_binding_name(line)
                {
                    set.insert(n); // 仅已知流变量的 try_clone 派生才算流(File 也有 try_clone,不放宽)
                }
            }
        }
    }
    set
}

/// 逐行标记是否落在 `#[cfg(test)] mod … { … }` 区块内(花括号配平;配不平向「不豁免」收敛)。
fn test_region_mask(text: &str) -> Vec<bool> {
    let lines: Vec<&str> = text.lines().collect();
    let mut mask = vec![false; lines.len()];
    let (mut armed, mut active, mut depth) = (false, false, 0i32);
    for (i, line) in lines.iter().enumerate() {
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;
        if active {
            depth += opens - closes;
            if depth <= 0 {
                active = false;
                depth = 0;
                continue;
            }
            mask[i] = true;
        } else if armed && line.contains("mod") && opens > 0 {
            armed = false;
            depth = opens - closes;
            if depth > 0 {
                active = true;
                mask[i] = true;
            }
        } else if line.contains("#[cfg(test)]") {
            armed = true;
        }
    }
    mask
}

/// v2 判定核心(独立于 repo_root,可单测):扫 crates 下全部 .rs。
/// 违规按文件聚合(协议形状沿 v1:{file, hits});返回(违规表, atomic.rs 收编命中数)。
fn write_paths_scan(crates_dir: &Path) -> (Vec<(String, usize)>, usize) {
    let disk_pats: Vec<String> = vec![
        ["fs", "write"].join("::"),
        ["File", "create"].join("::"),
        ["Open", "Options"].join(""),
        // v2 补录:Open+Options::new 的等价获取入口(字面量拼接,防本文件自匹配),堵获取面旁路
        ["File", "options"].join("::"),
        ["fs", "rename"].join("::"),
        // 删除类 API 名必须 join("") 两段相连(带分隔符永远匹配不到,清账时实测修正,沿 v1)
        ["remove", "_file"].join(""),
        ["fs", "copy"].join("::"),
    ];
    let atomic_suffix = PathBuf::from("cutforge-io").join("src").join("atomic.rs");
    // 流式写调用 needle 拼接构造(防判定器源码自匹配,沿 v1 join 惯例)
    let stream_call = [".", "write", "_all("].join("");
    let mut violations: Vec<(String, usize)> = Vec::new();
    let mut sanctioned = 0usize;
    let mut stack = vec![crates_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            // 测试上下文豁免(上下文规则):tests/ 目录段 = integration 测试夹具
            let in_tests_dir = p.components().any(|c| c.as_os_str() == "tests");
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let in_test = if in_tests_dir {
                vec![true; text.lines().count()]
            } else {
                test_region_mask(&text)
            };
            let is_atomic = p.ends_with(&atomic_suffix); // v2:收紧为唯一落盘点路径
            let streams = stream_var_names(&text);
            let mut hits = 0usize;
            for (i, line) in text.lines().enumerate() {
                if in_test[i] {
                    continue; // 单元测试夹具临时文件(#[cfg(test)] 区块),合法
                }
                let flagged = disk_pats.iter().any(|pat| line.contains(pat.as_str()))
                    || (line.contains(&stream_call) && !is_stream_write(line, &stream_call, &streams));
                if flagged {
                    if is_atomic {
                        sanctioned += 1;
                    } else {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                violations.push((
                    p.strip_prefix(crates_dir.parent().unwrap_or(crates_dir))
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/")),
                    hits,
                ));
            }
        }
    }
    violations.sort();
    (violations, sanctioned)
}

/// M2-4 判定器入口(v2):文件写入 API 只允许 cutforge-io/src/atomic.rs;规则明细见模块头。
pub fn check_write_paths(json: bool) -> i32 {
    let root = crate::repo_root();
    let (violations, sanctioned) = write_paths_scan(&root.join("crates"));
    let items: Vec<serde_json::Value> = violations
        .iter()
        .map(|(f, n)| json!({"file": f, "hits": n}))
        .collect();
    let data = json!({
        "sanctioned_atomic_hits": sanctioned,
        "violations": items,
        "version": "v2(T2.6 存量收口)",
        "rule": "盘面写 API 仅允许 cutforge-io/src/atomic.rs(v2:流式写按接收者类型定性,TcpStream/\
                 进程 stdio 豁免;测试上下文(tests/ 目录与 cfg(test) 区块)夹具豁免;生产代码不豁免;\
                 全部 API 字面量表见判定器 disk_pats 与头注释)",
    });
    if sanctioned > 0 && violations.is_empty() {
        crate::emit(json, true, "OK", "写入路径唯一:仅 atomic.rs 落盘,旁路写入 = 0", data)
    } else {
        crate::emit(
            json,
            false,
            "WRITE_PATH_VIOLATION",
            &format!("旁路写入 {} 处", violations.len()),
            data,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 临时根(cache.rs/doctor.rs TempRoot 同款;进程 id 防并行串台)。
    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(tag: &str) -> TempRoot {
            let dir = std::env::temp_dir().join(format!("cf-cli-gates-{tag}-{}", std::process::id()));
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

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    /// 净树全绿:api.js/projector.js 白名单生效;无目录级豁免(R4 已随 legacy/ 删除收口)。
    #[test]
    fn purity_v2_green_on_clean_tree() {
        let root = TempRoot::new("pg");
        write(&root.0, "apps/web/js/core/api.js", "const r = await fetch(path);\n");
        write(&root.0, "apps/web/js/core/projector.js", "timelineStore.set({ clips });\nprojectStore.set(p);\n");
        write(&root.0, "apps/web/js/panels/x.js", "export const x = 1;\n");
        let v = shell_purity_scan(&root.0, &root.0.join("apps/web"), &root.0.join("crates/wasm"));
        assert!(v.is_empty(), "净树必须全绿: {v:?}");
    }

    /// legacy/ 目录不再豁免(R4 收口):同目录树内出现旧壳形态字面量即违规。
    #[test]
    fn purity_v2_no_directory_exemption() {
        let root = TempRoot::new("pn");
        write(&root.0, "apps/web/legacy/app.js", "await fetch(\"/rpc\");\nconst t = startMs + 1;\n");
        let v = shell_purity_scan(&root.0, &root.0.join("apps/web"), &root.0.join("crates/wasm"));
        assert_eq!(v.len(), 2, "legacy 目录内违规必须照抓(R4 已收口): {v:?}");
        assert!(v.iter().any(|x| x.rule.contains("R3")), "{v:?}");
        assert!(v.iter().any(|x| x.rule.contains("R1")), "{v:?}");
    }

    /// 注入样例必抓:面板裸 fetch(R3)/ 面板写 timelineStore(R2)/ import 改名逃逸(R2)/ 语义字面量(R1)。
    #[test]
    fn purity_v2_catches_injected_violations() {
        let root = TempRoot::new("pi");
        write(&root.0, "apps/web/js/panels/x.js", "const r = fetch(\"/rpc\");\n");
        write(&root.0, "apps/web/js/render/bad.js", "timelineStore.set({ clips: [] });\n");
        write(&root.0, "apps/web/js/ui/alias.js", "import { projectStore as ps } from \"../core/store.js\";\n");
        write(&root.0, "apps/web/js/ui/sem.js", "import { spawn } from \"node:child_process\";\n");
        // 词边界:prefetch( 不算裸 fetch;window.fetch( 算
        write(&root.0, "apps/web/js/ui/wb.js", "el.prefetch(url);\nconst r = window.fetch(\"/x\");\n");
        let v = shell_purity_scan(&root.0, &root.0.join("apps/web"), &root.0.join("crates/wasm"));
        let rules: Vec<&str> = v.iter().map(|x| x.rule).collect();
        assert_eq!(v.len(), 5, "{v:?}");
        assert_eq!(rules.iter().filter(|r| r.contains("R3")).count(), 2, "{v:?}");
        assert_eq!(rules.iter().filter(|r| r.contains("R2")).count(), 2, "{v:?}");
        assert_eq!(rules.iter().filter(|r| r.contains("R1")).count(), 1, "{v:?}");
    }

    /// R5 色值扫描净树全绿:token 定义点/icons 登记点豁免生效(legacy 整树豁免已随 R4 收口移除)。
    #[test]
    fn color_v3_green_on_token_only_tree() {
        let root = TempRoot::new("cg");
        write(&root.0, "apps/web/css/tokens.css", ":root { --c: #4da3ff; --m: rgba(0, 0, 0, .5); }\n");
        write(&root.0, "apps/web/css/components/x.css", ".x { color: var(--c); border: 1px solid var(--m); }\n");
        write(&root.0, "apps/web/assets/icons.js", "const fill = \"currentColor\";\n");
        write(
            &root.0,
            "apps/web/js/render/theme.js",
            "export function cssVar(name) {\n  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();\n}\n",
        );
        let v = shell_purity_scan(&root.0, &root.0.join("apps/web"), &root.0.join("crates/wasm"));
        assert!(v.is_empty(), "R5 净树必须全绿(豁免登记点生效): {v:?}");
    }

    /// R5 注入样例必抓:css 裸 hex / js 裸 rgba / html 内联 hex 各计一处;var() 引用不误报。
    #[test]
    fn color_v3_catches_injected_color_literals() {
        let root = TempRoot::new("ci");
        write(&root.0, "apps/web/css/components/bad.css", ".bad { color: #ff0000; }\n");
        write(&root.0, "apps/web/js/panels/bad.js", "ctx.fillStyle = \"rgba(0, 0, 0, .5)\";\n");
        write(&root.0, "apps/web/index.html", "<div style=\"color: #abc\">x</div>\n");
        write(&root.0, "apps/web/css/components/ok.css", ".ok { color: var(--c); border-color: var(--line); }\n");
        let v = shell_purity_scan(&root.0, &root.0.join("apps/web"), &root.0.join("crates/wasm"));
        assert_eq!(v.len(), 3, "注入三处必须各计一处: {v:?}");
        assert!(v.iter().all(|x| x.rule.contains("R5")), "{v:?}");
    }

    /// R5 判定器单元口径:十六进制段长 3/4/6/8 才算色值;id 选择器/锚点/HTML 实体不误报。
    #[test]
    fn color_literal_detector_boundaries() {
        for hit in ["color: #fff;", "#fffa00", "outline: #abcd;", "rgba(0,0,0,.5)", "url(x) hsl(1)"] {
            assert!(color_literal_hit(hit), "必须命中: {hit}");
        }
        for miss in [
            "#ruler { color: var(--c); }",
            "href=\"#tab-notes\"",
            "&#65; 实体",
            "#timeline-wrap .clip",
            "console.log(\"#g\")", // 1 位段不是色值
        ] {
            assert!(!color_literal_hit(miss), "不得误报: {miss}");
        }
    }

    /// write-paths v2:socket 流写 / 进程 stdio / tests/ 目录 / #[cfg(test)] 区块全豁免,atomic 计收编。
    #[test]
    fn write_paths_v2_exempts_sockets_stdio_and_test_context() {
        let root = TempRoot::new("wg");
        write(
            &root.0,
            "crates/cutforge-io/src/atomic.rs",
            "fn w(p: &Path) {\n    let mut f = fs::File::create(p)?;\n    f.write_all(b\"x\")?;\n}\n",
        );
        write(
            &root.0,
            "crates/m/src/net.rs",
            "use std::net::TcpStream;\nfn h(mut stream: TcpStream) { stream.write_all(b\"hi\")?; }\n\
             let mut s = TcpStream::connect(addr)?;\ns.write_all(b\"x\")?;\nio::stdout().write_all(b\"y\")?;\n",
        );
        write(&root.0, "crates/m/tests/fixture.rs", "std::fs::write(p, b\"x\").unwrap();\n");
        write(
            &root.0,
            "crates/m/src/inner.rs",
            "#[cfg(test)]\nmod tests {\n    std::fs::write(p, b\"x\").unwrap();\n}\n",
        );
        let (v, sanctioned) = write_paths_scan(&root.0.join("crates"));
        assert!(v.is_empty(), "合法场景必须零违规: {v:?}");
        assert_eq!(sanctioned, 2, "atomic 盘面获取+write_all 各计 1: {sanctioned}");
    }

    /// 注入必抓:非流接收者的 write_all 与裸盘面写;File::options 补录进获取面。
    #[test]
    fn write_paths_v2_catches_disk_bypass() {
        let root = TempRoot::new("wb");
        write(
            &root.0,
            "crates/m/src/bad.rs",
            "fn b(mut f: Unknown) { f.write_all(b\"x\")?; }\nstd::fs::write(p, b\"y\")?;\n",
        );
        write(&root.0, "crates/m/src/opt.rs", "let o = File::options();\n");
        let (v, _) = write_paths_scan(&root.0.join("crates"));
        assert_eq!(v.len(), 2, "{v:?}");
        assert_eq!(v[0].1, 2, "bad.rs 两处命中应聚合: {v:?}");
    }
}
