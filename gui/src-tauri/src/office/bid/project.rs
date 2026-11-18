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
//! - 每章一文件（§7.1）：文件**首行**是 `# 文本` 或纯文本编号行（`一、工程概况`）→ 该行即章题，
//!   其余 `##`/`###` 降为节/子节；首行不是这两类（老式 `##` 章节文件、骨架文件）→ 保持旧语义
//!   （预置书名 H1，每个 `##` 各自成章），既有项目不破。
//! - 图片相对路径一律按**项目根目录**（`dir`）解析——契约把 `assets/` 放在项目根，
//!   所以章节里写 `![图注](assets/x.png)`，与 md 文件自身所在目录无关。
//! - 缺图（§7.4）：默认 `strict_images = false` → 跳过该图、图注留成段落，warning 进
//!   [`validate_project`] 的 problems；清单写 `"strict_images": true` 恢复硬错误。
//! - `number = true` 时合并后再统一重号（各文件先按 `number = false` 解析，避免每个文件
//!   各自从"第一章"重来），调用 [`renumber`]，口径与 `markdown.rs` 完全一致。
//!
//! 错误文案：`章节文件不存在: {p}` / `项目清单缺失: bid.project.json` / `章节目录为空: chapters/`。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::markdown::{detect_chapter_title, parse_chapter_markdown, parse_markdown_with, renumber};
use super::model::{BidExportRequest, Chapter};
use super::paths::{resolve_from, DEFAULT_STYLE_ID};
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
    /// 页数上限（§7.5：CLI `--max-pages` 优先于本字段；都没有则不设限）。
    /// 本片只落字段，优先级与 CLI 接线由后续片完成。
    #[serde(default)]
    pub max_pages: Option<u32>,
    /// 缺图是否硬错误（§7.4）：默认 `false` = 跳过图片 + warning，`true` = `Err("图片不存在")`。
    #[serde(default)]
    pub strict_images: bool,
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
        max_pages: None,
        strict_images: false,
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
        let (mut parsed, warnings) = parse_chapter(&manifest, dir, path)?;
        chapters.append(&mut parsed);
        // 缺图 warning 由 CLI 打印（`WARN 缺图: …`），本层只保证不阻断、不留悬空图片引用
        let _ = warnings;
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
        match parse_chapter(&manifest, dir, path) {
            Ok((mut parsed, warnings)) => {
                chapters.append(&mut parsed);
                // 缺图（§7.4）：不是硬错误，但要作为问题报出来（`缺图: <path>`）
                for warning in warnings {
                    push_unique(&mut problems, warning);
                }
            }
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

/// 解析单个章节文件 → `(章节, warnings)`。
///
/// 图片相对路径按**项目根**解析（契约把 `assets/` 放在项目根，与 `chapters/` 平级，
/// 故 md 里写 `assets/x.png`）。
///
/// 两条路径（§7.1）：
/// - 首行是 `# 文本` / 纯文本编号行，且**不等于书名** → 新语义：一文件一章，
///   调 [`parse_chapter_markdown`]（章题取首行，`##`/`###` 降为节/子节，缺图按
///   `manifest.strict_images` 降级）；
/// - 否则（首行是书名本身、`##` 开头的老章节文件、或根本没有可识别首行）→ 旧语义：
///   预置一行书名 H1 屏蔽文件自身 H1，每个 `##` 各自成章，缺图仍为硬错误。
fn parse_chapter(
    manifest: &ProjectManifest,
    base_dir: &Path,
    path: &Path,
) -> BidResult<(Vec<Chapter>, Vec<String>)> {
    let src = std::fs::read_to_string(path)
        .map_err(|_| BidError::new(format!("章节文件不存在: {}", path.display())))?;
    let fail = |e: BidError| BidError::new(format!("章节解析失败 {}: {e}", path.display()));

    let first = src
        .trim_start_matches('\u{feff}')
        .lines()
        .find(|line| !line.trim().is_empty());
    let detected = first.and_then(|line| detect_chapter_title(line.trim()));
    let is_book_title = detected
        .as_deref()
        .is_some_and(|t| t.trim() == manifest.title.trim());

    match detected {
        Some(_) if !is_book_title => {
            let fallback = fallback_chapter_title(path);
            let (chapter, warnings) = parse_chapter_markdown(
                &src,
                base_dir,
                false, // 各文件先不编号，合并后统一 renumber
                &fallback,
                manifest.strict_images,
            )
            .map_err(fail)?;
            Ok((vec![chapter], warnings))
        }
        _ => {
            // 旧语义：预置书名 H1，让解析器忽略文件自身 H1；`##` 各自成章
            let prefixed = format!("# {}\n\n{src}", manifest.title.trim());
            let (_title, chapters) = parse_markdown_with(&prefixed, base_dir, false).map_err(fail)?;
            Ok((chapters, Vec::new()))
        }
    }
}

/// 章题兜底：文件名去目录、扩展名与 `NN-` / `NN_` 前缀（`chapters/01-工程概况.md` → `工程概况`）。
fn fallback_chapter_title(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let digits = stem.chars().take_while(|c| c.is_ascii_digit()).count();
    let rest = &stem[digits..];
    let trimmed = rest
        .strip_prefix('-')
        .or_else(|| rest.strip_prefix('_'))
        .unwrap_or(rest);
    let trimmed = if trimmed.trim().is_empty() {
        stem.as_str()
    } else {
        trimmed
    };
    trimmed.trim().to_string()
}

fn push_unique(problems: &mut Vec<String>, msg: String) {
    if !problems.contains(&msg) {
        problems.push(msg);
    }
}

/// 解析模板真身：清单 `style_id` 决定用哪套样式（缺省 / 空白 → [`DEFAULT_STYLE_ID`]）。
///
/// 固定走 `resolve_from(style_id, None)`（无安装态概念，落到 dev 回退
/// `resources/templates/`）；非法 id / 文件缺失均为 `Err`，CLI 映射成 `ERROR …` exit 2。
pub fn resolve_template(style_id: Option<&str>) -> BidResult<PathBuf> {
    let style_id = style_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_STYLE_ID);
    resolve_from(style_id, None).map_err(BidError::new)
}

