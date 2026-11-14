use serde::{Deserialize, Serialize};

use super::{BidError, BidResult};

/// 版式参数（P0a 只做常量默认值 + 字段占位；toml 加载留 P1）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DocStyle {
    pub style_id: String,        // 默认 "bid-engineering-blue-v1"
    pub body_size_pt: u32,       // 12
    pub body_line_spacing: u32,  // 360（=1.5 倍，240*1.5）
    pub title1_size_pt: u32,     // 15
    pub title1_color: String,    // "253D57"
    pub title2_size_pt: u32,     // 14
    pub title2_color: String,    // "315276"
    pub body_font_ea: String,    // "宋体"
    pub margin_twips: u32,       // 680（1.2cm）
}

impl Default for DocStyle {
    fn default() -> Self {
        Self {
            style_id: "bid-engineering-blue-v1".to_string(),
            body_size_pt: 12,
            body_line_spacing: 360,
            title1_size_pt: 15,
            title1_color: "253D57".to_string(),
            title2_size_pt: 14,
            title2_color: "315276".to_string(),
            body_font_ea: "宋体".to_string(),
            margin_twips: 680,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidExportRequest {
    pub title: String,
    pub chapters: Vec<Chapter>,
    /// 绝对路径；父目录不存在则创建。
    /// `#[serde(default)]` 必填：fixture 不含该字段（由 bid_smoke 的 --out 注入），
    /// 无 default 会 `missing field` 直接起不来。
    #[serde(default)]
    pub output_path: String,
    #[serde(default)]
    pub style_id: Option<String>,        // None → bid-engineering-blue-v1
    #[serde(default)]
    pub figure_width_cm: Option<f32>,    // None → 15.5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter { pub title: String, pub blocks: Vec<Block> }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph { text: String },
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
    Figure { path: String, caption: String, #[serde(default)] ai_generated: bool },
}

#[derive(Debug, Clone, Serialize)]
pub struct BidExportResult {
    pub path: String, pub bytes: u64,
    pub chapters: usize, pub paragraphs: usize, pub tables: usize, pub figures: usize,
}

/// 填充过程统计（fill 返回，命令层映射成 BidExportResult）
#[derive(Debug, Clone, Default)]
pub struct FillStats { pub chapters: usize, pub paragraphs: usize, pub tables: usize, pub figures: usize }

impl BidExportRequest {
    pub fn validate(&self) -> BidResult<()> {
        if self.title.trim().is_empty() {
            return Err(BidError::new("标题不能为空"));
        }
        if self.chapters.is_empty() {
            return Err(BidError::new("至少需要一章"));
        }
        for (ci, chapter) in self.chapters.iter().enumerate() {
            if chapter.title.trim().is_empty() {
                return Err(BidError::new(format!("第 {} 章标题不能为空", ci + 1)));
            }
            for block in &chapter.blocks {
                match block {
                    Block::Paragraph { .. } => {}
                    Block::Table { header, rows } => {
                        if header.is_empty() {
                            return Err(BidError::new("表格表头不能为空"));
                        }
                        for (ri, row) in rows.iter().enumerate() {
                            if row.len() != header.len() {
                                return Err(BidError::new(format!("表格第 {} 行列数不符", ri + 1)));
                            }
                        }
                    }
                    Block::Figure { path, .. } => {
                        if path.trim().is_empty() {
                            return Err(BidError::new("图片路径不能为空"));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
