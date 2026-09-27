# ADR-0009:HTTP 层继续纯 std 手工加固,不引入 tiny_http/tokio/axum

日期:2026-09-27 · 状态:已采纳(2026-09-27 册一 T1.6 迭代落地)
关联:册一迭代计划 T1.6/AC-1.5/AC-1.6/AC-1.7 · 风险表 D-A1/A1-R2 · ADR-0006(可独立起步) · FLOW.md「M4/M5 若接 notify crate 须先补 ADR」同一纪律

## 背景

T1.6 要对 HTTP 层做三件事:静态托管从 4 条硬编码路径升级为 `/assets/*` 目录化、`/events` 从长轮询升级为 SSE、补齐连接纪律(读超时/体积上限/慢连接隔离)。动工前的决策点 D-A1:这些升级是在现有 `std::net::TcpStream` 手写栈上继续加固,还是引入极薄依赖(如 `tiny_http`)乃至框架(tokio/axum)?

现状事实:cutforge-mcp 的 HTTP 面是单机单用户本地服务(仅 127.0.0.1 + Bearer token),每连接一线程,读超时 + Connection: close 已存在;并发模型是"一条挂死连接只挂死它自己的线程",不存在 C10K 问题。全仓当前**零 async 运行时、零 Web 框架**(workspace 依赖仅 serde/serde_json 等序列化件),这是 C4 最小依赖纪律的刻意结果,不是欠账。

## 决策

1. **HTTP 层继续纯 std 手工加固**:`std::net` + 线程 + `std::sync` 完成静态目录化、SSE、连接纪律;`Cargo.toml` 零新增依赖。
2. 加固项收敛为明确数值并落测试:请求头/体读超时、请求体大小上限、总请求时限、SSE 心跳与流寿命、显式 `Connection: close`(唯一例外:SSE 流式连接);由 `crates/cutforge-mcp/tests/http_hardening.rs` 断言"一条挂死连接不阻塞其余请求、超时后连接被回收"(AC-1.7)。
3. **册七若 Editor API 需要 WebSocket,届时再走新 ADR 论证**引入面(候选只评估 `tungstenite` 一类极薄库;tokio 生态仍默认否决)。SSE 不构成引入依赖的理由——SSE 是"服务端单向逐块写 + 客户端断连感知",`std::io::Write` 原生覆盖。

## 取舍

- 手写栈要自己承担 HTTP 语义的正确性(chunked 编解码、多值头、100-continue 等都不做,仅支持 Content-Length 请求体)。换来:审计面 = 一个 ~300 行的读请求函数,无需信任框架的升级与 CVE 节奏;release 二进制不因 HTTP 框架膨胀(单二进制分发是 E1-2 的发布形态)。
- 单机单用户场景下,框架的收益(高并发、中间件生态)全部用不上;而它要求 async 运行时,会把"零 async"纪律从根上破坏(线程模型 → 任务模型的全仓传染)。

## 被否决的替代

- **tiny_http**:否决——虽是"极薄依赖",但引入后要重接现有 token 鉴权、`/media` Range、SSE 流式写出(它对升级响应的支持有限),迁移本身就有行为漂移风险(A1-R1 同款风险);省下的代码量不足以抵消新增的信任面与版本跟进义务。
- **tokio + axum/hyper**:否决——为单机单用户引入异步运行时是数量级错配;且全仓其余 crate(stdio 通道、渲染编排、脚本宿主)全是同步代码,只为一个辅通道引入运行时违反 C4 纪律。
- **标准库以外的 SSE/websocket 库(如 tungstenite 现在就进)**:否决——册七再议(见决策 3);现在的问题域(单向推送)SSE + 手写即可覆盖。

## 后果

- 连接纪律的每个数值(超时/上限)都是本仓代码里的具名常量,行为可测试、可审计;代价是 HTTP 语义空白(如 chunked 请求)要靠文档明示"不支持"。
- A1-R2 的双实现漂移风险同样适用本决策:SSE 与长轮询并存期,长轮询路径标注"兼容旧壳,册二完成后移除";册二收尾时删除降级面,单实现纪律恢复。
- 若册三+ 出现真实的多客户端/远程访问需求,本 ADR 需要重新开庭(届时瓶颈才有数据)。

## 落地证据

- `crates/cutforge-mcp/src/transport/http.rs`:共享 `read_request`(读超时/体积上限/总时限具名常量)与连接纪律注释;`transport/static_files.rs`:`/assets/*` 目录映射 + canonicalize 穿越防护 + MIME 表 + ETag/304(gzip/br 压缩显式不做,记录于代码注释);`transport/events.rs`:SSE 事件 Hub(复用 cutforge-io watcher,只扩发布面),长轮询降级路径原样保留。
- `crates/cutforge-mcp/tests/http_hardening.rs`:挂死连接隔离 + 回收 + 413 上限断言。
- `tools/e2e_static.py`、`tools/e2e_events.py`:AC-1.6 / AC-1.5 的可判定判据。
