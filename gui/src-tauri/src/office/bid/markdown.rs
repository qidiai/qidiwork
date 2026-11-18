//! 标书内容入口（P1-1 D2）：Markdown → `(文档标题, 章节)`。
//!
//! 契约见 `docs/施工规格-P1-1-2026-11-16.md` §1（唯一真值）：
//!
//! - 首个 `# ` 是文档标题；其后的 H1 忽略；全程无 H1 → `Err("缺少文档标题（# 标题）")`。
//! - `## ` → 新 [`Chapter`]；`### `/`#### ` → [`Block::Heading`]（level 3/4）；
//!   `##### ` 及以上不识别为标题，整行按普通段落文本保留。
//! - 空行分隔的普通文本 → 一个 [`Block::Paragraph`]，段内换行以空格连接后 trim。
//! - GFM 表格（`| a | b |` 紧跟 `| --- | --- |`）→ [`Block::Table`]，单元格 trim。
//!   而是"表头 + 分隔行"却列数不一致（含数据行列数不符）→ `Err("表格第 N 行列数不符")`，N 从表头算 1。
//! - 整行 `![图注](路径)` → `Single`（`{wide}` 后缀 → `Wide`，`{ai}` → 该图 `item.ai`
//!   且 `ai_generated`，可组合为 `{wide,ai}`，标记内逗号分隔并容忍空格）；连续 3 行图片
//!   （中间无空行）→ 一个 `ThreeUp` + `per_image`（图注取各自 `[]`；每张的 `{ai}` 记到各自
//!   `item.ai`，任一张带 `{ai}` → 整组 `ai_generated` 也为真）。
//!   标记内**只认** `wide` / `ai`（大小写敏感）：出现其它 token 或空 token（如 `{AI}`、
//!   `{width}`、`{wide,}`、`{}`）→ `Err("图片标记不识别: {tail}")`，tail 为花括号内原文，
//!   绝不静默降级成段落（那会丢图）。
//! - 围栏 ```` ```gantt [标题] ```` … ```` ``` ````（P1-5 §2）→ [`Block::Gantt`]：行格式
//!   `开始..结束 | 工序名`；空行忽略，格式不符 / 未闭合 / 无工序各有独立 Err 文案。
//! - 行内格式（`**粗体**`、链接）与 `\` 转义不解析，按纯文本原样保留；
//!   以 `>`/`-`/`*` 开头的行不做列表语义，按普通段落文本保留。
//!
//! 项目模式（一章一文件，§7.1）：[`parse_chapter_markdown`] 先读**真实首行**决定章题
//! （`# 文本` 或纯文本编号行，否则 `fallback_title`），正文 `##`→H3、`###`→H4；
//! 缺图默认降级为 warning（`strict_images = false`）：跳过该图、图注留成普通段落，
//! 绝不留下悬空图片引用。正文里**不做**纯文本编号识别（§7.2）。
//!
//! 编号（[`parse_markdown`] 默认开，[`parse_markdown_with`] 可关）：章标题未匹配
//! 可读编号前缀（见 `has_readable_number`：`第N章`、`一、`、`（一）`、`\d+(\.\d+)*[.、\s]`）时加
//! `第{中文数字}章 ` 前缀；`###`/`####` 未匹配
//! `^\d+(\.\d+)*` 时加 `{章}.{节}` / `{章}.{节}.{子节} ` 前缀；已编号的原样保留，
//! 但计数照常推进——显式 `第N章` / `N.M` 还会**同步**计数器，后续自动号从 M+1 接续。
//! H4 先于 H3 出现时把节号补为 1，产出 `N.1.K` 而非 `N.0.K`。
//!
//! 已知取舍：首个 `##` 之前的内容（H1 与导语）在模型中没有归属章节，直接忽略；
//! 图片相对路径按 `base_dir` 拼接，必须是存在的文件，否则 `Err("图片不存在: {路径}")`。

use std::path::{Path, PathBuf};

use super::model::{
    Block, CaptionMode, Chapter, FigureBlock, FigureItem, FigureLayout, GanttBlock, GanttTask,
};
use super::{BidError, BidResult};

/// 解析 Markdown → (文档标题, 章节)；图片路径按 base_dir 解析成绝对路径并校验存在。
pub fn parse_markdown(src: &str, base_dir: &Path) -> BidResult<(String, Vec<Chapter>)> {
    parse_markdown_with(src, base_dir, true)
}

/// 同 [`parse_markdown`]，`number = false` 时不做章节自动编号（`--no-number`）。
pub fn parse_markdown_with(
    src: &str,
    base_dir: &Path,
    number: bool,
) -> BidResult<(String, Vec<Chapter>)> {
    let lines = lines_of(src);
    let mut p = MdParser::new(base_dir, number, false, true);
    parse_lines(&mut p, &lines)?;
    p.flush_para();

    let title = p
        .title
        .ok_or_else(|| BidError::new("缺少文档标题（# 标题）"))?;
    Ok((title, p.chapters))
}

/// 项目模式（§7.1 / §7.2 / §7.4）：一章一文件 → `(章节, warnings)`。
///
/// 章题取文件**真实首行**（忽略前导空行）：
/// - `# 文本` → 文本；
/// - **纯文本编号行**（行长 ≤ 40、不含 `。；，`、且带可读编号前缀）→ 该行原文；
/// - 都不命中 → `fallback_title`（调用方传「文件名去 `NN-` 前缀」）。
///
/// 正文：`##` → [`Block::Heading`] level 3、`###` → level 4；正文里的 `#` 忽略、
/// `####`+ 不作标题（整行按普通段落保留，与单文件模式一致）；正文**不做**纯文本编号识别，
/// 故 `29.32万平方米五清十拆…` 这类句子绝不会被当成标题（§7.2）。
///
/// 缺图（§7.4）：`strict_images = true` → `Err("图片不存在: {path}")`（`--md` 单文件口径）；
/// `false` → **跳过该图**（不生成 drawing，不留悬空引用）、把它的图注留成普通段落，
/// 并把 `缺图: {path}` 记进第二个返回值（不阻断导出）。
///
/// 章题在本函数里**不**加 `第N章` 前缀：单文件的章号未知，合并场景请传 `number = false`
/// 后统一 [`renumber`]；`number = true` 只影响节/子节自动编号（按 1.M 计）。
pub fn parse_chapter_markdown(
    src: &str,
    base_dir: &Path,
    number: bool,
    fallback_title: &str,
    strict_images: bool,
) -> BidResult<(Chapter, Vec<String>)> {
    let lines = lines_of(src);
    let mut p = MdParser::new(base_dir, number, true, strict_images);

    let mut start = 0usize;
    let mut title = fallback_title.trim().to_string();
    if let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) {
        if let Some(detected) = detect_chapter_title(lines[first].trim()) {
            title = detected;
            start = first + 1; // 首行已作章题，不再喂给正文解析
        }
    }
    p.title = Some(title.clone());
    p.chapters.push(Chapter {
        title,
        blocks: Vec::new(),
    });

    parse_lines(&mut p, &lines[start..])?;
    p.flush_para();

    let mut chapters = std::mem::take(&mut p.chapters);
    let chapter = chapters.pop().unwrap_or_else(|| Chapter {
        title: p.title.clone().unwrap_or_default(),
        blocks: Vec::new(),
    });
    Ok((chapter, p.warnings))
}

