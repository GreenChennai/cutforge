//! CLI 二进制入口:全部逻辑在库中(便于 e2e 集成测试直接调用 `cutforge_cli::run`)。

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cutforge_cli::run(argv));
}
