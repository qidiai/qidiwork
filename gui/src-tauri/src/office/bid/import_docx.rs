//! P1-4 DOCX 导入：docx → Markdown（图片抽到 assets 目录）。
//!
//! 契约见 `docs/施工规格-P1-2-3-4-2026-11-16.md` §C（唯一真值）：
//!
//! - 读包走 [`DocxPackage::open`]；`word/document.xml` 用 `quick_xml::Reader` 事件流建极简树
//!   （不整串正则硬切），`word/_rels/document.xml.rels` 建 `rId → media target` 映射。
//! - 样式映射（`w:pStyle@w:val`，大小写与空格不敏感）：
//!   - `BidTitle1`：**首个** → `# `（书名），后续 → `## `（章）；`1` / `Heading1` / `Title` 同逻辑
//!   - `BidTitle2/3/4` → `###` / `####` / `####`（最高四级：markdown 侧不识别 H5+，
//!     `BidTitle4` 与 `BidTitle3` 同级，避免回灌出现可见的 `##### 文本` 段落）
//!   - 内置 `2` / `Heading2` → `##`；`3` → `###`；`4` → `####`
//!   - `BidCaption`（及 `Caption` / `题注` / `图注`）：上一块是图片则升级为该图图注
//!     `![图注](…)`，否则按普通段落输出
//! - `<w:tbl>` → GFM 表（首行表头 + `| --- |` 分隔行，单元格取纯文本）；
//!   整表无正文（三栏图桌）时不产出表格，改为逐行输出图行（3 张连排 → markdown 侧识别为 three_up）。
//! - `<a:blip r:embed>` → 查 rels 得 Target → 从包内取字节写 `<assets>/img-<n>.<ext>`，
//!   md 写 `<assets 目录名>/img-<n>.<ext>`。
//! - 跳过：页眉页脚（独立 part，本就不读）、目录域结果（`TOC` 域 / `TOC*` 样式）、
//!   `w:sdt` 包装（只取其 `sdtContent` 内容）、修订删除（`w:del`）、域代码（`w:instrText`）。
//!
//! 已知限制（详见 `docs/ops/bid-import.md`）：单元格内嵌套表格、`r:link` 外部链接图片不处理；
//! 带图单元格的表格按"表 + 表后图"输出；章节编号原样保留（导入不重编号，交给 `--md` 侧的编号器）。

use std::collections::BTreeMap;
use std::path::Path;

use quick_xml::escape::unescape;
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::Reader;

use super::docx::DocxPackage;
use super::{BidError, BidResult};

/// 导入统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub chapters: usize,
    pub headings: usize,
    pub paragraphs: usize,
    pub tables: usize,
    pub images: usize,
}

/// 读 docx → Markdown 文本；图片抽到 `assets_dir`，md 中写 `<assets 目录名>/img-<n>.<ext>`。
pub fn import_docx(docx: &Path, assets_dir: &Path) -> BidResult<(String, ImportStats)> {
    if !docx.is_file() {
        return Err(BidError::new(format!("文件不存在: {}", docx.display())));
    }
    let pkg = DocxPackage::open(docx)?;
    if !pkg.contains("word/document.xml") {
        return Err(BidError::new(format!(
            "不是 docx（缺 word/document.xml）: {}",
            docx.display()
        )));
    }
    let xml = pkg.text("word/document.xml")?;
    let rels = match pkg.contains("word/_rels/document.xml.rels") {
        true => {
            let rels_xml = pkg.text("word/_rels/document.xml.rels")?;
            parse_rels(&rels_xml)?
        }
        false => BTreeMap::new(),
    };

    let root = parse_xml(&xml)?;
    let body = root
        .child("body")
        .ok_or_else(|| BidError::new("word/document.xml 缺少 w:body"))?;

    let mut conv = Converter::new(&pkg, rels, assets_dir);
    conv.walk(&body.children)?;
    Ok((conv.render(), conv.stats))
}

// ---------------------------------------------------------------------------
// 极简 XML 树（只保留局部名：去命名空间前缀）
// ---------------------------------------------------------------------------