/// 去 BOM 后按行切分（两种模式共用）。
fn lines_of(src: &str) -> Vec<&str> {
    src.trim_start_matches('\u{feff}').lines().collect()
}

/// 逐行解析（单文件模式与项目模式共用；差异只在标题层级映射与缺图策略）。
fn parse_lines(p: &mut MdParser<'_>, lines: &[&str]) -> BidResult<()> {
    let mut i = 0;
    while i < lines.len() {
        let text = lines[i].trim();
        if text.is_empty() {
            p.flush_para();
            i += 1;
            continue;
        }
        // 横道图围栏（P1-5 §2）：```gantt [标题] … ``` → Block::Gantt
        if let Some(info) = text.strip_prefix("```gantt") {
            let (gantt, next) = parse_gantt_fence(lines, i, info)?;
            p.flush_para();
            p.push_block(Block::Gantt(gantt));
            i = next;
            continue;
        }
        // 图片：先吃掉整段连续图片行（空行即中断）。
        if let Some(first) = parse_image_line(text)? {
            let mut run = vec![first];
            i += 1;
            while i < lines.len() {
                match parse_image_line(lines[i].trim())? {
                    Some(spec) => {
                        run.push(spec);
                        i += 1;
                    }
                    None => break,
                }
            }
            p.flush_para();
            p.push_image_run(&run)?;
            continue;
        }
        if let Some((level, heading)) = parse_heading(text) {
            if p.chapter_mode {
                // 项目模式：章题已由首行确定，正文 `##`→H3、`###`→H4
                match level {
                    1 => {} // 正文里的 H1 忽略（与单文件模式"其后 H1 忽略"一致）
                    2 => {
                        p.flush_para();
                        p.push_heading(3, heading);
                    }
                    3 => {
                        p.flush_para();
                        p.push_heading(4, heading);
                    }
                    _ => p.para.push(text.to_string()), // H4+ 不作标题
                }
            } else {
                match level {
                    1 => {
                        if p.title.is_none() && !heading.is_empty() {
                            p.title = Some(heading.to_string());
                        }
                    }
                    2 => {
                        p.flush_para();
                        p.start_chapter(heading);
                    }
                    3 | 4 => {
                        p.flush_para();
                        p.push_heading(level as u8, heading);
                    }
                    _ => p.para.push(text.to_string()), // H5+ 不作标题
                }
            }
            i += 1;
            continue;
        }
        if let Some(next) = p.try_table(lines, i)? {
            i = next;
            continue;
        }
        p.para.push(text.to_string());
        i += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 合并后统一重号（P1-2：多文件合并后整体编号，`project::build_request` 调用）
// ---------------------------------------------------------------------------

/// 合并后统一重号：章号连续，`N.M` / `N.M.K` 随本章序号走；已编号的原样保留并同步计数器。
///
/// 口径与 [`parse_markdown_with`]（`number = true`）完全一致，区别只是作用在**已解析好**的
/// 章节序列上：各 md 先用 `number = false` 解析，合并完成后整体过一次，避免每个文件各自
/// 从「第一章」重来。显式 `第N章` / `N.M` 会同步计数器，后续自动号从 N+1 接续。
pub fn renumber(chapters: &mut [Chapter]) {
    let mut chapter_no = 0usize;
    for chapter in chapters.iter_mut() {
        let raw = chapter.title.trim().to_string();
        chapter_no = explicit_chapter_number(&raw).unwrap_or(chapter_no + 1);
        chapter.title = if raw.is_empty() || matches_chapter_number(&raw) {
            raw
        } else {
            format!("第{}章 {}", cn_number(chapter_no), raw)
        };

        let mut sec_no = 0usize;
        let mut sub_no = 0usize;
        for block in chapter.blocks.iter_mut() {
            let Block::Heading { level, text } = block else {
                continue;
            };
            let raw = text.trim().to_string();
            if *level == 3 {
                sec_no += 1;
                sub_no = 0;
                if let Some(m) = explicit_section_number(&raw) {
                    sec_no = m;
                }
            } else {
                if sec_no == 0 {
                    sec_no = 1; // H4 先于 H3：节号补 1，避免 N.0.K
                }
                sub_no += 1;
            }
            *text = if raw.is_empty() || matches_section_number(&raw) {
                raw
            } else if *level == 3 {
                format!("{chapter_no}.{sec_no} {raw}")
            } else {
                format!("{chapter_no}.{sec_no}.{sub_no} {raw}")
            };
        }
    }
}

/// 解析中的可变状态。
struct MdParser<'a> {
    base_dir: &'a Path,
    number: bool,
    title: Option<String>,
    chapters: Vec<Chapter>,
    para: Vec<String>,
    /// 已出现过的 H2 数（= 当前章序号，1 起）
    chapter_no: usize,
    /// 本章内 H3 序号
    sec_no: usize,
    /// 本节内 H4 序号
    sub_no: usize,
    /// 项目模式（一章一文件）：`##`→H3、`###`→H4，`start_chapter` 不再被调用
    chapter_mode: bool,
    /// 缺图是否硬错误（项目模式默认 `false`，见 [`parse_chapter_markdown`]）
    strict_images: bool,
    /// 非致命问题（目前只有 `缺图: {path}`）
    warnings: Vec<String>,
}

impl<'a> MdParser<'a> {
    fn new(base_dir: &'a Path, number: bool, chapter_mode: bool, strict_images: bool) -> Self {
        Self {
            base_dir,
            number,
            title: None,
            chapters: Vec::new(),
            para: Vec::new(),
            // 项目模式的章号未知（合并后由 renumber 决定），节号按 1 起算
            chapter_no: usize::from(chapter_mode),
            sec_no: 0,
            sub_no: 0,
            chapter_mode,
            strict_images,
            warnings: Vec::new(),
        }
    }

    /// 段落缓冲落盘：段内换行以空格连接后 trim；空段不产出块。
    fn flush_para(&mut self) {
        if self.para.is_empty() {
            return;
        }
        let text = self.para.join(" ");
        let text = text.trim().to_string();
        self.para.clear();
        if !text.is_empty() {
            self.push_block(Block::Paragraph { text });
        }
    }

    /// 块归属当前章；首个 H2 之前的内容忽略（见模块文档"已知取舍"）。
    fn push_block(&mut self, block: Block) {
        if let Some(chapter) = self.chapters.last_mut() {
            chapter.blocks.push(block);
        }
    }

    fn start_chapter(&mut self, text: &str) {
        let raw = text.trim();
        // 显式章号（`第N章`，N 为 1–10 中文或阿拉伯数字）同步计数器，后续自动章号接续
        self.chapter_no = match explicit_chapter_number(raw) {
            Some(n) => n,
            None => self.chapter_no + 1,
        };
        self.sec_no = 0;
        self.sub_no = 0;
        let title = if raw.is_empty() {
            String::new()
        } else if self.number && !matches_chapter_number(raw) {
            format!("第{}章 {}", cn_number(self.chapter_no), raw)
        } else {
            raw.to_string()
        };
        self.chapters.push(Chapter {
            title,
            blocks: Vec::new(),
        });
    }

    fn push_heading(&mut self, level: u8, text: &str) {
        let raw = text.trim();
        if level == 3 {
            self.sec_no += 1;
            self.sub_no = 0;
            // 显式节号 `N.M` 同步计数器（下一自动号 = M+1）
            if let Some(m) = explicit_section_number(raw) {
                self.sec_no = m;
            }
        } else {
            // H4 先于任何 H3 出现：节号补 1，避免产出 `N.0.K`
            if self.sec_no == 0 {
                self.sec_no = 1;
            }
            self.sub_no += 1;
        }
        let text = if raw.is_empty() {
            String::new()
        } else if self.number && !matches_section_number(raw) {
            match level {
                3 => format!("{}.{} {}", self.chapter_no, self.sec_no, raw),
                _ => format!("{}.{}.{} {}", self.chapter_no, self.sec_no, self.sub_no, raw),
            }
        } else {
            raw.to_string()
        };
        self.push_block(Block::Heading { level, text });
    }

    /// 图片序列 → 图块：每 3 张一组 `three_up`（逐图图注），余下的逐张 Single/Wide。
    ///
    /// 缺图（§7.4）：`strict_images` → `Err`；否则**跳过该图**（不生成 drawing）并记 warning，
    /// 其图注留成普通段落。分组只在「连续存在」的图片之间进行，避免缺图把三栏拆错。
    fn push_image_run(&mut self, run: &[ImageSpec]) -> BidResult<()> {
        let mut idx = 0;
        while idx < run.len() {
            if !image_exists(self.base_dir, &run[idx].path) {
                self.drop_image(&run[idx])?;
                idx += 1;
                continue;
            }
            let usable = run[idx..]
                .iter()
                .take_while(|spec| image_exists(self.base_dir, &spec.path))
                .count();
            if usable >= 3 {
                let mut items = Vec::with_capacity(3);
                for spec in &run[idx..idx + 3] {
                    items.push(FigureItem {
                        path: resolve_image(self.base_dir, &spec.path)?,
                        caption: caption_of(&spec.caption),
                        // 逐图标记：`{ai}` 只标本张，不串到同组其它图
                        ai: spec.ai,
                    });
                }
                self.push_block(Block::Figure(FigureBlock {
                    layout: FigureLayout::ThreeUp,
                    items,
                    caption: String::new(),
                    caption_mode: CaptionMode::PerImage,
                    // 组级标记：任一张带 `{ai}` → 整组 ai_generated（per_group 组图注要用；
                    // per_image 逐图标注走各自 item.ai，见 fill::caption_text）
                    ai_generated: run[idx..idx + 3].iter().any(|spec| spec.ai),
                }));
                idx += 3;
            } else {
                let spec = &run[idx];
                let layout = if spec.wide {
                    FigureLayout::Wide
                } else {
                    FigureLayout::Single
                };
                let item = FigureItem {
                    path: resolve_image(self.base_dir, &spec.path)?,
                    caption: caption_of(&spec.caption),
                    ai: spec.ai,
                };
                self.push_block(Block::Figure(FigureBlock {
                    layout,
                    items: vec![item],
                    caption: String::new(),
                    caption_mode: CaptionMode::PerImage,
                    ai_generated: spec.ai,
                }));
                idx += 1;
            }
        }
        Ok(())
    }

    /// 缺图处置（§7.4）：严格 → `Err("图片不存在: {path}")`；否则跳过图片、图注留成段落 + warning。
    ///
    /// docx 侧因此**不会**出现悬空图片引用（不进 [`Block::Figure`] 就没有 drawing）。
    fn drop_image(&mut self, spec: &ImageSpec) -> BidResult<()> {
        if self.strict_images {
            return Err(BidError::new(format!("图片不存在: {}", spec.path)));
        }
        self.warnings.push(format!("缺图: {}", spec.path));
        if let Some(caption) = caption_of(&spec.caption) {
            self.push_block(Block::Paragraph { text: caption });
        }
        Ok(())
    }

    /// 尝试在 `start` 处解析 GFM 表格；返回下一行下标，`None` = 不是表格（走段落）。
    ///
    /// 判定：本行以 `|` 开头，且下一行也以 `|` 开头、每个单元格都是分隔单元格。
    /// 一旦判定为表格，列数不一致（表头/分隔行/数据行）就是**硬错误**，不再静默降级成段落。
    fn try_table(&mut self, lines: &[&str], start: usize) -> BidResult<Option<usize>> {
        let head_line = lines[start].trim();
        if !head_line.starts_with('|') || start + 1 >= lines.len() {
            return Ok(None);
        }
        let sep_line = lines[start + 1].trim();
        if !sep_line.starts_with('|') {
            return Ok(None); // 下一行根本不是分隔行 → 普通段落
        }
        let sep = split_cells(sep_line);
        if sep.is_empty() || !sep.iter().all(|c| is_sep_cell(c)) {
            return Ok(None);
        }
        let header = split_cells(head_line);
        if header.len() != sep.len() {
            return Err(BidError::new("表格第 1 行列数不符"));
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut i = start + 2;
        while i < lines.len() {
            let line = lines[i].trim();
            if line.is_empty() || !line.starts_with('|') {
                break;
            }
            let cells = split_cells(line);
            if cells.len() != header.len() {
                // 表头 = 第 1 行、分隔行 = 第 2 行
                return Err(BidError::new(format!("表格第 {} 行列数不符", rows.len() + 3)));
            }
            rows.push(cells);
            i += 1;
        }
        self.flush_para();
        self.push_block(Block::Table { header, rows });
        Ok(Some(i))
    }
}

/// 横道图围栏（P1-5 §2）：```` ```gantt [标题] ```` 起、```` ``` ```` 止；
/// 返回 `(横道图块, 下一行下标)`。
///
/// - 首行 ```` ```gantt ```` 之后的文本 = 标题（可空 → 导出时不加图注）；
/// - 行格式 `开始..结束 | 工序名`，空行忽略，格式不符 → `Err("横道图行格式错误: {line}")`；
/// - 未闭合 → `Err("横道图代码块未闭合")`；无任何工序 → `Err("横道图至少需要一个工序")`。
fn parse_gantt_fence(lines: &[&str], start: usize, info: &str) -> BidResult<(GanttBlock, usize)> {
    let title = info.trim().to_string();
    let mut tasks: Vec<GanttTask> = Vec::new();
    let mut i = start + 1;
    let mut closed = false;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("```") {
            closed = true;
            i += 1;
            break;
        }
        if !line.is_empty() {
            tasks.push(parse_gantt_task(line)?);
        }
        i += 1;
    }
    if !closed {
        return Err(BidError::new("横道图代码块未闭合"));
    }
    if tasks.is_empty() {
        return Err(BidError::new("横道图至少需要一个工序"));
    }
    Ok((GanttBlock { title, tasks }, i))
}

/// 单工序行：`开始..结束 | 工序名`（日期格式/合法性由 `model::validate` 兜底）
fn parse_gantt_task(line: &str) -> BidResult<GanttTask> {
    let bad = || BidError::new(format!("横道图行格式错误: {line}"));
    let (range, name) = line.split_once('|').ok_or_else(bad)?;
    let (start, end) = range.split_once("..").ok_or_else(bad)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(bad());
    }
    Ok(GanttTask {
        name: name.to_string(),
        start: start.trim().to_string(),
        end: end.trim().to_string(),
    })
}

