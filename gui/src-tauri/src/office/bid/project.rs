//! P1-2 项目工作区：目录契约 ↔ `BidExportRequest`。
//!
//! 契约见 `docs/施工规格-P1-2-3-4-2026-11-16.md` §A（唯一真值）：
//!
//! ```text
//! <proj>/
//!   bid.project.json      # {"title":…,"style_id":null,"figure_width_cm":null,"chapters":[]}
//!   chapters/01-概述.md
//!   assets/
//! ```
//!
//! - `chapters` 为空或缺省 → 取 `chapters/*.md` 按**文件名升序**；显式列出则按列表顺序。
//! - 每个 md 按 [`parse_markdown_with`] 解析：**各文件自己的 H1 忽略**（书名取自 manifest），
//!   其 H2 各自成章；所有文件的章节按顺序拼接。
//! - `number = true` 时合并后再统一编号（各文件先按 `number = false` 解析，避免每个文件
//!   各自从"第一章"重来）；重号口径与 `markdown.rs` 一致：`第N章` / `N.M` / `N.M.K`，
//!   已编号的原样保留并同步计数器。
//!
//! 错误文案：`章节文件不存在: {p}` / `项目清单缺失: bid.project.json` / `章节目录为空: chapters/`。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::markdown::parse_markdown_with;
use super::model::{BidExportRequest, Block, Chapter};
use super::{BidError, BidResult};

/// 项目清单文件名
pub const MANIFEST_FILE: &str = "bid.project.json";
/// 章节目录名
pub const CHAPTERS_DIR: &str = "chapters";
/// 素材目录名
pub const ASSETS_DIR: &str = "assets";
/// `create_skeleton` 生成的示例章节（相对项目根）
pub const SKELETON_CHAPTER: &str = "chapters/01-概述.md";

/// `bid.project.json` 的模型（§A.2）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub title: String,
    #[serde(default)]
    pub style_id: Option<String>,
    #[serde(default)]
    pub figure_width_cm: Option<f32>,
    /// 为空 → 取 `chapters/*.md` 文件名升序；非空 → 以列表顺序为准
    #[serde(default)]
    pub chapters: Vec<String>,
}

/// 建骨架：`<dir>/bid.project.json` + `chapters/01-概述.md` + `assets/`。
///
/// 清单已存在 → `Err("项目已存在")`（CLI 映射成 `ERROR 项目已存在` exit 2）。
pub fn create_skeleton(dir: &Path, title: &str) -> BidResult<()> {
    let manifest_path = dir.join(MANIFEST_FILE);
    if manifest_path.exists() {
        return Err(BidError::new("项目已存在"));
    }
    std::fs::create_dir_all(dir.join(CHAPTERS_DIR))?;
    std::fs::create_dir_all(dir.join(ASSETS_DIR))?;

    let manifest = ProjectManifest {
        title: title.trim().to_string(),
        style_id: None,
        figure_width_cm: None,
        chapters: Vec::new(),
    };
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| BidError::new(format!("项目清单序列化失败: {e}")))?;
    std::fs::write(&manifest_path, format!("{json}\n"))?;
    std::fs::write(dir.join(SKELETON_CHAPTER), skeleton_markdown(title.trim()))?;
    Ok(())
}

/// 示例章节内容：H1 与 manifest 同名（解析时被忽略），两个 H2 保证 `build` 可跑通。
fn skeleton_markdown(title: &str) -> String {
    format!(
        "# {title}\n\n\
         ## 工程概况\n\n\
         （在此填写工程概况：建设地点、建设规模、总工期、主要工程内容等。）\n\n\
         ## 编制依据\n\n\
         （在此填写编制依据：招标文件、施工图纸、现行规范与验收标准等。）\n"
    )
}