/// 极简 DOM：`name`/属性键都去掉命名空间前缀（`w:p` → `p`，`r:embed` → `embed`）。
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    /// 元素的直接文本内容（`&entity;` 已还原）
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn from_start(e: &BytesStart) -> BidResult<Self> {
        let mut attrs = Vec::with_capacity(4);
        for attr in e.attributes() {
            let attr = attr.map_err(|e| BidError::new(format!("XML 属性解析失败: {e}")))?;
            attrs.push((local_name(attr.key.as_ref()), attr.value.into_owned()));
        }
        Ok(Self {
            name: local_name(e.name().as_ref()),
            attrs,
            text: String::new(),
            children: Vec::new(),
        })
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// 直接子元素（首个同名）
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// 后代元素（首个同名，深度优先）
    fn find(&self, name: &str) -> Option<&Node> {
        for child in &self.children {
            if child.name == name {
                return Some(child);
            }
            if let Some(hit) = child.find(name) {
                return Some(hit);
            }
        }
        None
    }
}

/// 去命名空间前缀：`w:p` → `p`
fn local_name(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}

/// 事件流 → 极简树（单根；文本/CDATA/实体都并入所属节点的 `text`）
fn parse_xml(xml: &str) -> BidResult<Node> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| BidError::new(format!("document.xml 解析失败: {e}")))?;
        match ev {
            Event::Start(e) => stack.push(Node::from_start(&e)?),
            Event::Empty(e) => {
                let node = Node::from_start(&e)?;
                attach(&mut stack, &mut root, node)?;
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| BidError::new("document.xml 标签未配对"))?;
                attach(&mut stack, &mut root, node)?;
            }
            Event::Text(t) => {
                let text = unescape(&t)
                    .map_err(|e| BidError::new(format!("XML 文本转义解析失败: {e}")))?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&text);
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.into_inner());
                }
            }
            Event::GeneralRef(r) => {
                let text = resolve_ref(&r);
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&text);
                }
            }
            Event::Eof => break,
            // Comment/Decl/PI/DocType 与内容无关
            _ => {}
        }
    }
    root.ok_or_else(|| BidError::new("document.xml 为空"))
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) -> BidResult<()> {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => {
            if root.is_some() {
                return Err(BidError::new("document.xml 出现多个根元素"));
            }
            *root = Some(node);
        }
    }
    Ok(())
}

/// `&entity;` / `&#10;` / `&#x20;` → 字符
fn resolve_ref(r: &BytesRef) -> String {
    let name: &str = r;
    match name {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        _ => {
            let code = if let Some(hex) = name.strip_prefix("#x") {
                u32::from_str_radix(hex, 16).ok()
            } else {
                name.strip_prefix('#').and_then(|dec| dec.parse::<u32>().ok())
            };
            code.and_then(char::from_u32).map(String::from).unwrap_or_default()
        }
    }
}

// ---------------------------------------------------------------------------
// rels：rId → media target
// ---------------------------------------------------------------------------

/// `document.xml.rels` 里的一条关系
struct Rel {
    target: String,
    external: bool,
}

