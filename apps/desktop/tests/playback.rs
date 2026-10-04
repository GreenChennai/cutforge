//! playback 模块测试挂载(I1-S1,docs/tickets/I1-S1)。
//!
//! main.rs 属接线方文件域(工单禁改);为让 `cargo test -p cutforge-desktop`
//! **真实编译并运行** playback 模块(含各文件 `#[cfg(test)]` 单测与 decoder
//! 集成测试),测试目标经 `#[path]` 直挂 `src/playback/mod.rs`。
//!
//! I1-S2 接线(main.rs 增 `mod playback;`)后本文件无需改动:模块被编译两次
//! (bin 一次、test 一次),行为等价、互不干扰;若届时模块迁入 lib 目标,
//! 可改写为 `use cutforge_desktop::playback;`。
#![allow(dead_code)] // 门面 API 在接线前未被测试全部引用,静默 dead_code

#[path = "../src/playback/mod.rs"]
mod playback;
