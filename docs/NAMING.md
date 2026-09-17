# NAMING · CutForge 命名落位结论与核查记录

> 依据：《CutForge × CutFlow 融合迭代计划书 v1.0》M0-1。
> 本文件是命名落位的**唯一权威记录**，含实测输出存档。
> 核查日期：2026-09-17（在线实测，非引用计划书旧数据）。

## 一、核查结论（落位表）

| 项 | 取值 | 核查结果 |
|---|---|---|
| 产品名 | **CutForge**（保留，接受同名仓库搜索劣势，换取 crates.io 全空的反向优势） | 语义拥挤已正视，见第四节 |
| GitHub 组织 | `cutforge-app`（新建） | FREE（HTTP 404） |
| 主仓库名 | `cutforge`（`cutforge-app/cutforge`，**非 Fork**） | 组织下空顺利落位 |
| Crate 前缀 | `cutforge` / `cutforge-core` / `cutforge-mcp` / `cutforge-schema` | 全部 FREE（HTTP 404） |
| npm 作用域 | **`@cutforge-app/*`** | FREE（HTTP 404） |
| npm 无作用域包 | 不使用（`cutforge` 虽 FREE，统一走作用域） | — |
| 域名 | `cutforge.app` **待人工 WHOIS 核查**（脚本无法完成）；不可用则备选 `cutforge.dev` / `cutforgehq.com` | 待办 |
| 商标 | 不做正式注册；仅在 `NOTICE.md` 声明名称主张 | 决策 |

## 二、实测输出存档（2026-09-17）

### crates.io（API `GET /api/v1/crates/<name>`，HTTP 404 = FREE）

```
crates.io cutforge:        HTTP 404 FREE
crates.io cutforge-core:   HTTP 404 FREE
crates.io cutforge-mcp:    HTTP 404 FREE
crates.io cutforge-schema: HTTP 404 FREE
search q=cutforge  →  {"crates":[],"meta":{"total":0}}
```

### npm（registry.npmjs.org，HTTP 404 = FREE）

```
npm cutforge:            HTTP 404 FREE
npm @cutforge/editor:    HTTP 200 TAKEN   ← 作用域 @cutforge 归他人所有，禁用
npm @cutforge-app/web:   HTTP 404 FREE
```

### GitHub 用户/组织（api.github.com/users/<name>，HTTP 404 = FREE）

```
gh user cutforge:     HTTP 200 TAKEN   ← 被普通用户占用，不可用
gh user cutforge-app: HTTP 404 FREE    ← 采用
gh user cutforgehq:   HTTP 404 FREE    ← 备选
gh user cutforgedev:  HTTP 404 FREE    ← 备选
```

### GitHub 同名仓库（`search/repositories?q=cutforge`，total_count=13，均 0–1 星）

```
reqayasa/cutforge            stars=0   （cutting stock optimization tool，非视频域）
Acorx/cutforge               stars=0   （Tauri+React 视频编辑器，同语义域）
cutforge/cutforge            stars=0
swapna989-ctrl/cutforge      stars=0
abarakus11/CutForge          stars=0
OQI24/cutforge               stars=0
qwert702/cutforge            stars=0   （agent-first AI 视频编辑器，同语义域）
Roitto-Design-Works/cutforge-dist stars=0
t2828w29zr-sketch/cutforge-mobile stars=0
nguyenminhduc9988/cutforge-studio stars=1 （含 MCP+CLI 的视频合成工具，同语义域）
```

> 同语义域 3 个：`Acorx/cutforge`、`qwert702/cutforge`、`nguyenminhduc9988/cutforge-studio`。

## 三、二选一决策记录

计划书 M0-1 要求明确二选一：

- **选择：① 保留 `CutForge`**。
- 理由：crates.io 与 npm 作用域全空（包发布零阻力）；GitHub 组织 `cutforge-app` 可用；语义拥挤仅影响搜索发现，不构成法律障碍（无商标注册、同名仓库均 0–1 星）。
- 缓解：README 首段一句话定位 + NOTICE.md 独立性声明。
- 若未来改为改名，必须按本文件第二节命令重跑全量核查并更新本文件。

## 四、红线关联

- 不得以 `OpenCut` / `opencut` 作为产品名、仓库名、包名、域名（计划书红线 4）。
- `CutForge` 名称主张写进 `NOTICE.md`；本文件不构成商标注册。

## 五、核查命令（可复现）

```bash
UA="cutforge-availability-check/1.0"
# crates.io 精确名
for c in cutforge cutforge-core cutforge-mcp cutforge-schema; do
  curl -s -o /dev/null -w "%{http_code}" -H "User-Agent: $UA" "https://crates.io/api/v1/crates/$c"
done
# npm 精确与作用域
curl -s -o /dev/null -w "%{http_code}" "https://registry.npmjs.org/cutforge"
curl -s -o /dev/null -w "%{http_code}" "https://registry.npmjs.org/@cutforge%2Feditor"
curl -s -o /dev/null -w "%{http_code}" "https://registry.npmjs.org/@cutforge-app%2Fweb"
# GitHub 用户
for u in cutforge cutforge-app cutforgehq cutforgedev; do
  curl -s -o /dev/null -w "%{http_code}" -H "User-Agent: $UA" "https://api.github.com/users/$u"
done
# GitHub 同名仓库
curl -s -H "User-Agent: $UA" "https://api.github.com/search/repositories?q=cutforge&per_page=10"
```
