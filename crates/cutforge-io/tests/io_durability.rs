//! V2-W1-IO 耐久性 TC 用例(审查报告 v2 §5 R-01/R-02/R-03/R-04/R-05/R-10 + §4 BUG-12)。
//! 纪律:先红后绿;只断言公开行为(命令结果、文件状态、错误码、契约字段)。

use std::path::{Path, PathBuf};
use std::time::Duration;

use cutforge_core::command::Command;
use cutforge_core::model::TrackKind;
use cutforge_core::oplog::Actor;
use cutforge_io::Workspace;
use cutforge_io::atomic;
use cutforge_io::backup::backup_file;
use cutforge_io::fresh::disk_fingerprint;
use cutforge_io::fsutil;
use cutforge_io::lock;
use cutforge_io::paths::{self, PROJECT_REL};
use cutforge_io::probe;
use cutforge_io::snapshot::{DEFAULT_INTERVAL_MS, DEFAULT_KEEP, config_from};
use cutforge_io::watcher::{Watcher, ensure_sync_daemon, sync_daemon_count};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 把文件 mtime 拨旧(伪造心跳过期/锁龄)。
fn set_mtime_old(p: &Path, ms_ago: u64) {
    let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
    f.set_modified(
        std::time::SystemTime::now()
            .checked_sub(Duration::from_millis(ms_ago))
            .unwrap(),
    )
    .unwrap();
}

/// 伪造持锁现场(内容含 pid/ts/boot;mtime 单独拨)。
fn forge_lock(root: &Path, pid: u32, age_ms: u64, mtime_age_ms: u64) -> PathBuf {
    let dir = root.join(".cutforge");
    fsutil::ensure(&dir).unwrap();
    let path = dir.join("lock");
    let boot = probe::pid_start_time(pid).unwrap_or_default();
    // boot 在 ts 前:ts 字段始终是行尾最后一个 token,旧版解析器(仅认 ts=)同样可读
    atomic::atomic_write(
        &path,
        format!("pid={pid} boot={boot} ts={}", now_ms() - age_ms).as_bytes(),
    )
    .unwrap();
    set_mtime_old(&path, mtime_age_ms);
    path
}

/// 样例工程 + n 笔真实 Op(rev 1..=n)。
fn project_with_ops(tag: &str, n: usize) -> PathBuf {
    let root = cutforge_io::tests_fixture(tag).unwrap();
    let mut ws = Workspace::open_exclusive(&root).unwrap();
    for i in 0..n {
        ws.apply(
            Command::TrackAdd {
                kind: TrackKind::Audio,
                request_id: Some(format!("tc-{tag}-{i}")),
            },
            Actor::user("tc"),
            Default::default(),
        )
        .unwrap();
    }
    drop(ws);
    root
}