/// 整行图片：`![图注](路径)` + 可选 `{wide}` / `{ai}` / `{wide,ai}` 后缀。
struct ImageSpec {
    caption: String,
    path: String,
    wide: bool,
    ai: bool,
}

fn parse_image_line(line: &str) -> BidResult<Option<ImageSpec>> {
    let Some(rest) = line.trim().strip_prefix("![") else {
        return Ok(None);
    };
    let Some(close) = rest.find("](") else {
        return Ok(None);
    };
    let caption = &rest[..close];
    let after = &rest[close + 2..];
    let Some(rparen) = after.rfind(')') else {
        return Ok(None);
    };
    let path = after[..rparen].trim();
    let tail = after[rparen + 1..].trim();
    let Some((wide, ai)) = parse_image_flags(tail)? else {
        return Ok(None); // 行尾另有内容 → 不是整行图片，按普通段落
    };
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(ImageSpec {
        caption: caption.trim().to_string(),
        path: path.to_string(),
        wide,
        ai,
    }))
}

/// 行尾标记 → `(wide, ai)`：`""` 表示无标记，`{a,b}` 内逗号分隔、容忍空格。
///
/// - `Ok(None)` = 行尾另有内容（不是整行图片），交由段落处理；
/// - 标记内只认 `wide` / `ai`（大小写敏感），**其它 token 或空 token** 一律
///   `Err("图片标记不识别: {tail}")`（tail = 花括号内原文）：静默降级会把整张图丢掉。
fn parse_image_flags(tail: &str) -> BidResult<Option<(bool, bool)>> {
    if tail.is_empty() {
        return Ok(Some((false, false)));
    }
    let Some(inner) = tail.strip_prefix('{').and_then(|t| t.strip_suffix('}')) else {
        return Ok(None);
    };
    let mut wide = false;
    let mut ai = false;
    for token in inner.split(',') {
        match token.trim() {
            "wide" => wide = true,
            "ai" => ai = true,
            _ => return Err(BidError::new(format!("图片标记不识别: {inner}"))),
        }
    }
    Ok(Some((wide, ai)))
}

