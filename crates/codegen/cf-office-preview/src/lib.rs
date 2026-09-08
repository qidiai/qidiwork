//! QidiWork 办公预览后端：docx/xlsx → 纯文本/ASCII 渲染
//!
//! 提供终端内的文档预览能力，对接 office-artifact / office-canvas 产物数据。
//! 图片/PDF 不在终端内渲染，统一降级为 `Unsupported`（由上层提示"系统打开"）。

pub mod docx;
pub mod xlsx;

/// 预览内容类型
#[derive(Clone, Debug)]
pub enum PreviewContent {
    /// docx 提取的纯文本段落
    Docx { text: String },
    /// xlsx 转 ASCII 矩阵（按 sheet 分组）
    Xlsx { sheets: Vec<AsciiSheet> },
    /// 不支持的类型（提示打开系统程序）
    Unsupported { name: String, size: u64 },
}

/// ASCII 工作表
#[derive(Clone, Debug)]
pub struct AsciiSheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    pub max_row: usize,
    pub max_col: usize,
}

/// 按扩展名路由预览；未知类型降级为 Unsupported。
pub fn preview_path(path: &str) -> PreviewContent {
    let lower = path.to_ascii_lowercase();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let result = if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") || lower.ends_with(".xls")
    {
        xlsx::xlsx_to_preview(path, 50, 12)
    } else if lower.ends_with(".docx") {
        docx::docx_to_preview(path)
    } else {
        Ok(PreviewContent::Unsupported {
            name: path.to_string(),
            size,
        })
    };
    result.unwrap_or_else(|_| PreviewContent::Unsupported {
        name: path.to_string(),
        size,
    })
}

/// 将 PreviewContent 渲染为纯文本（供 BlockViewerPane::for_plain_text 使用）。
pub fn render_to_text(content: &PreviewContent) -> String {
    match content {
        PreviewContent::Docx { text } => text.clone(),
        PreviewContent::Xlsx { sheets } => {
            let mut out = String::new();
            for (i, sheet) in sheets.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(&format!("── Sheet: {} ──\n", sheet.name));
                out.push_str(&xlsx::matrix_to_ascii(sheet.rows.clone()));
                if sheet.max_row > sheet.rows.len() {
                    out.push_str(&format!(
                        "\n… 截断: 共 {} 行 × {} 列，仅显示前 {} 行",
                        sheet.max_row,
                        sheet.max_col,
                        sheet.rows.len()
                    ));
                }
                out.push('\n');
            }
            out
        }
        PreviewContent::Unsupported { name, size } => {
            format!("暂不支持终端预览: {name} ({} bytes)\n按 o 使用系统程序打开", size)
        }
    }
}
