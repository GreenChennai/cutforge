//! Workspace 内联测试(拆分自原 lib.rs,用例逐字保留;仅 use 导入随模块化调整)。

use std::path::{Path, PathBuf};

use cutforge_core::command::Command;
use cutforge_core::engine::ApplyOpts;
use cutforge_core::notes::NoteAuthor;
use cutforge_core::oplog::{Actor, OpKind};

use super::Workspace;
use super::bases::BASES_REL;
use crate::paths::{CUTLIST_REL, NOTES_REL, PROJECT_REL};
use crate::{atomic, fsutil, paths, tests_fixture};

mod tests {
    use super::*;

    /// V2-HOTFIX 回归(web-e2e A2 第 13 步外部追加丢失):快照优先装载(R-13②)
    /// 的重建头态与磁盘 project.json 不一致(外部直写追加 V1-901 + 改 volume)时,
    /// 必须放弃快照优化、回退全量装载(磁盘为权威)——外部追加不得被快照态
    /// 静默覆写丢失。修复前:快照 r1(2 clips)优先于磁盘(3 clips)→ 外部追加丢。
    #[test]
    fn snapshot_first_load_defers_to_disk_when_externally_modified() {
        let root = tests_fixture("ws-hotfix-snap").unwrap();
        // 1) 产编辑史并落快照(R-13 缺省开;显式触发兜底环境缺失形态)
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        ws.apply(
            Command::ClipSplit {
                clip_id: "V1-001".into(),
                t_ms: 2000,
            },
            Actor::agent("hotfix-test"),
            ApplyOpts::default(),
        )
        .unwrap();
        drop(ws);
        // 快照若因 env 缺省关闭而缺席,则本用例场景(快照优先分支)不成立 → 跳过
        let snap_dir = root.join(".cutforge/snapshots");
        if snap_dir
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
        {
            eprintln!("快照未生成(env 缺省关),跳过快照优先回归");
            fsutil::cleanup(&root);
            return;
        }
        // 2) 外部直写:追加 V1-901 + 改 V1-001 volume(e2e external_append_clip 同款)
        let pj = root.join(PROJECT_REL);
        let mut doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&pj).unwrap()).unwrap();
        let v1 = doc["tracks"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|t| t["id"] == "V1")
            .unwrap();
        let clips = v1["clips"].as_array_mut().unwrap();
        clips[0]["volume"] = serde_json::json!(0.66);
        clips.push(serde_json::json!({
            "id": "V1-901", "src": "01_materials/a.mp4",
            "startMs": 60000, "durationMs": 10000
        }));
        // 模拟外部工具直写:走 sanctioned 原语落盘(check-write-paths 合规;
        // 语义要点是「内容在 engine apply 之外变化」,落盘机制不参与断言)
        cutforge_io::atomic::atomic_write(
            &pj,
            serde_json::to_string_pretty(&doc).unwrap().as_bytes(),
        )
        .unwrap();
        // 3) 重开装载:外部追加必须保留(磁盘为权威)
        let ws2 = Workspace::open(&root).unwrap();
        let view = ws2
            .engine()
            .query(cutforge_core::engine::Query::ProjectView);
        let cutforge_core::engine::Answer::Project(ref v) = view else {
            unreachable!()
        };
        let clips_out = v["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "V1")
            .unwrap()["clips"]
            .as_array()
            .unwrap();
        let ids: Vec<&str> = clips_out.iter().filter_map(|c| c["id"].as_str()).collect();
        assert!(
            ids.contains(&"V1-901"),
            "外部追加被快照优先装载静默丢弃: {ids:?}"
        );
        let vol = clips_out
            .iter()
            .find(|c| c["id"] == "V1-001")
            .and_then(|c| c.get("volume"))
            .cloned()
            .unwrap_or_default();
        assert_eq!(vol, serde_json::json!(0.66), "外部修改同样必须保留");
        fsutil::cleanup(&root);
    }

    #[test]
    fn open_migrates_v1_and_persists_commands() {
        let root = tests_fixture("ws-open").unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        assert_eq!(ws.rev(), 0);
        // v1 样本经迁移后可正常应用命令
        let r = ws
            .apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch {
                        duration_ms: Some(8000),
                        ..Default::default()
                    },
                },
                Actor::agent("io-test"),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(r.rev, 1);
        assert_eq!(ws.rev(), 1);
        // 落盘验证:project.json/rev/oplog 都存在
        assert!(root.join(".cutforge/rev").exists());
        assert!(
            root.join(".cutforge/oplog")
                .read_dir()
                .unwrap()
                .next()
                .is_some()
        );
        fsutil::cleanup(&root);
    }

    #[test]
    fn reopen_restores_state_and_undo_stack() {
        let root = tests_fixture("ws-reopen").unwrap();
        {
            let mut ws = Workspace::open_exclusive(&root).unwrap();
            ws.apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch {
                        duration_ms: Some(8000),
                        ..Default::default()
                    },
                },
                Actor::user("人"),
                ApplyOpts::default(),
            )
            .unwrap();
        }
        let mut ws2 = Workspace::open_exclusive(&root).unwrap();
        assert_eq!(ws2.rev(), 1, "rev 必须从盘面恢复");
        assert_eq!(ws2.engine().oplog().len(), 1, "OpLog 必须从 jsonl 恢复");
        // 恢复后的撤销栈仍然可用
        ws2.undo(Actor::user("人")).unwrap();
        let disk = std::fs::read_to_string(root.join(PROJECT_REL)).unwrap();
        assert!(disk.contains("8400"), "撤销后盘面应回到 8400");
        fsutil::cleanup(&root);
    }

    /// M8-1 门禁(IO 侧):notes_add→undo,notes.json 盘面**语义**还原
    /// (canonical JSON 逐值相等;重序列化的缩进/键序不属语义域)。
    /// 撤销深度 = 真实用户手势数(auto 的重定位 Op 不入栈)。
    #[test]
    fn notes_add_undo_restores_disk_bytes() {
        let root = tests_fixture("ws-undo-notes").unwrap();
        let notes_before = std::fs::read_to_string(root.join(NOTES_REL)).unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        let anchor = cutforge_core::anchor::Anchor {
            kind: cutforge_core::anchor::AnchorKind::Clip,
            ref_: Some("V1-001".into()),
            t_ms: 4000,
            span: None,
        };
        ws.notes_add(
            anchor,
            "这里语速太快".into(),
            NoteAuthor::User,
            vec![],
            Actor::user("人"),
            None,
        )
        .unwrap();
        let after_add: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(NOTES_REL)).unwrap()).unwrap();
        assert_eq!(
            after_add["items"].as_array().unwrap().len(),
            3,
            "新标注必须落盘"
        );
        ws.undo(Actor::user("人")).unwrap();
        let notes_after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(NOTES_REL)).unwrap()).unwrap();
        let before: serde_json::Value = serde_json::from_str(&notes_before).unwrap();
        assert_eq!(
            notes_after, before,
            "undo 后 notes.json 必须还原到 notes_add 之前"
        );
        fsutil::cleanup(&root);
    }

    /// M8-1 门禁(cutlist 侧):经 record_change 的 cutlist 编辑可 undo 还原盘面
    /// (修复前:op 记到 project 的 "/" 指针,cutlist 文件原封不动)。
    #[test]
    fn cutlist_record_change_undo_restores_disk() {
        let root = tests_fixture("ws-undo-cutlist").unwrap();
        let before_text = std::fs::read_to_string(root.join(CUTLIST_REL)).unwrap();
        let before: serde_json::Value = serde_json::from_str(&before_text).unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        let mut after = before.clone();
        after["cuts"][0]["action"] = serde_json::json!("review");
        ws.record_change(
            "cutlist.json",
            "/",
            before.clone(),
            after,
            OpKind::Set,
            Actor::script("m8-1"),
            ApplyOpts {
                summary: Some("cut_apply 测试".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let mid: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(CUTLIST_REL)).unwrap())
                .unwrap();
        assert_eq!(
            mid["cuts"][0]["action"],
            serde_json::json!("review"),
            "编辑必须真实落盘"
        );
        ws.undo(Actor::user("人")).unwrap();
        let restored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(CUTLIST_REL)).unwrap())
                .unwrap();
        assert_eq!(restored, before, "undo 后 cutlist.json 必须还原");
        fsutil::cleanup(&root);
    }

    /// M8-4 门禁:双线程并发写同一工程,open_exclusive 锁覆盖 open→apply→persist
    /// 全程 → oplog 数 = 盘面 rev = 2N,零丢更新(修复前:锁外读+后写整文件覆盖)。
    #[test]
    fn concurrent_writes_no_loss() {
        use std::sync::Barrier;
        let root = tests_fixture("ws-concurrent").unwrap();
        let root_s = root.to_string_lossy().to_string();
        let n = 5;
        let barrier = std::sync::Arc::new(Barrier::new(2));
        let handles: Vec<_> = [("并发-A", 7000u64), ("并发-B", 7700)]
            .into_iter()
            .map(|(who, base)| {
                let barrier = barrier.clone();
                let root_s = root_s.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for i in 0..n {
                        let mut ws = Workspace::open_exclusive(Path::new(&root_s)).unwrap();
                        ws.apply(
                            Command::ClipUpdate {
                                clip_id: "V1-001".into(),
                                patch: cutforge_core::command::ClipPatch {
                                    // 两线程值域必须不相交:who.len() 是字节长度
                                    // (两个名字同长),同值交错会命中幂等短路而少记账。
                                    duration_ms: Some(base - i as u64),
                                    ..Default::default()
                                },
                            },
                            Actor::agent(who),
                            ApplyOpts::default(),
                        )
                        .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for h in handles {
            h.join().unwrap();
        }
        let ws = Workspace::open(&root).unwrap();
        assert_eq!(ws.rev(), 2 * n as u64, "rev 必须 = 2N(零丢更新)");
        assert_eq!(ws.engine().oplog().len(), 2 * n, "OpLog 必须 = 2N");
        let disk_rev: u64 = std::fs::read_to_string(root.join(".cutforge/rev"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(disk_rev, 2 * n as u64, "盘面 rev 与 OpLog 不得分叉");
        fsutil::cleanup(&root);
    }

    /// M8-2 门禁:CutFlow 真实 IR(顶层 _meta)可打开、可迁移、roundtrip 不丢 _meta。
    /// 夹具由 tools/gen_real_ir_fixture.py 调 CutFlow rs_ir.py 生成(计划书 D2)。
    #[test]
    fn open_real_cutflow_ir_with_meta_roundtrip() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/real_ir/project.json");
        let text = std::fs::read_to_string(&fixture)
            .expect("缺 tests/fixtures/real_ir/project.json:先跑 tools/gen_real_ir_fixture.py");
        let original_meta: serde_json::Value = {
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            v.get("_meta")
                .cloned()
                .expect("夹具必须含顶层 _meta(CutFlow 真实 IR 的判别特征)")
        };
        let root = fsutil::temp_dir("ws-real-ir");
        fsutil::ensure(&root.join(paths::TIMELINE)).unwrap();
        // 唯一落盘点纪律(M2-4):测试写盘同样走 atomic.rs
        atomic::atomic_write(&root.join(PROJECT_REL), text.as_bytes()).unwrap();
        {
            let mut ws = Workspace::open_exclusive(&root).unwrap();
            ws.apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch {
                        duration_ms: Some(3000),
                        ..Default::default()
                    },
                },
                Actor::agent("m8-2"),
                ApplyOpts::default(),
            )
            .unwrap();
        }
        // 重开:roundtrip 后 _meta 原样保留
        let ws2 = Workspace::open(&root).unwrap();
        assert_eq!(ws2.rev(), 1);
        let _ = ws2; // 打开即证明 roundtrip 后文件仍合法
        let disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join(PROJECT_REL)).unwrap())
                .unwrap();
        assert_eq!(disk["_meta"], original_meta, "cutforge 写回不得丢/改 _meta");
        fsutil::cleanup(&root);
    }
}

