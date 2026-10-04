# V2-W1-MCP 安全与服务质量

范围:crates/cutforge-mcp/**(http/workspace_svc/resident/progress/plugin/dispatch 增分支)+ BUG-17 内核侧。

条目:BUG-10(Bearer 精确+恒定时间)、S-04(头解析规范化)、S-01(URL token→一次性会话凭据)、
BUG-11(SSE root pct_decode)、S-02(连接上限 64+503)、R-08(/media 流式+上限)、
R-09(resident 锁 per-project)、R-11(渲染队列落盘 .cutforge/render-queue.jsonl)、
R-14(幂等 HashSet 索引)、BUG-17 内核侧(render_frame 响应显式 framePath,旧扫描保留一版)、
BUG-19(media_thumbs 批量工具:mcp-tools.json/dispatch/media_tools/金样再生成/README 计数 84→85)、
S-03(插件强制层:Unix ulimit 包裹+声明层确认+违规禁用;Windows 留待,CI 为 ubuntu)。
报告文件:C:\Users\Administrator\Desktop\CutForge-迭代审查报告-v2.md。

状态:已完成(MCP 侧全绿;tool_parity 对拍被并行内核轮在途改动阻塞,见下验收注记)

## 验收注记(2026-10-05)
- 门禁:cargo test -p cutforge-mcp -j 2 = EXIT 0(96 测试);cargo clippy -p cutforge-mcp -p cutforge-render -j 2 -- -D warnings = EXIT 0;cargo build -p cutforge-render -p cutforge-cli -j 2 = EXIT 0。
- 金样:本工单唯一预期 delta = render_frame.json 增 framePath 键(已落);media_thumbs 不在 parity 序列内(tools/ 非本工单文件域),契约由 mcp-tools.json + TC-MCP-THUMB-001 锁定。
- tool_parity 对拍暂为 exit 2:确定性失败 text_add(serve 线程 panic 于 cutforge-core/src/engine/apply.rs:66「轨道 startMs 升序不变量」debug_assert,内核轮 BUG-05/A-03 在途代码)与 render_run 首调(同源级联),连带 23 个 DRIFT(op 序列移位)。已用保留 serve stderr 的驱动脚本取证。内核轮稳定并再生成金样后,本工单 golden delta 与对拍自然收敛。
- R-14 落在 MCP 层(常驻 HashSet 索引 + O(1) 预检);内核 has_request_id 全扫仍是正确性底线(core 不在本工单文件域)。
- S-03 强制层落在 cutforge-mcp::plugin 公开 API(check_filesystem_access/authorize_call/disable_plugin/process_spawn_command);cli plugin-call 拉起点的接线属 cli 文件域,后续轮换用 authorize_call 即可。
- R-09 并发缺陷联合验收修复(2026-10-05):根因 = 摘键式逐出(evict/_clear_for_tests 把 cell 摘出 map,在途借用的旧 Arc 与新建 cell 双锁并存 → 同工程真并发;原全局锁实现"查+插+用"同锁故无此窗口)。修法 = **cell 身份恒定**:Entry API 原子 get-or-insert + cell 入 map 永不摘除,逐出一律逻辑逐出(置 Option=None);强化测试 RESIDENT-002(8 线程×50 笔 barrier 冷启动并发 miss,max=1)与新增 RESIDENT-003(4 工程×3 线程混合负载,同工程 max=1 + 全局峰值≥2 双断言);resident 三连绿(--test-threads=8)、全 lib 8 测试线程两连绿(65/65)、cargo test -p cutforge-mcp EXIT=0(97)、clippy --all-targets EXIT=0。