fn oplog_files(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root.join(".cutforge/oplog"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    v.sort();
    v
}

// ---------------- R-01:TC-IO-ATOMIC-001(SIGKILL 代理测试) ----------------
// CI 无法模拟真掉电:用"写后立即异常退出(exit 9,无清理无二次 flush)"代理 +
// 代码评审清单(atomic.rs 落盘点必须 sync_all/sync_data)兜底。

#[test]
fn tc_io_atomic_001_sigkill_proxy() {
    if std::env::var("CUTFORGE_ATOMIC_PROXY").as_deref() == Ok("child") {
        let dir = PathBuf::from(std::env::var("CUTFORGE_PROXY_DIR").unwrap());
        // ① atomic_write 后立即异常退出
        atomic::atomic_write(&dir.join("aw.txt"), b"atomic-durable").unwrap();
        // ② append_line 后立即异常退出(oplog 追加形态)
        atomic::append_line(&dir.join("ap.jsonl"), "line-01\n").unwrap();
        std::process::exit(9);
    }
    let dir = fsutil::temp_dir("tc-atomic-proxy");
    fsutil::ensure(&dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    let status = std::process::Command::new(exe)
        .args(["tc_io_atomic_001_sigkill_proxy", "--exact", "--nocapture"])
        .env("CUTFORGE_ATOMIC_PROXY", "child")
        .env("CUTFORGE_PROXY_DIR", &dir)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(9), "子进程必须以异常退出模拟崩溃");
    assert_eq!(
        std::fs::read(dir.join("aw.txt")).unwrap(),
        b"atomic-durable",
        "atomic_write 后崩溃,重启读回数据必须在"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ap.jsonl")).unwrap(),
        "line-01\n",
        "append_line 后崩溃,行必须完整(半行由 R-03 修复模式兜底)"
    );
    fsutil::cleanup(&dir);
}

// ---------------- R-02:TC-IO-LOCK-001/002/003 ----------------

/// TC-IO-LOCK-001:活进程持锁超龄 → 第二实例必须失败,**不得接管**。
#[test]
fn tc_io_lock_001_alive_holder_not_taken_over() {
    let root = cutforge_io::tests_fixture("tc-lock-001").unwrap();
    forge_lock(&root, std::process::id(), 60_000, 60_000);
    let err = lock::acquire(&root, 30_000, 0)
        .err()
        .expect("活进程持锁 60s,第二实例必须 open 失败而非接管");
    assert!(
        err.to_string().contains("锁定"),
        "失败语义必须是'被锁定': {err}"
    );
    fsutil::cleanup(&root);
}

/// TC-IO-LOCK-002:强杀残留(pid 死 + 锁龄超)→ 30s 后可接管;
/// 但 pid 死而锁龄未到 → 仍不可接管。
#[test]
fn tc_io_lock_002_dead_holder_takeover_after_stale() {
    let root = cutforge_io::tests_fixture("tc-lock-002").unwrap();
    // 死 pid 用跨平台助手(Linux pid=1 是 init 恒活,硬编码高段也可能被占)
    let dead = probe::definitely_dead_pid();
    forge_lock(&root, dead, 120_000, 120_000);
    assert!(
        lock::acquire(&root, 30_000, 0).is_ok(),
        "pid 死 + 锁龄 120s + 心跳过期 → 必须接管"
    );
    drop_root_and_redo(&root, dead, 0, 120_000);
    fsutil::cleanup(&root);
}

fn drop_root_and_redo(root: &Path, pid: u32, age_ms: u64, mtime_age_ms: u64) {
    let _ = std::fs::remove_file(root.join(".cutforge/lock"));
    forge_lock(root, pid, age_ms, mtime_age_ms);
    assert!(
        lock::acquire(root, 30_000, 0).is_err(),
        "pid 死但锁龄未到 30s → 不得接管"
    );
}

/// TC-IO-LOCK-003:心跳正常(mtime 新鲜)但 pid 存活 → 不接管。
#[test]
fn tc_io_lock_003_fresh_heartbeat_alive_pid_no_takeover() {
    let root = cutforge_io::tests_fixture("tc-lock-003").unwrap();
    // 锁龄 60s 超阈值,但持锁方每 5s 更新 mtime(此处不拨旧 = 心跳新鲜)
    forge_lock(&root, std::process::id(), 60_000, 0);
    let err = lock::acquire(&root, 30_000, 0)
        .err()
        .expect("心跳新鲜 + pid 存活 → 必须等待,不得接管");
    assert!(
        err.to_string().contains("锁定"),
        "失败语义必须是'被锁定': {err}"
    );
    fsutil::cleanup(&root);
}

// ---------------- R-03:TC-IO-OPEN-001/002 ----------------

/// TC-IO-OPEN-001:末尾追加半行 → open 成功但必须返回 RepairReport
/// (备份 + 截断修复 + 丢失操作数),绝不静默。
#[test]
fn tc_io_open_001_tail_half_line_repairs_with_report() {
    let root = project_with_ops("tc-open-001", 4);
    // 伪造崩溃窗口:记账 rev=5 已落(先文件后记账),第 5 个 Op 只写了半行
    atomic::atomic_write(&root.join(".cutforge/rev"), b"5\n").unwrap();
    let log_file = oplog_files(&root).pop().unwrap();
    atomic::append_line(&log_file, "{\"op_id\":\"op-99\",\"ts\":\"20").unwrap();

    let ws = Workspace::open_exclusive(&root).unwrap();
    let rep = ws
        .repair_report()
        .expect("open 必须产出 RepairReport,绝不静默");
    assert_eq!(rep.truncated_files.len(), 1, "必须指明损坏分片: {rep:?}");
    assert_eq!(rep.lost_ops, 1, "rev 5 − 最后完整 Op rev 4 = 丢失 1 个操作");
    assert_eq!(rep.reconciled_revs, vec![5], "差异自愈补记 rev 5");
    let backup = rep
        .backup_path
        .clone()
        .expect("修复前必须自动备份 .cutforge/");
    assert!(backup.is_dir(), "备份目录必须在盘: {}", backup.display());
    assert!(backup.join("oplog").is_dir(), "备份必须包含 oplog");
    assert!(ws.rev() >= 5, "修复后 rev 以记账为准");
    drop(ws);
    // 修复报告必须落盘(用户可见的第二出口)
    assert!(root.join(".cutforge/repair-report.json").is_file());
    // 再开:物理截断已生效,无新修复动作
    let ws2 = Workspace::open_exclusive(&root).unwrap();
    assert!(
        ws2.repair_report().map(|r| r.has_actions()) != Some(true),
        "第二次 open 不得再报修复(截断已物理修复)"
    );
    let rep2_text = std::fs::read_to_string(root.join(".cutforge/repair-report.json")).unwrap();
    assert!(
        rep2_text.contains("\"truncatedFiles\""),
        "报告 JSON 契约字段必须在"
    );
    fsutil::cleanup(&root);
}

/// TC-IO-OPEN-002:中间分片损坏 → 同样修复,且截断只中断当前分片,
/// 后续分片照常装载(§2.3 措辞修正),报告指明分片名。
#[test]
fn tc_io_open_002_middle_shard_truncated_reports_name() {
    let root = project_with_ops("tc-open-002", 2);
    // 把真实两行 Op 重排成两个分片:分片 1(旧日)装 rev1 + 半行 rev2;分片 2(今日)装完整 rev2
    let log_file = oplog_files(&root).pop().unwrap();
    let text = std::fs::read_to_string(&log_file).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);
    let shard1 = log_file.parent().unwrap().join("20260101.jsonl");
    let shard2 = log_file.parent().unwrap().join("20260102.jsonl");
    // 半行:切到不超过一半长度的字符边界(行内含 CJK,不得劈开字符)
    let mut cut = lines[1].len() / 2;
    while cut > 0 && !lines[1].is_char_boundary(cut) {
        cut -= 1;
    }
    std::fs::write(&shard1, format!("{}\n{}", lines[0], &lines[1][..cut])).unwrap();
    std::fs::write(&shard2, format!("{}\n", lines[1])).unwrap();
    std::fs::remove_file(&log_file).unwrap();
    atomic::atomic_write(&root.join(".cutforge/rev"), b"2\n").unwrap();

    let ws = Workspace::open_exclusive(&root).unwrap();
    let rep = ws.repair_report().expect("中间分片损坏也必须报告");
    assert_eq!(
        rep.truncated_files,
        vec!["20260101.jsonl".to_string()],
        "必须指明分片名"
    );
    assert_eq!(
        ws.engine().oplog().ops().len(),
        2,
        "截断只中断当前分片,后续分片必须照常装载"
    );
    assert_eq!(ws.rev(), 2);
    fsutil::cleanup(&root);
}