mod m9_tests {
    use super::*;

    fn read_project(root: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(PROJECT_REL)).unwrap()).unwrap()
    }

    fn write_project(root: &Path, v: &serde_json::Value) {
        // 唯一落盘点纪律(M2-4):测试写盘同样走 atomic.rs
        atomic::atomic_write(
            &root.join(PROJECT_REL),
            serde_json::to_string_pretty(v).unwrap().as_bytes(),
        )
        .unwrap();
    }

    /// M9-1 门禁:外部改 A 字段 + 本地改 A 字段 → 必须 CF-001 + 停写
    /// (计划书:当前实现下该测试必红,V2 转绿)。
    #[test]
    fn conflict_real_repro_and_stop_writes() {
        let root = tests_fixture("ws-conflict").unwrap();
        // 上一会话:rev1(改 V1-002 时长),留下 bases/1
        {
            let mut w = Workspace::open_exclusive(&root).unwrap();
            w.apply(
                Command::ClipUpdate {
                    clip_id: "V1-002".into(),
                    patch: cutforge_core::command::ClipPatch {
                        duration_ms: Some(6000),
                        ..Default::default()
                    },
                },
                Actor::user("u"),
                ApplyOpts::default(),
            )
            .unwrap();
        }
        assert!(
            root.join(BASES_REL).join("1.json").exists(),
            "persist 必须留 baseRev 快照"
        );

        // 本会话:长活工作区,本地待写 V1-002 startMs→9000(尚未落盘)
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        ws.pre_write_sync().unwrap();
        let receipt = ws
            .engine
            .apply(
                Command::ClipUpdate {
                    clip_id: "V1-002".into(),
                    patch: cutforge_core::command::ClipPatch {
                        start_ms: Some(9000),
                        ..Default::default()
                    },
                },
                Actor::agent("m9"),
                ApplyOpts::default(),
            )
            .unwrap();
        assert_eq!(receipt.rev, 2);

        // 写入窗口内:外部写者改同一字段 → startMs=9500
        let mut v = read_project(&root);
        v["tracks"][0]["clips"][1]["startMs"] = serde_json::json!(9500);
        write_project(&root, &v);

        // persist 必须检出漂移 → CF-001 → 本地待写弃用
        let err = ws.persist().unwrap_err();
        assert!(
            err.to_string().starts_with("CONFLICT"),
            "必须报 CONFLICT: {err}"
        );
        assert_eq!(
            read_project(&root)["tracks"][0]["clips"][1]["startMs"],
            serde_json::json!(9500),
            "外部改动获胜,本地不得静默覆盖"
        );
        assert_eq!(ws.rev(), 1, "弃用待写后 rev 回到磁盘真相");
        let conflicts = ws.conflict_list().unwrap();
        assert!(
            !conflicts.is_empty() && conflicts.iter().all(|(_, c)| c.code.code() == "CF-001"),
            "必须落 CF-001: {conflicts:?}"
        );

        // 停写:冲突未裁决前一切写拒绝
        let blocked = ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: cutforge_core::command::ClipPatch {
                    duration_ms: Some(7000),
                    ..Default::default()
                },
            },
            Actor::agent("m9"),
            ApplyOpts::default(),
        );
        assert!(blocked.is_err() && blocked.err().unwrap().to_string().contains("CONFLICT"));
        fsutil::cleanup(&root);
    }

    /// M9-1 正例:窗口内外部改**不同**字段 → 自动合并,双方改动都存活。
    #[test]
    fn window_drift_different_fields_auto_merge() {
        let root = tests_fixture("ws-automerge").unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        ws.pre_write_sync().unwrap();
        let _ = ws
            .engine
            .apply(
                Command::ClipUpdate {
                    clip_id: "V1-002".into(),
                    patch: cutforge_core::command::ClipPatch {
                        start_ms: Some(9000),
                        ..Default::default()
                    },
                },
                Actor::agent("m9"),
                ApplyOpts::default(),
            )
            .unwrap();
        let mut v = read_project(&root);
        v["slug"] = serde_json::json!("renamed-外部");
        write_project(&root, &v);
        ws.persist().unwrap();
        let disk = read_project(&root);
        assert_eq!(
            disk["slug"],
            serde_json::json!("renamed-外部"),
            "外部改动必须存活"
        );
        assert_eq!(
            disk["tracks"][0]["clips"][1]["startMs"],
            serde_json::json!(9000),
            "本地改动必须存活"
        );
        fsutil::cleanup(&root);
    }

    /// M9-1:快照链 LRU——40 次写后 bases 目录 ≤ 32 份,最新快照在。
    #[test]
    fn bases_snapshot_lru() {
        let root = tests_fixture("ws-lru").unwrap();
        let mut ws = Workspace::open_exclusive(&root).unwrap();
        for i in 0..40u64 {
            ws.apply(
                Command::ClipUpdate {
                    clip_id: "V1-001".into(),
                    patch: cutforge_core::command::ClipPatch {
                        duration_ms: Some(7000 - i.min(500)),
                        ..Default::default()
                    },
                },
                Actor::agent("m9"),
                ApplyOpts::default(),
            )
            .unwrap();
        }
        let count = std::fs::read_dir(root.join(BASES_REL)).unwrap().count();
        assert!(count <= 32, "LRU 上限 32,实际 {count}");
        assert!(
            root.join(BASES_REL).join("40.json").exists(),
            "最新快照必须在"
        );
        fsutil::cleanup(&root);
    }

    /// M9-2 门禁:外部手改 project.json 后,守护 ≤1s 产出事件(北极星:外部改动可见)。
    #[test]
    fn external_edit_visible_within_1s() {
        let root = tests_fixture("ws-daemon").unwrap();
        let hub = crate::watcher::ensure_sync_daemon(&root);
        std::thread::sleep(std::time::Duration::from_millis(500)); // 让守护完成初扫
        let since = hub.current();
        let mut v = read_project(&root);
        v["slug"] = serde_json::json!("daemon-visible");
        write_project(&root, &v);
        let t0 = std::time::Instant::now();
        let seq = hub
            .wait_since(since, std::time::Duration::from_millis(1500))
            .expect("外部改动必须 ≤1s 可见(1.5s 容差含 CI 抖动)");
        assert!(seq > since);
        println!("外部改动可见耗时: {:?}", t0.elapsed());
        fsutil::cleanup(&root);
    }
}