/// 读 `<dir>/bid.project.json`；缺失 → `Err("项目清单缺失: bid.project.json")`。
pub fn load_manifest(dir: &Path) -> BidResult<ProjectManifest> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|_| BidError::new(format!("项目清单缺失: {MANIFEST_FILE}")))?;
    serde_json::from_str(&text)
        .map_err(|e| BidError::new(format!("项目清单解析失败 {MANIFEST_FILE}: {e}")))
}

/// 合并所有章节文件 → `BidExportRequest`（`output_path` 留空，由 CLI 覆盖）。
pub fn build_request(dir: &Path, number: bool) -> BidResult<BidExportRequest> {
    let manifest = load_manifest(dir)?;
    let paths = chapter_paths(dir, &manifest)?;

    let mut chapters: Vec<Chapter> = Vec::new();
    for path in &paths {
        let mut parsed = parse_chapter(&manifest.title, path)?;
        chapters.append(&mut parsed);
    }
    if number {
        renumber(&mut chapters);
    }

    Ok(BidExportRequest {
        title: manifest.title,
        chapters,
        output_path: String::new(),
        style_id: manifest.style_id,
        figure_width_cm: manifest.figure_width_cm,
    })
}

/// 体检：返回问题列表（空 = 健康）。清单缺失不 Err，而是作为一条问题返回，便于 `check` 汇总。
pub fn validate_project(dir: &Path) -> BidResult<Vec<String>> {
    let manifest = match load_manifest(dir) {
        Ok(m) => m,
        Err(e) => return Ok(vec![e.0]),
    };
    if manifest.title.trim().is_empty() {
        return Ok(vec!["项目标题为空".to_string()]);
    }

    let paths = match chapter_paths(dir, &manifest) {
        Ok(p) => p,
        Err(e) => return Ok(vec![e.0]),
    };

    let mut problems: Vec<String> = Vec::new();
    let mut chapters: Vec<Chapter> = Vec::new();
    for path in &paths {
        match parse_chapter(&manifest.title, path) {
            Ok(mut parsed) => chapters.append(&mut parsed),
            Err(e) => push_unique(&mut problems, e.0),
        }
    }

    // 结构层问题（空章标题、图注缺失等）只在文件层没问题时才查，避免同因重复报
    if problems.is_empty() {
        let req = BidExportRequest {
            title: manifest.title,
            chapters,
            output_path: String::new(),
            style_id: manifest.style_id,
            figure_width_cm: manifest.figure_width_cm,
        };
        if let Err(e) = req.validate() {
            problems.push(e.0);
        }
    }
    Ok(problems)
}

/// 章节文件路径列表：显式列表优先（缺文件即 Err），否则 `chapters/*.md` 文件名升序。
fn chapter_paths(dir: &Path, manifest: &ProjectManifest) -> BidResult<Vec<PathBuf>> {
    if manifest.chapters.is_empty() {
        let mut files = list_chapters_dir(&dir.join(CHAPTERS_DIR))?;
        if files.is_empty() {
            return Err(BidError::new("章节目录为空: chapters/"));
        }
        return Ok(std::mem::take(&mut files));
    }

    let mut out = Vec::with_capacity(manifest.chapters.len());
    for rel in &manifest.chapters {
        let path = join_rel(dir, rel);
        if !path.is_file() {
            return Err(BidError::new(format!(
                "章节文件不存在: {}",
                path.display()
            )));
        }
        out.push(path);
    }
    Ok(out)
}

/// 清单里的相对路径（`chapters/01-…md`）按段拼到项目根：避免 Windows 上出现
/// `proj\chapters/01.md` 这类混合分隔符；绝对路径原样采信。
fn join_rel(dir: &Path, rel: &str) -> PathBuf {
    let raw = Path::new(rel);
    if raw.is_absolute() {
        return raw.to_path_buf();
    }
    let mut path = dir.to_path_buf();
    for seg in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
        path.push(seg);
    }
    path
}

