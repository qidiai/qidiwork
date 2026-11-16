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
    BidExportRequest, Block, CaptionMode, DocStyle, FigureBlock, FigureItem, FigureLayout,
    FillStats,
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
                Block::Table { .. } => stats.tables += 1,
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
    // 9) 保存前重写 core.xml（P0c §2：暗标元数据清空；包内无该部件则跳过，不新建）
    if pkg.contains("docProps/core.xml") {
        pkg.set_text("docProps/core.xml", core_xml(&req.title));
    }
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
        Block::Table { header, rows } => Ok((render_table(header, rows, style)?, 0)),
        Block::Figure(fig) => render_figure(pkg, fig_no, fig, style, figure_width_cm),
    }
}

/// 表格渲染（§6.4 + §5.5 table.xml / table_row.xml）
fn render_table(header: &[String], rows: &[Vec<String>], style: &DocStyle) -> BidResult<String> {
    let cols = header.len();
    if cols == 0 {
        return Err(BidError::new("表格表头不能为空"));
    }
    // 内容区 10546 twips 均分，余数给首列
    let base = CONTENT_WIDTH_TWIPS / cols as u32;
    let rem = CONTENT_WIDTH_TWIPS % cols as u32;
    let widths: Vec<u32> = (0..cols).map(|c| base + if c == 0 { rem } else { 0 }).collect();
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

    let mut rows_xml = String::new();
    // 表头行：{{HEADER_FLAG}} = <w:tblHeader/>，单元格样式 BidTableHeader，首列不加底纹
    rows_xml.push_str(&render_row(&widths, header, "<w:tblHeader/>", "BidTableHeader", "", cant_split));
    // 数据行：flag 空串 + BidTableBody + 首列底纹
    for row in rows {
        rows_xml.push_str(&render_row(&widths, row, "", "BidTableBody", &first_col_shd, cant_split));
    }
    Ok(TABLE_XML
        .replace("{{GRID}}", &grid)
        .replace("{{ROWS}}", &rows_xml))
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
/// per_group 取 fig.caption；ai_generated 追加 style.figure.ai_label
fn caption_text(item: &FigureItem, fig: &FigureBlock, style: &DocStyle) -> String {
    let base = match fig.caption_mode {
        CaptionMode::PerImage => item.caption.clone().unwrap_or_else(|| fig.caption.clone()),
        CaptionMode::PerGroup => fig.caption.clone(),
    };
    if fig.ai_generated {
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
    use super::super::model::Chapter;
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
                        FigureItem { path: img.clone(), caption: None },
                        FigureItem { path: img.clone(), caption: None },
                        FigureItem { path: img.clone(), caption: None },
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
                        FigureItem { path: img.clone(), caption: Some("图1-1 甲".to_string()) },
                        FigureItem { path: img.clone(), caption: Some("图1-2 乙".to_string()) },
                        FigureItem { path: img.clone(), caption: Some("图1-3 丙".to_string()) },
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
                    items: vec![FigureItem { path: img.clone(), caption: Some("图1-1 总平面".to_string()) }],
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
                        FigureItem { path: img.clone(), caption: Some("图1-1 甲".to_string()) },
                        FigureItem { path: img.clone(), caption: Some("图1-2 乙".to_string()) },
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
}