/// `#..######` + 必需的一个空格（`#` 独占一行也算标题）。
fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if rest.is_empty() {
        return Some((hashes, ""));
    }
    let rest = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t'))?;
    Some((hashes, rest.trim()))
}

/// `| a | b |` → ["a", "b"]（首尾竖线可省，单元格 trim）。
fn split_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

/// GFM 分隔行单元格：`---` / `:---` / `---:` / `:---:`（至少 3 个短横）。
fn is_sep_cell(cell: &str) -> bool {
    let t = cell.trim();
    let t = t.strip_prefix(':').unwrap_or(t);
    let t = t.strip_suffix(':').unwrap_or(t);
    t.chars().count() >= 3 && t.chars().all(|c| c == '-')
}

/// 图片路径：相对 → `base_dir` 拼接；必须存在。
fn resolve_image(base_dir: &Path, raw: &str) -> BidResult<String> {
    let abs = image_path(base_dir, raw);
    if !abs.is_file() {
        return Err(BidError::new(format!("图片不存在: {raw}")));
    }
    Ok(abs.to_string_lossy().into_owned())
}

/// 图片绝对路径（不校验存在性；`is_absolute` 原样采信）。
fn image_path(base_dir: &Path, raw: &str) -> PathBuf {
    let path = Path::new(raw);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

/// 图片是否存在（缺图降级预检，不含 `Err`）。
fn image_exists(base_dir: &Path, raw: &str) -> bool {
    image_path(base_dir, raw).is_file()
}

fn caption_of(caption: &str) -> Option<String> {
    let c = caption.trim();
    if c.is_empty() {
        None
    } else {
        Some(c.to_string())
    }
}

/// 1..=10 → 一…十；其余用阿拉伯数字。
fn cn_number(n: usize) -> String {
    const CN: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    match CN.get(n.wrapping_sub(1)) {
        Some(s) if n > 0 => (*s).to_string(),
        _ => n.to_string(),
    }
}

/// 10 → 1（中文数字只认 1–10；其余 `None`）。
fn cn_to_number(s: &str) -> Option<usize> {
    const CN: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    CN.iter().position(|c| *c == s).map(|i| i + 1)
}

/// 前导十进制数字 → (值, 占用字节数)。
fn leading_usize(text: &str) -> Option<(usize, usize)> {
    let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n = digits.parse::<usize>().ok()?;
    Some((n, digits.len()))
}

/// `^第(.+)章` → 章序号（中文 1–10 或阿拉伯数字）；用于同步 `chapter_no`。
fn explicit_chapter_number(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('第')?;
    let idx = rest.find('章')?;
    if idx == 0 {
        return None; // "第章" 不算
    }
    let mid = &rest[..idx];
    mid.parse::<usize>().ok().or_else(|| cn_to_number(mid))
}

/// `^(\d+)\.(\d+)` → 第二个数字 M（`．` 全角句点同样接受）；用于同步 `sec_no`。
fn explicit_section_number(text: &str) -> Option<usize> {
    let (_, used) = leading_usize(text)?;
    let rest = &text[used..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix('．'))?;
    leading_usize(rest).map(|(n, _)| n)
}

/// 章标题是否已编号（命中任一可读编号前缀 → 不加 `第N章`）。
fn matches_chapter_number(text: &str) -> bool {
    has_readable_number(text)
}

/// 节/子节标题是否已编号：`^\d+(\.\d+)*` 或任一可读编号前缀。
///
/// 阿拉伯数字部分字面实现——`(\.\d+)*` 可匹配空串，故等价于"以 ASCII 数字开头"。
/// 这样 `1.1 概述`、`1.1概述`、`1 概述` 都按已编号原样保留，不会被二次加前缀；
/// 中文序号（`一、`、`（一）`）同样视为已编号（§7.2，防 `1.1 一、项目简介`）。
fn matches_section_number(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_digit()) || has_readable_number(text)
}

