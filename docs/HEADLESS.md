# HEADLESS · 无 UI 全链指南(批处理 / watch / CI 自动出片)

> 册七 T7.4。载体:`cutforge-cli` 的 `batch` / `watch` 子命令 + `tools/ci-example/`
> 用户样例 + `tools/e2e_headless.py` 全链验收(AC-7.4)。全部纯 CLI:不依赖编辑器壳,
> 渲染走 cutforge-render 直渲(与 serve 内嵌通道同一实现,不持工程锁)。

## 一、批处理清单:`cutforge-cli batch`

```bash
cutforge-cli batch batch.yaml --report out/batch-report.json
# 可选:--stop-on-fail(默认逐项到底);退出码 0 全成 / 2 有失败 / 3 渲染器缺失
```

清单**纯 JSON 或最小 YAML 子集**两态(YAML 子集纪律:2 空格缩进;`key: value`
冒号后必须有空格;`- ` 列表项;标量 = 裸串/引号串/整数/true/false;`#` 注释;
不支持锚点/多行/流式集合——解析器手写零依赖):

```yaml
# batch.yaml
version: 1
concurrency: 1          # 预留(当前单排队,语义=顺序执行)
stopOnFail: false
report: out/batch-report.json   # 也可用 --report 覆盖
defaults:               # 工程条目缺省渲染参数(条目同名键覆盖)
  format: mp4
  quality: high
projects:
  - root: D:/素材/工程甲
    name: 甲            # 可选(缺省取目录名)
    format: gif         # 与 render/render_run 参数同名(quality/crf/outMs/videoOnly/…)
  - root: D:/素材/工程乙
    ass: subtitles.ass  # 相对工程根;不存在时不烧录(与 render 同口径)
```

**报告 JSON schema 固化**:`docs/schemas/batch-report.schema.json`——逐工程回执
`{index, project, name, ok, code, output?, error?, durationMs}` + 汇总
`summary {total, ok, fail}`。CI 判红读 `summary.fail > 0` 即可;schema 对拍由
`tools/e2e_headless.py` 机械执行(生成物对拍)。

## 二、watch 模式:`cutforge-cli watch`

```bash
cutforge-cli watch D:/素材/工程甲 --ass subtitles.ass --format mp4
# 可选:--debounce-ms 500(轮询防抖窗口);--max-runs N(渲染 N 次后退出,脚本化验收用)
```

- 文件变更检测复用 `cutforge_io::watcher` 轮询器(与常驻同步守护同一忽略规则:
  oplog/成片输出/内部状态簿记不惊动);
- 启动即渲一次,此后每次变更防抖后自动重渲;Ctrl+C 退出(无残留状态);
- 渲染参数与 `render_run` 同名透传。

## 三、CI 自动出片样例:`tools/ci-example/`

给用户抄的最小工作流:

- `tools/ci-example/batch.example.yaml`——三工程批清单样例;
- `tools/ci-example/check_output.py`——产物校验器(时长对拍 ffprobe / 像素抽样非黑帧),
  零第三方依赖(ffmpeg/ffprobe 在 PATH);
- `tools/ci-example/workflow.example.yaml`——GitHub Actions 步骤样例(构建 → batch →
  check_output → 报告上传)。

用法(本地即可跑):

```bash
cutforge-cli batch tools/ci-example/batch.example.yaml --report out/report.json
python tools/ci-example/check_output.py out/report.json --expect-duration-ms 30000 --tolerance-ms 800
```

## 四、AC-7.4 全链验收

```bash
python tools/e2e_headless.py --bin target/debug/cutforge-cli
```

无 UI 全链:现场生成素材 → `new` 三工程 → `clip-update` 改字段 → 渲染 →
时长/像素抽样校验 → batch 三工程排队 → 报告 schema 对拍 → plugin-call 权限面
(越权负例)→ watch 改文件自动重渲一次。
