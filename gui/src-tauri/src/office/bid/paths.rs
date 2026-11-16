//! 标书模板路径解析（P0a §8.1）。
//!
//! dev: `CARGO_MANIFEST_DIR/resources/templates/`；安装态: `resource_dir()/templates/`。

use std::path::{Path, PathBuf};

use tauri::Manager;

/// 默认模板样式 id（对应 resources/templates/bid-engineering-blue-v1.docx）。
pub const DEFAULT_STYLE_ID: &str = "bid-engineering-blue-v1";

/// `style_id` 白名单校验：仅允许 `[a-z0-9-]`，防路径穿越。
fn validate_style_id(style_id: &str) -> Result<(), String> {
    if style_id.is_empty()
        || !style_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("非法的模板 id".to_string());
    }
    Ok(())
}

/// 可测核心：`resource_dir` 为 `None` 时只走 dev 回退。
pub(crate) fn resolve_from(style_id: &str, resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    validate_style_id(style_id)?;
    let name = format!("{style_id}.docx");
    if let Some(dir) = resource_dir {
        let candidate = dir.join("templates").join(&name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources/templates")
        .join(&name);
    if candidate.is_file() {
        return Ok(candidate);
    }
    Err(format!("模板未找到: {name}"))
}

/// 解析模板真身：安装态优先（resource_dir/templates/），dev 回退到仓库内 resources。
pub fn resolve_template_path(app: &tauri::AppHandle, style_id: &str) -> Result<PathBuf, String> {
    resolve_from(style_id, app.path().resource_dir().ok().as_deref())
}

/// lib 的开发态模板目录（`CARGO_MANIFEST_DIR/resources/templates`）。
///
/// 宏在本 lib 内展开，`CARGO_MANIFEST_DIR` 恒为 `gui/src-tauri`；
/// 独立 crate `qidiwork-cli`（`src-tauri/cli`）内的宏会指向该目录，故 CLI 侧一律走本函数/`resolve_for_exe`。
pub fn dev_templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/templates")
}

/// CLI 用解析：先 `current_exe` 同级 `templates/<id>.docx`（source=`resource`，安装/分发态），
/// 再本 lib 的 `resources/templates/<id>.docx`（source=`dev`，开发态）。
///
/// `source` 供 CLI 摘要打印（如 `"template_source":"resource"`），便于判定命中的是哪一支。
pub fn resolve_for_exe(style_id: &str) -> Result<(PathBuf, &'static str), String> {
    validate_style_id(style_id)?;
    let name = format!("{style_id}.docx");
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    if let Some(dir) = exe_dir {
        let candidate = dir.join("templates").join(&name);
        if candidate.is_file() {
            return Ok((candidate, "resource"));
        }
    }
    let candidate = dev_templates_dir().join(&name);
    if candidate.is_file() {
        return Ok((candidate, "dev"));
    }
    Err(format!("模板未找到: {name}"))
}

/// `--self-check` 只读探测入口：把 `resolve_from` 暴露给 bin crate。
///
/// 无副作用（只做 `is_file` 探测，不读内容、不写盘、不改状态），
/// 仅供 `qidiwork-gui.exe --self-check` 在安装态/开发态打印模板解析结果。
pub fn probe_resolve(style_id: &str, resource_dir: Option<&Path>) -> Result<PathBuf, String> {
    resolve_from(style_id, resource_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径穿越 / 非法字符一律拒绝（`../x`、`A/B` 均不得命中）。
    #[test]
    fn illegal_style_id_rejected() {
        for bad in ["../x", "A/B", "", "a.b", "a b", "a_b"] {
            assert!(resolve_from(bad, None).is_err(), "{bad} 应被拒绝");
        }
    }

    /// dev 回退：默认样式 id + `None` 解析到真实存在的模板文件。
    #[test]
    fn default_style_resolves_to_existing_dev_template() {
        let p = resolve_from(DEFAULT_STYLE_ID, None);
        assert!(p.is_ok(), "dev 模板应存在: {:?}", p.as_ref().err());
        if let Ok(path) = p {
            assert!(path.is_file(), "模板路径必须是文件: {path:?}");
        }
    }
}
