//! T2c2 导出冒烟 CLI（规格 §6.6 / §7.1）。
//!
//! 用法：`bid_smoke --fixture <json> --out <docx>`（顺序无关）。
//! 读 fixture → `BidExportRequest`（`output_path` 用 `--out` 覆盖；fixture 内
//! `Figure.path` 相对路径按 fixture 文件所在目录解析成绝对路径），
//! 调 `office::bid::fill::export` 导出，再调 `office::bid::verify::verify`
//! 用 fixture 的 `expect` 字段做分层校验。
//! 成功打印 `report.to_json()` + `EXPORT OK path=<绝对路径> bytes=<n>` 并 exit 0；
//! 任一 check 失败打印 `SMOKE FAIL <不合格 check 列表>` 并 exit 1；
//! 用法/IO/解析错误打印 `SMOKE FAIL <原因>` 并 exit 1。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use qidiwork_gui::office::bid::fill;
use qidiwork_gui::office::bid::model::{Block, BidExportRequest};
use qidiwork_gui::office::bid::verify::{self, Expect};

/// fixture 期望值口径（§7.1）
#[derive(serde::Deserialize)]
struct FixtureExpect {
    chapters: usize,
    figures: usize,
    tables: usize,
    paragraphs: usize,
}

/// `BidExportRequest` 无 `expect` 字段，必须 flatten 包一层（不改 model.rs）
#[derive(serde::Deserialize)]
struct Fixture {
    #[serde(flatten)]
    req: BidExportRequest,
    expect: FixtureExpect,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("SMOKE FAIL {msg}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let (fixture_path, out_arg) = parse_args()?;
    let fixture_dir = fixture_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // 读 fixture
    let text = std::fs::read_to_string(&fixture_path)
        .map_err(|e| format!("读 fixture 失败 {}: {e}", fixture_path.display()))?;
    let mut fixture: Fixture = serde_json::from_str(&text)
        .map_err(|e| format!("fixture JSON 解析失败: {e}"))?;

    // 图片路径解析：相对路径 → 相对 fixture 文件所在目录的绝对路径
    for chapter in &mut fixture.req.chapters {
        for block in &mut chapter.blocks {
            if let Block::Figure { path, .. } = block {
                if !Path::new(path.as_str()).is_absolute() {
                    *path = fixture_dir.join(path.as_str()).to_string_lossy().to_string();
                }
            }
        }
    }

    // output_path = --out 的 canonicalize 前绝对路径（父目录不存在就创建）
    let out_abs = if out_arg.is_absolute() {
        out_arg
    } else {
        std::env::current_dir()
            .map_err(|e| format!("取当前目录失败: {e}"))?
            .join(out_arg)
    };
    if let Some(parent) = out_abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建输出目录失败 {}: {e}", parent.display()))?;
    }
    fixture.req.output_path = out_abs.to_string_lossy().to_string();

    // 模板
    let template = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/templates/bid-engineering-blue-v1.docx");

    // 导出 + 校验
    let (_stats, bytes) = fill::export(&fixture.req, &template, &out_abs)
        .map_err(|e| format!("导出失败: {e}"))?;
    let report = verify::verify(
        &out_abs,
        Some(&Expect {
            chapters: fixture.expect.chapters,
            figures: fixture.expect.figures,
            tables: fixture.expect.tables,
            paragraphs: fixture.expect.paragraphs,
        }),
    )
    .map_err(|e| format!("校验失败: {e}"))?;
    println!("{}", report.to_json());

    if report.ok() {
        println!(
            "EXPORT OK path={} bytes={}",
            out_abs.display(),
            bytes
        );
        Ok(())
    } else {
        let bad: Vec<String> = report
            .checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| format!("{}:{}", c.name, c.detail))
            .collect();
        Err(format!("不合格 check: {}", bad.join("; ")))
    }
}

/// 参数解析：`--fixture <path>` 与 `--out <path>`，顺序无关；缺任一 → 用法错误。
fn parse_args() -> Result<(PathBuf, PathBuf), String> {
    let usage = "用法: bid_smoke --fixture <json> --out <docx>";
    let mut fixture: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fixture" => {
                fixture = args.next().map(PathBuf::from);
            }
            "--out" => {
                out = args.next().map(PathBuf::from);
            }
            _ => return Err(usage.to_string()),
        }
    }
    match (fixture, out) {
        (Some(f), Some(o)) => Ok((f, o)),
        _ => Err(usage.to_string()),
    }
}
