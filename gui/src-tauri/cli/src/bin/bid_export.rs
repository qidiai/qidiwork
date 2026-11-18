//! 标书导出 CLI（P0c 收尾 + P1-1 D3 `--md` + §7.3/§7.5/§7.6 压缩与页数上限）。
//!
//! 用法：
//! - `bid_export --input <req.json> --out <out.docx> [--template <tpl.docx>] [--compact] [--pages] [--max-pages <N>]`
//! - `bid_export --md <file.md> --out <out.docx> [--no-number] [--template <tpl.docx>] [--compact] [--pages] [--max-pages <N>]`
//! - `bid_export --check-only <docx> [--compact]`
//!
//! 导出路径：读请求 JSON → `office::bid::model::BidExportRequest`
//! （`output_path` 由 `--out` 覆盖；`Figure.path` 相对路径按请求文件所在目录解析），
//! 调 `office::bid::fill::export` 产出 docx，再调 `office::bid::verify::verify_with_style`
//! 做 24 项结构校验（`--compact` 时按压缩档位算期望值）。
//!
//! `--md` 走同一终点：`office::bid::markdown::parse_markdown_with(src, md 的父目录, !no_number)`
//! → `(文档标题, 章节)` → 同上导出 + 24 项校验（`--md` 与 `--input` 互斥）。
//!
//! 输出约定（供脚本消费）：
//! - 成功：一行 JSON 摘要 `{"path":…,"bytes":…,"chapters":…,"paragraphs":…,"tables":…,"figures":…,"template_source":…,"max_pages_source":…}`
//!   + `EXPORT OK`，exit 0（`template_source` 取值：`resource` = exe 同级 `templates/`、
//!   `dev` = lib 仓库内 `resources/templates`、`arg` = `--template` 显式指定）
//! - `--pages`（M1）：24 项 check 通过后再用本机 WPS 实测页数（`office::bid::pages::measure`），
//!   摘要里多一个 `"pages":N`，并在摘要行之后、`EXPORT OK` 之前打印 `PAGES=<n>`；
//!   测量失败只打 `WARN 页数测量失败: …`（stderr），不改退出码
//! - `--max-pages <N>`（S3 页数红线，§7.5）：优先级 **CLI `--max-pages` > 清单 `max_pages` > 无**
//!   （`--md` 取 md 所在目录或其上一级的 `bid.project.json`；`--input` 无清单语义），
//!   摘要恒带 `"max_pages_source":"cli|manifest|none"`；
//!   **仅当有上限或显式 `--pages` 时**才测页（§7.6）；`pages > max` →
//!   打印 `PAGES_EXCEEDED pages=<n> max=<M>` 并 exit 3（**docx 保留**，便于人工查看）；
//!   `pages <= max` → 正常 `EXPORT OK`，摘要里带 `"pages":n`
//! - 结构校验失败：`CHECK FAIL <失败 check 名,逗号分隔>`，exit 1
//! - 参数/读盘/导出错误：`ERROR <原因>`，exit 2
//! - `--check-only`：`{"checks":N,"ok":true,"warnings":n}` + `CHECK OK`，exit 0
//! - P2 告警通道（T1）：warnings 非空时在摘要 JSON 行之后、`EXPORT OK`/`CHECK OK`/`CHECK FAIL`
//!   之前逐条打印 `WARN <name> <detail>`（stdout）；告警不阻断、不影响退出码

use std::path::{Path, PathBuf};
use std::process::exit;

use qidiwork_gui::office::bid::fill;
use qidiwork_gui::office::bid::markdown;
use qidiwork_gui::office::bid::model::{BidExportRequest, Block, DocStyle};
use qidiwork_gui::office::bid::pages;
use qidiwork_gui::office::bid::paths::{DEFAULT_STYLE_ID, resolve_for_exe};
use qidiwork_gui::office::bid::project;
use qidiwork_gui::office::bid::verify::{self, VerifyReport};

const USAGE: &str =
    "用法: bid_export --input <req.json> --out <out.docx> [--template <tpl.docx>] [--compact] [--pages] [--max-pages <N>]\n       bid_export --md <file.md> --out <out.docx> [--no-number] [--template <tpl.docx>] [--compact] [--pages] [--max-pages <N>]\n       bid_export --check-only <docx> [--compact]";

