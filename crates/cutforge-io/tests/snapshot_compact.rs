//! OpLog 压实接线的真实文件级回归(R-13②③ 收口;工单 V2-W3-IO-compact-wiring):
//! - TC-IO-SNAP-003:压实前后 open 结果逐字节/语义相等(工程视图、rev、引擎
//!   内存全量 OpLog、撤销深度四面);
//! - TC-IO-SNAP-004:kill -9 在「快照 → 截断」两步间任意点中断(跨分片部分
//!   截断 / 截断完成)→ 下次 open 自愈,rev/双栈/虚拟历史与 golden 相等;
//! - TC-IO-SNAP-005:快照缺失 → 放弃压实(盘面字节零变化,行为同旧);
//!   旧日志缺 rev → 放弃压实 + open 回退全量装载。
//!
//! 全部用例走真实盘面(scaffold/真实分片文件/真实快照目录),不用内存替身。
//!
//! 债务登记(工单条目 5,不修):clips_patch 多轨 Op 面只取首轨 before/after
//! ——首轨无操作、其余轨有变更时该 Op 被幂等吞掉(KERNEL2 发现的既有边界,
//! core 层口径)。io 层观察:该边界与装载路径无关,快照优先与全量两条路径
//! 回放同一 Op 面,行为一致,按工单保持零变化。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{Answer, ApplyOpts, Query};
use cutforge_core::oplog::{Actor, Op, OpKind, OpTarget};
use cutforge_io::snapshot::SNAPSHOTS_DIR;
use cutforge_io::{Workspace, fsutil, snapshot};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn agent() -> Actor {
    Actor::agent("snap-io-tc")
}

/// 工程视图(ProjectView)的 serde 值(语义对拍用)。
fn view(ws: &Workspace) -> Value {
    match ws.engine().query(Query::ProjectView) {
        Answer::Project(v) => v,
        _ => unreachable!(),
    }
}

/// 引擎内存 OpLog 的逐 Op serde 值(虚拟全量历史对拍用)。
fn ops_value(ws: &Workspace) -> Vec<Value> {
    ws.engine()
        .oplog()
        .ops()
        .iter()
        .map(|o| serde_json::to_value(o).unwrap())
        .collect()
}

/// 同上,但剥离 `ts`:两面各自真实 apply 时 Op 的 ts 是各自的物理时钟,
/// 历史保真对拍的语义面不含墙上时钟。
fn ops_value_no_ts(ws: &Workspace) -> Vec<Value> {
    ops_value(ws)
        .into_iter()
        .map(|mut o| {
            o.as_object_mut().unwrap().remove("ts");
            o
        })
        .collect()
}

fn update(ws: &mut Workspace, vol: f64) {
    ws.apply(
        Command::ClipUpdate {
            clip_id: "V1-001".into(),
            patch: ClipPatch {
                volume: Some(vol),
                ..Default::default()
            },
        },
        agent(),
        ApplyOpts::default(),
    )
    .unwrap();
}

