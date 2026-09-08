//! xlsx → ASCII 矩阵渲染
//!
//! 使用 calamine 读取工作表数据，截断至 max_rows × max_cols。

use super::{AsciiSheet, PreviewContent};
use calamine::{open_workbook_auto, Data, Reader};

/// 将 xlsx 文件转为预览（每 sheet 取前 max_rows 行 × max_cols 列）
pub fn xlsx_to_preview(
    path: &str,
    max_rows: usize,
    max_cols: usize,
) -> Result<PreviewContent, String> {
    let mut wb = open_workbook_auto(path).map_err(|e| format!("open {path}: {e}"))?;
    let mut sheets = Vec::new();
    for name in wb.sheet_names().to_vec() {
        let range = wb
            .worksheet_range(&name)
            .map_err(|e| format!("sheet {name}: {e}"))?;
        let (h, w) = range.get_size();
        let mut rows = Vec::new();
        for r in 0..h.min(max_rows) {
            let mut row = Vec::new();
            for c in 0..w.min(max_cols) {
                row.push(range.get((r, c)).map(cell_to_string).unwrap_or_default());
            }
            rows.push(row);
        }
        sheets.push(AsciiSheet {
            name,
            rows,
            max_row: h,
            max_col: w,
        });
    }
    Ok(PreviewContent::Xlsx { sheets })
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f:.2}")
            }
        }
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => dt.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

/// 将单元格矩阵转为 ASCII 表格（│分隔，列宽按内容自适应，单格截断 24 字符）
pub fn matrix_to_ascii(rows: Vec<Vec<String>>) -> String {
    const MAX_CELL: usize = 24;
    if rows.is_empty() {
        return "(空表)".to_string();
    }
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if ncols == 0 {
        return "(空表)".to_string();
    }
    let mut widths = vec![3usize; ncols];
    for row in &rows {
        for (c, cell) in row.iter().enumerate() {
            let len = cell.chars().count().min(MAX_CELL);
            if len > widths[c] {
                widths[c] = len;
            }
        }
    }
    let mut out = String::new();
    for row in &rows {
        let mut line = String::new();
        for c in 0..ncols {
            let cell = row.get(c).cloned().unwrap_or_default();
            let truncated: String = if cell.chars().count() > MAX_CELL {
                let t: String = cell.chars().take(MAX_CELL - 1).collect();
                format!("{t}…")
            } else {
                cell
            };
            let pad = widths[c].saturating_sub(truncated.chars().count());
            line.push_str(&truncated);
            line.push_str(&" ".repeat(pad));
            line.push_str(if c + 1 < ncols { " │ " } else { "" });
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_ascii_aligns_columns() {
        let out = matrix_to_ascii(vec![
            vec!["名称".into(), "数量".into()],
            vec!["螺栓".into(), "120".into()],
        ]);
        assert!(out.contains('│'));
        assert!(out.contains("名称"));
        assert!(out.contains("120"));
    }

    #[test]
    fn matrix_ascii_truncates_long_cells() {
        let long = "x".repeat(80);
        let out = matrix_to_ascii(vec![vec![long]]);
        assert!(out.lines().next().unwrap().chars().count() <= 24);
        assert!(out.contains('…'));
    }

    #[test]
    fn matrix_ascii_empty() {
        assert_eq!(matrix_to_ascii(vec![]), "(空表)");
    }
}
