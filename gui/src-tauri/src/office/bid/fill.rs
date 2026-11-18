//! T2c：填充器（规格 §6.2）。
//!
//! 职责：把 BidExportRequest 灌进模板锚点，产出 docx。
//! - 章模板（BID_CHAPTER）整段取用，逐章替换 TITLE/BODY 后拼接
//! - 表格/图片/图注走 §5.5 片段库（include_str!）
//! - BID_TABLE_ROWS / BID_FIGURE_SINGLE / BID_FIGURE_3UP / BID_FIGURE_WIDE
//!   四个部件库块整块删成空串（BID_TOC 原样保留）
//! - 保存前把 word/header1.xml 的 {{DOC_TITLE}} 替换成文档标题
//! - 版式参数取 DocStyle::default()（P0b §4.2：fill 不新增参数，不做 toml 加载）

use std::path::Path;

use super::docx::{drawing_run, emu_from_cm, escape_xml, image_size_px, DocxPackage};
use super::model::{
    days_from_ymd, ymd_from_days, BidExportRequest, Block, CaptionMode, DocStyle, FigureBlock,
    FigureItem, FigureLayout, FillStats, GanttBlock,
};
use super::template::{apply_edits, content, find, index_sdt, SdtRange};
use super::template_gen::{
    ALL_TAGS, TAG_CHAPTER, TAG_CHAPTER_BODY, TAG_CHAPTER_TITLE, TAG_CHAPTERS, TAG_DOC_TITLE,
    TAG_FIGURE_3UP, TAG_FIGURE_SINGLE, TAG_FIGURE_WIDE, TAG_TABLE_ROWS,
};
use super::{BidError, BidResult};

/// §5.5 片段库（编译期内联）
const PARAGRAPH_XML: &str = include_str!("../../../resources/templates/fragments/paragraph.xml");
const TABLE_XML: &str = include_str!("../../../resources/templates/fragments/table.xml");
const TABLE_ROW_XML: &str = include_str!("../../../resources/templates/fragments/table_row.xml");
const FIGURE_SINGLE_XML: &str =
    include_str!("../../../resources/templates/fragments/figure_single.xml");
const FIGURE_WIDE_XML: &str = include_str!("../../../resources/templates/fragments/figure_wide.xml");
const FIGURE_3UP_XML: &str = include_str!("../../../resources/templates/fragments/figure_3up.xml");
const CAPTION_XML: &str = include_str!("../../../resources/templates/fragments/caption.xml");

/// 3up 不足 3 张时补的空单元格（§4.2：不拉伸、不报错、不生成图注）
const EMPTY_FIGURE_CELL: &str = "<w:tc><w:tcPr><w:tcW w:w=\"3515\" w:type=\"dxa\"/></w:tcPr><w:p><w:pPr><w:pStyle w:val=\"BidFigureCell\"/></w:pPr></w:p></w:tc>";

/// 内容区宽（twips）：A4 宽 11906 - 2×680 页边距 = 10546
const CONTENT_WIDTH_TWIPS: u32 = 10546;

/// §7.7 重要 4：列宽加权——单列占总宽的下限/上限（百分比）
const COL_MIN_PCT: u32 = 12;
const COL_MAX_PCT: u32 = 45;
/// 列权重下限：极短列（空/1 字）也不至于被压成 0 宽
const COL_MIN_WEIGHT: u64 = 4;
/// §7.7 重要 4：行内任一单元格字符数超过该值 → 该行取消 `cantSplit`（允许跨页）
const ROW_SPLIT_CHARS: usize = 60;

/// 横道图（P1-5 §3）：跨度 ≤ 112 天用 7 天/桶，否则 14 天/桶；桶数上限 16
const GANTT_BUCKET_DAYS: i64 = 7;
const GANTT_BUCKET_DAYS_WIDE: i64 = 14;
const GANTT_MAX_BUCKETS: i64 = 16;
/// 横道图首列（工序名）宽
const GANTT_NAME_WIDTH: u32 = 2600;
/// 覆盖桶底纹色（与 BidTableHeader 底色同值；不新增样式）
const GANTT_BAR_FILL: &str = "4877AC";

/// 锚点查找（缺失即报错，供 export 主流程与章内渲染共用）
fn anchor<'a>(ranges: &'a [SdtRange], tag: &str) -> BidResult<&'a SdtRange> {
    find(ranges, tag).ok_or_else(|| BidError::new(format!("模板缺少锚点 {tag}")))
}

/// 导出主流程（规格 §6.2 八步 + P0b §4.2 页眉替换）
pub fn export(req: &BidExportRequest, template: &Path, out: &Path) -> BidResult<(FillStats, u64)> {
    // 1) 校验 + 输出路径非空
    req.validate()?;
    if req.output_path.trim().is_empty() {
        return Err(BidError::new("输出路径为空"));
    }
    // P0b §4.2：版式参数内取默认（不做 toml 加载）
    let style = DocStyle::default();
    // 2) 开包，取 word/document.xml
    let mut pkg = DocxPackage::open(template)?;
    let xml = pkg.text("word/document.xml")?;
    // 3) 锚点索引，ALL_TAGS 逐个 find，缺失即报错
    let ranges = index_sdt(&xml)?;
    for tag in ALL_TAGS {
        anchor(&ranges, tag)?;
    }
    // 4) 部件取用：仅章模板需从原串切片
    let chapter_tpl = content(&xml, anchor(&ranges, TAG_CHAPTER)?);
    // 5) 章渲染：章模板再建索引，逐章替换 TITLE/BODY
    let chapter_ranges = index_sdt(chapter_tpl)?;
    let title_rng = anchor(&chapter_ranges, TAG_CHAPTER_TITLE)?;
    let body_rng = anchor(&chapter_ranges, TAG_CHAPTER_BODY)?;
    let mut chapters_xml = String::new();
    let mut stats = FillStats::default();
    let mut fig_no = 0u32;
    for chapter in &req.chapters {
        let title_para = format!(
            "<w:p><w:pPr><w:pStyle w:val=\"BidTitle1\"/><w:pageBreakBefore/><w:outlineLvl w:val=\"0\"/></w:pPr>\
             <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            escape_xml(&chapter.title)
        );
        let mut body_xml = String::new();
        for block in &chapter.blocks {
            match block {
                Block::Paragraph { .. } => stats.paragraphs += 1,
                Block::Heading { .. } => {}          // P1-1：小节标题不计入 paragraphs
                Block::Table { .. } => stats.tables += 1,
                // P1-5：横道图表不落 BidTableHeader，不计入 tables（与 verify doc.tables 同口径）
                Block::Gantt(_) => {}
                Block::Figure(fig) => stats.figures += fig.items.len(),
            }
            let (block_xml, captions) =
                render_block(block, &mut pkg, &mut fig_no, req.figure_width_cm, &style)?;
            stats.captions += captions;
            body_xml.push_str(&block_xml);
        }
        let mut edits = vec![
            (title_rng.content_start, title_rng.content_end, title_para),
            (body_rng.content_start, body_rng.content_end, body_xml),
        ];
        chapters_xml.push_str(&apply_edits(chapter_tpl, &mut edits)?);
    }
    stats.chapters = req.chapters.len();
    // 6) 文档装配：标题段 / 章区 / 四个部件库块整块删除（BID_TOC 不动）
    let doc_title = anchor(&ranges, TAG_DOC_TITLE)?;
    let chapters = anchor(&ranges, TAG_CHAPTERS)?;
    let table_rows = anchor(&ranges, TAG_TABLE_ROWS)?;
    let figure_single = anchor(&ranges, TAG_FIGURE_SINGLE)?;
    let figure_3up = anchor(&ranges, TAG_FIGURE_3UP)?;
    let figure_wide = anchor(&ranges, TAG_FIGURE_WIDE)?;
    let doc_title_para = format!(
        "<w:p><w:pPr><w:pStyle w:val=\"BidTitle1\"/><w:outlineLvl w:val=\"9\"/></w:pPr>\
         <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        escape_xml(&req.title)
    );
    let mut edits = vec![
        (doc_title.content_start, doc_title.content_end, doc_title_para),
        (chapters.content_start, chapters.content_end, chapters_xml),
        (table_rows.block_start, table_rows.block_end, String::new()),
        (figure_single.block_start, figure_single.block_end, String::new()),
        (figure_3up.block_start, figure_3up.block_end, String::new()),
        (figure_wide.block_start, figure_wide.block_end, String::new()),
    ];
    let new_xml = apply_edits(&xml, &mut edits)?;
    // 7) 写回 document.xml
    pkg.set_text("word/document.xml", new_xml);
    // 8) 保存前替换页眉标题（P0b §4.2）
    if pkg.contains("word/header1.xml") {
        let hdr = pkg.text("word/header1.xml")?;
        pkg.set_text("word/header1.xml", hdr.replace("{{DOC_TITLE}}", &escape_xml(&req.title)));
    }
    // 9) 保存前重写 core.xml（P0c §2：暗标元数据清空）；模板缺该部件时按同规格新建，
    //    否则 doc.metadata_anonymous 会因 core.xml 缺失而必然失败
    pkg.set_text("docProps/core.xml", core_xml(&req.title));
    // 10) 父目录不存在先创建，再保存
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    pkg.save(out)?;
    Ok((stats, std::fs::metadata(out)?.len()))
}