/// 驱动 undo 直到 NOTHING_TO_UNDO,返回撤销深度(双栈正确性的行为面观测)。
fn undo_depth(root: &Path) -> usize {
    let mut ws = Workspace::open(root).unwrap();
    let mut n = 0;
    while ws.undo(agent()).is_ok() {
        n += 1;
    }
    n
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let p = e.path();
        let t = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

/// oplog 分片文件(按文件名升序)。
fn oplog_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join(".cutforge/oplog"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files
}

/// 全部 oplog 分片的非空行数(盘面保留面断言用)。
fn oplog_line_count(root: &Path) -> usize {
    oplog_files(root)
        .iter()
        .map(|f| {
            let data = std::fs::read_to_string(f).unwrap();
            data.lines().filter(|l| !l.trim().is_empty()).count()
        })
        .sum()
}

/// 行内 Op 的 rev(测试侧行级校验;与 fresh.rs 指纹同口径的宽松解析)。
fn line_rev(line: &str) -> Option<u64> {
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("rev")?
        .as_u64()
}

/// 构造一条可解析的 bulk Op 行(压实体量夹具;不追求业务语义,追求与
/// scan/replay/栈重建的契约兼容)。
fn bulk_op_line(rev: u64) -> String {
    let op = Op {
        op_id: format!("op-{rev}"),
        ts: "2026-10-04T00:00:00Z".into(),
        actor: Actor::agent("tc-bulk"),
        target: OpTarget {
            file: "project.json".into(),
            path: "/slug".into(),
        },
        op_kind: OpKind::Set,
        before: json!("旧"),
        after: json!("新"),
        base_rev: format!("rev-{}", rev.saturating_sub(1)),
        rev: Some(rev),
        target_id: None,
        caused_by: None,
        summary: "压实体量夹具".into(),
        request_id: None,
        auto: None,
    };
    serde_json::to_string(&op).unwrap()
}

// ---------------------------------------------------------------- --
// TC-IO-SNAP-003:压实前后 open 结果相等(视图/rev/全量历史/撤销深度)
// ---------------------------------------------------------------- --
#[test]
fn tc_io_snap_003_open_before_and_after_compact_equal() {
    let root = cutforge_io::tests_fixture("snap-tc003").unwrap();
    let ref_root = fsutil::temp_dir("snap-tc003-ref");
    // 夹具中性量:bulk Op 的 before=after=夹具 slug(语义零效应,字节真实),
    // 使「全量装载(不回放)」与「快照优先增量回放」两条路径必然同态,
    // 对拍焦点集中在压实接线的 rev/历史/双栈保真上。
    let project_text = std::fs::read_to_string(root.join(cutforge_io::PROJECT_REL)).unwrap();
    let project_v: Value = serde_json::from_str(&project_text).unwrap();
    let slug = project_v["slug"].as_str().unwrap().to_string();
    let bulk_line = |rev: u64| {
        let op = Op {
            op_id: format!("op-{rev}"),
            ts: "2026-10-04T00:00:00Z".into(),
            actor: Actor::agent("tc003-bulk"),
            target: OpTarget {
                file: "project.json".into(),
                path: "/slug".into(),
            },
            op_kind: OpKind::Set,
            before: json!(slug),
            after: json!(slug),
            base_rev: format!("rev-{}", rev.saturating_sub(1)),
            rev: Some(rev),
            target_id: None,
            caused_by: None,
            summary: "压实体量夹具(语义中性)".into(),
            request_id: None,
            auto: None,
        };
        serde_json::to_string(&op).unwrap()
    };
    let append_bulk = |root: &Path, revs: std::ops::RangeInclusive<u64>| {
        let day = cutforge_core::timeutil::now_date_compact();
        let file = root.join(".cutforge/oplog").join(format!("{day}.jsonl"));
        for rev in revs {
            cutforge_io::atomic::append_line(
                &file,
                &format!(
                    "{}
",
                    bulk_line(rev)
                ),
            )
            .unwrap();
        }
    };

    // ---- ① 真实首笔(rev 1)+ 显式真实快照 r1(快照时刻工程/日志均为 rev 1 态;
    //        open 迁移落盘可能已产 r0,显式 r1 保证「截断确有前缀可删」)----
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        update(&mut ws, 0.51); // ≠ 夹具初始 volume(1.0),必非幂等短路
        assert_eq!(ws.rev(), 1, "首笔后 rev=1");
        snapshot::write_snapshot(&root, &root.join(SNAPSHOTS_DIR), 10).unwrap();
    }
    assert_eq!(
        snapshot::latest_snapshot_rev(&root),
        Some(1),
        "显式快照 r1 在盘"
    );

    // ---- ② 体量到 rev 1001:gap = 1001 - 1 = 1000,恰不越阈值(严格 >)----
    append_bulk(&root, 2..=1000);
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        assert_eq!(ws.rev(), 1000, "bulk 前段 rev 对账");
        assert!(!ws.compact_oplog().unwrap(), "gap < 阈值时必须拒绝压实");
    }
    append_bulk(&root, 1001..=1001);
    assert_eq!(oplog_line_count(&root), 1001, "夹具自检:盘面 1001 行");

    // ---- ③ 基准:全量路径 open(无快照副本)@rev 1001 ----
    copy_dir(&root, &ref_root);
    std::fs::remove_dir_all(ref_root.join(SNAPSHOTS_DIR)).unwrap();
    let ws_ref1001 = Workspace::open(&ref_root).unwrap();
    let view_1001 = view(&ws_ref1001);
    let ops_1001 = ops_value(&ws_ref1001);
    assert_eq!(ws_ref1001.rev(), 1001, "全量路径 rev 对账");

    // ---- ④ 快照优先路径(盘面前缀完整形态)必须与全量路径零差异 ----
    {
        let ws_pre = Workspace::open(&root).unwrap();
        assert_eq!(ws_pre.rev(), 1001, "未压实形态 rev");
        assert_eq!(
            view(&ws_pre),
            view_1001,
            "未压实形态:快照路径 ≡ 全量路径(视图)"
        );
        assert_eq!(
            ops_value(&ws_pre),
            ops_1001,
            "未压实形态:快照路径 ≡ 全量路径(OpLog)"
        );
    }

    // ---- ⑤ 两面各推进同一笔真实变更 → rev 1002:
    //        压实侧 gap 1001 > 阈值 → 写尾空闲压实自动截断 rev ≤ 1 前缀;
    //        基准侧(无快照)日志保持 1..=1002 完整在盘 ----
    {
        let mut ws = Workspace::open_for_write(&ref_root).unwrap();
        update(&mut ws, 0.9);
        assert_eq!(ws.rev(), 1002, "基准副本推进");
    }
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        update(&mut ws, 0.9);
        assert_eq!(ws.rev(), 1002, "压实侧推进");
    }
    assert_eq!(
        oplog_line_count(&root),
        1001,
        "压实后盘面只保留 rev > S 的后缀"
    );
    let ws_ref = Workspace::open(&ref_root).unwrap();
    let view_1002 = view(&ws_ref);
    let ops_1002 = ops_value_no_ts(&ws_ref);
    assert_eq!(ops_1002.len(), 1002, "基准副本全量历史 1002 条");

    // ---- ⑥ 首/末行 rev 可解析且正确(fresh.rs OplogSig「首/末行 rev+FNV」输入面)----
    let lines: Vec<String> = std::fs::read_to_string(&oplog_files(&root)[0])
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        line_rev(&lines[0]),
        Some(2),
        "截断点 = retain_from:首行即首条保留 Op"
    );
    assert_eq!(
        line_rev(lines.last().unwrap()),
        Some(1002),
        "末行即最后完整 Op(整行截断,无半行)"
    );

    // ---- ⑦ 压实后 open(快照优先,盘面仅后缀)与全量路径四面相等 ----
    let ws_post = Workspace::open(&root).unwrap();
    assert_eq!(ws_post.rev(), 1002, "压实后 open rev");
    assert_eq!(view(&ws_post), view_1002, "压实前后工程视图逐值相等");
    assert_eq!(
        ops_value_no_ts(&ws_post),
        ops_1002,
        "引擎内存必须持全量虚拟历史(前缀取自快照副本;护城河 6)"
    );
    assert!(
        ws_post.repair_report().is_none(),
        "压实后的干净盘面不得触发修复面"
    );

    // ---- ⑧ 指纹自洽:截断后指纹可计算、稳定、且与本地写入登记机制自洽 ----
    let fp = cutforge_io::fresh::disk_fingerprint(&root).expect("指纹必可计算");
    assert_eq!(
        fp,
        cutforge_io::fresh::disk_fingerprint(&root).unwrap(),
        "盘面未再变化,指纹必须稳定"
    );
    cutforge_io::fresh::note_local_write(&root);
    assert!(
        cutforge_io::fresh::is_last_local_write(&root, &fp),
        "压实后的盘面必须能作为本进程写入登记基准(守护免开判别不破)"
    );

    // ---- ⑨ 撤销深度(跨压实边界 undo 可用;按构造:1002 条全部入撤销栈)----
    {
        let mut ws = Workspace::open(&root).unwrap();
        let mut n = 0;
        while ws.undo(agent()).is_ok() {
            n += 1;
            assert!(n <= 1002, "撤销深度超过构造期望(全量 1002)");
        }
        assert_eq!(n, 1002, "压实后撤销深度必须 = 全量历史条数(双栈自愈)");
    }

    fsutil::cleanup(&root);
    fsutil::cleanup(&ref_root);
}

