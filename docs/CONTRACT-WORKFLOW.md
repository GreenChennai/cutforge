# CONTRACT-WORKFLOW · 新增一个 IR 字段的标准七步流水线

> 本文是 [FLOW.md](FLOW.md) 第六节「契约与单源体系」的**操作手册化**:给册四/册五大量加字段时
> 逐步照抄的流水线说明书。七步链:schema → 双端生成 → 内核模型 → ClipPatch → ui-fields →
> 壳控件 → 对拍夹具;每步给出**实际可执行命令**(全部以 v0.6 时点实码核实)、改哪里、怎么验证。
> 文中工具数量口径以 `schemas/mcp-tools.json` 实时为准(v0.6 时点:41 工具 = 13 查询 + 21 写 + 7 编排;
> `tools/check_doc_counts.py` 目前扫描 README / FLOW / ACCEPTANCE / capability-matrix 四文件,本文不在此列,
> 但口径纪律相同:写数字必须与 schema 一致,或干脆引用不重抄)。

## 〇、全景图

| 步 | 改哪里 | 产物/消费方 | 验证命令 | 门禁兜底 |
|---|---|---|---|---|
| 1 schema | `schemas/project.schema.json`(+ CutFlow 侧模板同步) | 双端校验器的唯一手写真相源 | `python -m json.tool …`、`python tools/validate_regression.py` | gate M1 `schemas-complete` |
| 2 双端生成 | 无手改(跑生成器 / 重编译) | `tools/_generated/cf_validate.py`;Rust `include_str!` | `python tools/schema_gen.py --check`、`cargo test -p cutforge-schema` | gate M1 `cargo-schema-tests` |
| 3 内核模型 | `crates/cutforge-core/src/model.rs` | IR 的 Rust 序列化形式(load/save 都经它) | `cargo test -p cutforge-core`、`cargo test -p cutforge-io --test roundtrip_semantic_eq` | gate M2 `roundtrip-semantic-eq` |
| 4 ClipPatch | `cutforge-core/src/command.rs` + `cutforge-mcp/src/dispatch.rs` + `schemas/mcp-tools.json` | 写通道(唯一写入口 Workspace::apply) | `cargo test -p cutforge-mcp --test protocol_conformance` | gate M4 `protocol` / `mcp-tools` |
| 5 ui-fields | `schemas/ui-fields.json` | 壳检查器可编辑字段集单一真相源 | `cargo run -q -p cutforge-cli -- check-ui-fields --json` | CLI 判定器(验收项,未入 gate.py) |
| 6 壳控件 | `apps/web/app.js`(FIELD_META*) | 控件形态;字段全集/分组由 `GET /ui-fields` 下发 | `python tools/e2e_edit_ops.py …`、`python tools/e2e_from_zero.py …` | CI web-e2e job |
| 7 对拍夹具 | `tests/regression/`、`crates/cutforge-render/tests/parity_matrix.rs` | 契约回归集 + 实渲对拍 | `python tools/validate_regression.py`、`cargo test --workspace --locked` | gate M0–M1 + CI rust-gates |

**次序不可乱**:3→4 之前先 1→2(schema 是引擎校验依据);5 必须在 4 之后(check-ui-fields 要从
command.rs 实码解析字段);6 依赖 5(壳按 /ui-fields 渲染分组)。

---

## 一、步骤 1:schema(唯一手写真相源)

**在哪改**:`schemas/project.schema.json`。

- 片段级字段 → `$defs.clip.properties`;片段级子对象(如 `fx` 的条目)→ `$defs.fxEntry` 一类公共定义 + `$ref`。
- 工程级字段 → 顶层 `properties`(如 `bgm`、`font`)。
- 所有对象必须 `additionalProperties: false`(幽灵字段显式报错,计划书 3.3 第 7/8 项的既有纪律)。
- 字段 `description` 写清**谁消费、哪个 ADR/分册**(它是双端实现者唯一的上下文来源)。

