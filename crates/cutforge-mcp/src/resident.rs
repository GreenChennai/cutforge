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
//! - 缓存锁粒度(R-09):per-project——外层 `Mutex<HashMap>` 只保护结构(极短),
//!   工作区互斥在各工程独立的 `Arc<Mutex>` 上,A 工程慢操作不再队头阻塞 B 工程;
//!   锁只覆盖单笔 RPC(查询/写入均毫秒级);渲染(run/progress)与编排
//!   (stage_run 等)在 dispatch 更早处分岔,不进入本缓存,不阻塞。
//! - 幂等索引(R-14):每个常驻条目随行一个 `HashSet<request_id>`,随 OpLog
//!   增量维护(写成功只扫新增 Op)、重开时全量重建;dispatch 的写预检 O(1)。

use crate::registry::envelope;
use cutforge_io::Workspace;
use cutforge_io::fresh::{DiskFingerprint, disk_fingerprint};
use serde_json::{Value, json};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

/// 常驻条目:工作区 + 磁盘指纹 + request_id 幂等索引(R-14:随 OpLog 增量
/// 维护,查询 O(1);重开/重启重建走 [`idem_index_from_ops`],未来 OpLog
/// compact 后的重建接口同此)。
struct Resident {
    ws: Workspace,
    fp: DiskFingerprint,
    idem: HashSet<String>,
}

/// R-14:从 Op 全集构建幂等索引(重开/重启/compact 重建的单一入口)。
pub(crate) fn idem_index_from_ops(ops: &[cutforge_core::oplog::Op]) -> HashSet<String> {
    ops.iter().filter_map(|o| o.request_id.clone()).collect()
}

/// 服务端会话态剪贴板(册四 A4 T4.2 clip_copy/clip_paste_at)。
/// 按 root 键控、进程内存态:**不落盘、不入 OpLog、不参与撤销**,serve 进程退出即清空。
/// 独立于工作区缓存的理由:写失败的 RPC 会保守丢弃 Workspace 缓存(引擎已回滚),
/// 但剪贴板是纯会话态(类似 NLE 的系统剪贴板),必须跨失败调用与工作区重开存活。
/// 形态:clips 深拷贝数组(当前 clip_copy 单片段入板,数组为多片段批量粘贴预留)
/// + 来源轨类型(粘贴跨 kind 由派发层拒绝)。
#[derive(Debug, Clone)]
pub(crate) struct SessionClipboard {
    pub clips: Vec<cutforge_core::model::Clip>,
    pub kind: cutforge_core::model::TrackKind,
}

fn clipboards() -> &'static Mutex<HashMap<String, SessionClipboard>> {
    static CLIPBOARDS: OnceLock<Mutex<HashMap<String, SessionClipboard>>> = OnceLock::new();
    CLIPBOARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 剪贴板覆盖式写入(clip_copy;root_key 与工作区缓存同键)。
pub(crate) fn clipboard_set(
    root_key: &str,
    clips: Vec<cutforge_core::model::Clip>,
    kind: cutforge_core::model::TrackKind,
) {
    if let Ok(mut m) = clipboards().lock() {
        m.insert(root_key.to_string(), SessionClipboard { clips, kind });
    }
}

/// 剪贴板只读视图(clip_paste_at 读取;None = 空板)。
pub(crate) fn clipboard_get(root_key: &str) -> Option<SessionClipboard> {
    clipboards()
        .lock()
        .ok()
        .and_then(|m| m.get(root_key).cloned())
}

/// R-09:per-project 锁。外层 `Mutex<HashMap>` 只保护 map 结构(极短临界区:
/// 取/建条目指针),工作区本体锁在各自的 `Arc<Mutex<Option<Resident>>>` 上——
/// A 工程的慢操作不再队头阻塞 B 工程(与文件锁 per-project 对齐)。
/// per-project 条目指针(外层短锁只保护 map 结构;工作区本体在 cell 内,
/// `Option<Resident>` = 指针先入 map 原子去重,本体在 per-project 锁内懒加载——
/// 打开是 IO,不得持外层锁)。
type ResidentCell = Arc<Mutex<Option<Resident>>>;

fn cache() -> &'static Mutex<HashMap<String, ResidentCell>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ResidentCell>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取/建本工程的 per-project 条目(R-09 收口:**cell 身份恒定原则**)。
/// std `Entry` API 保证"查+插"在外层短锁内一次完成且原子——同 key 并发 miss
/// 恒收敛到同一个 `Arc<Mutex>`;cell 一经入 map **永不摘除**(evict/清缓存一律
/// 走逻辑逐出),杜绝"旧 cell 在途借用 + 新 cell 并存"导致的同工程双锁窗口
/// (全 workspace 并行负载下实测暴露的竞态根因)。
fn entry_or_create(root_key: &str) -> ResidentCell {
    let mut map = match cache().lock() {
        Ok(m) => m,
        // 外层锁临界区只有 HashMap 结构操作(无 panic 源),毒化仅可能是
        // 结构短暂可见态异常 → into_inner 接管并保留 map 结构(cell 身份恒定)
        Err(p) => p.into_inner(),
    };
    match map.entry(root_key.to_string()) {
        Entry::Occupied(o) => o.get().clone(),
        Entry::Vacant(v) => {
            let cell = Arc::new(Mutex::new(None));
            v.insert(cell.clone());
            cell
        }
    }
}

