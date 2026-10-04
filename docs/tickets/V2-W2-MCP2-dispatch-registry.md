# V2-W2-MCP2 dispatch 注册表化(A-01)

范围:crates/cutforge-mcp/src/dispatch.rs(1135 行)拆分 + 注册表化;tests 与金样随之。

目标接口(报告 §7 A-01):
```rust
pub trait CommandHandler {
    fn name(&self) -> &'static str;
    fn schema(&self) -> &'static str;   // JSON Schema 片段,自动汇入文档
    fn handle(&self, args: Value, eng: &mut Engine) -> Result<Value, EngineError>;
}
```
- 分派/schema 校验/文档生成/tool_parity 覆盖清单全部从注册表派生;
- 新增命令 = 一个文件 + 注册一行;现有命令分支行为**逐字节零变化**(tool_parity 对拍为证);
- 免锁工具/AI 预演两个小 match 一并收编;
- TC-MCP-DISPATCH-001:注册表内每个 handler 的 schema 能校验其 golden 参数样例;
- mcp→render 依赖边(基线违规)在本轮一并消解或给出最小豁免方案(报告论证);
- 约束:dispatch.rs 拆后单文件 ≤800 行(报告 G-2 清单消项);tool_parity 0 DRIFT;96 测试全绿不回退。
状态:待第 1 波集成后开工