// ---------------------------------------------------------------- --
// TC-IO-SNAP-004:kill -9 中断截断 → open 自愈(rev/双栈/历史正确)
// ---------------------------------------------------------------- --
/// 把唯一分片拆成两个日切分片:day1 = rev ≤ cut,day2 = rev > cut。
fn split_oplog_into_two_days(root: &Path, cut: u64) {
    let files = oplog_files(root);
    assert_eq!(files.len(), 1, "夹具前提:单分片");
    let data = std::fs::read_to_string(&files[0]).unwrap();
    let (mut d1, mut d2) = (String::new(), String::new());
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        if line_rev(line).is_some_and(|r| r <= cut) {
            d1.push_str(line);
            d1.push('\n');
        } else {
            d2.push_str(line);
            d2.push('\n');
        }
    }
    let dir = root.join(".cutforge/oplog");
    std::fs::remove_file(&files[0]).unwrap();
    std::fs::write(dir.join("20260101.jsonl"), d1).unwrap();
    std::fs::write(dir.join("20260102.jsonl"), d2).unwrap();
}

#[test]
fn tc_io_snap_004_kill9_partial_truncation_recovers_on_open() {
    let root = cutforge_io::tests_fixture("snap-tc004").unwrap();
    let ref_root = fsutil::temp_dir("snap-tc004-ref");

    // ---- rev 1..4 四笔变更 → 真实快照 r4(快照时刻工程/日志均为 rev 4 态)----
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        for v in [0.1, 0.2, 0.3, 0.4] {
            update(&mut ws, v);
        }
        assert_eq!(ws.rev(), 4);
        snapshot::write_snapshot(&root, &root.join(SNAPSHOTS_DIR), 10).unwrap();
        // ---- rev 5..8:update/undo/redo/update(后缀含撤销语义)----
        update(&mut ws, 0.5);
        ws.undo(agent()).unwrap();
        ws.redo(agent()).unwrap();
        update(&mut ws, 0.6);
        assert_eq!(ws.rev(), 8);
    }

    // ---- golden:无快照副本上的全量路径 ----
    copy_dir(&root, &ref_root);
    std::fs::remove_dir_all(ref_root.join(SNAPSHOTS_DIR)).unwrap();
    let ws_ref = Workspace::open(&ref_root).unwrap();
    let (golden_view, golden_ops) = (view(&ws_ref), ops_value(&ws_ref));
    let golden_depth = {
        let d = undo_depth(&ref_root);
        assert!(d > 0, "夹具自检:golden 必须有可撤销深度");
        d
    };

    // ---- case A:步 2 未开始(日志原样 + 快照在盘)→ open == golden ----
    {
        let case = fsutil::temp_dir("snap-tc004-a");
        copy_dir(&root, &case);
        assert_case_equals_golden(
            &case,
            &golden_view,
            &golden_ops,
            golden_depth,
            "A 步2未开始",
        );
        fsutil::cleanup(&case);
    }

    // ---- case B:跨分片部分截断(day1 已截空、day2 未动;kill -9 在分片间)----
    {
        let case = fsutil::temp_dir("snap-tc004-b");
        copy_dir(&root, &case);
        split_oplog_into_two_days(&case, 4);
        let day1 = case.join(".cutforge/oplog/20260101.jsonl");
        std::fs::write(&day1, b"").unwrap(); // 步 2 已截 day1(rev ≤ 4 全被截)
        assert_case_equals_golden(
            &case,
            &golden_view,
            &golden_ops,
            golden_depth,
            "B 跨分片部分截断",
        );
        fsutil::cleanup(&case);
    }

    // ---- case C:截断完成(单分片只留 rev > 4)→ open == golden ----
    {
        let case = fsutil::temp_dir("snap-tc004-c");
        copy_dir(&root, &case);
        let file = &oplog_files(&case)[0];
        let data = std::fs::read_to_string(file).unwrap();
        let kept: String = data
            .lines()
            .filter(|l| !l.trim().is_empty() && line_rev(l).is_some_and(|r| r > 4))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(file, kept).unwrap();
        assert_eq!(oplog_line_count(&case), 4, "case C:盘面只留 rev > 4 后缀");
        assert_case_equals_golden(&case, &golden_view, &golden_ops, golden_depth, "C 截断完成");
        fsutil::cleanup(&case);
    }

    fsutil::cleanup(&root);
    fsutil::cleanup(&ref_root);
}

