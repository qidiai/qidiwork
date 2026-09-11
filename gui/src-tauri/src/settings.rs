//! 设置面(方案 v2 P2 发布阻塞项):读写 `~/.qidi/config.toml` 的模型
//! 选择(`[models].default`)与模型 API key(`[model.<id>].api_key`)。
//!
//! 纪律(照 cf-pager config_toml_edit 先例):
//! - toml_edit 保格式编辑,不碰文件里其他任何内容;
//! - 文件存在但解析失败 → 拒绝读写(绝不覆盖用户的坏文件);
//! - 写入走临时文件 + 原子 rename;
//! - API key 只进不出:读取接口永不回传密钥明文,仅回传 has_api_key。
//!
//! 已知取舍(k3 审计 W3):内核/TUI 也写同一 config.toml(无锁读-改-写),
//! 双方并发保存存在毫秒级丢失更新窗口;两侧均 toml_edit 保格式编辑,
//! 出现实际配置丢失时再引入文件锁。

use std::path::{Path, PathBuf};

use serde::Serialize;

/// config.toml 路径(与内核/TUI 同一数据面,方案 D7)。
pub fn config_path(home: &Path) -> PathBuf {
    home.join(".qidi").join("config.toml")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelOption {
    /// `[model.<id>]` 的 id(下拉框的值)。
    pub id: String,
    /// frontmatter 式 name 字段,缺省回落 id。
    pub name: String,
    pub base_url: Option<String>,
    pub has_api_key: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsInfo {
    /// 当前 `[models].default`(可能指向内置模型,不在 models 列表中)。
    pub default_model: Option<String>,
    pub models: Vec<ModelOption>,
}

/// 读取设置。文件不存在 → 空列表;解析失败 → Err(拒绝猜测)。
pub fn read_settings(path: &Path) -> Result<SettingsInfo, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SettingsInfo {
                default_model: None,
                models: Vec::new(),
            });
        }
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("{} 不是有效 TOML,拒绝读取: {e}", path.display()))?;

    let default_model = doc
        .get("models")
        .and_then(|i| i.get("default"))
        .and_then(|i| i.as_str())
        .map(str::to_string);

    let mut models = Vec::new();
    // 仅列出标准表:内联表(m = { … })与 dotted-key 定义的模型跳过——
    // 下拉框只提供可安全编辑 api_key 的条目(k3 审计 N3)
    if let Some(table) = doc.get("model").and_then(|i| i.as_table()) {
        for (id, item) in table {
            let Some(entry) = item.as_table() else {
                continue; // 非标准表(内联表等)不在下拉框提供编辑
            };
            let str_field = |key: &str| entry.get(key).and_then(|i| i.as_str()).map(str::to_string);
            models.push(ModelOption {
                id: id.to_string(),
                name: str_field("name").unwrap_or_else(|| id.to_string()),
                base_url: str_field("base_url"),
                has_api_key: entry
                    .get("api_key")
                    .and_then(|i| i.as_str())
                    .is_some_and(|k| !k.is_empty()),
            });
        }
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(SettingsInfo {
        default_model,
        models,
    })
}

