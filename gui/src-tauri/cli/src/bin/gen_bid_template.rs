//! T1b 模板生成器 CLI（规格 §5.6 + §7.3 压缩档位）。
//!
//! 用法：
//! - 无参 → 输出 lib 的开发态模板 `gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`
//!   （`paths::dev_templates_dir()`，宏在 lib 内展开；`DocStyle::default()`）
//! - `--compact` → 输出紧凑模板 `bid-engineering-blue-v1-compact.docx`
//!   （`DocStyle::default().compact()`：1.3 行距 / 1.0cm 边距 / 表字号 10.5pt）
//! - 有位置参数 → 第一个位置参数视为输出路径（父目录自动创建），`--compact` 可与它组合
//! 成功打印 `TEMPLATE OK path=<绝对路径> bytes=<n>`；失败打印 `TEMPLATE FAIL <原因>` 并 exit 1。

use std::path::{Path, PathBuf};

use qidiwork_gui::office::bid::model::DocStyle;
use qidiwork_gui::office::bid::paths::{DEFAULT_STYLE_ID, dev_templates_dir};
use qidiwork_gui::office::bid::template_gen;
use qidiwork_gui::office::bid::{BidError, BidResult};

const USAGE: &str = "用法: gen_bid_template [--compact] [<out.docx>]";

fn main() {
    let (out, style) = match parse_args() {
        Ok(v) => v,
        Err(e) => fail(&e),
    };
    if let Err(e) = ensure_parent_dir(&out) {
        fail(&e);
    }

    match template_gen::generate_to_with(&out, &style) {
        Ok(bytes) => {
            let abs = match absolute_path(&out) {
                Ok(p) => p,
                Err(e) => fail(&e),
            };
            println!("TEMPLATE OK path={} bytes={}", abs.display(), bytes);
        }
        Err(e) => fail(&e),
    }
}

/// 解析 `[--compact] [<out.docx>]`：
/// 默认路径 = 开发态模板目录下的 `<style_id>.docx`（`--compact` 时为 `<style_id>-compact.docx`）；
/// 位置参数（最多一个）覆盖输出路径；未知 flag → Err（不静默忽略）。
fn parse_args() -> BidResult<(PathBuf, DocStyle)> {
    let mut compact = false;
    let mut out: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--compact" => compact = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with('-') => {
                return Err(BidError::new(format!("未知参数 {other}\n{USAGE}")));
            }
            other => {
                if out.is_some() {
                    return Err(BidError::new(format!("多余的参数 {other}\n{USAGE}")));
                }
                out = Some(PathBuf::from(other));
            }
        }
    }
    let style = if compact {
        DocStyle::default().compact()
    } else {
        DocStyle::default()
    };
    let out = out.unwrap_or_else(|| dev_templates_dir().join(template_file_name(compact)));
    Ok((out, style))
}

/// 开发态默认文件名：默认 `bid-engineering-blue-v1.docx`，紧凑 `bid-engineering-blue-v1-compact.docx`
fn template_file_name(compact: bool) -> String {
    if compact {
        format!("{DEFAULT_STYLE_ID}-compact.docx")
    } else {
        format!("{DEFAULT_STYLE_ID}.docx")
    }
}

/// 父目录不存在则创建（含各级）
fn ensure_parent_dir(out: &Path) -> BidResult<()> {
    let parent = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return Ok(()),
    };
    std::fs::create_dir_all(parent)
        .map_err(|e| BidError::new(format!("创建父目录失败: {e}")))
}

/// 相对路径按当前工作目录绝对化（生成已完成，路径必然存在）
fn absolute_path(p: &Path) -> BidResult<PathBuf> {
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| BidError::new(format!("获取工作目录失败: {e}")))?;
    Ok(cwd.join(p))
}

/// 失败统一出口：打印 `TEMPLATE FAIL <原因>` 并 exit 1
fn fail(e: &BidError) -> ! {
    eprintln!("TEMPLATE FAIL {e}");
    std::process::exit(1);
}