/// case 断言:open 自愈 = rev / 工程视图 / 虚拟全量历史 / 撤销深度四面相等,
/// 且无修复面(这些工件不含半行截断)。
fn assert_case_equals_golden(
    case: &Path,
    golden_view: &Value,
    golden_ops: &[Value],
    golden_depth: usize,
    tag: &str,
) {
    let ws = Workspace::open(case).unwrap();
    assert_eq!(ws.rev(), 8, "{tag}: rev 必须自愈到 8");
    assert_eq!(&view(&ws), golden_view, "{tag}: 工程视图必须与 golden 相等");
    assert_eq!(
        ops_value(&ws),
        golden_ops,
        "{tag}: 虚拟历史必须与 golden 相等"
    );
    assert!(ws.repair_report().is_none(), "{tag}: 不得误报修复面");
    assert_eq!(
        undo_depth(case),
        golden_depth,
        "{tag}: 撤销深度必须与 golden 相等(双栈自愈)"
    );
}

// ---------------------------------------------------------------- --
// TC-IO-SNAP-005:快照缺失 → 放弃压实(盘面字节零变化,行为同旧)
// ---------------------------------------------------------------- --
#[test]
fn tc_io_snap_005_missing_snapshot_aborts_compact_and_keeps_old_behavior() {
    let root = cutforge_io::tests_fixture("snap-tc005").unwrap();
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        update(&mut ws, 0.5); // rev 1(首个 persist 自动落 r1)
        assert_eq!(ws.rev(), 1);
    }
    // 快照目录整体拿掉 = 快照缺失;体量夹具:追加 rev 2..=1002 的合法 Op 行
    //(无快照 → open 恒走全量路径,状态恒取盘面 project.json,与旧装载零差异)
    std::fs::remove_dir_all(root.join(SNAPSHOTS_DIR)).unwrap();
    {
        let day = cutforge_core::timeutil::now_date_compact();
        let file = root.join(".cutforge/oplog").join(format!("{day}.jsonl"));
        for rev in 2..=1002u64 {
            cutforge_io::atomic::append_line(&file, &format!("{}\n", bulk_op_line(rev))).unwrap();
        }
    }
    let before = std::fs::read(&oplog_files(&root)[0]).unwrap();
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        assert_eq!(ws.rev(), 1002, "全量路径 rev 对账与旧口径一致");
        assert!(!ws.compact_oplog().unwrap(), "无快照必须放弃压实");
    }
    let after = std::fs::read(&oplog_files(&root)[0]).unwrap();
    assert_eq!(before, after, "放弃压实必须盘面字节零变化");
    {
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.rev(), 1002);
        assert!(ws.repair_report().is_none(), "旧路径行为同旧:无修复面");
    }
    fsutil::cleanup(&root);
}

