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
    /// 该张图片本身是 AI 生成示意图（Markdown 里 `{ai}` 写在图片行尾）：
    /// `per_image` 时**逐图**决定是否追加 `figure.ai_label`，同组其它图不受影响。
    #[serde(default)] pub ai: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigureBlock {
    pub layout: FigureLayout,
    pub items: Vec<FigureItem>,
    #[serde(default)] pub caption: String,          // 组图注 / 单图缺省图注
    #[serde(default)] pub caption_mode: CaptionMode,
    #[serde(default)] pub ai_generated: bool,
}

/// 横道图单工序（P1-5）：`start`/`end` 为 `YYYY-MM-DD`（含首含尾）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GanttTask {
    pub name: String,
    pub start: String,
    pub end: String,
}

/// 横道图块（P1-5）：`title` 可空（空则导出时不加图注）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GanttBlock {
    #[serde(default)]
    pub title: String,
    pub tasks: Vec<GanttTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph { text: String },
    Heading { level: u8, text: String },            // ← P1-1：章内小节标题（渲染 BidTitle{level-1}）
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
    Figure(FigureBlock),                            // ← 新形态（newtype 变体，内部标签 serde 可处理）
    Gantt(GanttBlock),                              // ← P1-5：原生表格横道图
}

// ---------------------------------------------------------------------------
// 日期换算（P1-5 §0：不引日期库，civil-from-days 纯整数算法）
// ---------------------------------------------------------------------------

