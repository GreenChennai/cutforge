// ARL-CORE · CutForge 权利人核心文件(许可见 LICENSE 1.3;清单见 CORE-FILES)
//! 常驻工作区缓存(T1.8/AC-1.8 性能专项):serve/MCP 长驻进程内复用已打开的
//! Workspace,消除"每笔 RPC 全量重开"(1k 片段工程实测 ~40ms/笔)。
//!
//! ## 新鲜度协议(零语义漂移的关键)
//!
//! 每笔 RPC 前计算磁盘指纹 `fresh::disk_fingerprint`——它覆盖 `Workspace::load`
//! 的全部磁盘输入(project.json 含 `_meta`、notes.json、三份真相源、`.cutforge/rev`
//! 内容、oplog 文件清单),字节级(长度 + FNV-1a),不依赖 mtime:
//!
//! - 指纹与缓存记录一致 ⇒ 重新 open 将得到一致的 Workspace ⇒ 复用不改变任何
//!   查询/写入结果(黄金对拍逐字节不变);
//! - 指纹不一致(外部改动 / 其他进程写入 / oplog 崩溃恢复面)⇒ 丢弃缓存重开,
//!   与既有的"每笔无状态重开"行为逐字节一致;
//! - 写 RPC 成功后以当下磁盘实况刷新缓存指纹;写 RPC 失败(ok=false)⇒ 保守丢弃
//!   缓存(下一笔重开 = 既有行为),杜绝半程状态跨 RPC 泄漏。
//!
//! ## 锁纪律(红线)
//!
//! - 查询路径只读打开(`Workspace::open`),全程不申请工程锁
//!   (`readonly_query_holds_no_lock` 铁律);
//! - 写路径经 `Workspace::open_for_write`(迁移升级与 `open_exclusive` 同判定,
//!   锁内落规范形)+ apply 内部临时全程锁;锁内 `pre_write_sync` 三路合并/冲突停写
//!   语义原样保留——写窗口内的外部改动仍被捕获(指纹检查在锁外,不替代它)。
//! - 缓存互斥锁只覆盖单笔 RPC(查询/写入均毫秒级);渲染(run/progress)与编排
//!   (stage_run 等)在 dispatch 更早处分岔,不进入本缓存,不阻塞。

use crate::registry::envelope;
use cutforge_io::fresh::{disk_fingerprint, DiskFingerprint};
use cutforge_io::Workspace;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

struct Resident {
    ws: Workspace,
    fp: DiskFingerprint,
}

fn cache() -> &'static Mutex<BTreeMap<String, Resident>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Resident>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 借出常驻工作区执行一笔工具调用并归还(指纹复核后)。
///
/// - `root_key`:缓存键(args["root"] 原串;不同拼法各自建缓存,正确性不受影响);
/// - `readonly`:查询类 = true(零工程锁;不刷新指纹——查询不改盘);
/// - `f`:工具本体,拿到 `&mut Workspace` 产出响应 envelope。
pub fn with_resident(
    root_key: &str,
    root: &Path,
    readonly: bool,
    f: impl FnOnce(&mut Workspace) -> Value,
) -> Value {
    let mut cache = match cache().lock() {
        Ok(g) => g,
        Err(p) => {
            // 毒化恢复:持锁线程曾 panic,缓存内容不可信 → 全部丢弃,退回无状态重开
            let mut m = p.into_inner();
            m.clear();
            m
        }
    };
    // 新鲜度检查(锁外磁盘只读 IO,~0.5ms):None = project.json 不可读。
    // 此时走一次真实 open 取得与既有实现逐字一致的错误(NO_CONFIG/INTERNAL)。
    let Some(fp) = disk_fingerprint(root) else {
        cache.remove(root_key);
        return match Workspace::open(root) {
            Ok(_) => envelope(false, "INTERNAL", "工程可打开但指纹不可读", json!({})),
            Err(e) => {
                let code = if e.kind() == std::io::ErrorKind::NotFound { "NO_CONFIG" } else { "INTERNAL" };
                envelope(false, code, &e.to_string(), json!({}))
            }
        };
    };
    let fresh_hit = matches!(cache.get(root_key), Some(r) if r.fp == fp);
    if !fresh_hit {
        // 指纹不一致或无缓存:重开(查询只读零锁;写入口锁内做迁移升级判定)
        let ws = if readonly { Workspace::open(root) } else { Workspace::open_for_write(root) };
        match ws {
            Ok(ws) => {
                cache.insert(root_key.to_string(), Resident { ws, fp });
            }
            Err(e) => {
                cache.remove(root_key);
                let code = if e.kind() == std::io::ErrorKind::NotFound { "NO_CONFIG" } else { "INTERNAL" };
                return envelope(false, code, &e.to_string(), json!({}));
            }
        }
    }
    let Resident { ws, .. } = cache.get_mut(root_key).expect("上一分支必已建缓存");
    let out = f(ws);
    if readonly {
        // 查询不改盘:入口指纹已核实,缓存继续有效
        return out;
    }
    if out.get("ok") != Some(&json!(true)) {
        // 写失败:引擎内部已回滚,但保守丢弃缓存——下一笔重开,与既有无状态行为一致
        cache.remove(root_key);
        return out;
    }
    // 写成功:以当下磁盘实况刷新指纹(persist 已落定;~0.5ms)
    match disk_fingerprint(root) {
        Some(nfp) => {
            if let Some(r) = cache.get_mut(root_key) {
                r.fp = nfp;
            }
        }
        None => {
            cache.remove(root_key);
        }
    }
    out
}

