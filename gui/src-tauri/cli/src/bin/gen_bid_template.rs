//! T1b 模板生成器 CLI（规格 §5.6）。
//!
//! 无参 → 输出 lib 的开发态模板 `gui/src-tauri/resources/templates/bid-engineering-blue-v1.docx`
//! （`paths::dev_templates_dir()`，宏在 lib 内展开）；
//! 有参 → 第一个参数视为输出路径（父目录自动创建）。
//! 成功打印 `TEMPLATE OK path=<绝对路径> bytes=<n>`；失败打印 `TEMPLATE FAIL <原因>` 并 exit 1。

use std::path::{Path, PathBuf};

use qidiwork_gui::office::bid::paths::{DEFAULT_STYLE_ID, dev_templates_dir};
use qidiwork_gui::office::bid::template_gen;
use qidiwork_gui::office::bid::{BidError, BidResult};

fn main() {
    let out = output_path();
    if let Err(e) = ensure_parent_dir(&out) {
        fail(&e);
    }

    match template_gen::generate_to(&out) {
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

/// 无参 → lib 的开发态模板目录（`gui/src-tauri/resources/templates/`）；有参 → 第一个参数即输出路径。
///
/// 不做「exe 同级可写就写那里」的推断：开发态 exe 同级（`src-tauri/target/...`）恒可写，
/// 会把产物写到错误位置；分发/安装态请显式传参指定输出路径。
fn output_path() -> PathBuf {
    match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => dev_templates_dir().join(format!("{DEFAULT_STYLE_ID}.docx")),
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
