//! 页数实测 CLI（M1）：`bid_pages <docx>`。
//!
//! 用本机 WPS（COM `KWPS.Application`）只读打开 docx、`Repaginate` 后取**实测**页数/字数
//! （`office::bid::pages::measure`），不估算、不保存、不改动原文件。
//!
//! 输出约定（供脚本消费）：
//! - 成功：一行 JSON `{"pages":N,"words":M}` + `PAGES OK`，exit 0
//! - 参数/文件/测量失败：`ERROR <原因>`（stderr），exit 2
//! - 非 Windows：`ERROR 页数测量仅支持 Windows（需本机 WPS）`，exit 2

use std::path::PathBuf;
use std::process::exit;

use qidiwork_gui::office::bid::pages;

const USAGE: &str = "用法: bid_pages <docx>";

/// 参数 / 读盘 / 测量错误
const EXIT_ERROR: i32 = 2;

fn main() {
    let mut args = std::env::args().skip(1);
    let docx = match (args.next(), args.next()) {
        (Some(p), None) => PathBuf::from(p),
        (None, _) => error(&format!("缺少 <docx>\n{USAGE}")),
        (Some(_), Some(extra)) => error(&format!("多余参数 {extra}\n{USAGE}")),
    };
    match pages::measure(&docx) {
        Ok(stat) => {
            println!(
                "{}",
                serde_json::json!({ "pages": stat.pages, "words": stat.words })
            );
            println!("PAGES OK");
            exit(0)
        }
        Err(e) => error(&e.0),
    }
}

/// `ERROR <原因>` + exit 2
fn error(msg: &str) -> ! {
    eprintln!("ERROR {msg}");
    exit(EXIT_ERROR)
}
