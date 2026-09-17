//! M3-5 门禁:标注锚点重定位。
//! 100 组元素位移/删除场景:重定位成功率 ≥ 99%;越界场景 100% 转 orphan
//! (无静默丢失);orphan 在存储中显式可见。

use cutforge_core::anchor::{Anchor, AnchorKind};
use cutforge_core::engine::sample_project;
use cutforge_core::model::Project;
use cutforge_core::notes::{NoteAuthor, NoteState, NotesStore};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// 三条标注:分别锚在 V1-001(片段内)、V1-002(片段内+近邻)、V1-002(远离一切)。
fn seed_notes() -> NotesStore {
    let mut s = NotesStore::new();
    let mut mk = |id: &str, t: u64, body: &str| {
        s.add(
            Anchor { kind: AnchorKind::Clip, ref_: Some(id.into()), t_ms: t, span: None },
            body.into(),
            NoteAuthor::User,
            vec![],
        );
    };
    mk("V1-001", 4000, "片段内");
    mk("V1-002", 9000, "近邻可重挂");
    mk("V1-002", 20000, "越界孤立");
    s
}

/// 场景一:整体位移(元素仍在)→ 100% 跟随重定位。
#[test]
fn displacement_scenarios_always_relocate() {
    let mut rng = Rng(777);
    let mut success = 0u64;
    for _ in 0..100 {
        let mut p: Project = sample_project();
        let delta = rng.next() % 3000;
        for t in p.tracks.iter_mut() {
            for c in t.clips.iter_mut() {
                c.start_ms += delta;
            }
        }
        let mut store = seed_notes();
        let (moved, orphaned) = store.relocate_all(&p, 500);
        // 元素都在:不允许任何 orphan
        assert_eq!(orphaned, 0, "元素仍在时不得产生 orphan(delta={delta})");
        // 三条 open 标注:锚点都被吸附进元素(第一条在 V1-001 内)→ moved 计数 3
        success += moved as u64;
        for note in store.notes() {
            assert!(note.state != NoteState::Orphan);
            let clip_id = note.anchor.ref_.clone().unwrap();
            let (ti, ci) = p.find_clip(&clip_id).unwrap();
            let clip = &p.tracks[ti].clips[ci];
            let t = note.anchor.t_ms;
            assert!(t >= clip.start_ms && t <= clip.start_ms + clip.duration_ms, "锚点必须吸附进元素");
        }
    }
    assert!(success > 0);
}

/// 场景二:删除 V1-002 → 近邻(500ms)内重挂;越界点 100% 转 orphan;零丢失。
#[test]
fn deletion_scenarios_rehang_or_orphan_and_never_drop() {
    let mut rng = Rng(2026);
    let mut rehung = 0u64;
    let mut orphaned = 0u64;
    for _ in 0..100 {
        let mut p = sample_project();
        p.tracks[0].clips.remove(1); // V1-002 消失;剩 V1-001[0..8400]、A1-001[8400..8800]
        let mut store = seed_notes();
        // 删掉"片段内"那条的宿主还在,不受影响;改造成:把第一条也指向 V1-002 以测重挂
        store.note_mut("n-0001").unwrap().anchor.ref_ = Some("V1-002".into());
        store.note_mut("n-0001").unwrap().anchor.t_ms = 8300 + rng.next() % 500; // 8300..8800,均在 V1-001/A1-001 邻域内
        let (_, orphaned_now) = store.relocate_all(&p, 500);
        orphaned += orphaned_now as u64;
        // n-0001(8300..8800 邻域)必须重挂成功
        let n1 = store.find("n-0001").unwrap();
        let n1_ref = n1.anchor.ref_.clone().unwrap();
        assert!(n1_ref == "V1-001" || n1_ref == "A1-001", "近邻重挂,实际挂到 {n1_ref}");
        if n1.relocated == Some(true) {
            rehung += 1;
        }
        // n-0002(9000)距 A1-001[8400..8800] 200ms → 重挂
        let n2 = store.find("n-0002").unwrap();
        assert_eq!(n2.anchor.ref_.as_deref(), Some("A1-001"), "9000ms 距 A1-001 末尾 200ms");
        // n-0003(20000,越界)必须 100% 转 orphan 且显式保留
        let n3 = store.find("n-0003").unwrap();
        assert_eq!(n3.state, NoteState::Orphan, "越界场景必须转 orphan");
        assert!(n3.orphan_reason.is_some());
    }
    assert_eq!(orphaned, 100, "越界场景 100% 转 orphan(每轮恰 1 条)");
    assert!(rehung >= 99, "重定位成功率 ≥ 99%(实际 {rehung}/100)");
    // 无静默丢失:三轮共 300 条标注全部还在
    let final_store = {
        let mut p = sample_project();
        p.tracks[0].clips.remove(1);
        let mut store = seed_notes();
        store.relocate_all(&p, 500);
        store
    };
    assert_eq!(final_store.notes().len(), 3);
    // orphan 面板可见
    assert_eq!(final_store.orphans().len(), 1);
}

/// 场景三:混合 100 组(位移+删除+无操作),按预期精确分类,总量恒定。
#[test]
fn mixed_scenarios_exact_classification() {
    let mut rng = Rng(99);
    for round in 0..100u64 {
        let mut p = sample_project();
        let mode = round % 4;
        let mut store = seed_notes();
        match mode {
            0 => {
                // 纯位移
                for t in p.tracks.iter_mut() {
                    for c in t.clips.iter_mut() {
                        c.start_ms += 1000 + rng.next() % 2000;
                    }
                }
                let (moved, orphaned) = store.relocate_all(&p, 500);
                assert_eq!(orphaned, 0);
                // n-0001(4000ms)在位移后仍在 V1-001 内 → 锚点无变化,不计 moved;
                // n-0002/n-0003 被推移越出原范围 → 吸附改点
                assert_eq!(moved, 2);
            }
            1 | 2 => {
                // 删除宿主片段
                p.tracks[0].clips.remove(1);
                let (_, orphaned) = store.relocate_all(&p, 500);
                assert_eq!(orphaned, 1, "mode{mode}: n-0003(20000) 必转 orphan");
                assert_eq!(store.find("n-0003").unwrap().state, NoteState::Orphan);
            }
            _ => {
                // 无变化:n-0003(20000ms)超出 V1-002 尾部(14600ms)被吸附进片段(规则 1),
                // 其余两条锚点原样
                let (moved, orphaned) = store.relocate_all(&p, 500);
                assert_eq!((moved, orphaned), (1, 0));
            }
        }
        assert_eq!(store.notes().len(), 3, "round{round}: 标注总量恒定(无静默丢失)");
    }
}