/// 紧凑模板缺失时的固定错误行（§7.3：`--compact` 只认 `<style_id>-compact.docx`）
const COMPACT_TEMPLATE_MISSING: &str = "紧凑模板缺失（先运行 gen_bid_template --compact）";


/// 结构校验不合格
const EXIT_CHECK_FAIL: i32 = 1;
/// 参数 / 读盘 / 导出错误
const EXIT_ERROR: i32 = 2;
/// 页数超红线（S3：文件保留，便于用户查看）
const EXIT_PAGES_EXCEEDED: i32 = 3;

/// 页数上限解析结果（§7.5：CLI `--max-pages` > 清单 `max_pages` > 无）
struct PageCap {
    /// `None` = 不设限（不测页、不打印 pages）
    max: Option<u32>,
    /// 来源：`cli` / `manifest` / `none`（摘要 `"max_pages_source"`）
    source: &'static str,
}

impl PageCap {
    /// 仅 CLI 上限（`--input` 无清单语义）
    fn cli_only(max: Option<u32>) -> Self {
        match max {
            Some(n) => Self { max: Some(n), source: "cli" },
            None => Self { max: None, source: "none" },
        }
    }

    /// CLI 优先，未给才回落到 `base_dir`（或其上一级）的 `bid.project.json`
    fn resolve(max_cli: Option<u32>, base_dir: &Path) -> Self {
        if max_cli.is_some() {
            return Self::cli_only(max_cli);
        }
        match manifest_max_pages(base_dir) {
            Some(n) => Self { max: Some(n), source: "manifest" },
            None => Self { max: None, source: "none" },
        }
    }
}

/// `base_dir/bid.project.json`（其次 `base_dir/../bid.project.json`）的 `max_pages`。
///
/// 项目模式下稿子通常落在 `<proj>/chapters/`，故上一级也算一次；最近的清单优先，
/// 缺失 / 解析失败一律视为「无上限」（清单问题由 `bid_project check` 报，不在这里阻断导出）。
fn manifest_max_pages(base_dir: &Path) -> Option<u32> {
    let dirs = [Some(base_dir), base_dir.parent()];
    for dir in dirs.into_iter().flatten() {
        if let Ok(manifest) = project::load_manifest(dir) {
            return manifest.max_pages;
        }
    }
    None
}

/// 解析后的子命令
enum Cmd {
    /// `--input <req.json>`：请求 JSON 直出
    ExportJson {
        input: PathBuf,
        out: PathBuf,
        template: PathBuf,
        /// 模板命中来源：`resource` / `dev` / `arg`
        template_source: &'static str,
        /// `--compact`：模板取 `<style_id>-compact.docx`，校验按压缩档位
        compact: bool,
        /// `--pages`：导出后追加 WPS 实测页数
        pages: bool,
        /// `--max-pages <N>`：页数红线（超 → `PAGES_EXCEEDED` + exit 3）
        max_pages: Option<u32>,
    },
    /// `--md <file.md>`：Markdown 内容入口（`number = !--no-number`）
    ExportMd {
        md: PathBuf,
        out: PathBuf,
        template: PathBuf,
        /// 模板命中来源：`resource` / `dev` / `arg`
        template_source: &'static str,
        /// `--compact`：模板取 `<style_id>-compact.docx`，校验按压缩档位
        compact: bool,
        number: bool,
        /// `--pages`：导出后追加 WPS 实测页数
        pages: bool,
        /// `--max-pages <N>`：页数红线（超 → `PAGES_EXCEEDED` + exit 3）
        max_pages: Option<u32>,
    },
    CheckOnly {
        docx: PathBuf,
        /// `--compact`：按压缩档位校验（否则紧凑产物会误报版式失配）
        compact: bool,
    },
}