// ---------------- R-04:TC-IO-PERSIST-001 ----------------

/// TC-IO-PERSIST-001:project/rev 记账(5)> oplog max rev(4)→ open 自动补
/// ReconciledOp;undo 到该处明确拒绝并提示(不可再退)。
#[test]
fn tc_io_persist_001_reconciled_op_added_and_undo_refused() {
    let root = project_with_ops("tc-persist-001", 4);
    // 崩溃窗口:project.json 已含 rev5 的变更、记账 rev=5,但第 5 个 Op 从未写入 oplog
    atomic::atomic_write(&root.join(".cutforge/rev"), b"5\n").unwrap();

    let mut ws = Workspace::open_exclusive(&root).unwrap();
    let ops = ws.engine().oplog().ops();
    assert_eq!(ws.rev(), 5, "rev 以记账文件为准");
    let last = ops.last().expect("oplog 不得为空");
    assert_eq!(last.rev, Some(5), "必须补记 ReconciledOp(rev=5): {last:?}");
    assert!(
        last.summary.starts_with("[差异自愈]"),
        "补记 Op 必须自我声明: {}",
        last.summary
    );
    // 补记必须真实落盘(oplog 文件里第 5 行)
    let log_file = oplog_files(&root).pop().unwrap();
    let text = std::fs::read_to_string(&log_file).unwrap();
    assert_eq!(
        text.lines().filter(|l| !l.trim().is_empty()).count(),
        5,
        "补记 Op 必须落盘"
    );
    // undo 到 rev=5 处明确拒绝并提示(屏障:不可再退)
    let err = ws
        .undo(Actor::user("tc"))
        .expect_err("undo 必须在补记屏障处明确拒绝");
    assert!(err.to_string().contains("拒绝"), "拒绝必须带提示: {err}");
    assert_eq!(ws.rev(), 5, "拒绝后 rev 不得前进");
    // 修复报告在
    let rep = ws.repair_report().expect("差异自愈必须有报告");
    assert_eq!(rep.reconciled_revs, vec![5]);
    assert!(
        rep.summary().contains("不可再撤销"),
        "用户可见措辞: {}",
        rep.summary()
    );
    fsutil::cleanup(&root);
}

