//! 标书项目工作区 CLI（P1-2 / 规格 §A.3 + §7.3/§7.5/§7.6）。
//!
//! 用法：
//! - `bid_project new <dir> --title "某县…施工组织设计"`
//! - `bid_project build <dir> [-o out.docx] [--no-number] [--compact] [--max-pages <N>]`
//! - `bid_project check <dir>`
//!
//! `new`：建 `bid.project.json` + `chapters/01-概述.md` + `assets/`；清单已存在 → `ERROR 项目已存在`。
//! `build`：`project::build_request` 合并章节 → 按清单 `style_id` 取模板（缺省
//! `bid-engineering-blue-v1`；`--compact` 时取 `<style_id>-compact.docx`）`fill::export` 出 docx
//! （默认 `<dir>/out.docx`）→ `verify::verify_with_style(&out, None, style)` 24 项结构校验
//! → 一行 JSON 摘要 + `BUILD OK`。
//! `check`：`project::validate_project` → `PROJECT OK` / `PROJECT FAIL <问题,逗号分隔>`。
//!
//! 输出约定（供脚本消费）：
//! - `new` 成功：一行 JSON（`dir`/`manifest`）+ `NEW OK`，exit 0
//! - `build` 成功：一行 JSON 摘要 + `BUILD OK`，exit 0；结构校验失败 `BUILD FAIL <check 名>`，exit 1
//! - `build` 页数上限（S3 页数红线，§7.5）：优先级 **CLI `--max-pages` > 清单 `max_pages` > 无**，
//!   摘要恒带 `"max_pages_source":"cli|manifest|none"`；**仅当有上限时**才 WPS 实测页数
//!   （§7.6：无上限不测、不打印 pages），命中时摘要带 `"pages":n`；
//!   `pages > max` → 打印 `PAGES_EXCEEDED pages=<n> max=<M>` 并 exit 3（docx 保留）；
//!   测量失败只打 `WARN`（stderr），不改退出码
//! - `check`：`PROJECT OK` exit 0 / `PROJECT FAIL …` exit 1
//! - 参数 / 读盘 / 解析 / 导出错误：`ERROR <原因>`，exit 2

use std::path::{Path, PathBuf};
use std::process::exit;

use qidiwork_gui::office::bid::fill;
use qidiwork_gui::office::bid::model::{BidExportResult, DocStyle};
use qidiwork_gui::office::bid::pages;
use qidiwork_gui::office::bid::paths::DEFAULT_STYLE_ID;
use qidiwork_gui::office::bid::project;
use qidiwork_gui::office::bid::verify::{self, VerifyReport};

const USAGE: &str = "用法: bid_project new <dir> --title <书名>\n       \
                     bid_project build <dir> [-o <out.docx>] [--no-number] [--compact] [--max-pages <N>]\n       \
                     bid_project check <dir>";

/// 紧凑模板缺失时的固定错误行（§7.3：`--compact` 只认 `<style_id>-compact.docx`）
const COMPACT_TEMPLATE_MISSING: &str = "紧凑模板缺失（先运行 gen_bid_template --compact）";


/// 结构校验 / 项目体检不合格
const EXIT_CHECK_FAIL: i32 = 1;
/// 参数 / 读盘 / 解析 / 导出错误
const EXIT_ERROR: i32 = 2;
/// 页数超红线（S3：文件保留，便于用户查看）
const EXIT_PAGES_EXCEEDED: i32 = 3;

/// 解析后的子命令
enum Cmd {
    New { dir: PathBuf, title: String },
    Build {
        dir: PathBuf,
        out: Option<PathBuf>,
        number: bool,
        /// `--compact`：用 `<style_id>-compact.docx` 模板（压缩档位）
        compact: bool,
        /// `--max-pages <N>`：页数红线（超 → `PAGES_EXCEEDED` + exit 3）
        max_pages: Option<u32>,
    },
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
        Ok(Cmd::Build {
            dir,
            out,
            number,
            compact,
            max_pages,
        }) => run_build(&dir, out.as_deref(), number, compact, max_pages),
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
    let mut max_pages: Option<u32> = None;
    let mut no_number = false;
    let mut compact = false;
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
            "--max-pages" => {
                let value = it
                    .next()
                    .ok_or_else(|| format!("缺少 --max-pages 的值\n{USAGE}"))?;
                let n = value.parse::<u32>().map_err(|_| {
                    format!("--max-pages 需要非负整数，收到 {value}\n{USAGE}")
                })?;
                max_pages = Some(n);
            }
            "--no-number" => no_number = true,
            "--compact" => compact = true,
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
            if out.is_some() || no_number || max_pages.is_some() || compact {
                return Err(format!(
                    "new 不接受 -o/--out、--no-number、--compact 与 --max-pages\n{USAGE}"
                ));
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
                compact,
                max_pages,
            })
        }
        "check" => {
            if title.is_some() || out.is_some() || no_number || max_pages.is_some() || compact {
                return Err(format!(
                    "check 不接受 --title/-o/--no-number/--compact/--max-pages\n{USAGE}"
                ));
            }
            Ok(Cmd::Check { dir })
        }
        other => Err(format!("未知子命令 {other}\n{USAGE}")),
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