// ---------------------------------------------------------------- --
// TC-IO-SNAP-005(旧日志口径):oplog 存在缺 rev 的旧格式 Op →
// 放弃压实(与 compact::plan 同口径)+ open 回退全量装载(行为同旧)
// ---------------------------------------------------------------- --
#[test]
fn tc_io_snap_005b_legacy_log_without_rev_aborts_compact() {
    let root = cutforge_io::tests_fixture("snap-tc005b").unwrap();
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        update(&mut ws, 0.5); // rev 1(自动落 r1:快照在盘,逼迫「缺 rev 门」表态)
        assert_eq!(ws.rev(), 1);
    }
    // 尾接一条旧格式(缺 rev)Op + 一条带 rev 的完整 Op:
    // 末条 Op 的 rev(2)≥ 记账 rev(1)→ 不触发 R-03/R-04 修复面,
    // 纯测「缺 rev 放弃压实 + open 回退」本身。
    {
        let legacy = Op {
            op_id: "op-legacy-x".into(),
            ts: "2026-10-04T00:00:00Z".into(),
            actor: Actor::agent("tc005b"),
            target: OpTarget {
                file: "project.json".into(),
                path: "/slug".into(),
            },
            op_kind: OpKind::Set,
            before: json!("旧"),
            after: json!("新"),
            base_rev: "rev-1".into(),
            rev: None, // 旧格式:缺 rev(serde default;序列化时整个字段缺席)
            target_id: None,
            caused_by: None,
            summary: "旧格式 Op".into(),
            request_id: None,
            auto: None,
        };
        let day = cutforge_core::timeutil::now_date_compact();
        let file = root.join(".cutforge/oplog").join(format!("{day}.jsonl"));
        cutforge_io::atomic::append_line(
            &file,
            &format!("{}\n", serde_json::to_string(&legacy).unwrap()),
        )
        .unwrap();
        cutforge_io::atomic::append_line(&file, &format!("{}\n", bulk_op_line(2))).unwrap();
    }
    let before = std::fs::read(&oplog_files(&root)[0]).unwrap();
    {
        let mut ws = Workspace::open_for_write(&root).unwrap();
        assert_eq!(ws.rev(), 2, "缺 rev Op 不入 rev 链(顺推/对账旧口径)");
        assert!(!ws.compact_oplog().unwrap(), "旧日志缺 rev 必须放弃压实");
    }
    let after = std::fs::read(&oplog_files(&root)[0]).unwrap();
    assert_eq!(before, after, "放弃压实必须盘面字节零变化(不丢旧历史)");
    {
        let ws = Workspace::open(&root).unwrap();
        assert!(ws.repair_report().is_none(), "回退全量装载不得误报修复面");
        assert_eq!(
            view(&ws)["tracks"][0]["clips"][0]["volume"],
            json!(0.5),
            "回退全量装载后状态与旧路径一致"
        );
    }
    fsutil::cleanup(&root);
}
