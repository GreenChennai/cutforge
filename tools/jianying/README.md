# tools/jianying/ — 剪映草稿导出随包资产(册六 T6.2/ADR-0023)

`export_jianying` 编排工具的**随包脚本资产**。定位序(orchestrate.rs `resolve_script`):
env `CUTFLOW_REPO`(显式,调试/对拍)→ 工程内 → **本目录(随包)** → CutFlow 仓库回退
(一个版本期)。来源经编排响应 `scriptSource=bundled` 如实标注。

## 归属声明(provenance)

| 文件 | 来源 | 归属 |
|---|---|---|
| `rs_jy_draft.py` | CutFlow/skills/cutflow/scripts/(整文件收编) | CutForge 权利人(与 CutFlow 同主体;算法与实现归属保留) |
| `rs_paths.py` / `rs_common.py` / `rs_codes.py` / `segmentation.py` | 同上(依赖闭包整收,运行时不再依赖 CutFlow 仓库在位) | 同上 |
| `templates/jy59_empty_draft.json` | CutFlow/skills/cutflow/templates/(随包自包含) | 同上 |
| `vendor/pyJianYingDraft/` | 上游 vendored(含 LICENSE;MIT) | 上游原作者;MIT 许可随目录分发 |

ADR-0008 的「CutFlow 仓库边界」由 ADR-0023 显式修订:本目录作为 CutForge 随包独立
资产分发(开发树在 `tools/jianying/`;安装器落点 `<exe>/scripts/`,T6.4 收编)。

### 收编适配补钉(仅路径解析,算法零变化;CutFlow 侧重收编时须重放)

1. `rs_common.py`:`CONFIG_PATH` 支持 env `CUTFLOW_CONFIG` 显式指定(随包部署时
   config.json 不在 `parents[3]`;未设环境时与仓内布局同径);
2. `rs_jy_draft.py`:工程根三态派生 `_project_root`(v2/v1 project.json 在阶段目录
   → `parent.parent`;v3 扁平在根 → `parent`,ADR-0021)+ 模板路径改同目录
   `templates/`(随包自包含,不再 `parents[1]`)。

## 诚实标注

- 本导出面**仍依赖 Python 运行时**(`py -3`/`python3`/`python`),不冒充零依赖能力;
- **config.json(机器自备)**:剪映 exe / 草稿根 / root_meta 均为个人机器路径,不入库
  也不随包——env `CUTFLOW_CONFIG` 指向(或随包根放置)`config.json` 且含 `jianying59`
  段才可写草稿;缺 config 时 `--dry-run`(纯编译映射表)仍可用,如实 NO_CONFIG 不虚标;
- 草稿落点随 CutFlow 口径 = 工程区 `05_时间线工程/导出/剪映59/<名>/`(ADR-0052);
  v3 扁平工程当前也会创建该阶段目录落草稿(半成品单向出口,非交付物),v3 化挂册七
  headless 化一并评估;
- 单向出口:剪映侧精修不回流工程区;只写剪映 5.9 明文草稿,11.3+ 加密草稿永不读写;
  写前检测剪映进程(运行中即拒)。

## 依赖闭包(收编清单)

`rs_jy_draft.py` → `rs_paths.py`(阶段路径唯一真相源,ADR-0046)+ `rs_common.py`
(die/emit/load_config + pymediainfo→ffprobe shim)→ `segmentation.py`(标点口径,纯标准库)
+ `rs_codes.py`(退出码,纯标准库)+ `vendor/pyJianYingDraft`(MIT)。除此闭包外零
仓库外依赖;同步纪律:CutFlow 侧算法演进时**整文件重收编**(不做手工分叉)。
