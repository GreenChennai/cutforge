# CONTRIBUTING

感谢关注 CutForge。开始之前必须读清楚本项目的定位与上游关系。

## 与上游 OpenCut 的关系

- [OpenCut](https://github.com/OpenCut-app/OpenCut)（MIT）正在**活跃重写**中，其 README 明确说明当前架构设计期**暂不接受外部贡献**（原文："We're not set up to take outside contributions yet while the architecture is being designed."）。
- CutForge 是**独立实现**：我们借鉴其经典版 [opencut-classic](https://github.com/OpenCut-app/opencut-classic)（MIT，2026-05-17 归档）的**领域模型与交互设计**，未复制任何源代码文件。两仓的 MIT 许可全文逐字保留于 `LICENSE-OPENCUT.MIT`。
- CutForge **不是** OpenCut 的官方版本、分支或衍生发行版，双方无隶属或背书关系。
- 我们不与上游竞争，也不承诺同步其功能；接口形状按其公开路线图（Editor API / MCP server / headless / scripting）设计，保留未来兼容的可能。

## 接受什么贡献

项目早期（M0–M2 阶段）优先接受：

- 门禁失败报告与复现步骤（用 `python tools/gates/gate.py M<n> --json` 的完整输出）；
- 文档与术语修正（含错别字、口径不一致）；
- schema 契约的讨论意见（先开 issue 讨论，再动实现）。

功能类 PR 请先开 issue 对齐设计再动手，避免与路线图冲突。

## 工程约定（违反即被拒）

1. **结果协议**：所有脚本/命令输出 `{"ok":bool,"code":str,"message":str,"data":object}`，支持 `--json`。
2. **退出码**：`0`=通过；`2`=门禁失败；`3`=前置或环境缺失；`4`=内部错误。**不得把"环境缺失"报成 0。**
3. **完成判定**：每个里程碑唯一入口 `python tools/gates/gate.py M<n> --json`；人眼判断不作为通过依据；**不得为让门禁通过而放宽阈值**。
4. **契约优先**：任何新增/变更字段，先改 `schemas/`，再改实现；没有 schema 支撑的字段一律视为不存在。
5. **编码**：所有文件 UTF-8（无 BOM）；Python 必须 `from __future__ import annotations` 且纯标准库优先；含中文路径的 shell 命令必须加引号。
6. **内核纯净**：`cutforge-core` 不碰文件系统、不调 ffmpeg、不认识剪映；所有写操作必须经命令通道（Op），不存在旁路。
7. **合规红线**：不得修改 `LICENSE-OPENCUT.MIT`；不得以 `OpenCut`/`opencut` 命名任何产品、仓库、包、域名；不得复制上游源代码（只读参考策略见仓库 `docs/` 的计划书附录 C）。

## 门禁工程纪律（审查报告 v2 §8.2，违反即被拒）

1. **宣称即证据**：任何「门禁全绿 / 巨石清零 / 已支持」的结论，必须附**可复现命令 + 退出码 + 关键输出**；无证据的宣称一律视为未完成，写回待办。不得宣称未实测通过的门禁为通过（G-2 教训：A8 曾宣称「巨石清零」而 crates/ 实存多个 >800 行文件，已订正）。
2. **门禁要有反面测试**：每条门禁配一个「注入缺陷应红灯」的负例，否则门禁自身腐化无人知。现含 `TC-GATE-001/002/003`，统一入口：

   ```bash
   python tools/gates/negative_tests.py --json   # 退出码 0 = 被测门禁均能正确红灯
   ```

3. **门禁扫描面显式化**：每个门禁在本文档登记「扫描面 / 排除项 / 覆盖目录」，`apps/` 与 `crates/` 分开列，杜绝「以为扫了其实没扫」：

   | 门禁 | 扫描面 | 排除项 | 口径与分级 |
   |---|---|---|---|
   | `rust-line-limit`（A1） | `crates/**/*.rs`（非测试） | `tests/` 目录、`target/` | ≤800 行，**阻断**（现存超限清单见 A8 订正段，拆分进行中） |
   | `desktop-line-limit`（A1，G-1） | `apps/desktop/src/**/*.rs` | `ui/` 目录 | ≤600 行（桌面壳独立口径），**报告模式**；A-02 拆分落地后转阻断 |
   | `js-line-limit`（A2–A7） | `apps/web/**/*.js` + `index.html` | `min.*` 产物 | js ≤400 / html ≤120，**阻断** |
   | `desktop-box-leak`（A1，TC-GATE-002） | `apps/desktop/src/**/*.rs` | `ui/` 目录 | `Box::leak` = 0，**报告模式**；BUG-18 清零后转阻断 |
   | `desktop-color-purity`（A1，TC-GATE-003） | `apps/desktop/src/**/*.rs` | `ui/`（未来 `ui/theme.rs` 为唯一定义点） | 裸色值（hsl/rgb/#RRGGBB）= 0，**报告模式**；A-08 落地后转阻断 |
   | `check_changelog`（M2，G-4） | PR diff：`crates/*/src/**` + `apps/*/src/**` ↔ `CHANGELOG.md` | — | **警告级**（CI 传 PR base sha）；Unreleased 约定跑满一个迭代且无误报后转红线 |
   | `check-write-paths`（M2，已进 CI） | 全仓写入收口 `atomic.rs` | — | 旁路写入 = 0，**阻断** |
   | `check_license`（M0，G-5/BUG-15） | `LICENSE` / `LICENSE-OPENCUT.MIT` / `NOTICE.md` / README 措辞 / `CORE-FILES` | — | 三件套要素 + 清单条目存在性，**阻断**（反面测试 TC-GATE-001） |

## 门禁与验收

提交前在本地跑通对应里程碑门禁：

```bash
python tools/gates/gate.py M0 --json   # 通过则退出码 0
```

CI（`.github/workflows/gate.yml`）在 push/PR 时跑同一套门禁；Windows 才能跑的检查请本地复跑。