/// 保存:设 `[models].default`,可选写 `[model.<id>].api_key`。
/// model_id 必须已存在于 `[model.*]`(只改定义过的模型,不代建)。
pub fn save_settings(
    path: &Path,
    model_id: &str,
    api_key: Option<&str>,
) -> Result<(), String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("{} 不是有效 TOML,拒绝覆盖: {e}", path.display()))?;

    // model_id 存在性校验:不存在的模型不代建、不写 default(防手滑写坏配置)
    let Some(models) = doc.get_mut("model").and_then(|i| i.as_table_mut()) else {
        return Err("config.toml 中没有 [model.*] 定义,无可选模型".to_string());
    };
    if !models.contains_key(model_id) {
        return Err(format!("模型 {model_id} 未在 config.toml 中定义"));
    }

    if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        let Some(entry) = models.get_mut(model_id).and_then(|i| i.as_table_mut()) else {
            return Err(format!(
                "模型 {model_id} 的配置不是标准表,拒绝写入 api_key"
            ));
        };
        entry["api_key"] = toml_edit::value(key);
    }

    if doc.get("models").and_then(|i| i.as_table()).is_some() {
        // [models] 已是标准表,直接写入 default
    } else if doc.get("models").is_some() {
        // 非表值(字符串/数组等):覆盖它会静默改写用户配置,违反
        // "绝不覆盖"纪律——拒绝并让用户手工修复(k3 审计 W2)
        return Err("[models] 已存在但不是标准表,拒绝覆盖;请手工修复该段".to_string());
    } else {
        doc["models"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["models"]["default"] = toml_edit::value(model_id);

    // 原子写:同目录临时文件 + rename,防半截文件
    let tmp = path.with_extension("toml.tmp");
    if let Err(e) = std::fs::write(&tmp, doc.to_string()) {
        let _ = std::fs::remove_file(&tmp); // 半截 tmp 不留痕(k3 审计 N2)
        return Err(format!("写入临时文件失败: {e}"));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("原子替换失败: {e}")
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qidiwork-settings-{tag}-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn read_lists_models_and_default() {
        let p = tmp_path("read");
        std::fs::write(
            &p,
            "[models]\ndefault = \"m2\"\n\n[model.m1]\nname = \"M One\"\nbase_url = \"https://a/v1\"\n\n[model.m2]\nmodel = \"k3\"\napi_key = \"sk-x\"\n",
        )
        .unwrap();
        let s = read_settings(&p).unwrap();
        assert_eq!(s.default_model.as_deref(), Some("m2"));
        assert_eq!(s.models.len(), 2);
        let m1 = s.models.iter().find(|m| m.id == "m1").unwrap();
        assert_eq!(m1.name, "M One");
        assert!(!m1.has_api_key);
        let m2 = s.models.iter().find(|m| m.id == "m2").unwrap();
        assert!(m2.has_api_key);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn save_sets_default_preserving_comments_and_writes_key() {
        let p = tmp_path("save");
        std::fs::write(
            &p,
            "# 顶部注释\n[models]\ndefault = \"m1\" # 旧默认\n\n[model.m1]\nmodel = \"a\"\n",
        )
        .unwrap();
        save_settings(&p, "m1", Some("  sk-new  ")).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# 顶部注释"), "注释必须保留");
        assert!(text.contains("default = \"m1\""));
        assert!(text.contains("api_key = \"sk-new\""), "trim 后写入: {text}");
        assert!(!std::path::Path::new(&p.with_extension("toml.tmp")).exists());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn save_rejects_unknown_model_and_leaves_file_untouched() {
        let p = tmp_path("unknown");
        let original = "[model.m1]\nmodel = \"a\"\n";
        std::fs::write(&p, original).unwrap();
        let err = save_settings(&p, "ghost", None).unwrap_err();
        assert!(err.contains("未在 config.toml 中定义"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn save_rejects_non_table_models_section() {
        let p = tmp_path("nontable");
        let original = "models = \"oops\"\n\n[model.m1]\nmodel = \"a\"\n";
        std::fs::write(&p, original).unwrap();
        let err = save_settings(&p, "m1", None).unwrap_err();
        assert!(err.contains("不是标准表"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn refuses_to_overwrite_unparseable_file() {
        let p = tmp_path("bad");
        std::fs::write(&p, "这不是 TOML [[[").unwrap();
        assert!(read_settings(&p).is_err());
        assert!(save_settings(&p, "m1", None).is_err());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "这不是 TOML [[[",
            "坏文件必须原样保留"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let p = tmp_path("missing");
        let s = read_settings(&p).unwrap();
        assert!(s.models.is_empty());
        assert_eq!(s.default_model, None);
        // 空配置下保存应明确拒绝(没有可选模型)
        assert!(save_settings(&p, "m1", None).is_err());
    }
}