/// P0c §2 固定 core.xml：title=本次标题（escape_xml），creator/lastModifiedBy 置空；
/// 不写 cp:revision / dcterms:created/modified（避免时间戳泄露）
fn core_xml(title: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n  \
<dc:title>{}</dc:title>\n  \
<dc:creator></dc:creator>\n  \
<cp:lastModifiedBy></cp:lastModifiedBy>\n\
</cp:coreProperties>",
        escape_xml(title)
    )
}

/// 块渲染（§6.4）：段落 → paragraph 片段；表格 → render_table；图 → render_figure。
/// 返回 (块 xml, 本块新增的图注段数)
fn render_block(
    block: &Block,
    pkg: &mut DocxPackage,
    fig_no: &mut u32,
    figure_width_cm: Option<f32>,
    style: &DocStyle,
) -> BidResult<(String, usize)> {
    match block {
        Block::Paragraph { text } => Ok((
            PARAGRAPH_XML
                .replace("{{STYLE}}", "BidBody")
                .replace("{{TEXT}}", &escape_xml(text)),
            0,
        )),
        // P1-1：小节标题复用段落片段，样式名取 BidTitle{level-1}（level 3..=4 由 validate 保证；
        // H3→BidTitle2、H4→BidTitle3，BidTitle1 留给章标题）
        Block::Heading { level, text } => Ok((
            PARAGRAPH_XML
                .replace("{{STYLE}}", &format!("BidTitle{}", level - 1))
                .replace("{{TEXT}}", &escape_xml(text)),
            0,
        )),
        Block::Table { header, rows } => Ok((render_table(header, rows, style)?, 0)),
        Block::Gantt(gantt) => render_gantt(gantt),
        Block::Figure(fig) => render_figure(pkg, fig_no, fig, style, figure_width_cm),
    }
}

/// 横道图渲染（P1-5 §3）：全局 `min_start`/`max_end` → 时间桶（7 天，跨度 > 112 天改 14 天）
/// → 原生表格（`tblStyle=BidTable`，复用 `table.xml`/`table_row.xml`）。
///
/// 硬约束：**所有单元格（含表头行）一律 `BidTableBody`**，表头行也不用 `BidTableHeader`，
/// 因此业务表计数（`doc.tables` 按 `BidTableHeader` 切块）不受影响；行片段的
/// `{{CANTS_SPLIT}}` 固定 `<w:cantSplit/>`、`{{HEADER_FLAG}}` 为空。
///
/// 覆盖判定：任务覆盖某桶 ⇔ `task_start < bucket_end && task_end >= bucket_start`。
/// 标题非空 → 表格后追加一段 `BidCaption`；返回 (块 xml, 图注段数)。
fn render_gantt(gantt: &GanttBlock) -> BidResult<(String, usize)> {
    let empty_err = || BidError::new("横道图至少需要一个工序");
    if gantt.tasks.is_empty() {
        return Err(empty_err());
    }
    // 日期 → 天数（validate 已校验；此处仍兜底，避免直接调用时 panic）
    let mut spans: Vec<(i64, i64)> = Vec::with_capacity(gantt.tasks.len());
    for task in &gantt.tasks {
        match (days_from_ymd(&task.start), days_from_ymd(&task.end)) {
            (Some(s), Some(e)) if s <= e => spans.push((s, e)),
            _ => {
                return Err(BidError::new(format!(
                    "横道图日期非法: {}..{}",
                    task.start, task.end
                )));
            }
        }
    }
    let min_start = spans.iter().map(|(s, _)| *s).min().ok_or_else(empty_err)?;
    let max_end = spans.iter().map(|(_, e)| *e).max().ok_or_else(empty_err)?;
    let span = max_end - min_start + 1;
    let bucket_days = if span <= 112 { GANTT_BUCKET_DAYS } else { GANTT_BUCKET_DAYS_WIDE };
    let buckets = (span + bucket_days - 1) / bucket_days;
    if buckets > GANTT_MAX_BUCKETS {
        return Err(BidError::new("横道图跨度过大（>16 个时间桶）"));
    }
    let cols = buckets as usize + 1;

    // 列宽：首列 2600 twips，其余均分余宽（整除余数给首列）。
    // 横道图列宽按时间桶均分（列内容恒为空/一格），不参与 §7.7 的业务表列宽加权
    let rest = CONTENT_WIDTH_TWIPS - GANTT_NAME_WIDTH;
    let base = rest / buckets as u32;
    let rem = rest % buckets as u32;
    let mut widths: Vec<u32> = Vec::with_capacity(cols);
    widths.push(GANTT_NAME_WIDTH + rem);
    widths.extend(std::iter::repeat_n(base, buckets as usize));
    let grid: String = widths
        .iter()
        .map(|w| format!("<w:gridCol w:w=\"{w}\"/>"))
        .collect();

    // 表头行：`工序` + 各桶起始日（桶按自然日切，从 min_start 起算）。
    // 跨年（min_start 与 max_end 不同年）时带年份 `YYYY-MM-DD`，否则仍 `MM-DD`（列窄）
    let cross_year = ymd_from_days(min_start).0 != ymd_from_days(max_end).0;
    let mut header: Vec<String> = Vec::with_capacity(cols);
    header.push("工序".to_string());
    for b in 0..buckets {
        let (year, month, day) = ymd_from_days(min_start + b * bucket_days);
        header.push(if cross_year {
            format!("{year:04}-{month:02}-{day:02}")
        } else {
            format!("{month:02}-{day:02}")
        });
    }
    let mut rows_xml = render_gantt_row(&widths, &header, None);

    // 正文行：工序名 + 每桶一格（覆盖则加底纹、无文本）
    for (idx, task) in gantt.tasks.iter().enumerate() {
        let (task_start, task_end) = spans[idx];
        let mut fills: Vec<bool> = Vec::with_capacity(cols);
        fills.push(false);
        for b in 0..buckets {
            let bucket_start = min_start + b * bucket_days;
            let bucket_end = bucket_start + bucket_days;
            fills.push(task_start < bucket_end && task_end >= bucket_start);
        }
        let mut cells: Vec<String> = Vec::with_capacity(cols);
        cells.push(task.name.trim().to_string());
        cells.extend(std::iter::repeat_n(String::new(), buckets as usize));
        rows_xml.push_str(&render_gantt_row(&widths, &cells, Some(&fills)));
    }

    let mut out = TABLE_XML
        .replace("{{GRID}}", &grid)
        .replace("{{ROWS}}", &rows_xml);
    let title = gantt.title.trim();
    if title.is_empty() {
        return Ok((out, 0));
    }
    out.push_str(&CAPTION_XML.replace("{{TEXT}}", &escape_xml(title)));
    Ok((out, 1))
}

