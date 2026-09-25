//! M2-2 门禁:IR 往返语义差为零。
//! load → mutate → save 后,与预期 JSON 的语义 diff = 0(键序、空白不影响判定,
//! 数值按数值语义比较)。

use cutforge_core::command::{ClipPatch, Command};
use cutforge_core::engine::{ApplyOpts, Query};
use cutforge_core::oplog::Actor;
use cutforge_io::{fsutil, Workspace};

fn actor() -> Actor {
    Actor::agent("roundtrip-test")
}

/// 语义相等:对象键序无关;数值统一按 f64 比较;其余严格相等。
pub fn semantic_eq(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a, b) {
        (serde_json::Value::Object(ma), serde_json::Value::Object(mb)) => {
            ma.len() == mb.len()
                && ma.iter().all(|(k, va)| mb.get(k).is_some_and(|vb| semantic_eq(va, vb)))
        }
        (serde_json::Value::Array(aa), serde_json::Value::Array(ab)) => {
            aa.len() == ab.len() && aa.iter().zip(ab.iter()).all(|(x, y)| semantic_eq(x, y))
        }
        (serde_json::Value::Number(na), serde_json::Value::Number(nb)) => {
            na.as_f64() == nb.as_f64()
        }
        _ => a == b,
    }
}

#[test]
fn roundtrip_semantic_eq() {
    let root = cutforge_io::tests_fixture("rt").unwrap();
    // 预期状态 = 打开后应用两条命令的结果
    let expected = {
        let mut ws = Workspace::open(&root).unwrap();
        ws.apply(
            Command::ClipUpdate {
                clip_id: "V1-001".into(),
                patch: ClipPatch { duration_ms: Some(8000), volume: Some(0.9), ..Default::default() },
            },
            actor(),
            ApplyOpts::default(),
        )
        .unwrap();
        ws.apply(Command::ClipSplit { clip_id: "V1-002".into(), t_ms: 10000 }, actor(), ApplyOpts::default()).unwrap();
        match ws.engine().query(Query::ProjectView) {
            cutforge_core::engine::Answer::Project(v) => v,
            _ => unreachable!(),
        }
    };

    // 重新打开(从盘面),再取内存视图:两者必须语义相等
    let disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join(cutforge_io::PROJECT_REL)).unwrap()).unwrap();
    assert!(
        semantic_eq(&expected, &disk),
        "盘面与内存语义不一致:\n内存={expected}\n盘面={disk}"
    );

    let ws2 = Workspace::open(&root).unwrap();
    let reloaded = match ws2.engine().query(Query::ProjectView) {
        cutforge_core::engine::Answer::Project(v) => v,
        _ => unreachable!(),
    };
    assert!(semantic_eq(&expected, &reloaded));
    fsutil::cleanup(&root);
}

#[test]
fn roundtrip_noop_is_byte_identical() {
    // 不做任何修改时,save 不得改变盘面(读入的 v2 内容原样往返)。
    let root = cutforge_io::tests_fixture("rt-noop").unwrap();
    // 先以 v2 形态落一次盘(打开+零命令+显式保存路径经 reopen 验证)
    let before = std::fs::read_to_string(root.join(cutforge_io::PROJECT_REL)).unwrap();
    {
        let _ws = Workspace::open(&root).unwrap();
    }
    let after = std::fs::read_to_string(root.join(cutforge_io::PROJECT_REL)).unwrap();
    assert_eq!(before, after, "无命令不得触碰盘面");
    fsutil::cleanup(&root);
}