/// 可读编号前缀（§7.2）——命中任一即"人已经编过号"，自动编号不再叠加。模式：
///
/// - `^[一二三四五六七八九十百]+、`（`一、工程概况`）
/// - `^（[一二三四五六七八九十百]+）`（`（一）工程概况`）
/// - `^第[一二三四五六七八九十百0-9]+章`（`第一章` / `第1章`）
/// - `^\d+(\.\d+)*[.．、\s]`（`1.` / `1.1 ` / `2、`；`29.32万平方米` 不命中）
fn has_readable_number(text: &str) -> bool {
    starts_with_cn_ordinal(text)
        || starts_with_cn_paren(text)
        || starts_with_cn_chapter(text)
        || starts_with_arabic_ordinal(text)
}

/// 中文数字字符（`一`…`百`）个数。
fn cn_num_len(text: &str) -> usize {
    text.chars()
        .take_while(|c| matches!(c, '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百'))
        .count()
}

/// `^[一二三四五六七八九十百]+、`
fn starts_with_cn_ordinal(text: &str) -> bool {
    let n = cn_num_len(text);
    n > 0 && text.chars().nth(n) == Some('、')
}

/// `^（[一二三四五六七八九十百]+）`
fn starts_with_cn_paren(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('（') else {
        return false;
    };
    let n = cn_num_len(rest);
    n > 0 && rest.chars().nth(n) == Some('）')
}

/// `^第[一二三四五六七八九十百0-9]+章`
fn starts_with_cn_chapter(text: &str) -> bool {
    let Some(rest) = text.strip_prefix('第') else {
        return false;
    };
    let n = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百'))
        .count();
    n > 0 && rest.chars().nth(n) == Some('章')
}

/// `^\d+(\.\d+)*[.．、\s]`：多级号必须**以分隔符收尾**，故 `29.32万平方米` / `2.54万立方米`
/// 这类量词不算编号（§7.2 反例）。
fn starts_with_arabic_ordinal(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    if !chars.peek().is_some_and(char::is_ascii_digit) {
        return false;
    }
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            continue;
        }
        // `1.1`：点后仍是数字 → 多级号继续；否则点就是分隔符本身
        if (c == '.' || c == '．') && chars.peek().is_some_and(char::is_ascii_digit) {
            continue;
        }
        return matches!(c, '.' | '．' | '、' | ' ' | '\t');
    }
    false
}

/// 首行 → 章题：`# 文本`（非空）或**纯文本编号行**（§7.2 三重约束）；否则 `None`。
///
/// `None` 表示"这行不像章题"，调用方（`project::parse_chapter`）据此回退到老式
/// "每个 `##` 各自成章"语义——骨架文件与 `##` 开头的老章节文件都走这条。
pub fn detect_chapter_title(line: &str) -> Option<String> {
    let text = line.trim();
    if let Some((1, heading)) = parse_heading(text) {
        let heading = heading.trim();
        return if heading.is_empty() {
            None
        } else {
            Some(heading.to_string())
        };
    }
    if is_plain_chapter_title(text) {
        Some(text.to_string())
    } else {
        None
    }
}

/// 纯文本章题（§7.2）：行长 ≤ 40、不含 `。；，`、且带可读编号前缀。
///
/// 三条缺一不可——真实标书正文里的 `29.32万平方米五清十拆…，…。` 正是靠"逗号/句号"
/// 与"多级号未收尾"两条同时挡住的。
fn is_plain_chapter_title(text: &str) -> bool {
    !text.is_empty()
        && text.chars().count() <= 40
        && !text.chars().any(|c| matches!(c, '。' | '；' | '，'))
        && has_readable_number(text)
}