/// 横道图单行（P1-5 §3.5）：复用 `table_row.xml`，`{{CANTS_SPLIT}}`=`<w:cantSplit/>`、
/// `{{HEADER_FLAG}}`=空；`fills` 为 `None`（表头行）或每列是否加 `4877AC` 底纹。
/// 文本为空时不输出 run（bar 格只有空段落，与 §3.4 片段一致）。
fn render_gantt_row(widths: &[u32], texts: &[String], fills: Option<&[bool]>) -> String {
    let mut cells_xml = String::new();
    for (i, text) in texts.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(0);
        let shd = match fills {
            Some(f) if f.get(i).copied().unwrap_or(false) => format!(
                "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{GANTT_BAR_FILL}\"/>"
            ),
            _ => String::new(),
        };
        let run = if text.is_empty() {
            String::new()
        } else {
            format!(
                "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
                escape_xml(text)
            )
        };
        cells_xml.push_str(&format!(
            "<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/>{shd}</w:tcPr>\
             <w:p><w:pPr><w:pStyle w:val=\"BidTableBody\"/></w:pPr>{run}</w:p></w:tc>"
        ));
    }
    TABLE_ROW_XML
        .replace("{{CANTS_SPLIT}}", "<w:cantSplit/>")
        .replace("{{HEADER_FLAG}}", "")
        .replace("{{CELLS}}", &cells_xml)
}

/// 表格渲染（§6.4 + §5.5 table.xml / table_row.xml + §7.7 重要 4 列宽加权）
fn render_table(header: &[String], rows: &[Vec<String>], style: &DocStyle) -> BidResult<String> {
    let cols = header.len();
    if cols == 0 {
        return Err(BidError::new("表格表头不能为空"));
    }
    // 列宽：按列内容长度加权（非等分）；gridCol 与单元格 tcW 共用同一组宽度
    let widths = table_column_widths(header, rows, CONTENT_WIDTH_TWIPS);
    let grid: String = widths
        .iter()
        .map(|w| format!("<w:gridCol w:w=\"{w}\"/>"))
        .collect();

    // 数据行首列底纹（first_col_fill 空串 → 不输出该元素）
    let first_col_shd = if style.table.first_col_fill.is_empty() {
        String::new()
    } else {
        format!(
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            style.table.first_col_fill
        )
    };

    // cant_split：true → <w:cantSplit/>，false → 空串（表头行/数据行一致）
    let cant_split = if style.table.cant_split { "<w:cantSplit/>" } else { "" };
    // §7.7 重要 4：单行任一单元格 > 60 字 → 该行不 cantSplit，允许跨页
    //（否则整行被推到下一页，留下大片空白）
    let row_cant_split = |cells: &[String]| {
        if cells.iter().any(|c| char_len(c) > ROW_SPLIT_CHARS) {
            ""
        } else {
            cant_split
        }
    };

    let mut rows_xml = String::new();
    // 表头行：{{HEADER_FLAG}} = <w:tblHeader/>，单元格样式 BidTableHeader，首列不加底纹
    rows_xml.push_str(&render_row(
        &widths,
        header,
        "<w:tblHeader/>",
        "BidTableHeader",
        "",
        row_cant_split(header),
    ));
    // 数据行：flag 空串 + BidTableBody + 首列底纹
    for row in rows {
        rows_xml.push_str(&render_row(
            &widths,
            row,
            "",
            "BidTableBody",
            &first_col_shd,
            row_cant_split(row),
        ));
    }
    Ok(TABLE_XML
        .replace("{{GRID}}", &grid)
        .replace("{{ROWS}}", &rows_xml))
}

/// 字符数（按 Unicode 标量计：中文一字算 1）
fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// 列宽（§7.7 重要 4）：按列内容长度加权分配内容区宽度。
///
/// - 每列权重 = 该列（表头 + 全部数据行）单元格字符数的最大值，下限 `COL_MIN_WEIGHT`；
/// - 按权重初分后把单列夹到 `COL_MIN_PCT`..`COL_MAX_PCT`（12%..45%），越界列钉死，
///   余量按权重分给未夹列（多轮，直到不再有列越界）；
/// - 下限之和已超总宽（列数 ≥ 9）→ 全列按权重等比回缩（§7.7「不足时按比例回缩」）；
/// - 全部列都被夹死后仍有余量（3 列及以上）→ 先填「只触下限」的列（仍未超 45%）；
/// - 45% 上限本身不可满足（2 列：2×45% < 100%）→ 余量按权重摊给所有列，表格须占满内容宽。
///
/// 返回宽度之和恒等于 `total`。
fn table_column_widths(header: &[String], rows: &[Vec<String>], total: u32) -> Vec<u32> {
    let cols = header.len();
    if cols == 0 {
        return Vec::new();
    }
    let weights: Vec<u64> = (0..cols)
        .map(|c| {
            let mut max = header.get(c).map_or(0, |t| char_len(t));
            for row in rows {
                if let Some(t) = row.get(c) {
                    max = max.max(char_len(t));
                }
            }
            (max as u64).max(COL_MIN_WEIGHT)
        })
        .collect();
    let sum_w: u64 = weights.iter().sum();
    let min_w = (total as u64 * COL_MIN_PCT as u64 / 100) as f64;
    let max_w = (total as u64 * COL_MAX_PCT as u64 / 100) as f64;
    let max_cap = max_w.round() as u64;

    // 列太多：下限之和已超过总宽 → 全列按权重等比回缩
    if min_w * cols as f64 > total as f64 {
        return scale_to_total(&weights, sum_w, total);
    }

    let mut widths = vec![0u64; cols];
    let mut pinned = vec![false; cols];
    let mut remaining = total as f64;
    let mut free_weight = sum_w as f64;
    for _ in 0..cols {
        let mut any = false;
        for i in 0..cols {
            if pinned[i] || free_weight <= 0.0 {
                continue;
            }
            let share = remaining * (weights[i] as f64) / free_weight;
            if share < min_w {
                widths[i] = min_w.round() as u64;
                pinned[i] = true;
                remaining -= min_w.round();
                free_weight -= weights[i] as f64;
                any = true;
            } else if share > max_w {
                widths[i] = max_w.round() as u64;
                pinned[i] = true;
                remaining -= max_w.round();
                free_weight -= weights[i] as f64;
                any = true;
            } else {
                widths[i] = share.round() as u64;
            }
        }
        if !any {
            break;
        }
    }

    // 余量重分：未夹列 → 只触下限的列（≤45%）→（上限不可满足时）所有列
    let mut remain = total as i64 - widths.iter().sum::<u64>() as i64;
    if remain != 0 {
        if (cols as u64) * max_cap < total as u64 {
            // 2 列：45% 上限与「占满内容宽」不可兼得 → 按权重摊给所有列
            let all: Vec<usize> = (0..cols).collect();
            remain = spread_by_weight(&mut widths, &weights, &all, remain, u64::MAX);
        } else {
            let mut targets: Vec<usize> = (0..cols).filter(|&i| !pinned[i]).collect();
            if targets.is_empty() {
                targets = (0..cols).filter(|&i| widths[i] < max_cap).collect();
            }
            if !targets.is_empty() {
                remain = spread_by_weight(&mut widths, &weights, &targets, remain, max_cap);
            }
            if remain != 0 {
                let all: Vec<usize> = (0..cols).collect();
                remain = spread_by_weight(&mut widths, &weights, &all, remain, u64::MAX);
            }
        }
        // 兜底（理论不可达）：仍有余额就塞给当前最宽列，保证总和 == total
        while remain > 0 {
            let widest = (0..cols).max_by_key(|&i| widths[i]).unwrap_or(0);
            widths[widest] += 1;
            remain -= 1;
        }
    }
    widths.into_iter().map(|w| w as u32).collect()
}