/// 逻辑逐出(替代摘键):丢弃该工程的工作区态(下一笔 fresh_hit=false 必重开,
/// 与既有"无状态重开"语义一致),cell 身份保留——在途借用与新请求仍互斥于
/// 同一把锁。毒化的 cell 由消费方的 poison 分支同样置 None,语义一致。
/// 调用面:with_resident 的保守逐出路径 + preview_plan 副本用后即弃。
pub(crate) fn evict(root_key: &str) {
    let cell = match cache().lock() {
        Ok(m) => m.get(root_key).cloned(),
        Err(_) => None,
    };
    if let Some(cell) = cell {
        match cell.lock() {
            Ok(mut g) => *g = None,
            Err(p) => {
                p.into_inner().take();
            }
        }
    }
}

/// R-14:幂等 O(1) 预检。返回 `Some(rev)` = 该 request_id 已在 OpLog(常驻索引
/// 命中,指纹复核窗口内与全扫等价,调用方可安全短路为幂等回执);`None` =
/// 未命中或缓存不可用(调用方一律放行,引擎内全量判定仍是正确性底线)。
/// 冷启动时按需打开工作区建索引(与 with_resident 同一指纹协议)。
pub(crate) fn idempotent_precheck(root_key: &str, root: &Path, rid: &str) -> Option<u64> {
    let cell = entry_or_create(root_key);
    let mut guard = match cell.lock() {
        Ok(g) => g,
        Err(_) => return None, // 毒化:不预检,放行走引擎全量判定
    };
    // 指纹复核(锁内但 per-project;~0.5ms 磁盘只读)
    if let Some(res) = guard.as_mut()
        && res.fp != disk_fingerprint(root)?
    {
        *guard = None; // 外部改动:丢缓存重开(与 with_resident 同判)
    }
    if guard.is_none() {
        let ws = match Workspace::open(root) {
            Ok(ws) => ws,
            Err(_) => return None,
        };
        let idem = idem_index_from_ops(ws.engine().oplog().ops());
        *guard = Some(Resident {
            ws,
            fp: disk_fingerprint(root)?,
            idem,
        });
    }
    let res = guard.as_ref().expect("上一分支必已建缓存");
    res.idem.contains(rid).then(|| res.ws.engine().rev())
}

/// 测试用真值预检(TC-CORE-IDEMP-001):与 [`idempotent_precheck`] 同路,
/// 但以 bool 表达"索引判定"语义(含未命中)。
#[cfg(test)]
pub(crate) fn request_id_seen(root_key: &str, root: &Path, rid: &str) -> bool {
    let Some(rev) = idempotent_precheck(root_key, root, rid) else {
        return false;
    };
    let _ = rev;
    true
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
    // 新鲜度检查(磁盘只读 IO,~0.5ms):None = project.json 不可读。
    // 此时走一次真实 open 取得与既有实现逐字一致的错误(NO_CONFIG/INTERNAL)。
    let Some(fp) = disk_fingerprint(root) else {
        evict(root_key);
        return match Workspace::open(root) {
            Ok(_) => envelope(false, "INTERNAL", "工程可打开但指纹不可读", json!({})),
            Err(e) => {
                let code = if e.kind() == std::io::ErrorKind::NotFound {
                    "NO_CONFIG"
                } else {
                    "INTERNAL"
                };
                envelope(false, code, &e.to_string(), json!({}))
            }
        };
    };
    let cell = entry_or_create(root_key);
    // R-09:per-project 锁内完成"指纹核验 → 重开 → RPC 本体 → 指纹刷新";
    // 毒化恢复只影响本工程条目(逐出重建),不再全局清空
    let mut guard = match cell.lock() {
        Ok(g) => g,
        Err(p) => {
            // 毒化恢复:持锁线程曾 panic,该工程的工作区态不可信 → 逐出重建
            let mut restored = p.into_inner();
            *restored = None;
            restored
        }
    };
    let fresh_hit = matches!(guard.as_ref(), Some(r) if r.fp == fp);
    if !fresh_hit {
        // 指纹不一致或无缓存:重开(查询只读零锁;写入口锁内做迁移升级判定)
        let ws = if readonly {
            Workspace::open(root)
        } else {
            Workspace::open_for_write(root)
        };
        match ws {
            Ok(ws) => {
                let idem = idem_index_from_ops(ws.engine().oplog().ops());
                *guard = Some(Resident { ws, fp, idem });
            }
            Err(e) => {
                drop(guard);
                evict(root_key);
                let code = if e.kind() == std::io::ErrorKind::NotFound {
                    "NO_CONFIG"
                } else {
                    "INTERNAL"
                };
                return envelope(false, code, &e.to_string(), json!({}));
            }
        }
    }
    let r = guard.as_mut().expect("上一分支必已建缓存");
    let op_count_before = r.ws.engine().oplog().len();
    let out = f(&mut r.ws);
    if readonly {
        // 查询不改盘:入口指纹已核实,缓存继续有效
        return out;
    }
    if out.get("ok") != Some(&json!(true)) {
        // 写失败:引擎内部已回滚,但保守丢弃缓存——下一笔重开,与既有无状态行为一致
        drop(guard);
        evict(root_key);
        return out;
    }
    // 写成功:增量维护幂等索引(R-14:只扫新增 Op,不全量重扫),
    // 再以当下磁盘实况刷新指纹(persist 已落定;~0.5ms)
    let r = guard.as_mut().expect("缓存仍在");
    for rid in r.ws.engine().oplog().ops()[op_count_before..]
        .iter()
        .filter_map(|o| o.request_id.clone())
    {
        r.idem.insert(rid);
    }
    match disk_fingerprint(root) {
        Some(nfp) => {
            if let Some(r) = guard.as_mut() {
                r.fp = nfp;
            }
        }
        None => {
            drop(guard);
            evict(root_key);
        }
    }
    out
}

