//! 标书导入 CLI（P1-4）：docx → Markdown + 图片资产。
//!
//! 用法：`bid_import --input <old.docx> --out <new.md> [--assets <dir>]`
//!
//! - `--assets` 缺省 = `new.md` 同目录的 `assets/`（自动创建）。
//! - 转换走 `office::bid::import_docx::import_docx`（样式映射见该模块文档 / `docs/ops/bid-import.md`）。
//!
//! 输出约定（供脚本消费）：
//! - 成功：一行 JSON 摘要 `{"path":…,"assets":…,"chapters":…,"headings":…,"paragraphs":…,"tables":…,"images":…}`
//!   + `IMPORT OK`，exit 0
//! - 参数/读盘/解析/写盘错误：`ERROR <原因>`，exit 2

use std::path::{Path, PathBuf};
use std::process::exit;

use qidiwork_gui::office::bid::import_docx::import_docx;

const USAGE: &str = "用法: bid_import --input <old.docx> --out <new.md> [--assets <dir>]";

/// 参数 / 读盘 / 解析 / 写盘错误
const EXIT_ERROR: i32 = 2;

fn main() {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(v) => v,
        Err(msg) => error(&msg),
    };
    run(&parsed.input, &parsed.out, parsed.assets.as_deref());
}

/// 解析结果：`assets` 为 None 时按 out 同目录的 `assets/` 兜底
struct Args {
    input: PathBuf,
    out: PathBuf,
    assets: Option<PathBuf>,
}

/// `ERROR <原因>` + exit 2
fn error(msg: &str) -> ! {
    eprintln!("ERROR {msg}");
    exit(EXIT_ERROR)
}

fn flag_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("缺少 {flag} 的值\n{USAGE}"))
}

/// `--input` / `--out` 必填，`--assets` 可选，任意顺序；未知参数直接报错
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut assets: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = Some(flag_value(&mut args, "--input")?),
            "--out" => out = Some(flag_value(&mut args, "--out")?),
            "--assets" => assets = Some(flag_value(&mut args, "--assets")?),
            other => return Err(format!("未知参数 {other}\n{USAGE}")),
        }
    }
    let input = input.ok_or_else(|| format!("缺少 --input\n{USAGE}"))?;
    let out = out.ok_or_else(|| format!("缺少 --out\n{USAGE}"))?;
    Ok(Args { input, out, assets })
}

/// 相对路径补成基于 CWD 的绝对路径（不 canonicalize：目标可能尚不存在）
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|e| format!("取当前目录失败: {e}"))
}

fn run(input: &Path, out: &Path, assets: Option<&Path>) {
    let input_abs = match absolute(input) {
        Ok(p) => p,
        Err(e) => error(&e),
    };
    let out_abs = match absolute(out) {
        Ok(p) => p,
        Err(e) => error(&e),
    };
    // 缺省 assets = md 同目录 / assets/
    let assets_abs = match assets {
        Some(dir) => match absolute(dir) {
            Ok(p) => p,
            Err(e) => error(&e),
        },
        None => out_abs
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("assets"),
    };

    let (md, stats) = match import_docx(&input_abs, &assets_abs) {
        Ok(v) => v,
        Err(e) => error(&format!("导入失败: {e}")),
    };

    if let Some(parent) = out_abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error(&format!("创建输出目录失败 {}: {e}", parent.display()));
        }
    }
    if let Err(e) = std::fs::write(&out_abs, md.as_bytes()) {
        error(&format!("写 Markdown 失败 {}: {e}", out_abs.display()));
    }

    let summary = serde_json::json!({
        "path": out_abs.to_string_lossy(),
        "assets": assets_abs.to_string_lossy(),
        "chapters": stats.chapters,
        "headings": stats.headings,
        "paragraphs": stats.paragraphs,
        "tables": stats.tables,
        "images": stats.images,
        "bytes": md.len(),
    });
    match serde_json::to_string(&summary) {
        Ok(line) => println!("{line}"),
        Err(e) => error(&format!("摘要序列化失败: {e}")),
    }
    println!("IMPORT OK");
    exit(0)
}