/// 按权重把差额 `diff`（正=加、负=减）摊给 `targets`；加时单列不超过 `cap`。
/// 返回未能分配完的余量（全部目标列触 `cap` 时非 0）。
fn spread_by_weight(
    widths: &mut [u64],
    weights: &[u64],
    targets: &[usize],
    diff: i64,
    cap: u64,
) -> i64 {
    let mut remain = diff;
    if targets.is_empty() {
        return remain;
    }
    let total_weight = targets.iter().map(|&i| weights[i]).sum::<u64>().max(1);
    // 每轮把「按权重的份额」分给所有目标列；触 cap 的列本轮分不到，
    // 余下的差额进下一轮（最多 targets.len()+1 轮即可收敛，再加兜底一轮）
    for _ in 0..=targets.len() {
        if remain == 0 {
            break;
        }
        let want = remain.unsigned_abs();
        let mut moved: i64 = 0;
        for &i in targets {
            if remain == 0 {
                break;
            }
            let part = (want * weights[i] / total_weight).max(1).min(remain.unsigned_abs());
            if remain > 0 {
                let add = part.min(cap.saturating_sub(widths[i]));
                widths[i] += add;
                moved += add as i64;
            } else {
                let sub = part.min(widths[i]);
                widths[i] -= sub;
                moved -= sub as i64;
            }
        }
        remain -= moved;
        // 整除余数兜给最后一个目标列（受 cap 约束）
        if remain > 0 {
            if let Some(&last) = targets.last() {
                let add = (remain as u64).min(cap.saturating_sub(widths[last]));
                widths[last] += add;
                remain -= add as i64;
            }
        } else if remain < 0 {
            if let Some(&last) = targets.last() {
                let sub = remain.unsigned_abs().min(widths[last]);
                widths[last] -= sub;
                remain += sub as i64;
            }
        }
    }
    remain
}

/// 全列按权重等比缩放到 `total`（余数逐 twip 摊给前面的列）
fn scale_to_total(weights: &[u64], sum_w: u64, total: u32) -> Vec<u32> {
    let sum_w = sum_w.max(1);
    let mut widths: Vec<u64> = weights
        .iter()
        .map(|&w| total as u64 * w / sum_w)
        .collect();
    let assigned: u64 = widths.iter().sum();
    let mut rest = total as i64 - assigned as i64;
    let mut i = 0usize;
    while rest > 0 {
        widths[i % weights.len()] += 1;
        rest -= 1;
        i += 1;
    }
    widths.into_iter().map(|w| w as u32).collect()
}

/// 单行渲染（§5.5 table_row.xml）：表头行带 <w:tblHeader/>，数据行 flag 为空串；
/// `cant_split` 由 `style.table.cant_split` 决定（<w:cantSplit/> 或空串）；
/// `first_col_shd` 只用于数据行首列 tcPr（表头行传空串）
fn render_row(
    widths: &[u32],
    cells: &[String],
    header_flag: &str,
    style: &str,
    first_col_shd: &str,
    cant_split: &str,
) -> String {
    let mut cells_xml = String::new();
    for (i, text) in cells.iter().enumerate() {
        let w = widths.get(i).copied().unwrap_or(0);
        let shd = if i == 0 { first_col_shd } else { "" };
        cells_xml.push_str(&format!(
            "<w:tc><w:tcPr><w:tcW w:w=\"{w}\" w:type=\"dxa\"/>{shd}</w:tcPr>\
             <w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>\
             <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>",
            escape_xml(text)
        ));
    }
    TABLE_ROW_XML
        .replace("{{CANTS_SPLIT}}", cant_split)
        .replace("{{HEADER_FLAG}}", header_flag)
        .replace("{{CELLS}}", &cells_xml)
}

/// 图块渲染（§4.2 三布局）：按 fig.layout 分支。
/// 返回 (块 xml, 本块生成的图注段数)
fn render_figure(
    pkg: &mut DocxPackage,
    fig_no: &mut u32,
    fig: &FigureBlock,
    style: &DocStyle,
    figure_width_cm: Option<f32>,
) -> BidResult<(String, usize)> {
    match fig.layout {
        FigureLayout::Single | FigureLayout::Wide => {
            // single 走请求宽（默认 15.5cm）；wide 走内容区全宽
            let width_cm = match fig.layout {
                FigureLayout::Single => figure_width_cm.unwrap_or(15.5),
                _ => style.figure.wide_width_cm,
            };
            let item = &fig.items[0];
            let caption = caption_text(item, fig, style);
            let drawing = render_drawing(pkg, fig_no, &item.path, width_cm, 20.0, &caption)?;
            let caption_xml = CAPTION_XML.replace("{{TEXT}}", &escape_xml(&caption));
            let tpl = match fig.layout {
                FigureLayout::Wide => FIGURE_WIDE_XML,
                _ => FIGURE_SINGLE_XML,
            };
            Ok((
                tpl.replace("{{DRAWING}}", &drawing).replace("{{CAPTION}}", &caption_xml),
                1,
            ))
        }
        FigureLayout::ThreeUp => {
            let cell_w_cm = style.figure.cell_width_cm;
            let per_image = matches!(fig.caption_mode, CaptionMode::PerImage);
            let mut cells = String::new();
            let mut captions = 0usize;
            for item in &fig.items {
                let caption = caption_text(item, fig, style);
                // 每格：图片段 +（per_image 时）图注段；宽 cell_width_cm，高上限 10cm
                let drawing = render_drawing(pkg, fig_no, &item.path, cell_w_cm, 10.0, &caption)?;
                let cap = if per_image {
                    captions += 1;
                    CAPTION_XML.replace("{{TEXT}}", &escape_xml(&caption))
                } else {
                    String::new()
                };
                cells.push_str(&format!(
                    "<w:tc><w:tcPr><w:tcW w:w=\"3515\" w:type=\"dxa\"/></w:tcPr>\
                     <w:p><w:pPr><w:pStyle w:val=\"BidFigureCell\"/><w:spacing w:line=\"240\" w:lineRule=\"auto\"/><w:jc w:val=\"center\"/></w:pPr>{drawing}</w:p>{cap}</w:tc>"
                ));
            }
            // 不足 3 张补空单元格（§4.2：不拉伸、不报错、不生成图注）
            for _ in fig.items.len()..3 {
                cells.push_str(EMPTY_FIGURE_CELL);
            }
            let mut out = FIGURE_3UP_XML.replace("{{CELLS}}", &cells);
            if !per_image {
                // per_group：组图注放表格之后 1 段
                out.push_str(&CAPTION_XML.replace(
                    "{{TEXT}}",
                    &escape_xml(&caption_text(&fig.items[0], fig, style)),
                ));
                captions = 1;
            }
            Ok((out, captions))
        }
    }
}