// ---------------------------------------------------------------------------
// 单测（§A.4 五条；临时目录建在 std::env::temp_dir()，不入库 fixture）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::super::model::Block;
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
        write_manifest_full(dir, None, chapters);
    }

    /// 写清单（显式指定 `style_id`）
    fn write_manifest_full(dir: &Path, style_id: Option<String>, chapters: Vec<String>) {
        let manifest = ProjectManifest {
            title: TITLE.to_string(),
            style_id,
            figure_width_cm: None,
            max_pages: None,
            strict_images: false,
            chapters,
        };
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), json).unwrap();
    }

    /// 写清单原样 JSON（测 `max_pages` / `strict_images` 等字段解析）
    fn write_manifest_json(dir: &Path, json: &str) {
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
    ///
    /// §7.1 起章节文件是"一文件一章"：首行 `# <章题>` 即章题（书名仍取 manifest）。
    #[test]
    fn build_merges_files_in_order() {
        let dir = tmp_project("merge");
        write_manifest(&dir, Vec::new());
        // 故意先写 02，再写 01：合并顺序必须仍按文件名
        write_chapter(&dir, "02-施工部署.md", "# 施工部署\n\n先地下后地上。\n");
        write_chapter(&dir, "01-工程概况.md", "# 工程概况\n\n本工程为省级中心村建设项目。\n");

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

    /// 健康项目（图片放项目根 `assets/`，md 写 `assets/…`）→ `validate_project` 空 vec
    #[test]
    fn healthy_project_has_no_problems() {
        let dir = tmp_project("healthy");
        write_manifest(&dir, vec!["chapters/01-概述.md".to_string()]);
        std::fs::create_dir_all(dir.join(ASSETS_DIR)).unwrap();
        std::fs::copy(asset_png(), dir.join(ASSETS_DIR).join("image3.png")).unwrap();
        write_chapter(
            &dir,
            "01-概述.md",
            "## 工程概况\n\n### 项目简介\n\n![施工现场平面布置](assets/image3.png)\n\n正文。\n",
        );

        let problems = validate_project(&dir).unwrap();
        assert!(problems.is_empty(), "健康项目不应有问题: {problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 清单 `style_id` 生效：`build_request` 原样透传，`resolve_template` 按它取模板
    /// （不存在的 id → Err）；缺省（`None`）回落到默认模板。
    #[test]
    fn build_uses_manifest_style_id() {
        let dir = tmp_project("style-id");
        write_manifest_full(
            &dir,
            Some("no-such-style-v9".to_string()),
            vec!["chapters/01-概述.md".to_string()],
        );
        write_chapter(&dir, "01-概述.md", "## 工程概况\n\n正文。\n");

        let req = build_request(&dir, true).unwrap();
        assert_eq!(
            req.style_id.as_deref(),
            Some("no-such-style-v9"),
            "清单 style_id 应透传到请求"
        );

        let err = resolve_template(req.style_id.as_deref()).unwrap_err();
        assert!(
            err.to_string().contains("模板未找到"),
            "不存在的 style_id 应报模板缺失: {err}"
        );

        // 缺省 / 空白 → 默认模板仍可解析
        assert!(resolve_template(None).is_ok(), "None 应回落到默认模板");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // §7 项目模式：首行章题 / 缺图降级 / 老式文件兼容 / manifest 新字段
    // -----------------------------------------------------------------------

    /// §7.1 + §7.2：首行 `一、工程概况`（纯文本编号行）→ 一文件一章，标题不叠加"第一章"
    #[test]
    fn numbered_first_line_builds_one_chapter() {
        let dir = tmp_project("numbered-first");
        write_manifest(&dir, vec!["chapters/01-ch1.md".to_string()]);
        write_chapter(
            &dir,
            "01-ch1.md",
            "一、工程概况\n\n本项目为省级中心村建设项目。\n\n### 1.1 项目概况\n\n正文。\n",
        );

        let req = build_request(&dir, true).unwrap();
        assert_eq!(req.chapters.len(), 1, "一文件一章");
        assert_eq!(req.chapters[0].title, "一、工程概况", "不得出现'第一章 一、'");
        let heading = req.chapters[0]
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Heading { level, text } => Some((*level, text.clone())),
                _ => None,
            })
            .expect("应解析出节标题");
        assert_eq!(heading, (4u8, "1.1 项目概况".to_string()), "`###` 降为子节");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §7.4：项目模式缺图默认只记 warning（不阻断 build），`validate_project` 报"缺图"
    #[test]
    fn missing_image_warns_in_project_mode() {
        let dir = tmp_project("warn-img");
        write_manifest(&dir, vec!["chapters/01-ch1.md".to_string()]);
        write_chapter(
            &dir,
            "01-ch1.md",
            "一、工程概况\n\n![施工现场平面布置](assets/nope.png)\n\n正文。\n",
        );

        let req = build_request(&dir, true).unwrap();
        assert_eq!(req.chapters.len(), 1);
        assert!(
            !req.chapters[0]
                .blocks
                .iter()
                .any(|b| matches!(b, Block::Figure(_))),
            "缺图不得生成图片块（不留悬空引用）"
        );
        assert!(
            req.chapters[0]
                .blocks
                .iter()
                .any(|b| matches!(b, Block::Paragraph { text } if text == "施工现场平面布置")),
            "缺图的图注应保留为普通段落"
        );

        let problems = validate_project(&dir).unwrap();
        assert_eq!(problems, vec!["缺图: assets/nope.png".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §7.4：清单写 `"strict_images": true` → 缺图恢复硬错误
    #[test]
    fn strict_images_manifest_is_hard_error() {
        let dir = tmp_project("strict-img");
        write_manifest_json(
            &dir,
            &format!(
                r#"{{"title":"{TITLE}","strict_images":true,"chapters":["chapters/01-ch1.md"]}}"#
            ),
        );
        write_chapter(&dir, "01-ch1.md", "# 工程概况\n\n![平面布置](assets/nope.png)\n");

        let err = build_request(&dir, true).unwrap_err();
        assert!(
            err.to_string().contains("图片不存在: assets/nope.png"),
            "strict_images 应恢复硬错误: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 兼容（§7.1）：首行是 `##`（老式多章文件）→ 仍按每个 `##` 各自成章
    #[test]
    fn legacy_h2_chapter_file_still_splits() {
        let dir = tmp_project("legacy-h2");
        write_manifest(&dir, vec!["chapters/01-ch1.md".to_string()]);
        write_chapter(
            &dir,
            "01-ch1.md",
            "## 一、工程概况\n\n正文一。\n\n## 二、主要施工方法\n\n正文二。\n",
        );

        let req = build_request(&dir, true).unwrap();
        assert_eq!(req.chapters.len(), 2, "老式文件仍每 `##` 一章");
        assert_eq!(req.chapters[0].title, "一、工程概况", "中文序号不叠加章号");
        assert_eq!(req.chapters[1].title, "二、主要施工方法");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// manifest 新字段：`max_pages` / `strict_images` 缺省不炸、显式可读回
    #[test]
    fn manifest_max_pages_and_strict_images_default_and_set() {
        let dir = tmp_project("manifest-fields");
        write_manifest_json(&dir, &format!(r#"{{"title":"{TITLE}"}}"#));
        let manifest = load_manifest(&dir).unwrap();
        assert_eq!(manifest.max_pages, None, "缺省不设上限");
        assert!(!manifest.strict_images, "缺省缺图降级");

        write_manifest_json(
            &dir,
            &format!(r#"{{"title":"{TITLE}","max_pages":50,"strict_images":true}}"#),
        );
        let manifest = load_manifest(&dir).unwrap();
        assert_eq!(manifest.max_pages, Some(50));
        assert!(manifest.strict_images);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