// ---------------------------------------------------------------------------
// 单测（D2：7 条，图片经临时目录复制，不依赖仓库真实路径写法；+§7 项目模式 6 条）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    const IMAGE: &str = "img.png";

    fn asset_png() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/bid-export/assets/image3.png")
    }

    /// 临时目录 + 复制一张真 PNG（解析器只校验存在性，但仍用真图，避免假文件）。
    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qidi-bid-p11-md-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(asset_png(), dir.join(IMAGE)).unwrap();
        dir
    }

    fn headings(chapter: &Chapter) -> Vec<(u8, String)> {
        chapter
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Heading { level, text } => Some((*level, text.clone())),
                _ => None,
            })
            .collect()
    }

    /// 1 个 H1 + 3 个 H2 → 标题正确、3 章、自动编号 第一章/第二章/第三章
    #[test]
    fn parses_title_and_chapters() {
        let dir = tmp_dir("title");
        let src = "# 槐林镇2026年度和美乡村建设项目施工组织设计\n\n\
                   ## 工程概况\n\n本工程为省级中心村建设项目。\n\n\
                   ## 施工部署\n\n先地下后地上。\n\n\
                   ## 质量保证\n\n三检制。\n";
        let (title, chapters) = parse_markdown(src, &dir).unwrap();
        assert_eq!(title, "槐林镇2026年度和美乡村建设项目施工组织设计");
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "第一章 工程概况");
        assert_eq!(chapters[1].title, "第二章 施工部署");
        assert_eq!(chapters[2].title, "第三章 质量保证");
        assert_eq!(
            chapters[0].blocks.len(),
            1,
            "导语与空行不得产出多余块: {:?}",
            chapters[0].blocks
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H2 自带"第一章 已有" → 原样保留，下一章仍按序号递增为"第二章"
    #[test]
    fn keeps_existing_numbering() {
        let dir = tmp_dir("keep");
        let src = "# 标题\n\n## 第一章 已有章节\n\n正文一。\n\n## 新增章节\n\n正文二。\n";
        let (_, chapters) = parse_markdown(src, &dir).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(chapters[0].title, "第一章 已有章节");
        assert_eq!(chapters[1].title, "第二章 新增章节");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H3/H4 → Block::Heading level 3/4，编号 1.1 / 1.1.1
    #[test]
    fn parses_heading_levels() {
        let dir = tmp_dir("levels");
        let src = "# 标题\n\n## 工程概况\n\n### 项目简介\n\n#### 建设内容\n\n正文。\n";
        let (_, chapters) = parse_markdown(src, &dir).unwrap();
        assert_eq!(chapters.len(), 1);
        assert_eq!(
            headings(&chapters[0]),
            vec![
                (3u8, "1.1 项目简介".to_string()),
                (4u8, "1.1.1 建设内容".to_string()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// GFM 表格：3 列 2 行数据 → Block::Table 的 header/rows 正确
    #[test]
    fn parses_gfm_table() {
        let dir = tmp_dir("table");
        let src = "# 标题\n\n## 主要材料\n\n\
                   | 材料 | 数量 | 单价 |\n\
                   | --- | :---: | ---: |\n\
                   | 水泥 | 10 | 400 |\n\
                   | 中砂 | 20 | 100 |\n";
        let (_, chapters) = parse_markdown(src, &dir).unwrap();
        let table = chapters[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table { header, rows } => Some((header, rows)),
                _ => None,
            })
            .expect("未解析出表格");
        assert_eq!(table.0.len(), 3);
        assert_eq!(table.0.join(","), "材料,数量,单价");
        assert_eq!(table.1.len(), 2);
        assert_eq!(table.1[0].join(","), "水泥,10,400");
        assert_eq!(table.1[1].join(","), "中砂,20,100");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 单图 `![图1](img.png)` → Single + caption（路径解析为绝对且存在）
    #[test]
    fn parses_single_image() {
        let dir = tmp_dir("single");
        let src = format!("# 标题\n\n## 示意图\n\n![图1]({IMAGE})\n");
        let (_, chapters) = parse_markdown(&src, &dir).unwrap();
        let fig = chapters[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .expect("未解析出图块");
        assert_eq!(fig.layout, FigureLayout::Single);
        assert_eq!(fig.caption_mode, CaptionMode::PerImage);
        assert_eq!(fig.items.len(), 1);
        assert_eq!(fig.items[0].caption.as_deref(), Some("图1"));
        assert!(Path::new(&fig.items[0].path).is_absolute(), "{}", fig.items[0].path);
        assert!(Path::new(&fig.items[0].path).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 连续 3 行图片（无空行）→ 一个 three_up + 3 个逐图图注
    #[test]
    fn parses_three_up_images() {
        let dir = tmp_dir("threeup");
        let src = format!(
            "# 标题\n\n## 三栏\n\n![图一]({IMAGE})\n![图二]({IMAGE})\n![图三]({IMAGE})\n"
        );
        let (_, chapters) = parse_markdown(&src, &dir).unwrap();
        let figs: Vec<&FigureBlock> = chapters[0]
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .collect();
        assert_eq!(figs.len(), 1, "3 行图片应合成 1 个图块");
        assert_eq!(figs[0].layout, FigureLayout::ThreeUp);
        assert_eq!(figs[0].caption_mode, CaptionMode::PerImage);
        assert_eq!(figs[0].items.len(), 3);
        let caps: Vec<Option<&str>> = figs[0].items.iter().map(|i| i.caption.as_deref()).collect();
        assert_eq!(caps, vec![Some("图一"), Some("图二"), Some("图三")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 图片路径不存在 → Err 含"图片不存在"
    #[test]
    fn missing_image_rejected() {
        let dir = tmp_dir("missing");
        let src = "# 标题\n\n## 示意图\n\n![图1](not-exist.png)\n";
        let err = parse_markdown(src, &dir).unwrap_err();
        assert!(
            err.to_string().contains("图片不存在"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H2 后直接 H4（无 H3）→ 节号补 1，产出 1.1.1 而非 1.0.1
    #[test]
    fn h4_without_h3_gets_one_based_section() {
        let dir = tmp_dir("h4-first");
        let src = "# 标题\n\n## 工程概况\n\n#### 建设内容\n";
        let (_, chapters) = parse_markdown(src, &dir).unwrap();
        assert_eq!(
            headings(&chapters[0]),
            vec![(4u8, "1.1.1 建设内容".to_string())]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 显式 `### 1.5` → sec_no 同步为 5，下一个未编号 H3 为 1.6
    #[test]
    fn explicit_section_number_syncs_counter() {
        let dir = tmp_dir("sec-sync");
        let src = "# 标题\n\n## 工程概况\n\n### 1.5 已有\n\n### 新增\n";
        let (_, chapters) = parse_markdown(src, &dir).unwrap();
        assert_eq!(
            headings(&chapters[0]),
            vec![
                (3u8, "1.5 已有".to_string()),
                (3u8, "1.6 新增".to_string()),
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 表头 3 列、分隔行 2 列 → Err 含"表格第 1 行列数不符"（不静默降级为段落）
    #[test]
    fn table_column_mismatch_rejected() {
        let dir = tmp_dir("table-bad");
        let src = "# 标题\n\n## 主要材料\n\n\
                   | 材料 | 数量 | 单价 |\n\
                   | --- | --- |\n\
                   | 水泥 | 10 | 400 |\n";
        let err = parse_markdown(src, &dir).unwrap_err();
        assert!(
            err.to_string().contains("表格第 1 行列数不符"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `{wide}` 后缀 → FigureLayout::Wide
    #[test]
    fn wide_image_parsed() {
        let dir = tmp_dir("wide");
        let src = format!("# 标题\n\n## 示意图\n\n![图1]({IMAGE}){{wide}}\n");
        let (_, chapters) = parse_markdown(&src, &dir).unwrap();
        let fig = chapters[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .expect("未解析出图块");
        assert_eq!(fig.layout, FigureLayout::Wide);
        assert_eq!(fig.items[0].caption.as_deref(), Some("图1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 单图 `{ai}` 后缀 → ai_generated = true
    #[test]
    fn single_image_ai_flag() {
        let dir = tmp_dir("ai");
        let src = format!("# 标题\n\n## 示意图\n\n![图]({IMAGE}){{ai}}\n");
        let (_, chapters) = parse_markdown(&src, &dir).unwrap();
        let fig = chapters[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .expect("未解析出图块");
        assert!(fig.ai_generated);
        assert_eq!(fig.layout, FigureLayout::Single);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `{wide,ai}` 组合后缀 → Wide + ai_generated = true
    #[test]
    fn wide_and_ai_combined() {
        let dir = tmp_dir("wide-ai");
        let src = format!("# 标题\n\n## 示意图\n\n![图]({IMAGE}){{wide,ai}}\n");
        let (_, chapters) = parse_markdown(&src, &dir).unwrap();
        let fig = chapters[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .expect("未解析出图块");
        assert_eq!(fig.layout, FigureLayout::Wide);
        assert!(fig.ai_generated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// N1 新测：未知/空图片标记 → Err 含"图片标记不识别"（不得静默降级成段落丢图）
    #[test]
    fn unknown_image_flag_rejected() {
        let dir = tmp_dir("bad-flag");
        for bad in ["{AI}", "{width}", "{wide,}", "{}"] {
            let src = format!("# 标题\n\n## 示意图\n\n![图1]({IMAGE}){bad}\n");
            let err = parse_markdown(&src, &dir).unwrap_err();
            assert!(
                err.to_string().contains("图片标记不识别"),
                "bad={bad} 错误信息不符: {err}"
            );
        }
        // 花括号内原文原样回显（tail）
        let src = format!("# 标题\n\n## 示意图\n\n![图1]({IMAGE}){{wide,height}}\n");
        let err = parse_markdown(&src, &dir).unwrap_err();
        assert!(
            err.to_string().contains("图片标记不识别: wide,height"),
            "tail 应为花括号内原文: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 三栏里任一张带 `{ai}` → 整组 ai_generated = true
    #[test]
    fn three_up_ai_if_any() {
        let dir = tmp_dir("threeup-ai");
        let src = format!(
            "# 标题\n\n## 三栏\n\n![图一]({IMAGE})\n![图二]({IMAGE}){{ wide , ai }}\n![图三]({IMAGE})\n"
        );
        let (_, chapters) = parse_markdown(&src, &dir).unwrap();
        let figs: Vec<&FigureBlock> = chapters[0]
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .collect();
        assert_eq!(figs.len(), 1, "3 行图片应合成 1 个图块");
        assert_eq!(figs[0].layout, FigureLayout::ThreeUp);
        assert_eq!(figs[0].items.len(), 3);
        assert!(figs[0].ai_generated);
        // 逐图标记：只有第 2 张带 `{ai}`，不得整组都置 true（N1）
        let per_item: Vec<bool> = figs[0].items.iter().map(|i| i.ai).collect();
        assert_eq!(per_item, vec![false, true, false], "item.ai 应逐图标记");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gantt_of(chapter: &Chapter) -> &GanttBlock {
        chapter
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Gantt(g) => Some(g),
                _ => None,
            })
            .expect("未解析出横道图块")
    }

    /// P1-5 §2：围栏首行文本 = 标题，`开始..结束 | 工序名` 逐行成任务，空行忽略
    #[test]
    fn parses_gantt_fence() {
        let dir = tmp_dir("gantt");
        let src = "# 标题\n\n## 施工进度\n\n\
                   ```gantt 图4-1 施工进度横道图\n\
                   2026-12-01..2026-12-10 | 塘底清淤\n\
                   \n\
                   2026-12-05..2026-12-20 | 管网敷设\n\
                   2026-12-15..2026-12-25 | 路基整平\n\
                   ```\n";
        let (_, chapters) = parse_markdown(src, &dir).unwrap();
        let g = gantt_of(&chapters[0]);
        assert_eq!(g.title, "图4-1 施工进度横道图");
        let got: Vec<(String, String, String)> = g
            .tasks
            .iter()
            .map(|t| (t.name.clone(), t.start.clone(), t.end.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("塘底清淤".to_string(), "2026-12-01".to_string(), "2026-12-10".to_string()),
                ("管网敷设".to_string(), "2026-12-05".to_string(), "2026-12-20".to_string()),
                ("路基整平".to_string(), "2026-12-15".to_string(), "2026-12-25".to_string()),
            ]
        );
        assert_eq!(chapters[0].title, "第一章 施工进度");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1-5 §2：围栏无收尾 ```` ``` ```` → Err 且含"横道图代码块未闭合"
    #[test]
    fn gantt_unclosed_fence_rejected() {
        let dir = tmp_dir("gantt-unclosed");
        let src = "# 标题\n\n## 施工进度\n\n```gantt\n2026-12-01..2026-12-10 | 塘底清淤\n";
        let err = parse_markdown(src, &dir).unwrap_err();
        assert!(
            err.to_string().contains("横道图代码块未闭合"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1-5 §2：缺 `|` 或 `..`（含工序名为空）→ Err 且含"横道图行格式错误: <行（已 trim）>"
    #[test]
    fn gantt_bad_line_rejected() {
        let dir = tmp_dir("gantt-bad-line");
        for bad in ["2026-12-01..2026-12-10", "2026-12-01 2026-12-10 | 塘底清淤", "2026-12-01..2026-12-10 | "] {
            let src = format!("# 标题\n\n## 施工进度\n\n```gantt\n{bad}\n```\n");
            let err = parse_markdown(&src, &dir).unwrap_err();
            assert!(
                err.to_string()
                    .contains(&format!("横道图行格式错误: {}", bad.trim())),
                "bad={bad} 错误信息不符: {err}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1-5 §2：围栏内只有空行 → Err 且含"横道图至少需要一个工序"
    #[test]
    fn gantt_empty_tasks_rejected() {
        let dir = tmp_dir("gantt-empty");
        let src = "# 标题\n\n## 施工进度\n\n```gantt 图4-1\n\n\n```\n";
        let err = parse_markdown(src, &dir).unwrap_err();
        assert!(
            err.to_string().contains("横道图至少需要一个工序"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // §7 项目模式（一章一文件）：章题首行识别 / 编号不叠加 / 缺图降级
    // -----------------------------------------------------------------------

    fn plain(chapter: &Chapter) -> Vec<String> {
        chapter
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Paragraph { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn figures(chapter: &Chapter) -> Vec<&FigureBlock> {
        chapter
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Figure(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// ①§7.2：`一、工程概况` 已带中文序号 → 不再变成 `第一章 一、…`；计数器照常推进
    #[test]
    fn cn_numbered_chapter_not_double_numbered() {
        let mut chapters = vec![
            Chapter {
                title: "一、工程概况".to_string(),
                blocks: Vec::new(),
            },
            Chapter {
                title: "二、主要施工方法".to_string(),
                blocks: vec![Block::Heading {
                    level: 3,
                    text: "（一）施工准备".to_string(),
                }],
            },
            Chapter {
                title: "第9章 安全生产".to_string(),
                blocks: Vec::new(),
            },
            Chapter {
                title: "质量保证".to_string(),
                blocks: Vec::new(),
            },
        ];
        renumber(&mut chapters);
        assert_eq!(chapters[0].title, "一、工程概况", "中文序号不得叠加章号");
        assert_eq!(chapters[1].title, "二、主要施工方法");
        assert_eq!(chapters[2].title, "第9章 安全生产", "阿拉伯章号原样保留");
        assert_eq!(chapters[3].title, "第十章 质量保证", "无编号的仍按序号接续");
        assert_eq!(
            headings(&chapters[1]),
            vec![(3u8, "（一）施工准备".to_string())],
            "中文括号序号不得叠加节号"
        );
    }

    /// ②§7.1：纯文本首行 `一、工程概况` 识别为章题；正文 `###` → H4
    #[test]
    fn plain_number_first_line_is_chapter_title() {
        let dir = tmp_dir("plain-first");
        let src = "一、工程概况\n\n本项目为省级中心村建设项目。\n\n### 1.1 项目概况\n\n塘底清淤约2.54万立方米。\n";
        let (chapter, warnings) = parse_chapter_markdown(src, &dir, false, "01-ch1", true).unwrap();
        assert_eq!(chapter.title, "一、工程概况", "纯文本编号行应成为章题");
        assert_eq!(headings(&chapter), vec![(4u8, "1.1 项目概况".to_string())]);
        assert!(warnings.is_empty(), "无缺图不应有 warning: {warnings:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ②b§7.1：`# 一、工程概况` 同样识别为章题，且不被加 `第一章` 前缀
    #[test]
    fn h1_numbered_first_line_is_chapter_title() {
        let dir = tmp_dir("h1-first");
        let src = "# 二、主要施工方法\n\n正文一。\n\n## 2.1 施工顺序\n\n正文二。\n";
        let (chapter, _) = parse_chapter_markdown(src, &dir, false, "02-ch2", true).unwrap();
        assert_eq!(chapter.title, "二、主要施工方法");
        assert_eq!(headings(&chapter), vec![(3u8, "2.1 施工顺序".to_string())]);
        assert_eq!(plain(&chapter), vec!["正文一。".to_string(), "正文二。".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ③§7.1：首行既不是 H1 也不是编号行 → 用 fallback 章题（文件名去 `NN-`）
    #[test]
    fn chapter_title_falls_back_to_file_name() {
        let dir = tmp_dir("fallback");
        let src = "本项目为省级中心村建设项目。\n\n### 1.1 项目概况\n";
        let (chapter, _) = parse_chapter_markdown(src, &dir, false, "工程概况", true).unwrap();
        assert_eq!(chapter.title, "工程概况", "无可识别首行时用 fallback");
        assert_eq!(headings(&chapter), vec![(4u8, "1.1 项目概况".to_string())]);
        assert_eq!(plain(&chapter), vec!["本项目为省级中心村建设项目。".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ④§7.2：原文 ch1:98 / ch1:106 的"29.32万平方米…"/"2.54万立方米…" 不得成为标题
    #[test]
    fn numeric_body_lines_are_not_titles() {
        let dir = tmp_dir("not-title");
        let body_98 = "29.32万平方米五清十拆与各分项工程散布村域，单作业面工程量小，无法组织大规模连续施工，作业面转换频繁，队伍窝工与调度失效风险高。";
        let body_106 = "2.54万立方米清淤量分布于多口水塘，围堰修筑与抽排水受汛期水位制约，淤泥含水率高，处置去向与环保要求直接相关。";
        // 首行判定层：量词（多级号未收尾 / 含标点）都不算章题
        assert!(detect_chapter_title(body_98).is_none(), "长句不得成章题");
        assert!(detect_chapter_title(body_106).is_none());
        assert!(detect_chapter_title("29.32万平方米").is_none(), "带小数的量不是编号");
        assert!(detect_chapter_title("2.54万立方米").is_none());
        // 解析层：出现在正文里只能是段落，绝不产出标题
        let src = format!("# 工程概况\n\n{body_98}\n\n{body_106}\n");
        let (chapter, _) = parse_chapter_markdown(&src, &dir, false, "01", true).unwrap();
        assert!(
            headings(&chapter).is_empty(),
            "正文数字行不得产出标题: {:?}",
            headings(&chapter)
        );
        assert_eq!(plain(&chapter), vec![body_98.to_string(), body_106.to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⑤§7.4：`strict_images = true` → 缺图仍是硬错误（与 `--md` 口径一致）
    #[test]
    fn chapter_missing_image_strict_rejected() {
        let dir = tmp_dir("chapter-strict");
        let src = "# 工程概况\n\n![施工现场平面布置](assets/nope.png)\n";
        let err = parse_chapter_markdown(src, &dir, false, "01", true).unwrap_err();
        assert!(
            err.to_string().contains("图片不存在: assets/nope.png"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// ⑥§7.4：非 strict → 跳过缺图、图注留成段落、有 warning，其余结构照常
    #[test]
    fn chapter_missing_image_warns_and_continues() {
        let dir = tmp_dir("chapter-warn");
        let src = format!(
            "# 工程概况\n\n![缺失的图](assets/nope.png)\n\n### 1.1 项目概况\n\n![好图]({IMAGE})\n"
        );
        let (chapter, warnings) = parse_chapter_markdown(&src, &dir, false, "01", false).unwrap();
        assert_eq!(warnings, vec!["缺图: assets/nope.png".to_string()]);
        assert_eq!(
            headings(&chapter),
            vec![(4u8, "1.1 项目概况".to_string())],
            "缺图不得影响其余结构解析"
        );
        let figs = figures(&chapter);
        assert_eq!(figs.len(), 1, "缺图不生成 drawing，存在的图照常嵌入");
        assert_eq!(figs[0].items.len(), 1);
        assert_eq!(figs[0].items[0].caption.as_deref(), Some("好图"));
        assert_eq!(
            plain(&chapter),
            vec!["缺失的图".to_string()],
            "缺图的图注应保留为普通段落"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