// ---------------- R-05:TC-IO-FRESH-001 ----------------

/// TC-IO-FRESH-001:同长度重写 oplog 末行 → 指纹必变(resident 缓存失效重载)。
#[test]
fn tc_io_fresh_001_same_length_rewrite_changes_fingerprint() {
    let root = fsutil::temp_dir("tc-fresh-001");
    fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
    fsutil::ensure(&root.join(".cutforge/oplog")).unwrap();
    atomic::atomic_write(&root.join(PROJECT_REL), b"{\"a\":1}").unwrap();
    // 两行真实形态的 Op(首行 rev=1,末行 rev=2;两行等长)
    let line = |oid: &str, rev: u64, sum: &str| {
        format!(
            "{{\"op_id\":\"{oid}\",\"ts\":\"t\",\"actor\":{{\"kind\":\"user\",\"id\":\"t\"}},\"target\":{{\"file\":\"project.json\",\"path\":\"/slug\"}},\"op_kind\":\"set\",\"before\":null,\"after\":null,\"base_rev\":\"rev-0\",\"rev\":{rev},\"summary\":\"{sum}\"}}\n"
        )
    };
    let log = root.join(".cutforge/oplog/20260101.jsonl");
    atomic::append_line(&log, &line("op-1", 1, "快")).unwrap();
    atomic::append_line(&log, &line("op-2", 2, "快")).unwrap();
    let fp1 = disk_fingerprint(&root).expect("project 在盘,指纹必有");
    // 同长度重写末行("快"→"慢",字节数相同,长度不变)
    let body = std::fs::read_to_string(&log).unwrap();
    let replacement = line("op-2", 2, "慢").trim_end().to_string() + "\n";
    let rewritten = body.replace(&line("op-2", 2, "快"), &replacement);
    assert_eq!(rewritten.len(), body.len(), "测试前提:同长度重写");
    atomic::atomic_write(&log, rewritten.as_bytes()).unwrap();
    let fp2 = disk_fingerprint(&root).unwrap();
    assert_ne!(
        fp1, fp2,
        "同长度重写 oplog 末行,指纹必须变化(长度指纹漏检的洞)"
    );
    assert_ne!(fp1.cache_key(), fp2.cache_key(), "缓存键必须随之失效");
    fsutil::cleanup(&root);
}

// ---------------- R-10:TC-IO-WATCH-001/002 ----------------

/// TC-IO-WATCH-001:空闲轮询指数退避 250ms→4s,发现变更回落 250ms
/// (10 万文件素材库 CPU<1% 的机制保证:空闲期扫描频率受 4s 上限约束)。
#[test]
fn tc_io_watch_001_idle_backoff_and_reset() {
    let root = fsutil::temp_dir("tc-watch-001");
    fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
    atomic::atomic_write(&root.join(PROJECT_REL), b"{}").unwrap();
    let mut w = Watcher::new(&root, 250);
    assert_eq!(w.current_interval(), Duration::from_millis(250));
    let mut seq = Vec::new();
    for _ in 0..5 {
        let _ = w.poll(); // 空闲轮询:无事件 → 间隔翻倍
        seq.push(w.current_interval().as_millis());
    }
    assert_eq!(
        seq,
        vec![500, 1000, 2000, 4000, 4000],
        "空闲必须指数退避且 4s 封顶: {seq:?}"
    );
    // 变更 → 回落基准间隔
    atomic::atomic_write(&root.join(PROJECT_REL), b"{\"a\":1}").unwrap();
    let _ = w.poll();
    assert_eq!(
        w.current_interval(),
        Duration::from_millis(250),
        "发现变更必须回落"
    );
    fsutil::cleanup(&root);
}

