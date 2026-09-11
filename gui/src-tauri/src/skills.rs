//! 技能发现(方案 v2 P1):只读扫描 `~/.qidi/skills/*/SKILL.md` 的
//! frontmatter,供左区「常用技能」按钮展示与触发。
//!
//! 信任边界:技能来自用户本机 `~/.qidi`(与 TUI 同一数据面),GUI 只
//! 读取名称/描述做展示;点击触发是把技能名拼进自然语言任务经 ACP 下发,
//! 真正的技能执行与权限审批都在 agent 内核侧,GUI 不代执行。

use std::path::{Path, PathBuf};

use serde::Serialize;

/// 技能条目(目录名 = 技能 ID,与 agent 侧一致)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillInfo {
    /// 技能目录名,如 `bid-write`。
    pub name: String,
    /// frontmatter 的 name 字段,缺省回落目录名。
    pub display_name: String,
    /// frontmatter 的 description 字段(仅首行,缺省为空)。
    pub description: String,
}

/// skills 根目录。
pub fn skills_root(home: &Path) -> PathBuf {
    home.join(".qidi").join("skills")
}

/// 从 SKILL.md 头部提取 frontmatter 的 name/description。
/// 只认 `---` 围栏内 `key: value` 行的首个匹配,不做完整 YAML(避免为
/// 两三个字段引入依赖);值两侧成对引号剥掉。
fn parse_frontmatter(raw: &str) -> (Option<String>, Option<String>) {
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    // frontmatter 语法要求 `---` 是文件第一行(容忍 BOM);正文中间的
    // `---` 水平线不得误判为开栅栏
    let mut lines = raw.lines();
    let first = lines
        .next()
        .map(|l| l.trim_start_matches('\u{feff}').trim_end());
    if first != Some("---") {
        return (None, None);
    }
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        // 对称剥成对双/单引号(k3 补充审计:单引号值原样带引号展示)
        let value = match (value.strip_prefix('"'), value.strip_prefix('\'')) {
            (Some(v2), _) => v2.strip_suffix('"').unwrap_or(v2),
            (None, Some(v2)) => v2.strip_suffix('\'').unwrap_or(v2),
            (None, None) => value,
        };
        match key.trim() {
            "name" if name.is_none() => name = Some(value.to_string()),
            "description" if description.is_none() => description = Some(value.to_string()),
            _ => {}
        }
    }
    (name, description)
}

/// 单个 SKILL.md 的读取上限(异常大的文件视为损坏/恶意,跳过该技能)。
const MAX_SKILL_MD_BYTES: u64 = 1024 * 1024;

/// 扫描技能目录:每个含 SKILL.md 的子目录记一条。SKILL.md 缺失的目录
/// 静默跳过(与 TUI 技能发现一致:无 SKILL.md 即非技能);存在但读取
/// 失败(权限/编码等)记 warn 后跳过。
pub fn list_skills(root: &Path) -> Vec<SkillInfo> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(dir_name) = entry.file_name().into_string() else {
            continue;
        };
        let skill_md = path.join("SKILL.md");
        if let Ok(meta) = std::fs::metadata(&skill_md)
            && meta.len() > MAX_SKILL_MD_BYTES
        {
            tracing::warn!(dir = %dir_name, "SKILL.md 超过 1MB 上限,跳过该技能");
            continue;
        }
        let raw = match std::fs::read_to_string(&skill_md) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!(dir = %dir_name, error = %e, "SKILL.md 读取失败,跳过该技能");
                continue;
            }
        };
        let (name, description) = parse_frontmatter(&raw);
        list.push(SkillInfo {
            display_name: name.unwrap_or_else(|| dir_name.clone()),
            name: dir_name,
            description: description.unwrap_or_default(),
        });
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_parses_name_and_description() {
        let raw = "---\nname: bid-write\ndescription: \"标书编写:按大纲生成分章内容。\"\n---\n\n# 正文\n";
        let (name, description) = parse_frontmatter(raw);
        assert_eq!(name.as_deref(), Some("bid-write"));
        assert_eq!(description.as_deref(), Some("标书编写:按大纲生成分章内容。"));
    }

    #[test]
    fn frontmatter_without_quotes_and_missing_fields() {
        let raw = "---\nname: office-tools\nother: x\n---\nbody";
        let (name, description) = parse_frontmatter(raw);
        assert_eq!(name.as_deref(), Some("office-tools"));
        assert_eq!(description, None);
    }

    #[test]
    fn frontmatter_absent_yields_nothing() {
        let (name, description) = parse_frontmatter("# 纯正文,无围栏\n---\nname: 不算数\n");
        assert_eq!(name, None);
        assert_eq!(description, None);
    }

    #[test]
    fn frontmatter_tolerates_bom() {
        let raw = "\u{feff}---\nname: bid-review\n---\n";
        let (name, _) = parse_frontmatter(raw);
        assert_eq!(name.as_deref(), Some("bid-review"));
    }

    #[test]
    fn list_skills_reads_only_dirs_with_skill_md() {
        let tmp = std::env::temp_dir().join(format!("qidiwork-skills-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let real = tmp.join("bid-write");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("SKILL.md"), "---\nname: bid-write\ndescription: d\n---\n").unwrap();
        // 无 SKILL.md 的目录与非目录项都应跳过
        std::fs::create_dir_all(tmp.join("not-a-skill")).unwrap();
        std::fs::write(tmp.join("loose-file"), "x").unwrap();

        let skills = list_skills(&tmp);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "bid-write");
        assert_eq!(skills[0].display_name, "bid-write");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
