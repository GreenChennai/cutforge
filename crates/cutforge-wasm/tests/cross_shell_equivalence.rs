//! M5-1 门禁:跨壳工程等价。
//! 同一工程文件:原生壳投影(Workspace + Query)与 wasm ABI 投影(forge_* 字符串入口)
//! 的视图 diff = 0。wasm 出口与 native 出口编译自同一批纯函数,此测试在 native
//! 侧锁定两者行为一致;wasm32 目标的构建产物由 M5-2(wasm-pack)单独验证。

use cutforge_core::engine::{Answer, Query};
use cutforge_io::Workspace;

fn canonical(v: &serde_json::Value) -> String {
    cutforge_core::engine::canonical_json(v)
}

#[test]
fn cross_shell_equivalence() {
    for kind in ["talking-head", "talking-head+animation", "pure-animation"] {
        let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tests/regression/{kind}/project.json"));
        let text = std::fs::read_to_string(&sample).expect("样本存在");

        // ---- Web 壳路径:wasm ABI(字符串进出) ----
        let web_view: serde_json::Value =
            serde_json::from_str(&cutforge_wasm::forge_open(&text).expect("forge_open")).unwrap();
        let web_tl: serde_json::Value =
            serde_json::from_str(&cutforge_wasm::forge_timeline(&text).expect("forge_timeline")).unwrap();

        // ---- 桌面/CLI 壳路径:native 投影 ----
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let project = cutforge_core::model::migrate_from_value(&v).expect("迁移+校验");
        let native_view =
            cutforge_wasm::answer_to_value(&Answer::Project(serde_json::to_value(&project).unwrap()));

        assert_eq!(
            canonical(&native_view["value"]),
            canonical(&web_view["project"]),
            "全量视图跨壳不一致: {kind}"
        );

        let engine = cutforge_core::engine::Engine::new(project).unwrap();
        let native_tl = cutforge_wasm::answer_to_value(&engine.query(Query::Timeline));
        assert_eq!(
            canonical(&native_tl["rows"]),
            canonical(&web_tl["clips"]),
            "时间线投影跨壳不一致: {kind}"
        );
        let _ = Workspace::open; // 契约可见性
    }
}

#[test]
fn notes_projection_equal() {
    let reg = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/regression/talking-head");
    let notes_text = std::fs::read_to_string(reg.join("notes.json")).unwrap();
    let web: serde_json::Value =
        serde_json::from_str(&cutforge_wasm::forge_notes(&notes_text).unwrap()).unwrap();
    let store = cutforge_core::notes::NotesStore::from_value(
        &serde_json::from_str(&notes_text).unwrap(),
    )
    .unwrap();
    assert_eq!(web["counts"]["total"], serde_json::json!(store.notes().len()));
    assert_eq!(
        canonical(&web["orphans"]),
        canonical(&serde_json::json!(
            store.orphans().iter().map(|n| n.id.clone()).collect::<Vec<_>>()
        )),
    );
}
