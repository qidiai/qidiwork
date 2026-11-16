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
//! 编号（[`parse_markdown`] 默认开，[`parse_markdown_with`] 可关）：章标题未匹配
//! `^第.+章` 且未匹配 `^\d+[.．、]` 时加 `第{中文数字}章 ` 前缀；`###`/`####` 未匹配
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
    let lines: Vec<&str> = src.trim_start_matches('\u{feff}').lines().collect();
    let mut p = MdParser {
        base_dir,
        number,
        title: None,
        chapters: Vec::new(),
        para: Vec::new(),
        chapter_no: 0,
        sec_no: 0,
        sub_no: 0,
    };

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
            let (gantt, next) = parse_gantt_fence(&lines, i, info)?;
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
            i += 1;
            continue;
        }
        if let Some(next) = p.try_table(&lines, i)? {
            i = next;
            continue;
        }
        p.para.push(text.to_string());
        i += 1;
    }
    p.flush_para();

    let title = p
        .title
        .ok_or_else(|| BidError::new("缺少文档标题（# 标题）"))?;
    Ok((title, p.chapters))
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
}

impl MdParser<'_> {
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
    fn push_image_run(&mut self, run: &[ImageSpec]) -> BidResult<()> {
        let mut idx = 0;
        while idx < run.len() {
            if run.len() - idx >= 3 {
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
    let path = Path::new(raw);
    let abs: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    if !abs.is_file() {
        return Err(BidError::new(format!("图片不存在: {raw}")));
    }
    Ok(abs.to_string_lossy().into_owned())
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

/// 章标题是否已编号：`^第.+章` 或 `^\d+[.．、]`。
fn matches_chapter_number(text: &str) -> bool {
    if let Some(rest) = text.strip_prefix('第') {
        let mut chars = rest.chars();
        // `.+` 至少一个字符，其后必须出现 `章`（"第章"不算）
        if let Some(first) = chars.next() {
            if first != '章' && chars.any(|c| c == '章') {
                return true;
            }
        }
    }
    let digits = text.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && matches!(text.chars().nth(digits), Some('.' | '．' | '、'))
}

/// 节/子节标题是否已编号：`^\d+(\.\d+)*`。
///
/// 字面实现——`(\.\d+)*` 可匹配空串，故等价于"以 ASCII 数字开头"。
/// 这样 `1.1 概述`、`1.1概述`、`1 概述` 都按已编号原样保留，不会被二次加前缀。
fn matches_section_number(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// 单测（D2：7 条，图片经临时目录复制，不依赖仓库真实路径写法）
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
}
