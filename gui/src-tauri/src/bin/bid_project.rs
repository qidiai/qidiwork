//! 标书项目工作区 CLI（P1-2 / 规格 §A.3）。
//!
//! 用法：
//! - `bid_project new <dir> --title "某县…施工组织设计"`
//! - `bid_project build <dir> [-o out.docx] [--no-number]`
//! - `bid_project check <dir>`
//!
//! `new`：建 `bid.project.json` + `chapters/01-概述.md` + `assets/`；清单已存在 → `ERROR 项目已存在`。
//! `build`：`project::build_request` 合并章节 → `fill::export` 出 docx（默认 `<dir>/out.docx`）
//! → `verify::verify(&out, None)` 24 项结构校验 → 一行 JSON 摘要 + `BUILD OK`。
//! `check`：`project::validate_project` → `PROJECT OK` / `PROJECT FAIL <问题,逗号分隔>`。
//!
//! 输出约定（供脚本消费）：
//! - `new` 成功：一行 JSON（`dir`/`manifest`）+ `NEW OK`，exit 0
//! - `build` 成功：一行 JSON 摘要 + `BUILD OK`，exit 0；结构校验失败 `BUILD FAIL <check 名>`，exit 1
//! - `check`：`PROJECT OK` exit 0 / `PROJECT FAIL …` exit 1
//! - 参数 / 读盘 / 解析 / 导出错误：`ERROR <原因>`，exit 2

use std::path::{Path, PathBuf};
use std::process::exit;

use qidiwork_gui::office::bid::fill;
use qidiwork_gui::office::bid::model::BidExportResult;
use qidiwork_gui::office::bid::project;
use qidiwork_gui::office::bid::verify::{self, VerifyReport};

const USAGE: &str = "用法: bid_project new <dir> --title <书名>\n       \
                     bid_project build <dir> [-o <out.docx>] [--no-number]\n       \
                     bid_project check <dir>";

/// 结构校验 / 项目体检不合格
const EXIT_CHECK_FAIL: i32 = 1;
/// 参数 / 读盘 / 解析 / 导出错误
const EXIT_ERROR: i32 = 2;

/// 解析后的子命令
enum Cmd {
    New { dir: PathBuf, title: String },
    Build { dir: PathBuf, out: Option<PathBuf>, number: bool },
    Check { dir: PathBuf },
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        exit(0);
    }
    match parse_args(args) {
        Ok(Cmd::New { dir, title }) => run_new(&dir, &title),
        Ok(Cmd::Build { dir, out, number }) => run_build(&dir, out.as_deref(), number),
        Ok(Cmd::Check { dir }) => run_check(&dir),
        Err(msg) => error(&msg),
    }
}

/// `ERROR <原因>` + exit 2
fn error(msg: &str) -> ! {
    eprintln!("ERROR {msg}");
    exit(EXIT_ERROR)
}

/// 参数解析：`<子命令> <dir> [flags]`，flag 与目录位置无关
fn parse_args(args: Vec<String>) -> Result<Cmd, String> {
    let mut it = args.into_iter();
    let sub = it.next().ok_or_else(|| format!("缺少子命令\n{USAGE}"))?;

    let mut positional: Vec<String> = Vec::new();
    let mut title: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut no_number = false;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--title" => {
                title = Some(
                    it.next()
                        .ok_or_else(|| format!("缺少 --title 的值\n{USAGE}"))?,
                );
            }
            "-o" | "--out" => {
                let value = it
                    .next()
                    .ok_or_else(|| format!("缺少 {arg} 的值\n{USAGE}"))?;
                out = Some(PathBuf::from(value));
            }
            "--no-number" => no_number = true,
            other if other.starts_with('-') => {
                return Err(format!("未知参数 {other}\n{USAGE}"));
            }
            other => positional.push(other.to_string()),
        }
    }

    let dir = match positional.as_slice() {
        [d] => PathBuf::from(d),
        [] => return Err(format!("缺少项目目录\n{USAGE}")),
        _ => return Err(format!("多余的参数 {}\n{USAGE}", positional[1..].join(" "))),
    };

    match sub.as_str() {
        "new" => {
            if out.is_some() || no_number {
                return Err(format!("new 不接受 -o/--out 与 --no-number\n{USAGE}"));
            }
            let title = title.ok_or_else(|| format!("new 需要 --title\n{USAGE}"))?;
            Ok(Cmd::New { dir, title })
        }
        "build" => {
            if title.is_some() {
                return Err(format!("build 不接受 --title（书名取清单）\n{USAGE}"));
            }
            Ok(Cmd::Build {
                dir,
                out,
                number: !no_number,
            })
        }
        "check" => {
            if title.is_some() || out.is_some() || no_number {
                return Err(format!("check 不接受 --title/-o/--no-number\n{USAGE}"));
            }
            Ok(Cmd::Check { dir })
        }
        other => Err(format!("未知子命令 {other}\n{USAGE}")),
    }
}

