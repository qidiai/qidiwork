//! 标书导出 CLI（P0c 收尾）。
//!
//! 用法：
//! - `bid_export --input <req.json> --out <out.docx> [--template <tpl.docx>]`
//! - `bid_export --check-only <docx>`
//!
//! 导出路径：读请求 JSON → `office::bid::model::BidExportRequest`
//! （`output_path` 由 `--out` 覆盖；`Figure.path` 相对路径按请求文件所在目录解析），
//! 调 `office::bid::fill::export` 产出 docx，再调 `office::bid::verify::verify(&out, None)`
//! 做 24 项结构校验。
//!
//! 输出约定（供脚本消费）：
//! - 成功：一行 JSON 摘要 `{"path":…,"bytes":…,"chapters":…,"paragraphs":…,"tables":…,"figures":…}`
//!   + `EXPORT OK`，exit 0
//! - 结构校验失败：`CHECK FAIL <失败 check 名,逗号分隔>`，exit 1
//! - 参数/读盘/导出错误：`ERROR <原因>`，exit 2
//! - `--check-only`：`{"checks":N,"ok":true}` + `CHECK OK`，exit 0

use std::path::{Path, PathBuf};
use std::process::exit;

use qidiwork_gui::office::bid::fill;
use qidiwork_gui::office::bid::model::{BidExportRequest, BidExportResult, Block};
use qidiwork_gui::office::bid::verify::{self, VerifyReport};

const USAGE: &str =
    "用法: bid_export --input <req.json> --out <out.docx> [--template <tpl.docx>]\n       bid_export --check-only <docx>";

/// 结构校验不合格
const EXIT_CHECK_FAIL: i32 = 1;
/// 参数 / 读盘 / 导出错误
const EXIT_ERROR: i32 = 2;

/// 解析后的子命令
enum Cmd {
    Export {
        input: PathBuf,
        out: PathBuf,
        template: PathBuf,
    },
    CheckOnly {
        docx: PathBuf,
    },
}

fn main() {
    match parse_args(std::env::args().skip(1)) {
        Ok(Cmd::Export {
            input,
            out,
            template,
        }) => run_export(&input, &out, &template),
        Ok(Cmd::CheckOnly { docx }) => run_check_only(&docx),
        Err(msg) => error(&msg),
    }
}

/// 默认模板：编译期 crate 目录下的 bid-engineering-blue-v1.docx
fn default_template() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/templates/bid-engineering-blue-v1.docx")
}

/// `ERROR <原因>` + exit 2
fn error(msg: &str) -> ! {
    eprintln!("ERROR {msg}");
    exit(EXIT_ERROR)
}

/// 参数解析：`--input`/`--out`/`--template` 任意顺序；`--check-only` 单独使用
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Cmd, String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut template: Option<PathBuf> = None;
    let mut check_only: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--template" => template = args.next().map(PathBuf::from),
            "--check-only" => check_only = args.next().map(PathBuf::from),
            other => return Err(format!("未知参数 {other}\n{USAGE}")),
        }
    }
    if let Some(docx) = check_only {
        if input.is_some() || out.is_some() || template.is_some() {
            return Err(format!("--check-only 不能与 --input/--out/--template 混用\n{USAGE}"));
        }
        return Ok(Cmd::CheckOnly { docx });
    }
    match (input, out) {
        (Some(input), Some(out)) => Ok(Cmd::Export {
            input,
            out,
            template: template.unwrap_or_else(default_template),
        }),
        _ => Err(format!("缺少 --input 或 --out\n{USAGE}")),
    }
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

/// 图路径：相对路径按请求文件所在目录解析（与 bid_smoke 同口径）
fn resolve_figure_paths(req: &mut BidExportRequest, base: &Path) {
    for chapter in &mut req.chapters {
        for block in &mut chapter.blocks {
            if let Block::Figure(fig) = block {
                for item in &mut fig.items {
                    if !Path::new(item.path.as_str()).is_absolute() {
                        item.path = base.join(item.path.as_str()).to_string_lossy().to_string();
                    }
                }
            }
        }
    }
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

/// 导出：请求 JSON → docx → 结构校验
fn run_export(input: &Path, out: &Path, template: &Path) -> ! {
    let text = match std::fs::read_to_string(input) {
        Ok(t) => t,
        Err(e) => error(&format!("读请求文件失败 {}: {e}", input.display())),
    };
    let mut req: BidExportRequest = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => error(&format!("请求 JSON 解析失败: {e}")),
    };

    let base = input.parent().map(Path::to_path_buf).unwrap_or_default();
    resolve_figure_paths(&mut req, &base);

    let out_abs = match absolute(out) {
        Ok(p) => p,
        Err(e) => error(&e),
    };
    if let Some(parent) = out_abs.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error(&format!("创建输出目录失败 {}: {e}", parent.display()));
        }
    }
    req.output_path = out_abs.to_string_lossy().to_string();

    let (stats, bytes) = match fill::export(&req, template, &out_abs) {
        Ok(v) => v,
        Err(e) => error(&format!("导出失败: {e}")),
    };
    let report = match verify::verify(&out_abs, None) {
        Ok(r) => r,
        Err(e) => error(&format!("校验失败: {e}")),
    };

    if !report.ok() {
        println!("CHECK FAIL {}", failed_names(&report));
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
    println!("EXPORT OK");
    exit(0)
}

/// 只校验：`verify(&p, None)`
fn run_check_only(docx: &Path) -> ! {
    let report = match verify::verify(docx, None) {
        Ok(r) => r,
        Err(e) => error(&format!("校验失败 {}: {e}", docx.display())),
    };
    let n = report.checks.len();
    if report.ok() {
        println!("{}", serde_json::json!({ "checks": n, "ok": true }));
        println!("CHECK OK");
        exit(0);
    }
    println!("{}", serde_json::json!({ "checks": n, "ok": false }));
    println!("CHECK FAIL {}", failed_names(&report));
    exit(EXIT_CHECK_FAIL)
}