/// `chapters/` 下 `.md` 文件（文件名升序）；目录不存在按空处理（由调用方报"章节目录为空"）。
fn list_chapters_dir(dir: &Path) -> BidResult<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| BidError::new(format!("读取章节目录失败: {e}")))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
        if !is_md {
            continue;
        }
        files.push((entry.file_name().to_string_lossy().into_owned(), path));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files.into_iter().map(|(_, p)| p).collect())
}

/// 解析单个章节文件：图片路径按该文件所在目录解析；文件自身 H1 被预置书名屏蔽掉。
fn parse_chapter(title: &str, path: &Path) -> BidResult<Vec<Chapter>> {
    let src = std::fs::read_to_string(path).map_err(|_| {
        BidError::new(format!("章节文件不存在: {}", path.display()))
    })?;
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let src = with_doc_title(title, &src);
    let (_title, chapters) = parse_markdown_with(&src, &base, false)
        .map_err(|e| BidError::new(format!("章节解析失败 {}: {e}", path.display())))?;
    Ok(chapters)
}

/// 预置一行书名 H1：解析器见到它就不再采信文件自己的 H1（§A.1"各文件自己的 H1 忽略"），
/// 同时让没有 H1 的章节文件也能解析（否则解析器报"缺少文档标题"）。
fn with_doc_title(title: &str, src: &str) -> String {
    format!("# {title}\n\n{src}")
}

fn push_unique(problems: &mut Vec<String>, msg: String) {
    if !problems.contains(&msg) {
        problems.push(msg);
    }
}

// ---------------------------------------------------------------------------
// 合并后统一重号（口径对齐 markdown.rs；该模块的编号函数为私有，故在此复刻最小集）
// ---------------------------------------------------------------------------

/// 合并后统一重号：章号连续，`N.M` / `N.M.K` 随本章序号走；已编号的原样保留并同步计数器。
fn renumber(chapters: &mut [Chapter]) {
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
    Some((digits.parse::<usize>().ok()?, digits.len()))
}

/// `^第(.+)章` → 章序号（中文 1–10 或阿拉伯数字）。
fn explicit_chapter_number(text: &str) -> Option<usize> {
    let rest = text.strip_prefix('第')?;
    let idx = rest.find('章')?;
    if idx == 0 {
        return None; // "第章" 不算
    }
    let mid = &rest[..idx];
    mid.parse::<usize>().ok().or_else(|| cn_to_number(mid))
}

/// `^(\d+)\.(\d+)` → 第二个数字（`．` 全角句点同样接受）。
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
        if let Some(first) = chars.next() {
            if first != '章' && chars.any(|c| c == '章') {
                return true;
            }
        }
    }
    let digits = text.chars().take_while(|c| c.is_ascii_digit()).count();
    digits > 0 && matches!(text.chars().nth(digits), Some('.' | '．' | '、'))
}