fn main() {
    match parse_args(std::env::args().skip(1)) {
        Ok(Cmd::ExportJson {
            input,
            out,
            template,
            template_source,
            compact,
            pages,
            max_pages,
        }) => run_export(&input, &out, &template, template_source, compact, pages, max_pages),
        Ok(Cmd::ExportMd {
            md,
            out,
            template,
            template_source,
            compact,
            number,
            pages,
            max_pages,
        }) => run_export_md(&md, &out, &template, template_source, compact, number, pages, max_pages),
        Ok(Cmd::CheckOnly { docx, compact }) => run_check_only(&docx, compact),
        Err(msg) => error(&msg),
    }
}

/// 默认模板：先 exe 同级 `templates/`（source=`resource`），再 lib 的 `resources/templates`
/// （source=`dev`）。
///
/// 解析逻辑与编译期 crate 目录宏都留在 lib（`paths::resolve_for_exe`）：本 crate
/// 是 `src-tauri/cli`，宏在此处展开会指向错误目录，故本文件不得出现该宏。
fn default_template() -> Result<(PathBuf, &'static str), String> {
    resolve_for_exe(DEFAULT_STYLE_ID)
}

/// `--compact` 模板：`<DEFAULT_STYLE_ID>-compact.docx`（同样 resource → dev 两支）。
///
/// 找不到 → 固定错误行 `ERROR 紧凑模板缺失（先运行 gen_bid_template --compact）` + exit 2。
fn compact_template() -> Result<(PathBuf, &'static str), String> {
    let id = format!("{DEFAULT_STYLE_ID}-compact");
    resolve_for_exe(&id).map_err(|_| COMPACT_TEMPLATE_MISSING.to_string())
}

/// 版式真值：紧凑档位 / 默认档位（供 `verify_with_style`）
fn style_for(compact: bool) -> DocStyle {
    if compact {
        DocStyle::default().compact()
    } else {
        DocStyle::default()
    }
}

/// `ERROR <原因>` + exit 2
fn error(msg: &str) -> ! {
    eprintln!("ERROR {msg}");
    exit(EXIT_ERROR)
}

/// 取 `--flag` 后面紧跟的值；缺值直接报错（不静默回落/不落到「缺 --input 或 --out」）
fn flag_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<PathBuf, String> {
    args.next()
        .map(PathBuf::from)
        .ok_or_else(|| format!("缺少 {flag} 的值\n{USAGE}"))
}

