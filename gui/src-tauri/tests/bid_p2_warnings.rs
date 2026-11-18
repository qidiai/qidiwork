#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! P2 验收测试（指挥官编写，锁定禁改）：verify 新增 warnings 告警通道。
//!
//! 契约（规格书 T1）：
//! - 有效字号告警 `font.size_min`：document.xml 中含 w:t 文本的 run，按
//!   直接 rPr.sz > rStyle > pStyle > docDefaults 解析有效字号（half-points），
//!   任一 < 21（10.5pt）即告警；解析不到字号的 run 跳过（不误报）。
//! - 图注/图片静态几何告警 `figure.layout_risk`：
//!   a) wp:extent/@cy 超过内容区高（sectPr pgSz.h − pgMar.top − pgMar.bottom，twips×635 EMU）；
//!   b) 含 drawing 的段之后紧邻段不是 BidCaption 图注段（图注分离）。
//! - warnings 不影响 ok()、不影响 24 条 checks 的数量/名称/顺序；to_json 含 warnings 数组。

use std::path::{Path, PathBuf};

use qidiwork_gui::office::bid::docx::DocxPackage;
use qidiwork_gui::office::bid::fill::export;
use qidiwork_gui::office::bid::model::{
    BidExportRequest, Block, CaptionMode, Chapter, DocStyle, FigureBlock, FigureItem, FigureLayout,
};
use qidiwork_gui::office::bid::verify::{verify, verify_with_style, VerifyReport};

const TEMPLATE: &str = "resources/templates/bid-engineering-blue-v1.docx";
const COMPACT_TEMPLATE: &str = "resources/templates/bid-engineering-blue-v1-compact.docx";

fn asset_png() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/bid-export/assets/image3.png")
        .to_string_lossy()
        .to_string()
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qidi-bid-p2-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 1 章 1 段 1 图（single 布局，per_image 图注）的最小导出请求
fn req_with_figure(out: &Path) -> BidExportRequest {
    BidExportRequest {
        title: "P2 验收样本".to_string(),
        chapters: vec![Chapter {
            title: "第一章 概况".to_string(),
            blocks: vec![
                Block::Paragraph { text: "验收样本文本。".to_string() },
                Block::Figure(FigureBlock {
                    layout: FigureLayout::Single,
                    items: vec![FigureItem {
                        path: asset_png(),
                        caption: Some("图1-1 验收示意图".to_string()),
                        ai: false,
                    }],
                    caption: "图1-1 验收示意图".to_string(),
                    caption_mode: CaptionMode::PerImage,
                    ai_generated: false,
                }),
            ],
        }],
        output_path: out.to_string_lossy().to_string(),
        style_id: None,
        figure_width_cm: None,
    }
}

fn export_clean(tag: &str) -> PathBuf {
    let dir = tmp_dir(tag);
    let out = dir.join("clean.docx");
    export(&req_with_figure(&out), Path::new(TEMPLATE), &out).unwrap();
    out
}

fn warn_names(report: &VerifyReport) -> Vec<String> {
    report.warnings.iter().map(|w| w.name.clone()).collect()
}