fn parse_rels(xml: &str) -> BidResult<BTreeMap<String, Rel>> {
    let mut reader = Reader::from_str(xml);
    let mut map = BTreeMap::new();
    loop {
        let ev = reader
            .read_event()
            .map_err(|e| BidError::new(format!("document.xml.rels 解析失败: {e}")))?;
        match ev {
            Event::Start(e) | Event::Empty(e) => {
                if local_name(e.name().as_ref()) == "Relationship" {
                    let mut id: Option<String> = None;
                    let mut target: Option<String> = None;
                    let mut external = false;
                    for attr in e.attributes() {
                        let attr = attr
                            .map_err(|e| BidError::new(format!("Relationship 属性解析失败: {e}")))?;
                        match local_name(attr.key.as_ref()).as_str() {
                            "Id" => id = Some(attr.value.into_owned()),
                            "Target" => target = Some(attr.value.into_owned()),
                            "TargetMode" => external = attr.value.eq_ignore_ascii_case("External"),
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(target)) = (id, target) {
                        map.insert(id, Rel { target, external });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(map)
}

/// `media/image1.png` / `/word/media/image1.png` / `../media/x.png` → 包内条目名
fn media_entry_name(target: &str) -> String {
    let target = target.trim();
    if let Some(rest) = target.strip_prefix('/') {
        return rest.to_string();
    }
    if target.starts_with("word/") {
        return target.to_string();
    }
    let mut parts: Vec<&str> = vec!["word"];
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// 条目名后缀（小写；无后缀回落 png）
fn extension_of(entry: &str) -> String {
    let name = entry.rsplit('/').next().unwrap_or(entry);
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => "png".to_string(),
    }
}

// ---------------------------------------------------------------------------
// 段落模型
// ---------------------------------------------------------------------------

/// 段落最小模型（正文 / 单元格共用）
struct Para {
    style: Option<String>,
    text: String,
    /// 段落内图片的 rId（按出现顺序）
    rids: Vec<String>,
    /// 含 `TOC` 域（目录结果，跳过）
    toc: bool,
}

impl Para {
    fn from(node: &Node) -> Self {
        let style = node
            .child("pPr")
            .and_then(|p| p.child("pStyle"))
            .and_then(|s| s.attr("val"))
            .map(str::to_string);
        let mut text = String::new();
        collect_text(node, &mut text);
        let mut rids = Vec::new();
        collect_rids(node, &mut rids);
        let toc = node
            .find("fldSimple")
            .and_then(|f| f.attr("instr"))
            .map(|i| i.to_ascii_uppercase().contains("TOC"))
            .unwrap_or(false)
            || node
                .find("instrText")
                .map(|n| n.text.to_ascii_uppercase().contains("TOC"))
                .unwrap_or(false);
        Self {
            style,
            text: text.trim().to_string(),
            rids,
            toc,
        }
    }
}

/// 拼接 run 文本：`w:t` 原文，`w:tab`/`w:br` 记一个空格；跳过属性、域代码、修订删除
fn collect_text(node: &Node, out: &mut String) {
    for child in &node.children {
        match child.name.as_str() {
            "t" => out.push_str(&child.text),
            "tab" | "br" | "cr" => out.push(' '),
            "noBreakHyphen" => out.push('-'),
            "pPr" | "rPr" | "sectPr" | "instrText" | "delText" | "del" | "moveFrom" | "fldChar"
            | "bookmarkStart" | "bookmarkEnd" | "commentReference" | "footnoteReference"
            | "endnoteReference" | "softHyphen" => {}
            _ => collect_text(child, out),
        }
    }
}

fn collect_rids(node: &Node, out: &mut Vec<String>) {
    for child in &node.children {
        if child.name == "blip" {
            if let Some(rid) = child.attr("embed") {
                out.push(rid.to_string());
            }
        }
        collect_rids(child, out);
    }
}

/// `<w:tbl>` → 行 × 单元格 × 段落（嵌套表格不展开）
fn table_rows(node: &Node) -> Vec<Vec<Vec<Para>>> {
    let mut rows = Vec::new();
    for tr in &node.children {
        if tr.name != "tr" {
            continue;
        }
        let mut cells = Vec::new();
        for tc in &tr.children {
            if tc.name != "tc" {
                continue;
            }
            let mut paras = Vec::new();
            collect_cell_paras(tc, &mut paras);
            cells.push(paras);
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    rows
}

fn collect_cell_paras(node: &Node, out: &mut Vec<Para>) {
    for child in &node.children {
        match child.name.as_str() {
            "p" => out.push(Para::from(child)),
            "sdt" => {
                if let Some(content) = child.child("sdtContent") {
                    collect_cell_paras(content, out);
                }
            }
            "customXml" | "smartTag" | "ins" => collect_cell_paras(child, out),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 样式映射
// ---------------------------------------------------------------------------

fn normalize_style(style: &str) -> String {
    style
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// 标题样式 → md 标题层级（1 为动态：首个书名，其余降为 2）
fn heading_level(style: &str) -> Option<u8> {
    match normalize_style(style).as_str() {
        "bidtitle1" | "1" | "heading1" | "title" | "标题" => Some(1),
        "bidtitle2" => Some(3),
        "bidtitle3" => Some(4),
        // 往返安全：markdown 侧只认 `#..####`，BidTitle4 与 BidTitle3 同级，不输出 H5+
        "bidtitle4" => Some(4),
        "2" | "heading2" | "标题2" => Some(2),
        "3" | "heading3" | "标题3" => Some(3),
        "4" | "heading4" | "标题4" => Some(4),
        _ => None,
    }
}

fn is_caption_style(style: Option<&str>) -> bool {
    match style {
        Some(s) => matches!(
            normalize_style(s).as_str(),
            "bidcaption" | "caption" | "题注" | "图注"
        ),
        None => false,
    }
}

fn is_toc_style(style: Option<&str>) -> bool {
    match style {
        Some(s) => {
            let n = normalize_style(s);
            n.contains("toc") || n.starts_with("目录")
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// 转换器
// ---------------------------------------------------------------------------

/// 一张图（md 链接 + 可选图注）
struct Img {
    link: String,
    caption: Option<String>,
}

/// md 输出块
enum Chunk {
    Heading { level: u8, text: String },
    Para(String),
    Table(String),
    /// 连续图行（同块内以单个换行相连 → markdown 侧识别 three_up）
    Images(Vec<Img>),
}

struct Converter<'a> {
    pkg: &'a DocxPackage,
    rels: BTreeMap<String, Rel>,
    assets_dir: &'a Path,
    /// md 里图片链接的目录前缀（assets 目录名，缺省 `assets`）
    link_prefix: String,
    chunks: Vec<Chunk>,
    stats: ImportStats,
    /// 已出现首个一级标题（书名）
    title_seen: bool,
}

impl<'a> Converter<'a> {
    fn new(pkg: &'a DocxPackage, rels: BTreeMap<String, Rel>, assets_dir: &'a Path) -> Self {
        let link_prefix = assets_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "assets".to_string());
        Self {
            pkg,
            rels,
            assets_dir,
            link_prefix,
            chunks: Vec::new(),
            stats: ImportStats::default(),
            title_seen: false,
        }
    }

    fn walk(&mut self, nodes: &[Node]) -> BidResult<()> {
        for node in nodes {
            match node.name.as_str() {
                "p" => {
                    let para = Para::from(node);
                    self.paragraph(&para)?;
                }
                "tbl" => {
                    let rows = table_rows(node);
                    self.table(&rows)?;
                }
                // w:sdt 包装只取内容（sdtPr 里的 tag 不是正文）
                "sdt" => {
                    if let Some(content) = node.child("sdtContent") {
                        self.walk(&content.children)?;
                    }
                }
                "customXml" | "smartTag" | "ins" | "moveTo" => self.walk(&node.children)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn paragraph(&mut self, para: &Para) -> BidResult<()> {
        if para.toc || is_toc_style(para.style.as_deref()) {
            return Ok(());
        }
        let style = para.style.as_deref().unwrap_or("");

        // 图注：上一块是图片 → 升级为该图图注（不额外产段落）
        if is_caption_style(Some(style)) && !para.text.is_empty() {
            if let Some(Chunk::Images(items)) = self.chunks.last_mut() {
                if let Some(last) = items.last_mut() {
                    if last.caption.is_none() {
                        last.caption = Some(para.text.clone());
                        return Ok(());
                    }
                }
            }
        }

        if let Some(level) = heading_level(style) {
            if !para.text.is_empty() {
                let level = self.resolve_level(level);
                self.stats.headings += 1;
                if level == 2 {
                    self.stats.chapters += 1;
                }
                self.chunks.push(Chunk::Heading {
                    level,
                    text: para.text.clone(),
                });
            }
        } else if !para.text.is_empty() {
            self.stats.paragraphs += 1;
            self.chunks.push(Chunk::Para(para.text.clone()));
        }

        if !para.rids.is_empty() {
            self.push_images(&para.rids)?;
        }
        Ok(())
    }

    /// 首个一级标题作书名（`#`），其后降为章（`##`）；输出最高四级（markdown 侧不认 H5+）
    fn resolve_level(&mut self, level: u8) -> u8 {
        if level != 1 {
            return level.min(4);
        }
        if self.title_seen {
            2
        } else {
            self.title_seen = true;
            1
        }
    }

    fn push_images(&mut self, rids: &[String]) -> BidResult<()> {
        let mut items = Vec::with_capacity(rids.len());
        for rid in rids {
            items.push(Img {
                link: self.extract_image(rid)?,
                caption: None,
            });
        }
        if !items.is_empty() {
            self.chunks.push(Chunk::Images(items));
        }
        Ok(())
    }

    fn cell_images(&mut self, cell: &[Para]) -> BidResult<Vec<Img>> {
        let mut items: Vec<Img> = Vec::new();
        for para in cell {
            if !para.rids.is_empty() {
                for rid in &para.rids {
                    items.push(Img {
                        link: self.extract_image(rid)?,
                        caption: None,
                    });
                }
            } else if !para.text.is_empty() && is_caption_style(para.style.as_deref()) {
                if let Some(last) = items.last_mut() {
                    if last.caption.is_none() {
                        last.caption = Some(para.text.clone());
                    }
                }
            }
        }
        Ok(items)
    }

    fn table(&mut self, rows: &[Vec<Vec<Para>>]) -> BidResult<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let texts: Vec<Vec<String>> = rows
            .iter()
            .map(|row| row.iter().map(|cell| cell_text(cell)).collect())
            .collect();
        let has_text = texts.iter().flatten().any(|t| !t.is_empty());

        // 纯图表格（三栏图桌 / 图墙）：逐行输出图行，不产 GFM 表
        if !has_text {
            for row in rows {
                let mut items: Vec<Img> = Vec::new();
                for cell in row {
                    items.extend(self.cell_images(cell)?);
                }
                if !items.is_empty() {
                    self.chunks.push(Chunk::Images(items));
                }
            }
            return Ok(());
        }

        // 有正文：出 GFM 表；表内图片按顺序挪到表后单独输出
        let mut loose: Vec<Img> = Vec::new();
        for row in rows {
            for cell in row {
                loose.extend(self.cell_images(cell)?);
            }
        }
        self.stats.tables += 1;
        self.chunks.push(Chunk::Table(render_gfm(&texts)));
        for img in loose {
            self.chunks.push(Chunk::Images(vec![img]));
        }
        Ok(())
    }

    /// 图片 rId → 字节写盘 → 返回 md 链接
    fn extract_image(&mut self, rid: &str) -> BidResult<String> {
        let (target, external) = match self.rels.get(rid) {
            Some(rel) => (rel.target.clone(), rel.external),
            None => return Err(BidError::new(format!("图片关系缺失: {rid}"))),
        };
        if external {
            return Err(BidError::new(format!(
                "不支持外部链接图片: {rid} → {target}"
            )));
        }
        let entry = media_entry_name(&target);
        let bytes = self
            .pkg
            .entries
            .get(&entry)
            .ok_or_else(|| BidError::new(format!("图片条目不存在: {entry}")))?;
        let ext = extension_of(&entry);
        let n = self.stats.images + 1;
        let file = format!("img-{n}.{ext}");
        std::fs::create_dir_all(self.assets_dir)?;
        std::fs::write(self.assets_dir.join(&file), bytes)?;
        self.stats.images = n;
        Ok(format!("{}/{file}", self.link_prefix))
    }

    fn render(&self) -> String {
        let blocks: Vec<String> = self
            .chunks
            .iter()
            .map(chunk_markdown)
            .filter(|s| !s.trim().is_empty())
            .collect();
        blocks.join("\n\n")
    }
}

fn chunk_markdown(chunk: &Chunk) -> String {
    match chunk {
        Chunk::Heading { level, text } => format!("{} {}", "#".repeat(*level as usize), text),
        Chunk::Para(text) | Chunk::Table(text) => text.clone(),
        Chunk::Images(items) => items
            .iter()
            .map(|img| match &img.caption {
                Some(caption) => format!("![{}]({})", clean_caption(caption), img.link),
                None => format!("![]({})", img.link),
            })
            .collect::<Vec<String>>()
            .join("\n"),
    }
}

/// 图注里的 `]` 会让 `![..](..)` 语法失效，替换成空格（其余原样）
fn clean_caption(caption: &str) -> String {
    caption.replace(']', " ").trim().to_string()
}

/// 单元格纯文本：段落以空格连接；有图的单元格里，图注段不算表格文本
fn cell_text(cell: &[Para]) -> String {
    let has_image = cell.iter().any(|p| !p.rids.is_empty());
    let mut parts: Vec<&str> = Vec::new();
    for para in cell {
        if para.text.is_empty() || is_toc_style(para.style.as_deref()) {
            continue;
        }
        if has_image && is_caption_style(para.style.as_deref()) {
            continue;
        }
        parts.push(para.text.as_str());
    }
    parts.join(" ")
}

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// 行 × 单元格文本 → GFM：首行表头 + `| --- |` 分隔行，列数按最大列补齐
fn render_gfm(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        out.push('|');
        for c in 0..cols {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            out.push(' ');
            out.push_str(&escape_cell(cell));
            out.push_str(" |");
        }
        out.push('\n');
        if i == 0 {
            out.push('|');
            for _ in 0..cols {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::path::PathBuf;

    const DOC_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><w:body>";
    const DOC_TAIL: &str = "</w:body></w:document>";
    const RELS_EMPTY: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>";
    const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>";

    /// 合成一个最小 docx（只有 document.xml + 空 rels），返回 (docx 路径, assets 目录)
    fn fixture(name: &str, body: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "qidi-bid-c-import-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("in.docx");
        let mut pkg = DocxPackage {
            entries: BTreeMap::new(),
        };
        pkg.set_text("[Content_Types].xml", CONTENT_TYPES.to_string());
        pkg.set_text("word/_rels/document.xml.rels", RELS_EMPTY.to_string());
        pkg.set_text(
            "word/document.xml",
            format!("{DOC_HEAD}{body}{DOC_TAIL}"),
        );
        pkg.save(&path).unwrap();
        (path, dir.join("assets"))
    }

    /// 1. 回环：自产 docx（1 章 + 1 段 + 1 张 2×2 表 + 1 图）→ 导入，统计与链接正确
    #[test]
    fn imports_our_export() {
        use crate::office::bid::fill;
        use crate::office::bid::model::{
            BidExportRequest, Block, CaptionMode, Chapter, FigureBlock, FigureItem, FigureLayout,
        };

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let template = root.join("resources/templates/bid-engineering-blue-v1.docx");
        let image = root.join("../tests/bid-export/assets/image3.png");
        assert!(template.is_file(), "缺少模板 {}", template.display());
        assert!(image.is_file(), "缺少图片 {}", image.display());

        let dir = std::env::temp_dir().join(format!(
            "qidi-bid-c-import-roundtrip-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let docx = dir.join("roundtrip.docx");
        let assets = dir.join("assets");

        // 自产样本：不依赖 tests/bid-export/out 下的预置产物（干净环境也能跑）
        let req = BidExportRequest {
            title: "C-fix 回环样本".to_string(),
            chapters: vec![Chapter {
                title: "第一章 回环".to_string(),
                blocks: vec![
                    Block::Paragraph {
                        text: "回环段落".to_string(),
                    },
                    Block::Table {
                        header: vec!["项".to_string(), "值".to_string()],
                        rows: vec![vec!["甲".to_string(), "乙".to_string()]],
                    },
                    Block::Figure(FigureBlock {
                        layout: FigureLayout::Single,
                        items: vec![FigureItem {
                            path: image.to_string_lossy().to_string(),
                            caption: Some("图1-1 回环图".to_string()),
                        }],
                        caption: String::new(),
                        caption_mode: CaptionMode::default(),
                        ai_generated: false,
                    }),
                ],
            }],
            output_path: docx.to_string_lossy().to_string(),
            style_id: None,
            figure_width_cm: None,
        };
        fill::export(&req, &template, &docx).unwrap();
        assert!(docx.is_file(), "导出失败 {}", docx.display());

        let (md, stats) = import_docx(&docx, &assets).unwrap();
        assert_eq!(stats.chapters, 1, "chapters={} md={md}", stats.chapters);
        assert_eq!(stats.tables, 1, "tables={} md={md}", stats.tables);
        assert_eq!(stats.images, 1, "images={} md={md}", stats.images);
        assert!(md.starts_with("# C-fix 回环样本"), "{md}");
        assert!(md.contains("\n\n## 第一章 回环"), "{md}");
        assert!(md.contains("| 项 | 值 |"), "{md}");
        assert!(md.contains("| --- | --- |"), "{md}");
        assert!(md.contains("![图1-1 回环图](assets/img-1.png)"), "{md}");
        assert!(assets.join("img-1.png").is_file());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 2. 内置 Word 样式 `w:val="1"`：首个 → `#`，后续 → `##`
    #[test]
    fn builtin_heading_styles_mapped() {
        let body = concat!(
            "<w:p><w:pPr><w:pStyle w:val=\"1\"/></w:pPr><w:r><w:t>某县高标准农田施工组织设计</w:t></w:r></w:p>",
            "<w:p><w:pPr><w:pStyle w:val=\"1\"/></w:pPr><w:r><w:t>第一章 工程概况</w:t></w:r></w:p>",
            "<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>第一节 编制依据</w:t></w:r></w:p>",
            "<w:p><w:pPr><w:pStyle w:val=\"BidTitle4\"/></w:pPr><w:r><w:t>五级降四级</w:t></w:r></w:p>",
        );
        let (docx, assets) = fixture("builtin", body);
        let (md, stats) = import_docx(&docx, &assets).unwrap();

        assert!(md.starts_with("# 某县高标准农田施工组织设计"), "{md}");
        assert!(md.contains("\n\n## 第一章 工程概况"), "{md}");
        assert!(md.contains("\n\n## 第一节 编制依据"), "{md}");
        // P1-4：BidTitle4 与 BidTitle3 同级，最高 `####`（markdown 侧不认 H5+）
        assert!(md.contains("\n\n#### 五级降四级"), "{md}");
        assert!(!md.contains("#####"), "不应输出 H5：{md}");
        assert_eq!(stats.headings, 4);
        assert_eq!(stats.chapters, 2, "两个 ## 即两章：{md}");
    }

    /// 3. 2×2 表格 → GFM（首行表头 + `| --- |` 分隔行）
    #[test]
    fn table_to_gfm() {
        let body = concat!(
            "<w:tbl>",
            "<w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr>",
            "<w:tr><w:tc><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>d</w:t></w:r></w:p></w:tc></w:tr>",
            "</w:tbl>",
        );
        let (docx, assets) = fixture("table", body);
        let (md, stats) = import_docx(&docx, &assets).unwrap();

        assert!(md.contains("| a | b |"), "{md}");
        assert!(md.contains("| --- | --- |"), "{md}");
        assert!(md.contains("| c | d |"), "{md}");
        assert_eq!(stats.tables, 1);
        assert_eq!(stats.paragraphs, 0);
    }

    /// 3b. 单元格含 `|` → 输出 `\|`（不转义会把 GFM 表格列切错）
    #[test]
    fn table_cell_pipe_escaped() {
        let body = concat!(
            "<w:tbl>",
            "<w:tr><w:tc><w:p><w:r><w:t>a|b</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc></w:tr>",
            "<w:tr><w:tc><w:p><w:r><w:t>d</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>e</w:t></w:r></w:p></w:tc></w:tr>",
            "</w:tbl>",
        );
        let (docx, assets) = fixture("pipe", body);
        let (md, _) = import_docx(&docx, &assets).unwrap();

        assert!(md.contains("| a\\|b | c |"), "{md}");
        assert!(!md.contains("| a|b |"), "单元格未转义：{md}");
    }

    /// 4. 不存在的路径 → Err
    #[test]
    fn missing_file_error() {
        let dir = std::env::temp_dir().join(format!("qidi-bid-c-import-miss-{}", std::process::id()));
        let docx = dir.join("不存在的样本.docx");
        let _ = std::fs::remove_file(&docx);
        let err = import_docx(&docx, &dir.join("assets")).unwrap_err();
        assert!(err.to_string().contains("文件不存在"), "{err}");
    }
}