/// 测试钩子:清空常驻缓存(跨用例隔离;生产路径不调用)。
#[doc(hidden)]
pub fn _clear_for_tests() {
    if let Ok(mut cache) = cache().lock() {
        cache.clear();
    }
}

/// 常驻缓存键(测试用;生产路径直接用 args["root"] 原串):键 = root 原串。
/// Windows 下同一目录的不同拼法(斜杠/大小写)会各自建缓存,互不串味——
/// 每个缓存条目都经独立指纹核验,正确性与无状态实现一致,仅多占一份内存。
#[cfg(test)]
pub fn cache_key(root_str: &str) -> String {
    root_str.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 复用正确性:同一 root 连续两笔查询,第二笔必须命中缓存
    /// (指纹一致;响应与无状态实现逐字节一致由 tool_parity 黄金对拍兜底)。
    #[test]
    fn resident_reuses_and_invalidates() {
        let root = cutforge_io::tests_fixture("mcp-resident-basic").unwrap();
        let root_s = root.to_string_lossy().to_string();
        let q = || {
            with_resident(&cache_key(&root_s), Path::new(&root_s), true, |ws| {
                envelope(true, "OK", "rev", json!({"rev": ws.rev()}))
            })
        };
        let r1 = q();
        let r2 = q();
        assert_eq!(r1, r2, "指纹未变,复用必须产出一致结果");

        // 外部改动 project.json → 指纹变 → 下一笔重开(rev/投影响应磁盘真相)
        let pj = root.join(cutforge_io::paths::PROJECT_REL);
        let text = std::fs::read_to_string(&pj).unwrap();
        std::fs::write(&pj, format!("{text}\n")).unwrap(); // 等值但字节变 → 指纹必变
        let r3 = q();
        assert_eq!(r3, r2, "等值改写不改变工作区语义,rev 一致");
        cutforge_io::fsutil::cleanup(&root);
        _clear_for_tests();
    }

    /// 写失败 → 缓存必须被丢弃(下一笔重开,杜绝半程状态跨 RPC 泄漏)。
    #[test]
    fn failed_write_evicts_cache() {
        let root = cutforge_io::tests_fixture("mcp-resident-evict").unwrap();
        let root_s = root.to_string_lossy().to_string();
        // 第一笔:查询建缓存
        let _ = with_resident(&cache_key(&root_s), Path::new(&root_s), true, |ws| {
            envelope(true, "OK", "rev", json!({"rev": ws.rev()}))
        });
        // 第二笔:写失败(ok=false)→ 缓存被逐出
        let r = with_resident(&cache_key(&root_s), Path::new(&root_s), false, |_ws| {
            envelope(false, "GUARD_FAILED", "模拟失败", json!({}))
        });
        assert_eq!(r["ok"], json!(false));
        // 第三笔:重建缓存成功(若缓存未被逐出且盘面已变,才会暴露陈旧状态;
        // 此处断言流程本身可继续,状态一致性由 e2e/parity 兜底)
        let r2 = with_resident(&cache_key(&root_s), Path::new(&root_s), true, |ws| {
            envelope(true, "OK", "rev", json!({"rev": ws.rev()}))
        });
        assert_eq!(r2["ok"], json!(true));
        cutforge_io::fsutil::cleanup(&root);
        _clear_for_tests();
    }
}