/// 默认模板：编译期 crate 目录下的 bid-engineering-blue-v1.docx
fn default_template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/templates/bid-engineering-blue-v1.docx")
}

/// 相对路径补成基于 CWD 的绝对路径（不做 canonicalize：目标文件可能尚不存在）
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|e| format!("取当前目录失败: {e}"))
}

/// 失败 check 名（`,` 分隔；失败时至少一项）
fn failed_names(report: &VerifyReport) -> String {
    report
        .checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.name.as_str())
        .collect::<Vec<&str>>()
        .join(",")
}

/// `new`：建骨架 + 一行 JSON 摘要 + `NEW OK`
fn run_new(dir: &Path, title: &str) -> ! {
    if let Err(e) = project::create_skeleton(dir, title) {
        error(&e.to_string());
    }
    let summary = serde_json::json!({
        "dir": dir.display().to_string(),
        "manifest": dir.join(project::MANIFEST_FILE).display().to_string(),
    });
    println!("{summary}");
    println!("NEW OK");
    exit(0)
}

/// `build`：合并 → 导出 → 24 项校验 → 摘要 + `BUILD OK`
fn run_build(dir: &Path, out: Option<&Path>, number: bool) -> ! {
    let mut req = match project::build_request(dir, number) {
        Ok(r) => r,
        Err(e) => error(&e.to_string()),
    };
    if let Err(e) = req.validate() {
        error(&e.to_string());
    }

    let out_path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| dir.join("out.docx"));
    let out_abs = match absolute(&out_path) {
        Ok(p) => p,
        Err(e) => error(&e),
    };
    if let Some(parent) = out_abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error(&format!("创建输出目录失败 {}: {e}", parent.display()));
        }
    }
    req.output_path = out_abs.to_string_lossy().to_string();

    let (stats, bytes) = match fill::export(&req, &default_template(), &out_abs) {
        Ok(v) => v,
        Err(e) => error(&format!("导出失败: {e}")),
    };
    let report = match verify::verify(&out_abs, None) {
        Ok(r) => r,
        Err(e) => error(&format!("校验失败: {e}")),
    };
    if !report.ok() {
        println!("BUILD FAIL {}", failed_names(&report));
        exit(EXIT_CHECK_FAIL);
    }

    let summary = BidExportResult {
        path: out_abs.to_string_lossy().to_string(),
        bytes,
        chapters: stats.chapters,
        paragraphs: stats.paragraphs,
        tables: stats.tables,
        figures: stats.figures,
    };
    match serde_json::to_string(&summary) {
        Ok(line) => println!("{line}"),
        Err(e) => error(&format!("摘要序列化失败: {e}")),
    }
    println!("BUILD OK");
    exit(0)
}

/// `check`：`PROJECT OK` / `PROJECT FAIL <问题,逗号分隔>`
fn run_check(dir: &Path) -> ! {
    match project::validate_project(dir) {
        Ok(problems) if problems.is_empty() => {
            println!("PROJECT OK");
            exit(0)
        }
        Ok(problems) => {
            println!("PROJECT FAIL {}", problems.join(","));
            exit(EXIT_CHECK_FAIL)
        }
        Err(e) => error(&e.to_string()),
    }
}