/// 取 `--flag` 后面的非负整数；缺值 / 非法值都报错（不静默回落）
fn flag_u32(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<u32, String> {
    let raw = args
        .next()
        .ok_or_else(|| format!("缺少 {flag} 的值\n{USAGE}"))?;
    raw.parse::<u32>()
        .map_err(|_| format!("{flag} 需要非负整数，收到 {raw}\n{USAGE}"))
}

/// 参数解析：`--input`/`--md`/`--out`/`--template`/`--max-pages` 任意顺序；
/// `--no-number`/`--pages`/`--compact` 为开关；`--check-only` 单独使用（可与 `--compact` 搭配）；
/// `--md` 与 `--input` 互斥；`--compact` 与 `--template` 互斥；
/// 任何带值 flag 缺值都报「缺少 <flag> 的值」并 exit 2
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Cmd, String> {
    let mut input: Option<PathBuf> = None;
    let mut md: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut template: Option<PathBuf> = None;
    let mut check_only: Option<PathBuf> = None;
    let mut no_number = false;
    let mut pages = false;
    let mut compact = false;
    let mut max_pages: Option<u32> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = Some(flag_value(&mut args, "--input")?),
            "--md" => md = Some(flag_value(&mut args, "--md")?),
            "--out" => out = Some(flag_value(&mut args, "--out")?),
            "--template" => template = Some(flag_value(&mut args, "--template")?),
            "--no-number" => no_number = true,
            "--pages" => pages = true,
            "--compact" => compact = true,
            "--max-pages" => max_pages = Some(flag_u32(&mut args, "--max-pages")?),
            "--check-only" => check_only = Some(flag_value(&mut args, "--check-only")?),
            other => return Err(format!("未知参数 {other}\n{USAGE}")),
        }
    }
    if let Some(docx) = check_only {
        if input.is_some() || md.is_some() || out.is_some() || template.is_some() || no_number {
            return Err(format!(
                "--check-only 不能与 --input/--md/--out/--template/--no-number 混用\n{USAGE}"
            ));
        }
        if pages || max_pages.is_some() {
            return Err(format!(
                "--check-only 不能与 --pages/--max-pages 混用\n{USAGE}"
            ));
        }
        return Ok(Cmd::CheckOnly { docx, compact });
    }
    if md.is_some() && input.is_some() {
        // 规格 P1-1 D3：互斥（错误行不带 USAGE，便于脚本精确匹配）
        return Err("--md 与 --input 不能同时使用".to_string());
    }
    if no_number && md.is_none() {
        return Err(format!("--no-number 仅与 --md 同时使用\n{USAGE}"));
    }
    if compact && template.is_some() {
        return Err(format!(
            "--compact 不能与 --template 同时使用（--compact 已确定模板）\n{USAGE}"
        ));
    }
    // `--template` 显式指定优先（source=`arg`）；`--compact` 取 `<style_id>-compact.docx`；
    // 否则走 lib 的 exe/dev 双支解析
    let (template, template_source) = match template {
        Some(explicit) => (explicit, "arg"),
        None if compact => compact_template()?,
        None => default_template()?,
    };
    match (md, input, out) {
        (Some(md), _, Some(out)) => Ok(Cmd::ExportMd {
            md,
            out,
            template,
            template_source,
            compact,
            number: !no_number,
            pages,
            max_pages,
        }),
        (None, Some(input), Some(out)) => Ok(Cmd::ExportJson {
            input,
            out,
            template,
            template_source,
            compact,
            pages,
            max_pages,
        }),
        // 缺参分两种情况报，便于脚本精确匹配（仍附 USAGE 便于人工排障）
        (md, input, _) => {
            if md.is_none() && input.is_none() {
                Err(format!("需要 --input 或 --md\n{USAGE}"))
            } else {
                Err(format!("缺少 --out\n{USAGE}"))
            }
        }
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

/// 导出：请求 JSON → docx → 结构校验（`--input` 无清单语义，页数上限只认 CLI）
fn run_export(
    input: &Path,
    out: &Path,
    template: &Path,
    template_source: &'static str,
    compact: bool,
    pages: bool,
    max_pages: Option<u32>,
) -> ! {
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
    export_and_verify(
        req,
        out,
        template,
        template_source,
        compact,
        pages,
        PageCap::cli_only(max_pages),
    )
}

/// 导出：Markdown → `parse_markdown_with(src, md 父目录, !no_number)` → 同上导出 + 校验。
/// 页数上限优先级（§7.5）：CLI `--max-pages` > md 所在目录（或上一级）清单 `max_pages` > 无。
fn run_export_md(
    md: &Path,
    out: &Path,
    template: &Path,
    template_source: &'static str,
    compact: bool,
    number: bool,
    pages: bool,
    max_pages: Option<u32>,
) -> ! {
    let md_abs = match absolute(md) {
        Ok(p) => p,
        Err(e) => error(&e),
    };
    let src = match std::fs::read_to_string(&md_abs) {
        Ok(t) => t,
        Err(e) => error(&format!("读 Markdown 文件失败 {}: {e}", md_abs.display())),
    };
    // 图片相对路径按 Markdown 文件所在目录解析（规格 P1-1 §1）
    let base = md_abs.parent().map(Path::to_path_buf).unwrap_or_default();
    let (title, chapters) = match markdown::parse_markdown_with(&src, &base, number) {
        Ok(v) => v,
        Err(e) => error(&format!("Markdown 解析失败: {e}")),
    };
    let req = BidExportRequest {
        title,
        chapters,
        output_path: String::new(), // 由 --out 覆盖
        style_id: None,
        figure_width_cm: None,
    };
    let cap = PageCap::resolve(max_pages, &base);
    export_and_verify(req, out, template, template_source, compact, pages, cap)
}

/// 导出 + 24 项结构校验 + 摘要输出（`--input` / `--md` 共用的收尾）
///
/// `with_pages`（`--pages`）/ `cap.max`（`--max-pages` 或清单 `max_pages`）：校验通过后用本机
/// WPS 实测页数（`pages::measure`），两者同时给出时**只测一次**并复用结果；
/// **若既无上限也没显式 `--pages` → 完全不测页**（§7.6：不为每次导出拉 WPS）。
/// 测量失败只打 `WARN`，不影响摘要与退出码（无法判定超页时不误报 exit 3）。
fn export_and_verify(
    mut req: BidExportRequest,
    out: &Path,
    template: &Path,
    template_source: &'static str,
    compact: bool,
    with_pages: bool,
    cap: PageCap,
) -> ! {
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
    let report = match verify::verify_with_style(&out_abs, None, &style_for(compact)) {
        Ok(r) => r,
        Err(e) => error(&format!("校验失败: {e}")),
    };

    if !report.ok() {
        println!("CHECK FAIL {}", failed_names(&report));
        exit(EXIT_CHECK_FAIL);
    }

    // `--pages` / 上限：本机 WPS 实测（失败只警告，不改退出码、摘要不带 pages）
    // 两个来源共用一次测量：同时给出不重复拉起 WPS
    let mut measured: Option<u32> = None;
    if with_pages || cap.max.is_some() {
        match pages::measure(&out_abs) {
            Ok(stat) => measured = Some(stat.pages),
            Err(e) => eprintln!("WARN 页数测量失败: {e}"),
        }
    }
    let summary = ExportSummary {
        path: out_abs.to_string_lossy().to_string(),
        bytes,
        chapters: stats.chapters,
        paragraphs: stats.paragraphs,
        tables: stats.tables,
        figures: stats.figures,
        template_source,
        max_pages_source: cap.source,
        pages: measured,
    };
    match serde_json::to_string(&summary) {
        Ok(line) => println!("{line}"),
        Err(e) => error(&format!("摘要序列化失败: {e}")),
    }
    // P2 告警通道（T1）：摘要 JSON 行之后逐条 WARN（不阻断、不改退出码）
    for w in &report.warnings {
        println!("WARN {} {}", w.name, w.detail);
    }
    // 摘要行之后、`EXPORT OK` 之前补一行机器可读的页数（无上限且无 --pages 时不打印）
    if let Some(pages) = measured {
        // S3 红线：实测超页 → 保留 docx（便于人工查看）但 exit 3，脚本可据此阻断发布
        if let Some(max) = cap.max.filter(|max| pages::exceeds(pages, Some(*max))) {
            println!("PAGES_EXCEEDED pages={pages} max={max}");
            exit(EXIT_PAGES_EXCEEDED);
        }
        println!("PAGES={pages}");
    }
    println!("EXPORT OK");
    exit(0)
}

/// 摘要行（字段顺序与原 `BidExportResult` 一致，追加 `template_source` / `max_pages_source`；
/// 测到页数才追加 `pages`）
#[derive(serde::Serialize)]
struct ExportSummary {
    path: String,
    bytes: u64,
    chapters: usize,
    paragraphs: usize,
    tables: usize,
    figures: usize,
    /// `resource`（exe 同级 templates/）/ `dev`（仓库内 resources/templates）/ `arg`（--template）
    template_source: &'static str,
    /// 页数上限来源（§7.5）：`cli` / `manifest` / `none`（恒输出）
    max_pages_source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pages: Option<u32>,
}

/// 只校验：`verify_with_style(&p, None, style)`（`--compact` 时按压缩档位）
fn run_check_only(docx: &Path, compact: bool) -> ! {
    let report = match verify::verify_with_style(docx, None, &style_for(compact)) {
        Ok(r) => r,
        Err(e) => error(&format!("校验失败 {}: {e}", docx.display())),
    };
    let n = report.checks.len();
    let wn = report.warnings.len();
    if report.ok() {
        println!("{}", serde_json::json!({ "checks": n, "ok": true, "warnings": wn }));
        for w in &report.warnings {
            println!("WARN {} {}", w.name, w.detail);
        }
        println!("CHECK OK");
        exit(0);
    }
    println!("{}", serde_json::json!({ "checks": n, "ok": false, "warnings": wn }));
    for w in &report.warnings {
        println!("WARN {} {}", w.name, w.detail);
    }
    println!("CHECK FAIL {}", failed_names(&report));
    exit(EXIT_CHECK_FAIL)
}
