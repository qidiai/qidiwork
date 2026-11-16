// QidiWork GUI 二进制壳:逻辑全部在 lib(qidiwork_gui)。
// `--mock-agent` 分支:传输层测试/手动联调入口,绝不进入 Tauri 装配。
// `--self-check` 分支:安装态模板解析自检,只读打印后退出,同样不进入 Tauri 装配。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use qidiwork_gui::office::bid::paths::{DEFAULT_STYLE_ID, probe_resolve};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // 自检分支:不加 cfg 门控,release 安装态也要能跑(现场核对模板解析)。
    // 位置必须在一切 Tauri 装配之前,避免拉起窗口/单实例锁。
    if args.iter().any(|a| a == "--self-check") {
        std::process::exit(self_check());
    }
    // mock 分支仅存于测试/调试构建;生产 release 无此暴露面。
    #[cfg(any(test, debug_assertions))]
    if args.iter().any(|a| a == "--mock-agent") {
        std::process::exit(qidiwork_gui::mock_agent::run(&args));
    }
    let _ = args;
    std::process::exit(qidiwork_gui::run());
}

/// 安装态/开发态模板解析自检:逐行打印关键事实,成功返回 0、解析失败返回 1。
///
/// 注意:Tauri NSIS 安装包把 `resources/` 平铺到 exe 同级目录,故
/// `app.path().resource_dir()`（需 AppHandle 才能取）与"exe 所在目录"等价;
/// 自检阶段没有 AppHandle,直接用 exe 目录拼出同一候选路径验证该等价关系。
fn self_check() -> i32 {
    println!("VERSION={}", env!("CARGO_PKG_VERSION"));

    let exe = std::env::current_exe().ok();
    let exe_dir: Option<PathBuf> = exe.as_deref().and_then(Path::parent).map(Path::to_path_buf);
    match exe_dir.as_deref() {
        Some(dir) => println!("EXE_DIR={}", dir.display()),
        None => println!("EXE_DIR=<unknown>"),
    }

    let file_name = format!("{DEFAULT_STYLE_ID}.docx");

    // 安装态候选(等价 resource_dir()/templates/<id>.docx)
    match exe_dir.as_deref() {
        Some(dir) => {
            let candidate = dir.join("templates").join(&file_name);
            println!("CAND_RESOURCE={}", candidate.display());
            println!("EXISTS_RESOURCE={}", candidate.is_file());
        }
        None => {
            println!("CAND_RESOURCE=<unknown>");
            println!("EXISTS_RESOURCE=false");
        }
    }

    // 开发态候选(CARGO_MANIFEST_DIR/resources/templates/<id>.docx)
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("templates")
        .join(&file_name);
    println!("CAND_DEV={}", dev.display());
    println!("EXISTS_DEV={}", dev.is_file());

    match probe_resolve(DEFAULT_STYLE_ID, exe_dir.as_deref()) {
        Ok(path) => {
            println!("RESOLVED={}", path.display());
            0
        }
        Err(err) => {
            println!("RESOLVED_ERR={err}");
            1
        }
    }
}
