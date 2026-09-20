# ADR-0008:对 jianying-headless 只学其方法与能力,不复制代码(许可约束,跨仓)

日期:2026-09-20 · 状态:已确认(2026-09-20 用户拍板:只学其方法与能力,具体代码不抄)
关联:同伴仓 CutFlow `docs/adr/0044-jianying-headless只学方法与能力不复制代码.md`(同一决策的 CutFlow 侧落点,内容同源) · NOTICE.md 第四节

## 背景

jianying-headless 项目证明了"程序化生成剪映草稿"这条路可行,其工程方法(计划编译分层、protect 保护区、RMS 辅助选切点、剪后重转写对账、独立副本编辑、帧数严格门禁、版本/签名/哈希核对、诚实验收文档)对本项目有直接参考价值。但其许可为 **Personal Learning and Non-Commercial Use**:禁止 republish / mirror / package / market 派生作品,商业使用需作者书面授权;且其仅支持 Apple Silicon macOS + 剪映 11.5,与本项目(Windows + 剪映 5.9)不同源——直接抄代码既不合法,也不兼容。

## 决策

1. **学习其方法与能力,一律自行实现、自定字段名**;**不复制其源码,不照搬其专有字段与结构**。
2. **剪映出口统一编排 CutFlow 侧 `rs_jy_draft.py`**(pyJianYingDraft / MIT,5.9 明文草稿):CutForge 的 `export_jianying` 是编排方(scriptArgs 原样透传),不引入 jianying-headless 的任何代码、字段或依赖,也不在 Rust 侧另造一套草稿语义。
3. **合规边界写进 NOTICE 与诚实验收文档**:借鉴其方法落地时,在验收文档中显式标注"方法来自其纪律,实现与字段为本项目自有"。
4. 若确需更深借鉴(超出方法层面的引用),先按其许可条款取得作者书面授权,未取得前不做。

## 取舍

- 放弃"直接移植一套成熟实现"的短期便利。
- 换来:合法合规(规避派生作品禁止条款)、跨平台可行(macOS-only 前提不适用本项目)、草稿语义单点(仍归 CutFlow 编译层),不制造 Rust/Python 双实现漂移面。

## 被否决的替代

- **移植其代码**:否决——许可不符(Personal Learning and Non-Commercial)且平台不符(macOS/剪映 11.5 vs Windows/剪映 5.9)。
- **照搬其字段名与草稿结构**:否决——仍属派生,且会污染五份 schema 的唯一契约真相源。
- **在 Rust 侧自研第二套草稿编译**:否决——与"编排同一 rs_jy_draft.py"的对拍纪律冲突,双实现必然漂移。

## 后果

- 剪映相关能力演进继续由 CutFlow 侧编译层驱动,CutForge 只编排与对拍;两仓对拍(同一 IR 经编排调用与直跑 CLI 得到逐键相同的草稿计划)成为防漂移硬闸。
- 本决策为跨仓决策,两仓 ADR 互相引用;后续任何对 jianying-headless 的新引用必须先过第 4 条。

## 落地证据

- CutForge 侧:`export_jianying` 编排同一 `rs_jy_draft.py`、scriptArgs 原样透传;对拍断言(源断言 + schema 断言,同一 IR 经编排调用形态与直跑 CLI 得到逐键相同的草稿计划)在 `tests/test_jy_bridge.py`。
- CutFlow 侧合规自查机制(阶段六):`skills/cutflow/rules/jianying-verification.md`——方法声明行明文标注"只学方法,不抄内容/字段/代码";「已验证 / 未验证 / 拒绝」三分;「拒绝」区列明不做项(剪映 11.3+ 加密草稿、原生无头导出等);草稿编译层为自行实现,字段口径自定义。
- Rust 依赖树无 jianying-headless 任何成分:workspace 各 crate(`crates/`)与 `Cargo.toml` 不含对它的引用;剪映相关第三方依赖仅 pyJianYingDraft(MIT),声明见 `NOTICE.md` 第四节。