/// `build`：合并 → 导出 → 24 项校验 → （有上限时）页数红线 → 摘要 + `BUILD OK`
///
/// 页数上限优先级（§7.5）：CLI `--max-pages` > 清单 `max_pages` > 无；
/// 摘要恒带 `"max_pages_source":"cli|manifest|none"`，**无上限不跑 WPS**（§7.6）。
fn run_build(
    dir: &Path,
    out: Option<&Path>,
    number: bool,
    compact: bool,
    max_pages_cli: Option<u32>,
) -> ! {
    let manifest = match project::load_manifest(dir) {
        Ok(m) => m,
        Err(e) => error(&e.to_string()),
    };
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

    // 模板按清单 `style_id` 取（缺省 → DEFAULT_STYLE_ID；不存在/非法 → ERROR exit 2）；
    // `--compact` 改取 `<style_id>-compact.docx`（缺失 → 固定错误行 exit 2）
    let style_id = req.style_id.as_deref().unwrap_or(DEFAULT_STYLE_ID);
    let template = if compact {
        let compact_id = format!("{style_id}-compact");
        match project::resolve_template(Some(&compact_id)) {
            Ok(p) => p,
            Err(_) => error(COMPACT_TEMPLATE_MISSING),
        }
    } else {
        match project::resolve_template(req.style_id.as_deref()) {
            Ok(p) => p,
            Err(e) => error(&format!("模板解析失败: {e}")),
        }
    };
    // 版式真值跟模板走：紧凑模板 = 压缩档位，默认模板 = 默认档位
    let style = if compact {
        DocStyle::default().compact()
    } else {
        DocStyle::default()
    };

    let (stats, bytes) = match fill::export(&req, &template, &out_abs) {
        Ok(v) => v,
        Err(e) => error(&format!("导出失败: {e}")),
    };
    let report = match verify::verify_with_style(&out_abs, None, &style) {
        Ok(r) => r,
        Err(e) => error(&format!("校验失败: {e}")),
    };
    if !report.ok() {
        println!("BUILD FAIL {}", failed_names(&report));
        exit(EXIT_CHECK_FAIL);
    }

    // §7.5 上限优先级：CLI 显式值覆盖清单，都没设 → 不设限
    let (max_pages, max_pages_source) = match max_pages_cli {
        Some(n) => (Some(n), "cli"),
        None => match manifest.max_pages {
            Some(n) => (Some(n), "manifest"),
            None => (None, "none"),
        },
    };

    // §7.6：只有**有上限**时才拉起 WPS 实测（无上限不测、不打印 pages）；
    // 测量失败只警告：无法判定超页时不误报 exit 3
    let measured = match max_pages {
        Some(_) => match pages::measure(&out_abs) {
            Ok(stat) => Some(stat.pages),
            Err(e) => {
                eprintln!("WARN 页数测量失败: {e}");
                None
            }
        },
        None => None,
    };

    let summary = BidExportResult {
        path: out_abs.to_string_lossy().to_string(),
        bytes,
        chapters: stats.chapters,
        paragraphs: stats.paragraphs,
        tables: stats.tables,
        figures: stats.figures,
    };
    // 摘要 = `BidExportResult` 的 JSON + 上限来源（恒有）+ 实测页数（有上限且测得才追加）
    let mut summary_value = match serde_json::to_value(&summary) {
        Ok(v) => v,
        Err(e) => error(&format!("摘要序列化失败: {e}")),
    };
    if let Some(obj) = summary_value.as_object_mut() {
        obj.insert(
            "max_pages_source".to_string(),
            serde_json::Value::from(max_pages_source),
        );
        if let Some(n) = measured {
            obj.insert("pages".to_string(), serde_json::Value::from(n));
        }
    }
    match serde_json::to_string(&summary_value) {
        Ok(line) => println!("{line}"),
        Err(e) => error(&format!("摘要序列化失败: {e}")),
    }
    // S3 红线：实测超页 → 保留 docx（便于人工查看）但 exit 3，脚本可据此阻断发布
    if let (Some(n), Some(max)) = (measured, max_pages) {
        if pages::exceeds(n, Some(max)) {
            println!("PAGES_EXCEEDED pages={n} max={max}");
            exit(EXIT_PAGES_EXCEEDED);
        }
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