/// 图注文本（§4.2 唯一真值）：per_image 取 item.caption（single/wide 回退 fig.caption）；
/// per_group 取 fig.caption；AI 标注按模式取：
/// `per_image` → **逐图 `item.ai`**（同组非 AI 图不误标），
/// `per_group` → 组级 `fig.ai_generated`（组图注只有一条）。
fn caption_text(item: &FigureItem, fig: &FigureBlock, style: &DocStyle) -> String {
    let (base, ai) = match fig.caption_mode {
        CaptionMode::PerImage => (
            item.caption.clone().unwrap_or_else(|| fig.caption.clone()),
            item.ai,
        ),
        CaptionMode::PerGroup => (fig.caption.clone(), fig.ai_generated),
    };
    if ai {
        format!("{base}{}", style.figure.ai_label)
    } else {
        base
    }
}

/// 图片 drawing 段（§6.3 尺寸算法）：宽 = width_cm；高按像素比换算；
/// 高超过 max_h_cm 上限时按高反算宽，保持比例
fn render_drawing(
    pkg: &mut DocxPackage,
    fig_no: &mut u32,
    path: &str,
    width_cm: f32,
    max_h_cm: f32,
    descr: &str,
) -> BidResult<String> {
    let (px_w, px_h) = image_size_px(Path::new(path))?;
    let mut width_em = emu_from_cm(width_cm);
    let mut height_em = (width_em * px_h as u64 + px_w as u64 / 2) / px_w as u64;
    // 高超过上限按高反算宽，保持比例
    let max_h_em = emu_from_cm(max_h_cm);
    if height_em > max_h_em {
        width_em = max_h_em * px_w as u64 / px_h as u64;
        height_em = max_h_em;
    }
    *fig_no += 1;
    let png = std::fs::read(path)?;
    let rid = pkg.add_png(&png, &format!("fig-{fig_no}"))?;
    Ok(drawing_run(&rid, width_em, height_em, *fig_no, descr))
}