**双仓同步**:同一字段必须同步到 CutFlow 侧模板
`CutFlow/skills/cutflow/templates/project.schema.json`(该仓独立演进;v2 M14 批次即"与 CutFlow v0.20
ADR-0053~0059 同步",见 `git show c5b509d`)。CutFlow 先行或 CutForge 先行都可,但**同批必须双边落**。

**怎么验证**:

```bash
python -m json.tool schemas/project.schema.json > /dev/null && echo OK   # JSON 语法
python tools/validate_regression.py                                       # 回归集 16/16(要求 100%)
```

**门禁**:`python tools/gates/gate.py M1 --json` 中的 `schemas-complete`(五 schema + 生成物齐备,
Rust `include_str!` 与 Python 生成校验器双向引用)。

---

## 二、步骤 2:双端生成(Python 生成物 + Rust 编译期嵌入)

契约只有一份手写面,消费有两端,**两端机制不同**:

**Python 端(生成器)**——重新生成 `tools/_generated/cf_validate.py`(纯 stdlib 自包含校验器 +
v1→v2 迁移器;文件头 AUTO-GENERATED,禁止手改):

```bash
python tools/schema_gen.py            # 再生成(改完 schema 必跑)
python tools/schema_gen.py --check    # 验证:0=生成物与 schema 一致;2=GEN_STALE(过期)
```

没跑生成器就提交 → `--check` 退出码 2,gate M1 的 `schemas-complete` 当场红。这是漂移的**第一道闸**。

**Rust 端(无生成步骤)**——`crates/cutforge-schema/src/lib.rs` 用 `include_str!` 编译期嵌入
project/wordline/cutlist/notes/oplog 五 schema 及 `mcp-tools.json`、`ui-fields.json`、
`constants.ratios.json`;`crates/cutforge-schema/build.rs` 做存在性闸 + `cargo:rerun-if-changed`。
改 schema 后**重新编译即吸入**,无须任何生成命令:

```bash
cargo build -p cutforge-schema        # 重编译(rerun-if-changed 触发)
cargo test -p cutforge-schema         # 双端对拍 + 迁移幂等(dual_validation 等 4 测试)
```

`cargo test -p cutforge-schema`(`crates/cutforge-schema/tests/dual_validation.rs`)是**双端一致性闸**:

- `dual_validation_equivalence`:同一批回归样本,Rust 引擎与 Python 生成校验器结论逐样本一致(含负例注入双端同拒);
- `migrate_py_rust_semantic_eq` / `migrate_idempotent`:双端迁移器输出语义相等、迁移幂等。

**常量(仅当动 fps/canvas 等允许集)**——唯一真相源在 CutFlow(`rs_common.RATIOS` + `platforms.json`),
生成物 `schemas/constants.ratios.json` 勿手改:

```bash
python tools/gen_constants.py --check   # 零漂移;不等退出码 2 并点名键差异
```

**门禁**:gate M1 = `schemas-complete` + `regression-schema` + `constants-no-drift` + `cargo-schema-tests`。

---

## 三、步骤 3:内核模型(cutforge-core/model)

**在哪改**:`crates/cutforge-core/src/model.rs`。

- 结构体加 `Option<T>` 字段,配 `#[serde(default, skip_serializing_if = "Option::is_none")]`;
  camelCase 由结构体上的 `#[serde(rename_all = "camelCase")]` 统一承担,键名无须手写。
- 枚举约束(schema `enum`)放 schema 层,模型层从宽收 `Option<String>`——现状 `Motion`/`Transition` 均如此,
  枚举外的值由 engine 落盘前的 schema 校验拒绝(SCHEMA_INVALID)。

**本步的铁律——"不加载 = 必丢"**:serde 默认**静默忽略**未知字段。schema 声明了某字段而模型没接,
该字段会在 CutForge 的"读→写"一轮后从 project.json 消失——schema 校验照样绿(校验的是已丢字段后的
模型),门禁全无所觉。所以 **schema 加字段,模型必须同批承接**;实在暂不承接,须在 PR/文档显式声明
"此为 CutFlow 单边字段,CutForge 写会丢弃"(worked example 一节有实案)。

**怎么验证**:

```bash
cargo test -p cutforge-core                                    # 模型/引擎单测
cargo test -p cutforge-io --test roundtrip_semantic_eq         # M2-2:load→mutate→save 语义 diff=0
```

外加一步**手搓往返实验**(roundtrip 夹具不含你的新字段时,这是唯一能抓"静默丢弃"的手段):

```bash
./target/debug/cutforge-cli.exe new "%TEMP%/cf-exp" --slug exp       # 空工程
# 手工往 05_时间线工程/project.json 注入新字段 → 一笔 clip-update → 回读确认字段仍在
./target/debug/cutforge-cli.exe clip-update "%TEMP%/cf-exp" V1-001 --volume 0.8
```

**门禁**:gate M2 `roundtrip-semantic-eq`(注意:它测的是夹具内字段,新字段要加进夹具才能被它保护,见步骤 7)。

---

## 四、步骤 4:ClipPatch 与命令通道(含 MCP 契约)

**在哪改**(三处同 commit):

1. `crates/cutforge-core/src/command.rs`:
   - 片段级标量 → `struct ClipPatch` 加 `Option` 字段(字段级 Op 的 before/after 由此派生);
   - 片段级嵌套对象 → 独立子 patch(如 `TransitionPatch`/`MotionPatch`),`ClipPatch` 持 `Option<子patch>`,
     内部再按字段合并(None 不改);
   - **工程级字段不经 ClipPatch** → 独立 patch + 独立 `Command`(先例:`BgmPatch` + `Command::BgmSet/BgmClear`,
     应用点在 `BgmPatch::apply_to`,指针 `/bgm/…`)。
2. `crates/cutforge-mcp/src/dispatch.rs`:为写工具加参数抽取(`p["新字段"].as_xxx()`);新 Command 分支接到
   `ws.apply(...)`。**唯一写入口**是 Workspace::apply 八步(FLOW.md 5.1),不得旁路写文件。
3. `schemas/mcp-tools.json`:对应工具的 `inputSchema.properties` 声明新参数。加字段**不动 tools 数组**
   (工具数口径不变);真要加新工具,须同步 `_doc` 口径句、`cutforge-mcp` 注册表、以及
   `protocol_conformance.rs` 里的 `41 / 13 / 21 / 7` 断言——三处一处不落,M4 门禁红。

**怎么验证**:

```bash
cargo test -p cutforge-core                              # patch 合并/is_empty 单测
cargo test -p cutforge-mcp --test protocol_conformance   # M4-5:envelope/码表/注册表↔契约↔dispatch 三方对拍
```

`protocol_conformance` 会用空参探针逐一证明"已注册即已实现"、枚举外值被 schema 层拒
(先例:transition_set 传 `"type":"爆闪"` → SCHEMA_INVALID)。

**门禁**:gate M4 `protocol`、`mcp-tools`(注册表与 mcp-tools.json 逐名一致 + 双通道工具集恒等)、
`doc-tool-counts`(见第六节)。

---

## 五、步骤 5:ui-fields(检查器单一真相源)

**在哪改**:`schemas/ui-fields.json`。

- `editable`:壳检查器**允许编辑**的字段,按分组(分组名即 UI legend);新字段加进对应分组。
- `readonly`:投影里有、ClipPatch 尚未承接的字段 → 检查器只读展示(E4-3),等内核承接后再移入 editable。
- 嵌套对象只写对象名(先例:`"转场": ["transition"]`),子字段控件由壳的 FIELD_META_V2 展开(步骤 6)。
- **工程级字段不进本文件**:ui-fields 只管片段检查器;BGM 等工程级面板由壳直连 `bgm_set`。

**机械校验(单一真相源的牙齿)**——`cutforge-cli check-ui-fields` 从 `command.rs` **实码**解析
`struct ClipPatch` 字段集,要求 ui-fields editable ⊆ ClipPatch,防"内核支持 N、壳只给 3"的漂移复发:

```bash
cargo run -q -p cutforge-cli -- check-ui-fields --json
# OK:ui-fields 合规:N 个可编辑字段全部被 ClipPatch 支撑
# 违规 → UI_FIELDS_VIOLATION,逐字段点名"编辑会成幻觉"
```

**门禁现状(诚实说明)**:该判定器是 CLI 子命令 + 阶段验收项,**未注册进 gate.py / CI**
(V2 里程碑不新增 gate.py 注册,见 FLOW.md 7.1);改 ui-fields 的 PR 必须手工跑上面这条命令。

---

## 六、步骤 6:壳控件(apps/web 消费 ui-fields)

**机制(壳纯度)**:壳**不读** `schemas/ui-fields.json` 文件——它由 `cutforge-mcp` 编译期嵌入
(`crates/cutforge-mcp/src/registry.rs` 的 `UI_FIELDS_JSON`),经 `GET /ui-fields` 下发
(`workspace_svc.rs`;e2e 断言无 token 401 / 带 token 200)。壳只负责**控件形态**,
字段全集与分组一律以下发为准:

- `FIELD_META`(`apps/web/app.js`):标量字段的输入形态(number 的 step/min/max、text);
  **纯标量新字段壳可零改动**——落到未知字段时自动用 text 输入框,但要下拉/范围/人话 label 就得补条目;
- `FIELD_META_V2`:枚举下拉与嵌套字段(先例:`transition.type` 下拉、`motion.inMs` 数字);
  写路径 `collectNestedPatches` 把带点字段收拢为 `patch.transition` / `patch.motion` 发给 `clip_update`。

**在哪改**:`apps/web/app.js`(FIELD_META / FIELD_META_V2 / collectNestedPatches)+
`apps/web/index.html`(面板骨架)。改完 **必须重编 cutforge-mcp**(ui-fields 是编译期嵌入):

```bash
cargo build -p cutforge-mcp -p cutforge-cli --locked
```

**怎么验证(e2e,CI web-e2e job 同款命令)**:

```bash
python tools/e2e_edit_ops.py  --bin target/debug/cutforge-mcp                       # M10 三段:分割/检查器/撤销
python tools/e2e_from_zero.py --bin target/debug/cutforge-mcp --cli target/debug/cutforge-cli
# 从零剪:新建→导入→/ui-fields 鉴权与分组断言→改字段→导出
```

**门禁**:CI `web-e2e` job(gate.yml);壳不持真相由 gate M5 `shell-purity` 兜底
(`cargo run -q -p cutforge-cli -- check-shell-purity --json`)。

---

## 七、步骤 7:对拍夹具(契约回归 + 渲染对拍)

**契约回归集**:`tests/regression/<videoType 三类>/`(talking-head / pure-animation / talking-head+animation,
各含 project / wordline / cutlist / notes / oplog 五件)。新字段改变合法形态时补样本:

```bash
python tools/validate_regression.py     # 100% 零豁免;gate M1 regression-schema 同命令
```

**渲染语义字段**(影响画面/声音的字段)加两道:

```bash
cargo test -p cutforge-render --test parity_matrix    # 九项实渲对拍夹具(渲染语义锁)
python tools/parity_check.py                          # 本地:与 CutFlow rs_render 逐项对拍(需 ffmpeg+CutFlow 仓;缓存 .gate/parity.json)
```

**真实 IR 夹具**(CutFlow rs_ir 真管线产物,防"夹具全是构造样"):

```bash
python tools/gen_real_ir_fixture.py          # 生成 tests/fixtures/real_ir/project.json(含 _meta)
python tools/gen_real_ir_fixture.py --check  # CI rust-gates 同款只读校验
```

**全量收口**(提交前本地跑,CI 同款):

```bash
cargo test --workspace --locked
python tools/gates/gate.py M0 --json
python tools/gates/gate.py M1 --json
```

**门禁**:M0/M1 入 CI;M2–M6 与 e2e 为本地阻断项(CI 缺依赖如实报退出码 3,不误报通过,见 FLOW.md 七节)。

---

## 八、Worked example:`bgm.assetId`(解剖 c5b509d)

`git show c5b509d`("v2 M14 双仓同步")一次带入 9 个契约面:clip.fx/huazi/font/assetId、
motion.inFx/outFx、transition.fx、bgm.assetId、fxEntry、顶层 font/effects。以 `bgm.assetId`
(字符串,指向素材库 manifest.json 的稳定 id,CutFlow 分册01/ADR-0053)逐步对照:

| 步 | 状态 | 实录 |
|---|---|---|
| 1 schema | ✅ | `schemas/project.schema.json` 的 `bgm` 对象加 `assetId`(type string,description 写明 `bgm.set --assetId / asset.swap` 消费);CutFlow 侧模板同字段已同步(实测 `CutFlow/skills/cutflow/templates/project.schema.json` 的 bgm.properties 含 assetId) |
| 2 双端生成 | ✅ | 同提交再生成 `tools/_generated/cf_validate.py`(diff 仅指纹 2 行);Rust 侧 include_str! 无须生成,重编译即吸入 |
| 3 内核模型 | ❌ **断** | `model.rs` 的 `struct Bgm { src, gain_db, ducking, loop_ }` **无 asset_id**。实测:临时工程 project.json 注入 `bgm.assetId` 与顶层 `font`,跑一笔 `cutforge-cli clip-update` 后**两字段从盘面消失**(serde 静默丢未知字段;schema 校验的是丢弃后的模型,照样绿) |
| 4 ClipPatch | ❌ 断 | `BgmPatch` 无 assetId;`dispatch.rs` 的 `bgm_set` 只抽 src/gainDb/ducking/loop;`mcp-tools.json` 的 bgm_set inputSchema 同样无。即 CutForge 侧没有任何写通道能设置该字段 |
| 5 ui-fields | — | 不适用(工程级字段不经 ui-fields) |
| 6 壳控件 | — | 壳 BGM 面板四控件(src/音量/闪避/循环)与 BgmPatch 对齐,无 assetId 入口 |
| 7 对拍夹具 | ✅(只护到 schema 层) | 回归集校验过、cf_validate 一致——但双端对拍只比对"校验结论",不测"模型是否携带",故步骤 3/4 的断点门禁全绿 |

**解剖结论**:`bgm.assetId` 现状是 **CutFlow 单边字段**——schema 层双仓已同步,渲染/换素材由 CutFlow 消费;
CutForge 读写一轮会把它**静默抹掉**。册二/册五若要在 CutForge 壳里换 BGM 素材,须按流水线补
步骤 3(`Bgm.asset_id`)→ 4(`BgmPatch.asset_id` + bgm_set 参数 + mcp-tools.json)→ 6(BGM 面板控件),
并在步骤 7 把带 `assetId` 的样本纳入 roundtrip/回归夹具——现在的夹具不含它,门禁测不出。
同类字段(c5b509d 批次)处境相同,册五动"特效/花字/字体"前先补内核承接。

---

## 九、常见错误与门禁拦截对照

| # | 错误 | 后果 | 谁拦 |
|---|---|---|---|
| 1 | 只改 schema,忘跑 `schema_gen.py` | cf_validate 过期 | `schema_gen.py --check` → GEN_STALE(2);gate M1 `schemas-complete` 红 |
| 2 | schema 加了字段,模型没接 | CutForge 读写一轮**静默丢弃**该字段 | **现无门禁**(schema 校验的是丢弃后的模型)。防御:步骤 3 手搓往返实验 + 把字段纳入回归/roundtrip 夹具 |
| 3 | ui-fields 声明了 ClipPatch 没有的字段 | 壳可编辑、内核不认,编辑成幻觉 | `cutforge-cli check-ui-fields` → UI_FIELDS_VIOLATION(逐字段点名;未入 gate.py,须手工跑) |
| 4 | dispatch 收了参数,mcp-tools.json 没声明(或反之) | 契约面与实现漂移 | **protocol_conformance 只对工具名与 schema 存在性,不逐属性比对**——纪律:两处同 commit 同步 |
| 5 | 加/删工具后 `_doc` 口径或文档数字漂移 | 文档口径漂移 | `python tools/check_doc_counts.py` → [DRIFT] 逐条列 文件:行(gate M4 `doc-tool-counts`;README/FLOW/ACCEPTANCE/capability-matrix 受扫,历史叙述须带"时点/当时/新增") |
| 6 | 手改 `constants.ratios.json` | 常量漂移 | `gen_constants.py --check` → CONST_DRIFT(2);gate M1 `constants-no-drift` |
| 7 | 手改 `tools/_generated/cf_validate.py` | 生成物被覆盖/漂移 | `schema_gen.py --check` 逐字节比对,不等即 GEN_STALE |
| 8 | 壳直接读 schemas/ 文件 | 破坏壳纯度 | gate M5 `shell-purity`;壳只许 `GET /ui-fields` |
| 9 | 新枚举值只改模型不改 schema | 写盘前被拒 | engine 落盘前 schema 校验 → SCHEMA_INVALID(engine 步骤 3) |
| 10 | patch 塞了 ClipPatch 没有的键 | serde 静默忽略,同值短路"看似成功" | **现无门禁**;纪律:mcp-tools.json inputSchema 声明面 = ClipPatch 字段面,同 commit 改 |
| 11 | 渲染语义字段只加 schema 不加 parity 夹具 | 双端渲染漂移测不出 | `parity_matrix.rs` 九项只锁已有语义;新字段须扩夹具(人工纪律) |

## 十、速查表

```bash
# ---- 步骤 1/2 schema 与生成 ----
python -m json.tool schemas/project.schema.json > /dev/null && echo OK
python tools/schema_gen.py && python tools/schema_gen.py --check
python tools/gen_constants.py --check
# ---- 步骤 3 内核模型 ----
cargo test -p cutforge-core
cargo test -p cutforge-io --test roundtrip_semantic_eq
# ---- 步骤 4 命令通道 ----
cargo test -p cutforge-mcp --test protocol_conformance
# ---- 步骤 5 ui-fields ----
cargo run -q -p cutforge-cli -- check-ui-fields --json
# ---- 步骤 6 壳 ----
cargo build -p cutforge-mcp -p cutforge-cli --locked
python tools/e2e_edit_ops.py  --bin target/debug/cutforge-mcp
python tools/e2e_from_zero.py --bin target/debug/cutforge-mcp --cli target/debug/cutforge-cli
# ---- 步骤 7 对拍与收口 ----
python tools/validate_regression.py
cargo test -p cutforge-render --test parity_matrix
python tools/gen_real_ir_fixture.py --check
cargo test --workspace --locked
python tools/gates/gate.py M0 --json && python tools/gates/gate.py M1 --json
# ---- 口径对拍(改了工具集/文档数字后必跑)----
python tools/check_doc_counts.py
```

> 维护纪律:本文描述的是**机制**而非数量;数量(工具数/字段集/分组)以
> `schemas/mcp-tools.json`、`schemas/ui-fields.json` 实时为准。机制变化(如生成器换址、
> 门禁注册)才改本文。