/// 节/子节标题是否已编号：以 ASCII 数字开头即视为已编号。
fn matches_section_number(text: &str) -> bool {
    text.chars().next().is_some_and(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// 单测（§A.4 五条；临时目录建在 std::env::temp_dir()，不入库 fixture）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    const TITLE: &str = "某县2026年度高标准农田建设项目施工组织设计";

    fn tmp_project(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qidi-bid-p12-proj-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 写清单（`chapters` 为空 → 走目录扫描分支）
    fn write_manifest(dir: &Path, chapters: Vec<String>) {
        let manifest = ProjectManifest {
            title: TITLE.to_string(),
            style_id: None,
            figure_width_cm: None,
            chapters,
        };
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), json).unwrap();
    }

    fn write_chapter(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir.join(CHAPTERS_DIR)).unwrap();
        std::fs::write(dir.join(CHAPTERS_DIR).join(name), body).unwrap();
    }

    fn asset_png() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/bid-export/assets/image3.png")
    }

    /// §A.4-1：`new` 后三个路径存在，manifest 可读回
    #[test]
    fn skeleton_created() {
        let dir = tmp_project("skeleton");
        create_skeleton(&dir, TITLE).unwrap();

        assert!(dir.join(MANIFEST_FILE).is_file(), "清单应生成");
        assert!(dir.join(SKELETON_CHAPTER).is_file(), "示例章节应生成");
        assert!(dir.join(ASSETS_DIR).is_dir(), "assets 目录应生成");

        let manifest = load_manifest(&dir).unwrap();
        assert_eq!(manifest.title, TITLE);
        assert!(manifest.chapters.is_empty(), "骨架默认走目录扫描");
        assert!(manifest.style_id.is_none());

        // 重复 new → 项目已存在
        let err = create_skeleton(&dir, TITLE).unwrap_err();
        assert_eq!(err.to_string(), "项目已存在");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §A.4-2：两文件各 1 章 → 2 章，顺序按文件名升序（与写盘先后无关）
    #[test]
    fn build_merges_files_in_order() {
        let dir = tmp_project("merge");
        write_manifest(&dir, Vec::new());
        // 故意先写 02，再写 01：合并顺序必须仍按文件名
        write_chapter(&dir, "02-施工部署.md", "# 文件自己的书名（应被忽略）\n\n## 施工部署\n\n先地下后地上。\n");
        write_chapter(&dir, "01-工程概况.md", "# 另一个书名（应被忽略）\n\n## 工程概况\n\n本工程为省级中心村建设项目。\n");

        let req = build_request(&dir, true).unwrap();
        assert_eq!(req.title, TITLE, "书名取 manifest，不用文件 H1");
        assert_eq!(req.chapters.len(), 2);
        assert_eq!(req.chapters[0].title, "第一章 工程概况");
        assert_eq!(req.chapters[1].title, "第二章 施工部署");
        assert_eq!(req.chapters[0].blocks.len(), 1);
        assert!(req.output_path.is_empty(), "output_path 由 CLI 覆盖");

        // --no-number：原样保留，不加章号
        let raw = build_request(&dir, false).unwrap();
        assert_eq!(raw.chapters[0].title, "工程概况");
        assert_eq!(raw.chapters[1].title, "施工部署");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §A.4-3：manifest 列了不存在文件 → Err 含"章节文件不存在"
    #[test]
    fn missing_chapter_reported() {
        let dir = tmp_project("missing-ch");
        write_manifest(&dir, vec!["chapters/99-不存在.md".to_string()]);
        let err = build_request(&dir, true).unwrap_err();
        assert!(
            err.to_string().contains("章节文件不存在"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §A.4-4：chapters 目录为空 → Err 含"章节目录为空"
    #[test]
    fn empty_chapters_rejected() {
        let dir = tmp_project("empty-ch");
        write_manifest(&dir, Vec::new());
        std::fs::create_dir_all(dir.join(CHAPTERS_DIR)).unwrap();
        let err = build_request(&dir, true).unwrap_err();
        assert!(
            err.to_string().contains("章节目录为空"),
            "错误信息不符: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §A.4-5：md 引用不存在的图片 → `validate_project` 恰好 1 条问题
    #[test]
    fn validate_reports_missing_image() {
        let dir = tmp_project("missing-img");
        write_manifest(&dir, vec!["chapters/01-概述.md".to_string()]);
        std::fs::create_dir_all(dir.join(ASSETS_DIR)).unwrap();
        std::fs::copy(asset_png(), dir.join(ASSETS_DIR).join("image3.png")).unwrap();
        write_chapter(
            &dir,
            "01-概述.md",
            "## 工程概况\n\n![施工现场平面布置](assets/image3.png)\n\n![缺失的图](assets/nope.png)\n",
        );

        let problems = validate_project(&dir).unwrap();
        assert_eq!(problems.len(), 1, "应恰好 1 条问题: {problems:?}");
        assert!(
            problems[0].contains("图片不存在"),
            "问题信息不符: {}",
            problems[0]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