/// `YYYY-MM-DD` → 距 1970-01-01 的天数；格式非法/日期不存在（如 2 月 30 日）→ `None`
pub fn days_from_ymd(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b[..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let year = s[..4].parse::<i64>().ok()?;
    let month = s[5..7].parse::<u32>().ok()?;
    let day = s[8..].parse::<u32>().ok()?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// 距 1970-01-01 的天数 → `(年, 月, 日)`（civil-from-days，Howard Hinnant）
pub fn ymd_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    const DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month == 2 && is_leap(year) {
        return 29;
    }
    DAYS[month as usize - 1]
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month as i64 + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
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
                    Block::Heading { level, text } => {
                        if !(3..=4).contains(level) {
                            return Err(BidError::new("标题级别非法（3-4）"));
                        }
                        if text.trim().is_empty() {
                            return Err(BidError::new("标题文本为空"));
                        }
                    }
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
                    // P1-5 §1：至少一个工序；工序名非空；日期可解析且 start <= end
                    Block::Gantt(gantt) => {
                        if gantt.tasks.is_empty() {
                            return Err(BidError::new("横道图至少需要一个工序"));
                        }
                        for task in &gantt.tasks {
                            if task.name.trim().is_empty() {
                                return Err(BidError::new("横道图工序名不能为空"));
                            }
                            match (days_from_ymd(&task.start), days_from_ymd(&task.end)) {
                                (Some(s), Some(e)) if s <= e => {}
                                _ => {
                                    return Err(BidError::new(format!(
                                        "横道图日期非法: {}..{}",
                                        task.start, task.end
                                    )));
                                }
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

// ---------------------------------------------------------------------------
// 单测（B2 返工：validate 负例 4 条，不导出文件）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn req_with(chapter: Chapter) -> BidExportRequest {
        BidExportRequest {
            title: "t".to_string(),
            chapters: vec![chapter],
            output_path: String::new(),
            style_id: None,
            figure_width_cm: None,
        }
    }

    fn fig_chapter(layout: FigureLayout, items: Vec<FigureItem>, caption: &str, mode: CaptionMode) -> Chapter {
        Chapter {
            title: "c".to_string(),
            blocks: vec![Block::Figure(FigureBlock {
                layout,
                items,
                caption: caption.to_string(),
                caption_mode: mode,
                ai_generated: false,
            })],
        }
    }

    fn item(path: &str, caption: Option<&str>) -> FigureItem {
        FigureItem { path: path.to_string(), caption: caption.map(str::to_string), ai: false }
    }

    /// B2 新测 1：single 传 2 张图 → Err 且含"单图/宽图只能有 1 张图"
    #[test]
    fn single_two_items_rejected() {
        let ch = fig_chapter(
            FigureLayout::Single,
            vec![item("a.png", None), item("b.png", None)],
            "c",
            CaptionMode::PerImage,
        );
        let err = req_with(ch).validate().unwrap_err();
        assert!(
            err.to_string().contains("单图/宽图只能有 1 张图"),
            "错误信息不符: {err}"
        );
    }

    /// B2 新测 2：three_up 传 4 张图 → Err
    #[test]
    fn three_up_four_items_rejected() {
        let ch = fig_chapter(
            FigureLayout::ThreeUp,
            vec![
                item("a.png", Some("1")),
                item("b.png", Some("2")),
                item("c.png", Some("3")),
                item("d.png", Some("4")),
            ],
            "",
            CaptionMode::PerImage,
        );
        let err = req_with(ch).validate().unwrap_err();
        assert!(
            err.to_string().contains("三栏图桌最多 3 张图"),
            "错误信息不符: {err}"
        );
    }

    /// B2 新测 3：three_up + per_image 且某项无 caption → Err 且含"三栏图桌逐图图注不能为空"
    #[test]
    fn three_up_per_image_missing_caption_rejected() {
        let ch = fig_chapter(
            FigureLayout::ThreeUp,
            vec![
                item("a.png", Some("1")),
                item("b.png", None),
                item("c.png", Some("3")),
            ],
            "",
            CaptionMode::PerImage,
        );
        let err = req_with(ch).validate().unwrap_err();
        assert!(
            err.to_string().contains("三栏图桌逐图图注不能为空"),
            "错误信息不符: {err}"
        );
    }

    /// B2 新测 4：per_group 且组 caption 为空 → Err 且含"图注不能为空"
    #[test]
    fn per_group_empty_caption_rejected() {
        let ch = fig_chapter(
            FigureLayout::Single,
            vec![item("a.png", None)],
            "",
            CaptionMode::PerGroup,
        );
        let err = req_with(ch).validate().unwrap_err();
        assert!(
            err.to_string().contains("图注不能为空"),
            "错误信息不符: {err}"
        );
    }

    fn gantt_chapter(tasks: Vec<GanttTask>) -> Chapter {
        Chapter {
            title: "c".to_string(),
            blocks: vec![Block::Gantt(GanttBlock { title: String::new(), tasks })],
        }
    }

    fn task(name: &str, start: &str, end: &str) -> GanttTask {
        GanttTask {
            name: name.to_string(),
            start: start.to_string(),
            end: end.to_string(),
        }
    }

    /// P1-5：tasks 为空 → Err 且含"横道图至少需要一个工序"
    #[test]
    fn gantt_empty_tasks_rejected() {
        let err = req_with(gantt_chapter(Vec::new())).validate().unwrap_err();
        assert!(
            err.to_string().contains("横道图至少需要一个工序"),
            "错误信息不符: {err}"
        );
    }

    /// P1-5：start > end / 日期不存在 → Err 且含"横道图日期非法"
    #[test]
    fn gantt_bad_dates_rejected() {
        for (start, end) in [
            ("2026-12-20", "2026-12-01"), // 逆序
            ("2026-02-30", "2026-03-01"), // 2 月无 30 日
            ("2026-1-1", "2026-01-02"),   // 格式不符
            ("2026/01/01", "2026-01-02"), // 分隔符不符
        ] {
            let err = req_with(gantt_chapter(vec![task("开挖", start, end)]))
                .validate()
                .unwrap_err();
            assert!(
                err.to_string().contains("横道图日期非法"),
                "{start}..{end} 错误信息不符: {err}"
            );
        }
        // 同日 start == end 合法
        req_with(gantt_chapter(vec![task("开挖", "2026-12-01", "2026-12-01")]))
            .validate()
            .unwrap();
    }

    /// P1-5：日期换算互逆（civil-from-days 纯整数算法），跨闰年与 1970 前均可
    #[test]
    fn date_conversion_roundtrip() {
        for (days, ymd) in [
            (0i64, (1970i64, 1u32, 1u32)),
            (20_454, (2026, 1, 1)),
            (20_639, (2026, 7, 5)),
            (20_788, (2026, 12, 1)),
            (-1, (1969, 12, 31)),
        ] {
            assert_eq!(ymd_from_days(days), ymd, "days={days}");
            let (y, m, d) = ymd;
            let s = format!("{y:04}-{m:02}-{d:02}");
            assert_eq!(days_from_ymd(&s), Some(days), "{s}");
        }
        // 2026-12-01 → 2026-12-25 相隔 24 天（含首含尾 span = 25）
        let span = days_from_ymd("2026-12-25").unwrap() - days_from_ymd("2026-12-01").unwrap() + 1;
        assert_eq!(span, 25);
    }
}
