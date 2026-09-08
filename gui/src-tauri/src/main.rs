// QidiWork GUI 二进制壳:逻辑全部在 lib(qidiwork_gui)。
// `--mock-agent` 分支:传输层测试/手动联调入口,绝不进入 Tauri 装配。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // mock 分支仅存于测试/调试构建;生产 release 无此暴露面。
    #[cfg(any(test, debug_assertions))]
    if args.iter().any(|a| a == "--mock-agent") {
        std::process::exit(qidiwork_gui::mock_agent::run(&args));
    }
    let _ = args;
    std::process::exit(qidiwork_gui::run());
}