/// A2/A4/A6/A7 基线：干净导出 → 24 条 checks 全过、warnings 为空
#[test]
fn clean_export_has_no_warnings() {
    let out = export_clean("clean");
    let report = verify(&out, None).unwrap();
    assert_eq!(report.checks.len(), 24);
    assert!(report.ok());
    assert!(
        report.warnings.is_empty(),
        "干净导出不得有误报告警: {:?}",
        report.warnings
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

/// A2 阳性：注入 8pt（w:sz=16）直接字号 run → 必出 font.size_min
#[test]
fn small_direct_font_triggers_warning() {
    let out = export_clean("smallfont");
    let mut pkg = DocxPackage::open(&out).unwrap();
    let doc = pkg.text("word/document.xml").unwrap();
    let injected = doc.replace(
        "</w:body>",
        "<w:p><w:r><w:rPr><w:sz w:val=\"16\"/></w:rPr><w:t>八磅小字</w:t></w:r></w:p></w:body>",
    );
    assert_ne!(doc, injected, "注入点必须存在");
    pkg.set_text("word/document.xml", injected);
    pkg.save(&out).unwrap();

    let report = verify(&out, None).unwrap();
    assert!(
        warn_names(&report).contains(&"font.size_min".to_string()),
        "8pt 直接字号必须触发 font.size_min: {:?}",
        report.warnings
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

/// A2 样式链优先级：pStyle=BidBody（12pt）但 run 直接 sz=16 → 按直接值告警；
/// 同段另一个 run 直接 sz=24 → 不得把整段判小（解析粒度是 run 不是段）
#[test]
fn direct_sz_overrides_paragraph_style() {
    let out = export_clean("stylechain");
    let mut pkg = DocxPackage::open(&out).unwrap();
    let doc = pkg.text("word/document.xml").unwrap();
    let injected = doc.replace(
        "</w:body>",
        "<w:p><w:pPr><w:pStyle w:val=\"BidBody\"/></w:pPr>\
         <w:r><w:rPr><w:sz w:val=\"16\"/></w:rPr><w:t>小字</w:t></w:r>\
         <w:r><w:rPr><w:sz w:val=\"24\"/></w:rPr><w:t>正常字</w:t></w:r></w:p></w:body>",
    );
    pkg.set_text("word/document.xml", injected);
    pkg.save(&out).unwrap();

    let report = verify(&out, None).unwrap();
    assert!(
        warn_names(&report).contains(&"font.size_min".to_string()),
        "直接 sz=16 必须压过 pStyle 的 12pt: {:?}",
        report.warnings
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

/// A2 阴性边界：紧凑档表字号 10.5pt（w:sz=21）= 阈值，不得误报
#[test]
fn compact_table_font_is_not_below_threshold() {
    let dir = tmp_dir("compact");
    let out = dir.join("compact.docx");
    export(&req_with_figure(&out), Path::new(COMPACT_TEMPLATE), &out).unwrap();
    let report = verify_with_style(&out, None, &DocStyle::default().compact()).unwrap();
    assert_eq!(report.checks.len(), 24);
    assert!(report.ok());
    assert!(
        !warn_names(&report).contains(&"font.size_min".to_string()),
        "10.5pt 等于阈值不得告警: {:?}",
        report.warnings
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A4 阳性：wp:extent cy=12000000 EMU（33.3cm）> 内容区高（约 27.7cm）→ figure.layout_risk
#[test]
fn oversized_image_height_triggers_warning() {
    let out = export_clean("tallimg");
    let mut pkg = DocxPackage::open(&out).unwrap();
    let doc = pkg.text("word/document.xml").unwrap();
    let mut tampered = String::new();
    let mut rest = doc.as_str();
    let mut replaced = 0usize;
    while let Some(pos) = rest.find("<wp:extent ") {
        tampered.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let end = tail.find("/>").expect("extent 必须自闭合");
        let tag = &tail[..end];
        let cx = tag
            .split("cx=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("extent 必须有 cx");
        tampered.push_str(&format!("<wp:extent cx=\"{cx}\" cy=\"12000000\""));
        rest = &tail[end..];
        replaced += 1;
    }
    tampered.push_str(rest);
    assert!(replaced >= 1, "样本必须含至少一张图");
    pkg.set_text("word/document.xml", tampered);
    pkg.save(&out).unwrap();

    let report = verify(&out, None).unwrap();
    assert!(
        warn_names(&report).contains(&"figure.layout_risk".to_string()),
        "超高图必须触发 figure.layout_risk: {:?}",
        report.warnings
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

/// A4 阳性：drawing 段与图注段之间插入普通段 → 图注分离告警
#[test]
fn separated_caption_triggers_warning() {
    let out = export_clean("sep");
    let mut pkg = DocxPackage::open(&out).unwrap();
    let doc = pkg.text("word/document.xml").unwrap();
    let cap = "<w:p><w:pPr><w:pStyle w:val=\"BidCaption\"/></w:pPr>";
    let pos = doc.find(cap).expect("样本必须含图注段");
    let injected = format!(
        "{}<w:p><w:pPr><w:pStyle w:val=\"BidBody\"/></w:pPr><w:r><w:t>插入的间隔段</w:t></w:r></w:p>{}",
        &doc[..pos],
        &doc[pos..]
    );
    pkg.set_text("word/document.xml", injected);
    pkg.save(&out).unwrap();

    let report = verify(&out, None).unwrap();
    assert!(
        warn_names(&report).contains(&"figure.layout_risk".to_string()),
        "图注分离必须触发 figure.layout_risk: {:?}",
        report.warnings
    );
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

/// A6：告警不阻断——带告警的产物 ok() 仍由 24 条 checks 决定，checks 数量不变
#[test]
fn warnings_do_not_affect_ok_or_checks() {
    let out = export_clean("nonblock");
    let mut pkg = DocxPackage::open(&out).unwrap();
    let doc = pkg.text("word/document.xml").unwrap();
    let injected = doc.replace(
        "</w:body>",
        "<w:p><w:r><w:rPr><w:sz w:val=\"16\"/></w:rPr><w:t>小字</w:t></w:r></w:p></w:body>",
    );
    pkg.set_text("word/document.xml", injected);
    pkg.save(&out).unwrap();

    let report = verify(&out, None).unwrap();
    assert!(!report.warnings.is_empty(), "本测试前提：必须已产生告警");
    assert_eq!(report.checks.len(), 24, "checks 数量不得受告警影响");
    assert!(report.ok(), "告警不得影响 ok()");
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}

/// A7：to_json 输出含 warnings 数组（机器可读契约）
#[test]
fn report_json_contains_warnings() {
    let out = export_clean("json");
    let report = verify(&out, None).unwrap();
    let value: serde_json::Value = serde_json::from_str(&report.to_json()).unwrap();
    assert!(value.get("warnings").is_some(), "to_json 必须含 warnings 字段");
    assert!(value["warnings"].is_array(), "warnings 必须是数组");
    assert_eq!(value["checks"].as_array().unwrap().len(), 24);
    let _ = std::fs::remove_dir_all(out.parent().unwrap());
}