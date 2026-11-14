use serde::{Deserialize, Serialize};

use super::{BidError, BidResult};

/// 版式参数（P0b §3.1：全量字段 + 派生换算；toml 加载留 P1）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DocStyle {
    pub style_id: String,              // "bid-engineering-blue-v1"
    pub page: PageStyle,
    pub body: BodyStyle,
    pub heading1: HeadingStyle,
    pub heading2: HeadingStyle,
    pub heading3: HeadingStyle,
    pub heading4: HeadingStyle,
    pub table: TableStyleCfg,
    pub figure: FigureStyleCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PageStyle { pub paper: String /* "A4V" */, pub margin_cm: f32 /* 1.2 */ }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BodyStyle {
    pub font: String,          // "宋体"
    pub size_pt: u32,          // 12
    pub line_spacing: f32,     // 1.5  → w:line = (240*1.5).round() = 360
    pub align: String,         // "justify" → w:jc both
    pub first_line_indent_chars: u32, // 2 → w:firstLineChars="200"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HeadingStyle { pub size_pt: u32, pub bold: bool, pub align: String /* center|left */, pub color: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TableStyleCfg {
    pub border_pt: f32,        // 0.5 → w:sz = round(0.5*8)=4
    pub outer_color: String,   // "74A6CC"
    pub inner_color: String,   // "B0D0E8"
    pub header_fill: String,   // "4877AC"
    pub header_color: String,  // "FFFFFF"
    pub first_col_fill: String,// "E6F2F9"（P0b 新增字段，B2 才消费）
    pub cant_split: bool,      // true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FigureStyleCfg {
    pub caption_size_pt: u32,  // 12
    pub caption_align: String, // "center"
    pub ai_label: String,      // "（AI 生成示意图）"
    pub wide_width_cm: f32,    // 18.6（内容区全宽）
    pub cell_width_cm: f32,    // 5.8（3up 单格图宽）
}

impl Default for PageStyle {
    fn default() -> Self {
        Self { paper: "A4V".to_string(), margin_cm: 1.2 }
    }
}

impl Default for BodyStyle {
    fn default() -> Self {
        Self {
            font: "宋体".to_string(),
            size_pt: 12,
            line_spacing: 1.5,
            align: "justify".to_string(),
            first_line_indent_chars: 2,
        }
    }
}

impl Default for HeadingStyle {
    fn default() -> Self {
        Self { size_pt: 12, bold: true, align: "left".to_string(), color: "000000".to_string() }
    }
}

impl Default for TableStyleCfg {
    fn default() -> Self {
        Self {
            border_pt: 0.5,
            outer_color: "74A6CC".to_string(),
            inner_color: "B0D0E8".to_string(),
            header_fill: "4877AC".to_string(),
            header_color: "FFFFFF".to_string(),
            first_col_fill: "E6F2F9".to_string(),
            cant_split: true,
        }
    }
}

impl Default for FigureStyleCfg {
    fn default() -> Self {
        Self {
            caption_size_pt: 12,
            caption_align: "center".to_string(),
            ai_label: "（AI 生成示意图）".to_string(),
            wide_width_cm: 18.6,
            cell_width_cm: 5.8,
        }
    }
}

impl Default for DocStyle {
    fn default() -> Self {
        Self {
            style_id: "bid-engineering-blue-v1".to_string(),
            page: PageStyle::default(),
            body: BodyStyle::default(),
            heading1: HeadingStyle { size_pt: 15, align: "center".to_string(), color: "253D57".to_string(), ..Default::default() },
            heading2: HeadingStyle { size_pt: 14, align: "center".to_string(), color: "315276".to_string(), ..Default::default() },
            heading3: HeadingStyle { size_pt: 14, ..Default::default() },
            heading4: HeadingStyle { size_pt: 12, ..Default::default() },
            table: TableStyleCfg::default(),
            figure: FigureStyleCfg::default(),
        }
    }
}

impl DocStyle {
    /// pt → half-points（w:sz / w:szCs 单位）
    pub fn half_points(&self, pt: u32) -> u32 { pt * 2 }
    /// 行距倍数 → w:line（240 = 单倍行距）
    pub fn line_twips(&self) -> u32 { (240.0 * self.body.line_spacing).round() as u32 }
    /// 页边距 cm → twips
    pub fn margin_twips(&self) -> u32 { (self.page.margin_cm / 2.54 * 1440.0).round() as u32 }
    /// 边框 pt → eighth-points（w:sz 单位）
    pub fn border_eighth_pt(&self) -> u32 { (self.table.border_pt * 8.0).round() as u32 }
    /// 首行缩进字符数 → w:firstLineChars（×100）
    pub fn first_line_chars_x100(&self) -> u32 { self.body.first_line_indent_chars * 100 }
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

/// 图布局（P0b §4.1）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FigureLayout { Single, Wide, ThreeUp }

/// 图注模式（P0b §4.1）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptionMode { #[default] PerImage, PerGroup }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigureItem {
    pub path: String,
    #[serde(default)] pub caption: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigureBlock {
    pub layout: FigureLayout,
    pub items: Vec<FigureItem>,
    #[serde(default)] pub caption: String,          // 组图注 / 单图缺省图注
    #[serde(default)] pub caption_mode: CaptionMode,
    #[serde(default)] pub ai_generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph { text: String },
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
    Figure(FigureBlock),                            // ← 新形态（newtype 变体，内部标签 serde 可处理）
}

#[derive(Debug, Clone, Serialize)]
pub struct BidExportResult {
    pub path: String, pub bytes: u64,
    pub chapters: usize, pub paragraphs: usize, pub tables: usize, pub figures: usize,
}

/// 填充过程统计（fill 返回，命令层映射成 BidExportResult）
#[derive(Debug, Clone, Default)]
pub struct FillStats { pub chapters: usize, pub paragraphs: usize, pub tables: usize, pub figures: usize, pub captions: usize }

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
                    Block::Figure(fig) => {
                        if fig.items.is_empty() {
                            return Err(BidError::new("图模型 items 不能为空"));
                        }
                        match fig.layout {
                            FigureLayout::Single | FigureLayout::Wide => {
                                if fig.items.len() != 1 {
                                    return Err(BidError::new("单图/宽图只能有 1 张图"));
                                }
                            }
                            FigureLayout::ThreeUp => {
                                if fig.items.len() > 3 {
                                    return Err(BidError::new("三栏图桌最多 3 张图"));
                                }
                            }
                        }
                        for (i, item) in fig.items.iter().enumerate() {
                            if item.path.trim().is_empty() {
                                return Err(BidError::new(format!("第 {} 张图片路径不能为空", i + 1)));
                            }
                        }
                        match fig.caption_mode {
                            CaptionMode::PerGroup => {
                                if fig.caption.trim().is_empty() {
                                    return Err(BidError::new("组图注不能为空"));
                                }
                            }
                            CaptionMode::PerImage => {
                                if fig.layout == FigureLayout::ThreeUp {
                                    for (i, item) in fig.items.iter().enumerate() {
                                        if item.caption.as_deref().is_none_or(|c| c.trim().is_empty()) {
                                            return Err(BidError::new(format!("三栏图桌逐图图注不能为空（第 {} 张）", i + 1)));
                                        }
                                    }
                                } else if fig.items[0].caption.as_deref().is_none_or(|c| c.trim().is_empty())
                                    && fig.caption.trim().is_empty()
                                {
                                    return Err(BidError::new("图注不能为空"));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