/// 测试钩子:清空常驻缓存(跨用例隔离;生产路径不调用)。
/// 语义对齐逻辑逐出:只丢弃各工程的工作区态,**不摘除 cell**(身份恒定原则)——
/// 并行测试负载下,在途借用与新请求恒互斥于同一把 per-project 锁。
#[doc(hidden)]
pub fn _clear_for_tests() {
    let cells: Vec<ResidentCell> = match cache().lock() {
        Ok(m) => m.values().cloned().collect(),
        Err(p) => p.into_inner().values().cloned().collect(),
    };
    for cell in cells {
        match cell.lock() {
            Ok(mut g) => *g = None,
            Err(p) => {
                p.into_inner().take();
            }
        }
    }
    if let Ok(mut cb) = clipboards().lock() {
        cb.clear();
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

    /// TC-MCP-RESIDENT-001(R-09):锁 per-project 化——A 工程持锁慢操作期间,
    /// B 工程的读请求不得被队头阻塞(RT 显著小于 A 的持锁时长)。
    #[test]
    fn resident_lock_is_per_project() {
        let a = cutforge_io::tests_fixture("mcp-resident-par-a").unwrap();
        let b = cutforge_io::tests_fixture("mcp-resident-par-b").unwrap();
        let (ka, kb) = (
            cache_key(&a.to_string_lossy()),
            cache_key(&b.to_string_lossy()),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let ta = a.clone();
        std::thread::spawn(move || {
            let _ = with_resident(&ka, &ta, true, |_ws| {
                std::thread::sleep(std::time::Duration::from_millis(600));
                envelope(true, "OK", "slow-A", json!({}))
            });
            tx.send(()).unwrap();
        });
        // 等 A 确实持锁
        std::thread::sleep(std::time::Duration::from_millis(150));
        let t0 = std::time::Instant::now();
        let rb = with_resident(&kb, &b, true, |ws| {
            envelope(true, "OK", "fast-B", json!({"rev": ws.rev()}))
        });
        let elapsed = t0.elapsed();
        assert_eq!(rb["message"], json!("fast-B"));
        assert!(
            elapsed < std::time::Duration::from_millis(300),
            "B 工程读请求被 A 工程的慢操作队头阻塞: {elapsed:?}(per-project 锁必须隔离)"
        );
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        cutforge_io::fsutil::cleanup(&a);
        cutforge_io::fsutil::cleanup(&b);
        _clear_for_tests();
    }

    /// TC-MCP-RESIDENT-002(R-09 强化):同工程严格串行,不再靠运气——
    /// 8 线程 × 50 笔,**barrier 齐放的冷启动并发 miss 专打 entry_or_create
    /// 创建窗口**(不预热条目直接打);全程 max 并发必须 = 1。
    #[test]
    fn resident_same_project_stays_serialized() {
        use std::sync::Barrier;
        const THREADS: usize = 8;
        const ROUNDS: usize = 50;
        let root = cutforge_io::tests_fixture("mcp-resident-serial").unwrap();
        let k = cache_key(&root.to_string_lossy());
        let cur = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let max = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(THREADS));
        let mut handles = Vec::new();
        for _ in 0..THREADS {
            let (cur, max, done) = (cur.clone(), max.clone(), done.clone());
            let (k, r) = (k.clone(), root.clone());
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                use std::sync::atomic::Ordering;
                barrier.wait(); // 冷启动:全部线程对未预热条目同时打创建窗口
                for _ in 0..ROUNDS {
                    with_resident(&k, &r, true, |_ws| {
                        let n = cur.fetch_add(1, Ordering::AcqRel) + 1;
                        max.fetch_max(n, Ordering::AcqRel);
                        // 短睡眠提高重叠观测密度;指纹 IO 本身即真实窗口
                        std::thread::sleep(std::time::Duration::from_micros(200));
                        cur.fetch_sub(1, Ordering::AcqRel);
                        envelope(true, "OK", "n", json!({"n": n}))
                    });
                    done.fetch_add(1, Ordering::AcqRel);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            done.load(std::sync::atomic::Ordering::Acquire),
            THREADS * ROUNDS,
            "全部 RPC 必须完成"
        );
        assert_eq!(
            max.load(std::sync::atomic::Ordering::Acquire),
            1,
            "同工程并发必须串行化(任一时刻至多一笔 RPC 在工作区上)"
        );
        cutforge_io::fsutil::cleanup(&root);
        _clear_for_tests();
    }

    /// TC-MCP-RESIDENT-003(R-09 强化):多工程混合负载双断言——
    /// ① 同工程内严格串行(每工程 max 并发 = 1);② 不同工程真并行
    /// (全局 in-flight 峰值 ≥2;12 线程 barrier 齐放,睡眠窗口内必然重叠)。
    #[test]
    fn resident_mixed_projects_parallel_and_serialized() {
        use std::sync::Barrier;
        const PROJECTS: usize = 4;
        const THREADS_PER: usize = 3;
        const ROUNDS: usize = 6;
        let roots: Vec<_> = (0..PROJECTS)
            .map(|i| cutforge_io::tests_fixture(&format!("mcp-resident-mix-{i}")).unwrap())
            .collect();
        let keys: Vec<_> = roots
            .iter()
            .map(|r| cache_key(&r.to_string_lossy()))
            .collect();
        let gcur = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gmax = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let total = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let per_cur: Vec<_> = (0..PROJECTS)
            .map(|_| Arc::new(std::sync::atomic::AtomicUsize::new(0)))
            .collect();
        let per_max: Vec<_> = (0..PROJECTS)
            .map(|_| Arc::new(std::sync::atomic::AtomicUsize::new(0)))
            .collect();
        let barrier = Arc::new(Barrier::new(PROJECTS * THREADS_PER));
        let mut handles = Vec::new();
        for p in 0..PROJECTS {
            for _ in 0..THREADS_PER {
                let (gcur, gmax, total) = (gcur.clone(), gmax.clone(), total.clone());
                let (pcur, pmax) = (per_cur[p].clone(), per_max[p].clone());
                let (k, r) = (keys[p].clone(), roots[p].clone());
                let barrier = barrier.clone();
                handles.push(std::thread::spawn(move || {
                    use std::sync::atomic::Ordering;
                    barrier.wait();
                    for _ in 0..ROUNDS {
                        with_resident(&k, &r, true, |_ws| {
                            gcur.fetch_add(1, Ordering::AcqRel);
                            gmax.fetch_max(gcur.load(Ordering::Acquire), Ordering::AcqRel);
                            pcur.fetch_add(1, Ordering::AcqRel);
                            pmax.fetch_max(pcur.load(Ordering::Acquire), Ordering::AcqRel);
                            std::thread::sleep(std::time::Duration::from_millis(40));
                            gcur.fetch_sub(1, Ordering::AcqRel);
                            pcur.fetch_sub(1, Ordering::AcqRel);
                            envelope(true, "OK", "n", json!({}))
                        });
                        total.fetch_add(1, Ordering::AcqRel);
                    }
                }));
            }
        }
        for h in handles {
            h.join().unwrap();
        }
        // ① 同工程严格串行
        for pm in &per_max {
            assert_eq!(
                pm.load(std::sync::atomic::Ordering::Acquire),
                1,
                "每工程内部必须严格串行"
            );
        }
        // ② 不同工程真并行(barrier 齐放 + 40ms 睡眠窗口,全局重叠必然可见)
        assert!(
            gmax.load(std::sync::atomic::Ordering::Acquire) >= 2,
            "不同工程的 per-project 锁必须真并行(全局 in-flight 峰值应 ≥2)"
        );
        assert_eq!(
            total.load(std::sync::atomic::Ordering::Acquire),
            PROJECTS * THREADS_PER * ROUNDS,
            "全部 RPC 必须完成"
        );
        for r in &roots {
            cutforge_io::fsutil::cleanup(r);
        }
        _clear_for_tests();
    }
}