/// TC-IO-WATCH-002:工作区强引用全部丢弃 → 守护线程退出(线程数回落)。
#[test]
fn tc_io_watch_002_daemon_thread_exits_after_drop() {
    let root = fsutil::temp_dir("tc-watch-002");
    fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
    atomic::atomic_write(&root.join(PROJECT_REL), b"{}").unwrap();
    let baseline = sync_daemon_count();
    let hub = ensure_sync_daemon(&root);
    let grew = (0..100).any(|_| {
        if sync_daemon_count() > baseline {
            true
        } else {
            std::thread::sleep(Duration::from_millis(20));
            false
        }
    });
    assert!(grew, "守护线程必须被拉起");
    drop(hub);
    let fell = (0..300).any(|_| {
        if sync_daemon_count() == baseline {
            true
        } else {
            std::thread::sleep(Duration::from_millis(20));
            false
        }
    });
    assert!(
        fell,
        "强引用丢弃后守护线程必须退出并回落线程数(Weak 引用纪律)"
    );
    fsutil::cleanup(&root);
}

// ---------------- BUG-12:TC-IO-BACKUP-001/002 ----------------

/// TC-IO-BACKUP-001:同秒两次 persist → 两个备份目录都在(秒级时间戳不得互覆)。
#[test]
fn tc_io_backup_001_same_second_two_dirs() {
    let root = fsutil::temp_dir("tc-backup-001");
    fsutil::ensure(&root).unwrap();
    let d1 = backup_file(&root, PROJECT_REL, b"{\"v\":1}").unwrap();
    let d2 = backup_file(&root, PROJECT_REL, b"{\"v\":2}").unwrap();
    assert_ne!(
        d1.parent(),
        d2.parent(),
        "同秒两次备份不得共用目录: {} vs {}",
        d1.display(),
        d2.display()
    );
    // backup_file 返回的是备份文件本体(目录/文件名)
    assert_eq!(std::fs::read(&d1).unwrap(), b"{\"v\":1}");
    assert_eq!(std::fs::read(&d2).unwrap(), b"{\"v\":2}");
    assert!(d1.is_file() && d2.is_file(), "两个备份文件必须都在盘");
    fsutil::cleanup(&root);
}

/// TC-IO-BACKUP-002:超 50 代 → 保留 50 代,最旧代被清理(清理失败仅 warn)。
/// 跨秒分桶验证"最旧先清"(同秒内目录序含随机后缀,不保证创建序)。
#[test]
fn tc_io_backup_002_retention_prunes_to_50() {
    let root = fsutil::temp_dir("tc-backup-002");
    fsutil::ensure(&root).unwrap();
    // 伪造 10 个更旧的代(名字字典序最小,必排最前)
    let backup_root = root.join(paths::STATE).join("backup");
    for i in 0..10 {
        fsutil::ensure(&backup_root.join(format!("20260101-0000{i:02}-0000-aaaa"))).unwrap();
    }
    // 桶 1:45 代(更旧秒);桶 2:10 代(最新秒,跨 1.1s 保证时间戳分桶)
    let mut newest: Vec<PathBuf> = Vec::new();
    for i in 0..45 {
        backup_file(&root, PROJECT_REL, format!("{{\"i\":{i}}}").as_bytes()).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(1100));
    for i in 45..55 {
        newest.push(backup_file(&root, PROJECT_REL, format!("{{\"i\":{i}}}").as_bytes()).unwrap());
    }
    let dirs: Vec<String> = std::fs::read_dir(&backup_root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        dirs.len(),
        50,
        "必须保留 50 代(10 伪造 + 45 + 10 = 65 → 清 15): {}",
        dirs.len()
    );
    assert!(
        !backup_root.join("20260101-000000-0000-aaaa").exists(),
        "伪造最旧代必须被清理"
    );
    for p in &newest {
        assert!(p.is_file(), "最新秒桶的备份必须全数存活: {}", p.display());
    }
    fsutil::cleanup(&root);
}

// ---------------- R-13①:快照缺省开 ----------------

#[test]
fn snapshot_default_on_env_can_disable() {
    let (iv, keep) = config_from(None, None);
    assert_eq!(
        iv,
        Some(Duration::from_millis(DEFAULT_INTERVAL_MS)),
        "缺省必须开(5min)"
    );
    assert_eq!(DEFAULT_INTERVAL_MS, 300_000);
    assert_eq!(keep, DEFAULT_KEEP);
    assert_eq!(keep, 10);
    let (iv, _) = config_from(Some("0"), None);
    assert_eq!(iv, None, "env 显式 0 必须仍可关");
    let (iv, keep) = config_from(Some("60000"), Some("3"));
    assert_eq!(iv, Some(Duration::from_secs(60)));
    assert_eq!(keep, 3);
}
