//! tracing 日志初始化(方案 v2 M1 审计登记决策:tracing 家族,与
//! M3 将引入的 cf-acp-lib 输出统一采集)。
//!
//! 输出:Tauri app_log_dir 下的按日滚动文件;`RUST_LOG` 覆盖级别,
//! 默认 `info`。测试环境无 subscriber 时 tracing 宏自动 no-op。

use std::path::PathBuf;

pub fn init(app_log_dir: Option<PathBuf>) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    match app_log_dir {
        Some(dir) => {
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("日志目录创建失败({e}),回退控制台输出: {dir:?}");
                init_console(filter);
                return;
            }
            let appender = tracing_appender::rolling::daily(dir, "qidiwork-gui.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            // guard 必须活到进程结束: leaking 是 non_blocking appender 的
            // 官方推荐用法(否则日志线程随 guard drop 而停)。
            std::boxed::Box::leak(Box::new(guard));
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(writer)
                .with_ansi(false)
                .init();
            tracing::info!("文件日志已启用");
        }
        None => init_console(filter),
    }
}

fn init_console(filter: tracing_subscriber::EnvFilter) {
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .init();
}