// ---------------------------------------------------------------------------
// 单测（规格 §6.2 验收：最小样本导出 / 转义 / 缺锚点报错 / 3up per_group）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use super::super::model::{Chapter, GanttTask};
    use super::super::verify::{self, Expect};
    use std::collections::BTreeMap;

    const TEMPLATE: &str = "resources/templates/bid-engineering-blue-v1.docx";
    fn asset_png() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/bid-export/assets/image3.png")
            .to_string_lossy()
            .to_string()
    }

    fn minimal_req(body: &str, out: &Path) -> BidExportRequest {
        BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: body.to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        }
    }

    /// 最小样本（§7.1 口径）：2 章 4 段 1 表 1 图 → verify 16 项全过
    #[test]
    fn export_minimal_project_passes_verify() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-fill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("minimal.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "槐林镇2026年度和美乡村省级中心村建设项目施工组织设计".to_string(),
            chapters: vec![
                Chapter {
                    title: "第一章 工程概况".to_string(),
                    blocks: vec![
                        Block::Paragraph { text: "本工程为槐林镇2026年度和美乡村省级中心村建设项目，主要建设内容包括道路硬化、排水管网、文化广场及绿化提升。".to_string() },
                        Block::Paragraph { text: "施工组织设计遵循先地下后地上、先深后浅的原则，确保工序衔接与成品保护。".to_string() },
                        Block::Table {
                            header: vec!["工序节点".into(), "施工方法及控制参数".into(), "质量控制要点".into()],
                            rows: vec![
                                vec!["塘底清淤".into(), "长臂挖机分段清淤，深度偏差≤50mm".into(), "隐蔽验收合格后方可回填".into()],
                                vec!["路基整平".into(), "推土机粗平+平地机精平，压实度≥93%".into(), "每200m检测一点".into()],
                            ],
                        },
                        Block::Figure(FigureBlock {
                            layout: FigureLayout::Single,
                            items: vec![FigureItem {
                                path: img.clone(),
                                caption: Some("图1-1 塘底整平与排水示意".to_string()),
                                ai: true,
                            }],
                            caption: "图1-1 塘底整平与排水示意".to_string(),
                            caption_mode: CaptionMode::PerImage,
                            ai_generated: true,
                        }),
                    ],
                },
                Chapter {
                    title: "第二章 施工部署".to_string(),
                    blocks: vec![
                        Block::Paragraph { text: "项目部设项目经理1人、技术负责人1人，按专业划分三个作业队。".to_string() },
                        Block::Paragraph { text: "总工期90日历天，其中土方与管网45天，道路与广场30天，绿化及收尾15天。".to_string() },
                    ],
                },
            ],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        let (stats, bytes) = export(&req, Path::new(TEMPLATE), &out).unwrap();
        assert_eq!((stats.chapters, stats.paragraphs, stats.tables, stats.figures, stats.captions), (2, 4, 1, 1, 1));
        assert!(bytes > 0);
        let report = verify::verify(&out, Some(&Expect { chapters: 2, figures: 1, captions: 1, tables: 1, paragraphs: 4 })).unwrap();
        let bad: Vec<String> = report.checks.iter().filter(|c| !c.ok)
            .map(|c| format!("{}:{}", c.name, c.detail)).collect();
        assert!(bad.is_empty(), "不合格 check: {}", bad.join("; "));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 正文含 <&>"' → document.xml 必须含对应实体串
    #[test]
    fn export_escapes_text() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("esc.docx");
        let req = minimal_req("<&>\"'", &out);
        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert!(
            doc.contains("&lt;&amp;&gt;&quot;&apos;"),
            "document.xml 未含五实体转义串: {doc}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// P1-1：Heading → BidTitle{level-1}（level 3→BidTitle2、4→BidTitle3 各一段），
    /// 且不计入 stats.paragraphs。
    /// 计数依据：先查模板 `word/document.xml`，实测 BidTitle1=2、BidTitle2/3/4=0 处
    /// （BidTitle2/3/4 仅出现在 `word/styles.xml` 的样式定义里，而导出件只读 document.xml，
    /// 样式定义不参与计数）→ 导出件中的 BidTitle2/3 全部来自本次 Heading 渲染。
    #[test]
    fn heading_renders_expected_styles() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p11-hdg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("heading.docx");
        let req = BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "第一章 工程概况".to_string(),
                blocks: vec![
                    Block::Heading { level: 3, text: "1.1.1 人员".to_string() },
                    Block::Heading { level: 4, text: "1.1.1.1 班组".to_string() },
                    Block::Paragraph { text: "正文".to_string() },
                ],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        let (stats, _) = export(&req, Path::new(TEMPLATE), &out).unwrap();
        assert_eq!(stats.paragraphs, 1, "Heading 不应计入 paragraphs");
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        for level in 2..=3u8 {
            let needle = format!("w:pStyle w:val=\"BidTitle{level}\"");
            assert!(
                doc.matches(&needle).count() >= 1,
                "导出件缺 {needle}: {doc}"
            );
        }
        // 样式与文本配对（level 3/4 的标题文本分别落在 BidTitle2/BidTitle3 段里）
        assert!(
            doc.contains(r#"<w:pStyle w:val="BidTitle2"/></w:pPr><w:r><w:t xml:space="preserve">1.1.1 人员</w:t>"#),
            "level 3 标题未渲染到 BidTitle2: {doc}"
        );
        assert!(
            doc.contains(r#"<w:pStyle w:val="BidTitle3"/></w:pPr><w:r><w:t xml:space="preserve">1.1.1.1 班组</w:t>"#),
            "level 4 标题未渲染到 BidTitle3: {doc}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// P1-1：level=2 / level=5 → validate Err 且含"标题级别非法（3-4）"
    #[test]
    fn heading_invalid_level_rejected() {
        for level in [2u8, 5u8] {
            let req = BidExportRequest {
                title: "t".to_string(),
                chapters: vec![Chapter {
                    title: "c".to_string(),
                    blocks: vec![Block::Heading { level, text: "小节".to_string() }],
                }],
                output_path: String::new(),
                style_id: None,
                figure_width_cm: None,
            };
            let err = req.validate().unwrap_err();
            assert!(
                err.to_string().contains("标题级别非法（3-4）"),
                "level={level} 错误信息不符: {err}"
            );
        }
    }

    /// 自造无锚点最小包 → export 必须 Err 且信息含"模板缺少锚点"
    #[test]
    fn export_missing_anchor_fails() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-miss-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tpl = dir.join("tpl.docx");
        let mut pkg = DocxPackage { entries: BTreeMap::new() };
        pkg.set_text("[Content_Types].xml", "<Types/>".to_string());
        pkg.set_text("_rels/.rels", "<Relationships/>".to_string());
        pkg.set_text("word/document.xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body/></w:document>".to_string());
        pkg.set_text("word/_rels/document.xml.rels", "<Relationships/>".to_string());
        pkg.save(&tpl).unwrap();

        let out = dir.join("out.docx");
        let err = export(&minimal_req("b", &out), &tpl, &out).unwrap_err();
        assert!(err.to_string().contains("模板缺少锚点"), "错误信息不符: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B2a 新测：3up per_group 3 张图 → drawing=3、BidCaption=1、<w:tc>=3 且 <w:tr>=1
    #[test]
    fn three_up_per_group_has_one_caption() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-3up-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("threeup.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Figure(FigureBlock {
                    layout: FigureLayout::ThreeUp,
                    items: vec![
                        FigureItem { path: img.clone(), caption: None, ai: false },
                        FigureItem { path: img.clone(), caption: None, ai: false },
                        FigureItem { path: img.clone(), caption: None, ai: false },
                    ],
                    caption: "图1-1 三栏总示意".to_string(),
                    caption_mode: CaptionMode::PerGroup,
                    ai_generated: false,
                })],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        let (stats, _) = export(&req, Path::new(TEMPLATE), &out).unwrap();
        assert_eq!((stats.figures, stats.captions), (3, 1));
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert_eq!(doc.matches("<w:drawing").count(), 3, "3up 须 3 个 drawing: {doc}");
        assert_eq!(doc.matches("BidCaption").count(), 1, "per_group 只 1 段图注: {doc}");
        assert_eq!(doc.matches("<w:tc>").count(), 3, "3up 须 3 个单元格: {doc}");
        assert_eq!(doc.matches("<w:tr>").count(), 1, "3up 须 1 行: {doc}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// B2 新测 7：3up per_image 3 图 → <w:drawing>=3、BidCaption=3、<w:tc>=3
    #[test]
    fn three_up_per_image_three_captions() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-3up-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("threeup-cap.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Figure(FigureBlock {
                    layout: FigureLayout::ThreeUp,
                    items: vec![
                        FigureItem { path: img.clone(), caption: Some("图1-1 甲".to_string()), ai: false },
                        FigureItem { path: img.clone(), caption: Some("图1-2 乙".to_string()), ai: false },
                        FigureItem { path: img.clone(), caption: Some("图1-3 丙".to_string()), ai: false },
                    ],
                    caption: String::new(),
                    caption_mode: CaptionMode::PerImage,
                    ai_generated: false,
                })],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert_eq!(doc.matches("<w:drawing").count(), 3, "3 图须 3 个 drawing: {doc}");
        assert_eq!(doc.matches("BidCaption").count(), 3, "per_image 须 3 段图注: {doc}");
        assert_eq!(doc.matches("<w:tc>").count(), 3, "3up 须 3 个单元格: {doc}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// N1 新测：3 张里只有第 2 张带 `{ai}` → 导出件仅 1 条图注追加「（AI 生成示意图）」
    #[test]
    fn three_up_marks_only_ai_items() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-n1-ai-item-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("threeup-ai.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Figure(FigureBlock {
                    layout: FigureLayout::ThreeUp,
                    items: vec![
                        FigureItem { path: img.clone(), caption: Some("图1-1 甲".to_string()), ai: false },
                        FigureItem { path: img.clone(), caption: Some("图1-2 乙".to_string()), ai: true },
                        FigureItem { path: img.clone(), caption: Some("图1-3 丙".to_string()), ai: false },
                    ],
                    caption: String::new(),
                    caption_mode: CaptionMode::PerImage,
                    // 组级也为真（Markdown 解析器口径：任一 item.ai → 组级 true），
                    // 但 per_image 逐图标注只看各 item.ai，不得把同组非 AI 图一起标
                    ai_generated: true,
                })],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert_eq!(doc.matches("BidCaption").count(), 3, "per_image 须 3 段图注: {doc}");
        // 图注 run 里的标注只 1 处（descpr/descr 属性里也会带标注，故按 `</w:t>` 收尾计数）
        assert_eq!(
            doc.matches("（AI 生成示意图）</w:t>").count(),
            1,
            "只有第 2 张该带 AI 标注: {doc}"
        );
        assert!(
            doc.contains("<w:t xml:space=\"preserve\">图1-2 乙（AI 生成示意图）</w:t>"),
            "AI 标注应紧跟第 2 张图注: {doc}"
        );
        assert!(!doc.contains("图1-1 甲（AI 生成示意图）"), "第 1 张不得被误标: {doc}");
        assert!(!doc.contains("图1-3 丙（AI 生成示意图）"), "第 3 张不得被误标: {doc}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// B2 新测 8：wide 1 图 → drawing=1 且 cx="6696000"（18.6cm 内容区全宽）
    #[test]
    fn wide_uses_full_content_width() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-wide-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("wide.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Figure(FigureBlock {
                    layout: FigureLayout::Wide,
                    items: vec![FigureItem { path: img.clone(), caption: Some("图1-1 总平面".to_string()), ai: false }],
                    caption: String::new(),
                    caption_mode: CaptionMode::PerImage,
                    ai_generated: false,
                })],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert_eq!(doc.matches("<w:drawing").count(), 1, "wide 须 1 个 drawing: {doc}");
        assert!(
            doc.contains("cx=\"6696000\""),
            "wide 须用 18.6cm 全宽 cx=6696000: {doc}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// B2 新测 9：3up 2 图 → <w:tc>=3、drawing=2、BidCaption=2（第 3 格补空单元格）
    #[test]
    fn three_up_with_two_items_fills_empty_cell() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-3up-2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("threeup-2.docx");
        let img = asset_png();
        let req = BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Figure(FigureBlock {
                    layout: FigureLayout::ThreeUp,
                    items: vec![
                        FigureItem { path: img.clone(), caption: Some("图1-1 甲".to_string()), ai: false },
                        FigureItem { path: img.clone(), caption: Some("图1-2 乙".to_string()), ai: false },
                    ],
                    caption: String::new(),
                    caption_mode: CaptionMode::PerImage,
                    ai_generated: false,
                })],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        assert_eq!(doc.matches("<w:tc>").count(), 3, "2 图须补空单元格至 3: {doc}");
        assert_eq!(doc.matches("<w:drawing").count(), 2, "2 图须 2 个 drawing: {doc}");
        assert_eq!(doc.matches("BidCaption").count(), 2, "per_image 须 2 段图注: {doc}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// B2 新测 10：标题含 <&> → header1.xml 不含 {{DOC_TITLE}}、含 &lt;&amp;&gt;
    #[test]
    fn header_title_replaced_and_escaped() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-t2c1-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("hdr.docx");
        let req = BidExportRequest {
            title: "<&>".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: "b".to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let hdr = pkg.text("word/header1.xml").unwrap();
        assert!(!hdr.contains("{{DOC_TITLE}}"), "页眉占位符未替换: {hdr}");
        assert!(hdr.contains("&lt;&amp;&gt;"), "页眉未含转义后标题: {hdr}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// P0c：导出后 core.xml 不含 qidiwork、含空 creator、title 为本次标题
    #[test]
    fn metadata_is_anonymous() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p0c-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("meta.docx");
        let req = BidExportRequest {
            title: "暗标元数据测试标题".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: "b".to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let core = pkg.text("docProps/core.xml").unwrap();
        assert!(!core.contains("qidiwork"), "core.xml 泄漏 qidiwork: {core}");
        assert!(
            core.contains("<dc:creator></dc:creator>"),
            "core.xml creator 未置空: {core}"
        );
        assert!(
            core.contains("<dc:title>暗标元数据测试标题</dc:title>"),
            "core.xml title 未写入本次标题: {core}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// P0c：标题含 <&> → core.xml 里写的是转义形态 &lt;&amp;&gt;（不得裸写 XML 元字符）
    #[test]
    fn metadata_title_is_escaped() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p0c-meta-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("meta-esc.docx");
        let req = BidExportRequest {
            title: "<&>".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: "b".to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let core = pkg.text("docProps/core.xml").unwrap();
        assert!(
            core.contains("<dc:title>&lt;&amp;&gt;</dc:title>"),
            "core.xml title 未转义: {core}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// P0c：导出件 core.xml 不含时间戳/修订号（dcterms:created / dcterms:modified / cp:revision）
    #[test]
    fn metadata_has_no_timestamp_or_revision() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p0c-meta-ts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("meta-ts.docx");
        let req = BidExportRequest {
            title: "暗标时间戳测试".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: "b".to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };

        export(&req, Path::new(TEMPLATE), &out).unwrap();
        let pkg = DocxPackage::open(&out).unwrap();
        let core = pkg.text("docProps/core.xml").unwrap();
        for needle in ["dcterms:created", "dcterms:modified", "cp:revision"] {
            assert!(!core.contains(needle), "core.xml 泄漏 {needle}: {core}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// P0c：模板包内没有 docProps/core.xml 时，导出须新建同规格 core.xml（creator 为空）
    #[test]
    fn missing_core_xml_gets_created() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p0c-meta-mk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 1) 由模板派生一个「无 core.xml」的模板
        let mut tpl = DocxPackage::open(Path::new(TEMPLATE)).unwrap();
        assert!(tpl.contains("docProps/core.xml"), "模板本身应含 core.xml");
        tpl.entries.remove("docProps/core.xml");
        let bare_tpl = dir.join("tpl-no-core.docx");
        tpl.save(&bare_tpl).unwrap();
        assert!(!DocxPackage::open(&bare_tpl).unwrap().contains("docProps/core.xml"));

        // 2) 用它导出 → 新包必须补出 core.xml
        let out = dir.join("meta-mk.docx");
        let req = BidExportRequest {
            title: "补建元数据".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Paragraph { text: "b".to_string() }],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };
        export(&req, &bare_tpl, &out).unwrap();

        let pkg = DocxPackage::open(&out).unwrap();
        assert!(pkg.contains("docProps/core.xml"), "导出件缺少补建的 core.xml");
        let core = pkg.text("docProps/core.xml").unwrap();
        assert!(
            core.contains("<dc:creator></dc:creator>"),
            "补建的 core.xml creator 未置空: {core}"
        );
        assert!(
            core.contains("<cp:lastModifiedBy></cp:lastModifiedBy>"),
            "补建的 core.xml 缺空 lastModifiedBy: {core}"
        );
        assert!(
            core.contains("<dc:title>补建元数据</dc:title>"),
            "补建的 core.xml title 不正确: {core}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// B2 新测 11：cant_split=false 渲染不含 <w:cantSplit/>；true 则含（直接调私有 render_row）
    #[test]
    fn cant_split_false_omits_element() {
        let widths = [3516u32, 3515, 3515];
        let cells = ["a".to_string()];
        let off = render_row(&widths, &cells, "", "BidTableBody", "", "");
        assert!(
            !off.contains("<w:cantSplit/>"),
            "cant_split=false 不得输出 <w:cantSplit/>: {off}"
        );
        let on = render_row(&widths, &cells, "", "BidTableBody", "", "<w:cantSplit/>");
        assert!(
            on.contains("<w:cantSplit/>"),
            "cant_split=true 须输出 <w:cantSplit/>: {on}"
        );
        // 语义兜底：render_table 按 style.table.cant_split 布尔计算该串
        let mut style = DocStyle::default();
        style.table.cant_split = false;
        let tbl = render_table(&["h".to_string()], &[vec!["b".to_string()]], &style).unwrap();
        assert!(!tbl.contains("<w:cantSplit/>"), "cant_split=false 全表不得含: {tbl}");
        let tbl_on = render_table(&["h".to_string()], &[vec!["b".to_string()]], &DocStyle::default()).unwrap();
        assert!(tbl_on.contains("<w:cantSplit/>"), "cant_split=true 全表须含: {tbl_on}");
    }

    // -----------------------------------------------------------------------
    // §7.7 重要 4：真实表格列宽加权 + cantSplit 长行放行（3 条）
    // -----------------------------------------------------------------------

    /// 取 XML 里全部 `<w:gridCol w:w="…"/>` 的宽度
    fn grid_widths(xml: &str) -> Vec<u32> {
        const PREFIX: &str = "<w:gridCol w:w=\"";
        let mut out = Vec::new();
        let mut rest = xml;
        while let Some(p) = rest.find(PREFIX) {
            let after = &rest[p + PREFIX.len()..];
            let end = after.find('"').expect("gridCol 宽度未闭合");
            out.push(after[..end].parse::<u32>().expect("gridCol 宽度非数字"));
            rest = &after[end..];
        }
        out
    }

    /// ① 2 列表（短/长）：宽度不得等分，长文列更宽，总宽仍为内容区宽 10546
    #[test]
    fn two_column_table_widths_are_weighted() {
        let header = vec!["序号".to_string(), "施工内容".to_string()];
        let long = "塘底清淤及外运：清除塘底淤泥约 2.54 万立方米，运距 15 公里，含晾晒与压实".to_string();
        let rows = vec![
            vec!["1".to_string(), long.clone()],
            vec!["2".to_string(), long.clone()],
        ];
        let xml = render_table(&header, &rows, &DocStyle::default()).unwrap();
        let widths = grid_widths(&xml);
        assert_eq!(widths.len(), 2, "两列表须 2 个 gridCol: {xml}");
        assert_ne!(widths[0], widths[1], "列宽不得等分: {widths:?}");
        assert!(widths[1] > widths[0], "长文列须更宽: {widths:?}");
        assert_eq!(widths.iter().sum::<u32>(), 10546, "总宽须等于内容区宽: {widths:?}");
        // gridCol 与单元格 tcW 用同一组宽度
        for (i, w) in widths.iter().enumerate() {
            assert!(
                xml.contains(&format!("<w:tcW w:w=\"{w}\" w:type=\"dxa\"/>")),
                "第 {i} 列 tcW 未与 gridCol 共用宽度 {w}: {xml}"
            );
        }
    }

    /// ② 8 列表：总宽仍为 10546，且每列不低于 12%（1265 twips）
    #[test]
    fn eight_column_table_keeps_total_and_min_share() {
        let header: Vec<String> = (1..=8).map(|i| format!("列{i}")).collect();
        let rows = vec![
            (1..=8).map(|i| "x".repeat(i)).collect::<Vec<String>>(),
            (1..=8)
                .map(|i| format!("内容{i}{}", "y".repeat(i * 2)))
                .collect::<Vec<String>>(),
        ];
        let xml = render_table(&header, &rows, &DocStyle::default()).unwrap();
        let widths = grid_widths(&xml);
        assert_eq!(widths.len(), 8, "八列表须 8 个 gridCol: {widths:?}");
        assert_eq!(
            widths.iter().sum::<u32>(),
            10546,
            "八列表总宽须仍为 10546: {widths:?}"
        );
        let min_share = 10546 * 12 / 100; // 1265
        for (i, w) in widths.iter().enumerate() {
            assert!(
                *w >= min_share,
                "第 {} 列宽 {w} 低于 12%（{min_share}）: {widths:?}",
                i + 1
            );
            assert!(*w <= 10546 * 45 / 100, "第 {} 列宽 {w} 超过 45%: {widths:?}", i + 1);
        }
    }

    /// ③ 含 158 字单元格的行不得 cantSplit（允许跨页）；短行照旧 cantSplit
    #[test]
    fn long_cell_row_allows_page_break() {
        let header = vec!["项".to_string(), "说明".to_string()];
        let short_row = vec!["a".to_string(), "短文本".to_string()];
        let long_row = vec!["b".to_string(), "长".repeat(158)];
        let xml = render_table(&header, &[short_row, long_row], &DocStyle::default()).unwrap();
        let rows_xml: Vec<&str> = xml.split("<w:tr>").skip(1).collect();
        assert_eq!(rows_xml.len(), 3, "须 3 行（表头 + 2 数据行）: {xml}");
        assert!(
            rows_xml[0].contains("<w:cantSplit/>"),
            "表头行须保留 cantSplit: {}",
            rows_xml[0]
        );
        assert!(
            rows_xml[1].contains("<w:cantSplit/>"),
            "短行须保留 cantSplit: {}",
            rows_xml[1]
        );
        assert!(
            !rows_xml[2].contains("<w:cantSplit/>"),
            "158 字单元格所在行须允许跨页: {}",
            rows_xml[2]
        );
        // 全表统计：3 行里恰 2 行 cantSplit
        assert_eq!(xml.matches("<w:cantSplit/>").count(), 2, "{xml}");
    }


    // -----------------------------------------------------------------------
    // P1-5 横道图（3 条）
    // -----------------------------------------------------------------------

    fn gantt_task(name: &str, start: &str, end: &str) -> GanttTask {
        GanttTask {
            name: name.to_string(),
            start: start.to_string(),
            end: end.to_string(),
        }
    }

    fn gantt_req(tasks: Vec<GanttTask>, title: &str, out: &Path) -> BidExportRequest {
        BidExportRequest {
            title: "t".to_string(),
            chapters: vec![Chapter {
                title: "c".to_string(),
                blocks: vec![Block::Gantt(GanttBlock {
                    title: title.to_string(),
                    tasks,
                })],
            }],
            output_path: out.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        }
    }

    /// P1-5 §5：3 工序 → 一张 BidTable 表（无 BidTableHeader/不影响 doc.tables）、
    /// 底纹次数 = 覆盖桶数（≥ 工序数）、单元格全 BidTableBody、表头为桶起始日 MM-DD。
    #[test]
    fn gantt_renders_native_table() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p15-gantt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("gantt.docx");
        let req = gantt_req(
            vec![
                gantt_task("塘底清淤", "2026-12-01", "2026-12-10"),
                gantt_task("管网敷设", "2026-12-05", "2026-12-20"),
                gantt_task("路基整平", "2026-12-15", "2026-12-25"),
            ],
            "",
            &out,
        );

        let (stats, _) = export(&req, Path::new(TEMPLATE), &out).unwrap();
        assert_eq!(stats.tables, 0, "横道图表不计入 tables");
        let pkg = DocxPackage::open(&out).unwrap();
        let doc = pkg.text("word/document.xml").unwrap();
        // span = 25 天 → 7 天/桶 → 4 桶（12-01/12-08/12-15/12-22）
        assert_eq!(doc.matches("<w:gridCol").count(), 5, "1 工序列 + 4 桶: {doc}");
        for label in ["工序", "12-01", "12-08", "12-15", "12-22"] {
            assert!(doc.contains(label), "表头缺 {label}: {doc}");
        }
        // 覆盖桶：清淤 2 + 管网 3 + 整平 2 = 7
        assert_eq!(
            doc.matches(r#"w:fill="4877AC""#).count(),
            7,
            "底纹格数应等于覆盖桶数: {doc}"
        );
        assert!(
            doc.matches(r#"w:fill="4877AC""#).count() >= 3,
            "底纹格数应 ≥ 工序数: {doc}"
        );
        // 业务表口径：无 BidTableHeader（doc.tables 计数不受影响）；单元格全 BidTableBody
        assert!(!doc.contains("BidTableHeader"), "横道图不得用 BidTableHeader: {doc}");
        assert_eq!(
            doc.matches(r#"w:pStyle w:val="BidTableBody""#).count(),
            20,
            "4 行 × 5 列全用 BidTableBody: {doc}"
        );
        assert_eq!(doc.matches("BidCaption").count(), 0, "标题为空不加图注: {doc}");
        assert_eq!(doc.matches("<w:cantSplit/>").count(), 4, "每行都 cantSplit: {doc}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// N1 新测：跨年（2026-12-20..2027-01-10）→ 表头带年份 `YYYY-MM-DD`，不再只有 `MM-DD`
    #[test]
    fn cross_year_gantt_labels_include_year() {
        let block = GanttBlock {
            title: String::new(),
            tasks: vec![gantt_task("跨年工序", "2026-12-20", "2027-01-10")],
        };
        let (xml, _) = render_gantt(&block).unwrap();
        // span 22 天 → 7 天/桶 → 4 桶：12-20 / 12-27 / 01-03 / 01-10
        assert_eq!(xml.matches("<w:gridCol").count(), 5, "1 + 4 桶: {xml}");
        for label in ["2026-12-20", "2026-12-27", "2027-01-03", "2027-01-10"] {
            assert!(xml.contains(label), "跨年表头缺 {label}: {xml}");
        }
        assert!(
            !xml.contains(">12-20<") && !xml.contains(">01-03<"),
            "跨年时必须带年份，不得退回 MM-DD: {xml}"
        );
    }

    /// P1-5 §3.2：跨度 > 112 天 → 14 天/桶（2026-01-01..06-30 共 181 天 → 13 桶）
    #[test]
    fn gantt_wide_span_uses_14_day_buckets() {
        let block = GanttBlock {
            title: "图4-1 施工进度横道图".to_string(),
            tasks: vec![gantt_task("全工序", "2026-01-01", "2026-06-30")],
        };
        let (xml, captions) = render_gantt(&block).unwrap();
        assert_eq!(captions, 1, "非空标题 → 1 段图注");
        assert_eq!(xml.matches("<w:gridCol").count(), 14, "1 + 13 桶: {xml}");
        for label in ["01-01", "01-15", "06-18"] {
            assert!(xml.contains(label), "14 天桶表头缺 {label}: {xml}");
        }
        assert!(!xml.contains("01-08"), "14 天桶不应出现 7 天步长的 01-08: {xml}");
        assert!(xml.contains("BidCaption"), "非空标题须追加图注段: {xml}");
    }

    /// P1-5 §3.2：桶数上限 16（224 天恰好 16 桶可导出，225 天 → 17 桶一律报错）
    #[test]
    fn gantt_span_too_large_rejected() {
        let ok = GanttBlock {
            title: String::new(),
            tasks: vec![gantt_task("工序", "2026-01-01", "2026-08-12")], // span 224 → 16 桶
        };
        let (xml, _) = render_gantt(&ok).unwrap();
        assert_eq!(xml.matches("<w:gridCol").count(), 17, "16 桶 + 工序列: {xml}");

        let over = GanttBlock {
            title: String::new(),
            tasks: vec![gantt_task("工序", "2026-01-01", "2026-08-13")], // span 225 → 17 桶
        };
        let err = render_gantt(&over).unwrap_err();
        assert!(
            err.to_string().contains("横道图跨度过大（>16 个时间桶）"),
            "错误信息不符: {err}"
        );
    }
}
