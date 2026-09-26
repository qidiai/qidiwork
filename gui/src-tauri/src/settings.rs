//! 设置面(方案 v2 P2 发布阻塞项):读写 `~/.qidi/config.toml` 的模型
//! 选择(`[models].default`)与模型 API key(`[model.<id>].api_key`),
//! 并读写授权程度档位(`[ui].permission_mode` + 智能自动 allow 规则组)。
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
    /// `[ui].permission_mode` 原样透传;缺省/无法解析时为 None(前端回退谨慎询问)。
    pub permission_mode: Option<String>,
    /// `[permission].allow` 是否恰为智能自动只读规则组(见 [`SMART_AUTO_RULES`])。
    pub smart_auto_rules_active: bool,
}

/// 读取设置。文件不存在 → 空列表;解析失败 → Err(拒绝猜测)。
pub fn read_settings(path: &Path) -> Result<SettingsInfo, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SettingsInfo {
                default_model: None,
                models: Vec::new(),
                permission_mode: None,
                smart_auto_rules_active: false,
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
    let permission_mode = doc
        .get("ui")
        .and_then(|i| i.get("permission_mode"))
        .and_then(|i| i.as_str())
        .map(str::to_string);
    let smart_auto_rules_active = read_smart_rules_active(&doc);
    Ok(SettingsInfo {
        default_model,
        models,
        permission_mode,
        smart_auto_rules_active,
    })
}

/// `[permission].allow` 是否恰为智能自动只读规则组(逐元素比对,顺序与
/// 内容都要一致;用户手工改过的 allow 不算,回显不说谎)。
fn read_smart_rules_active(doc: &toml_edit::DocumentMut) -> bool {
    doc.get("permission")
        .and_then(|i| i.get("allow"))
        .and_then(|i| i.as_array())
        .is_some_and(|items| {
            items.len() == SMART_AUTO_RULES.len()
                && items
                    .iter()
                    .zip(SMART_AUTO_RULES)
                    .all(|(v, e)| v.as_str() == Some(e))
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

    write_default(path, &mut doc, model_id)
}

/// 写 `[models].default` 并原子落盘。`[models]` 已存在但不是标准表时拒绝
/// 覆盖——静默改写用户配置违反"绝不覆盖"纪律(k3 审计 W2)。
fn write_default(
    path: &Path,
    doc: &mut toml_edit::DocumentMut,
    model_id: &str,
) -> Result<(), String> {
    if doc.get("models").and_then(|i| i.as_table()).is_some() {
        // [models] 已是标准表,直接写入 default
    } else if doc.get("models").is_some() {
        return Err("[models] 已存在但不是标准表,拒绝覆盖;请手工修复该段".to_string());
    } else {
        doc["models"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    doc["models"]["default"] = toml_edit::value(model_id);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 {} 失败: {e}", parent.display()))?;
    }
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

/// 仅设 `[models].default`,**不要求**该 id 已定义于 `[model.*]`。
///
/// 与 [`save_settings`] 语义不同:后者面向"改一个本地已定义的模型(可顺带
/// 写 api_key)",故校验本地存在性防手滑;本函数面向引导页——Step 3 列的是
/// 服务端 `/v1/models` 目录里的模型,内核 `resolve_default_model` 拿
/// `[models].default` 去**服务端 catalog** 匹配(先按条目 key,再按 `model`
/// 字段),本地 `[model.*]` 只是可选覆盖。用 save_settings 写服务端 id 会
/// 一律得到"模型 X 未在 config.toml 中定义",引导第三步因此全废。
pub fn set_default_model(path: &Path, model_id: &str) -> Result<(), String> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err("模型 id 不能为空".to_string());
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("{} 不是有效 TOML,拒绝覆盖: {e}", path.display()))?;
    write_default(path, &mut doc, model_id)
}

/// 智能自动档放行的只读规则组(写入 `[permission].allow`,每项为裸工具名
/// = 该工具全通配;五项均在内核 cf-workspace `rules.rs::tool_name_to_filter`
/// 白名单内,由 `resolution.rs::parse_toml_permission_section` 的紧凑数组
/// 语法解析)。
pub const SMART_AUTO_RULES: [&str; 5] = ["Read", "Grep", "Glob", "WebSearch", "WebFetch"];

/// 设置授权程度档位(设置面「授权程度」,T3)。
///
/// - `ask`   → `permission_mode = "default"`,移除智能自动规则块;
/// - `smart` → `permission_mode = "default"`,写入只读 allow 规则组;
/// - `trust` → `permission_mode = "always-approve"`,移除智能自动规则块。
///
/// 未知档位在读文件/落盘前直接 Err,文件原样不动。生效时机(双层):
/// allow 规则对之后启动的会话生效(cf-shell spawn.rs 逐会话解析),
/// permission_mode 对之后启动的内核进程生效(permissions.rs 启动读取)。
pub fn set_permission_level(path: &Path, level: &str) -> Result<(), String> {
    let mode = permission_mode_for_level(level)?;
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("{} 不是有效 TOML,拒绝覆盖: {e}", path.display()))?;

    // [ui].permission_mode:档位写值;[ui] 非标准表时拒绝静默改写
    let ui = doc
        .entry("ui")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
    let Some(ui) = ui.as_table_like_mut() else {
        return Err("[ui] 已存在但不是标准表,拒绝覆盖;请手工修复该段".to_string());
    };
    ui.insert("permission_mode", toml_edit::value(mode));

    // [permission].allow:smart 写入只读规则组;ask/trust 移除规则块
    if level == "smart" {
        let permission = doc
            .entry("permission")
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        let Some(permission) = permission.as_table_like_mut() else {
            return Err("[permission] 已存在但不是标准表,拒绝覆盖;请手工修复该段".to_string());
        };
        // 就地改造已存在的 allow 数组:保留键值 Item 及其 decor(键前注释),
        // 整体 insert 会丢弃旧 Item 的 decor,导致用户注释丢失。
        if let Some(existing) = permission.get_mut("allow") {
            if let Some(arr) = existing.as_array_mut() {
                arr.clear();
                for rule in SMART_AUTO_RULES {
                    arr.push(rule);
                }
            } else {
                return Err(
                    "[permission].allow 已存在但不是数组,拒绝覆盖;请手工修复该键".to_string(),
                );
            }
        } else {
            let mut arr = toml_edit::Array::new();
            for rule in SMART_AUTO_RULES {
                arr.push(rule);
            }
            permission.insert("allow", toml_edit::value(arr));
        }
    } else if let Some(permission) = doc.get_mut("permission") {
        // 无 [permission] 段时无需清理
        let Some(permission) = permission.as_table_like_mut() else {
            return Err("[permission] 已存在但不是标准表,拒绝覆盖;请手工修复该段".to_string());
        };
        permission.remove("allow");
    }

    atomic_write(path, &doc.to_string())
}

/// 档位 → `[ui].permission_mode` 取值;未知档位在此拒绝(落盘前)。
fn permission_mode_for_level(level: &str) -> Result<&'static str, String> {
    match level {
        "ask" | "smart" => Ok("default"),
        "trust" => Ok("always-approve"),
        other => Err(format!("未知授权档位: {other}")),
    }
}

/// 原子落盘(与 [`write_default`]/[`create_model`] 同一模式):
/// 同目录临时文件 + rename,失败时清理半截 tmp。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 {} 失败: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("写入临时文件失败: {e}"));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("原子替换失败: {e}")
    })?;
    Ok(())
}

/// TOML 表名安全校验:仅允许 `[A-Za-z0-9_-]`,拒绝点号/引号/空白等
/// 会改变表名解析或注入结构的字符。
fn is_safe_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 新建 `[model.<id>]`(首次运行引导,发布链审计):GUI 此前只能改
/// 已存在模型的 key,新用户没有任何模型定义时无法从零配出第一个模型。
/// 写入内核 `[model.<id>]` 字段子集:model/base_url/name(+可选 api_key)。
pub fn create_model(
    path: &Path,
    id: &str,
    model: &str,
    base_url: &str,
    name: Option<&str>,
    api_key: Option<&str>,
) -> Result<(), String> {
    if !is_safe_model_id(id) {
        return Err("模型 id 只允许字母、数字、- 和 _(不能含点号/空格/引号)".to_string());
    }
    if model.trim().is_empty() || base_url.trim().is_empty() {
        return Err("model 与 base_url 不能为空".to_string());
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = content
        .parse()
        .map_err(|e| format!("{} 不是有效 TOML,拒绝覆盖: {e}", path.display()))?;

    let models = doc
        .entry("model")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
    let Some(models) = models.as_table_like_mut() else {
        return Err("[model] 已存在但不是表,拒绝写入;请手工修复该段".to_string());
    };
    if models.contains_key(id) {
        return Err(format!("模型 {id} 已存在,请改用其他 id 或直接编辑其 API Key"));
    }
    let mut entry = toml_edit::Table::new();
    entry["model"] = toml_edit::value(model.trim());
    entry["base_url"] = toml_edit::value(base_url.trim());
    let display = name.map(str::trim).filter(|n| !n.is_empty());
    entry["name"] = toml_edit::value(display.unwrap_or(id));
    if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        entry["api_key"] = toml_edit::value(key);
    }
    models.insert(id, toml_edit::Item::Table(entry));

    // 尚无默认模型时,新模型即为默认(第一个模型立即可用)
    if doc
        .get("models")
        .and_then(|i| i.get("default"))
        .and_then(|i| i.as_str())
        .is_none()
    {
        if doc.get("models").and_then(|i| i.as_table()).is_some() {
            // [models] 已是标准表,直接写入 default
        } else if doc.get("models").is_some() {
            return Err("[models] 已存在但不是标准表,拒绝覆盖;请手工修复该段".to_string());
        } else {
            doc["models"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        doc["models"]["default"] = toml_edit::value(id);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 {} 失败: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    if let Err(e) = std::fs::write(&tmp, doc.to_string()) {
        let _ = std::fs::remove_file(&tmp);
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
    fn set_default_model_accepts_id_absent_from_local_table() {
        // 引导页 Step 3 的核心回归:服务端目录里的 id 不在 [model.*] 中,
        // 也必须能写进 [models].default(save_settings 会在这里报错)。
        let p = tmp_path("set-default");
        std::fs::write(
            &p,
            "# 用户注释\n[models]\ndefault = \"m1\" # 旧默认\n\n[model.m1]\nmodel = \"a\"\n",
        )
        .unwrap();
        assert!(save_settings(&p, "KIMI-K3", None).is_err(), "save_settings 仍拒绝未定义 id");
        set_default_model(&p, "  KIMI-K3  ").unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# 用户注释"), "注释必须保留");
        assert!(text.contains("default = \"KIMI-K3\""), "trim 后写入: {text}");
        assert!(text.contains("[model.m1]"), "不得代建本地模型块");
        assert_eq!(read_settings(&p).unwrap().default_model.as_deref(), Some("KIMI-K3"));
        assert!(!std::path::Path::new(&p.with_extension("toml.tmp")).exists());
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn set_default_model_creates_models_table_when_absent() {
        let p = tmp_path("set-default-new");
        std::fs::write(&p, "[model.m1]\nmodel = \"a\"\n").unwrap();
        set_default_model(&p, "auto").unwrap();
        assert_eq!(read_settings(&p).unwrap().default_model.as_deref(), Some("auto"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn set_default_model_rejects_blank_and_missing_file() {
        let p = tmp_path("set-default-blank");
        std::fs::write(&p, "[models]\ndefault = \"m1\"\n").unwrap();
        assert!(set_default_model(&p, "   ").is_err());
        assert_eq!(read_settings(&p).unwrap().default_model.as_deref(), Some("m1"), "空 id 不得改动文件");
        // 文件不存在 → 新建(引导期用户还没跑过内核时就是这种状态)
        let fresh = tmp_path("set-default-fresh");
        set_default_model(&fresh, "GLM-5.3-FLASH").unwrap();
        assert_eq!(
            read_settings(&fresh).unwrap().default_model.as_deref(),
            Some("GLM-5.3-FLASH")
        );
        std::fs::remove_file(&fresh).ok();
    }

    #[test]
    fn set_default_model_refuses_non_table_models_section() {
        let p = tmp_path("set-default-nontable");
        std::fs::write(&p, "models = \"oops\"\n").unwrap();
        let err = set_default_model(&p, "m9").unwrap_err();
        assert!(err.contains("不是标准表"), "必须拒绝静默覆盖: {err}");
        assert!(
            std::fs::read_to_string(&p).unwrap().contains("models = \"oops\""),
            "报错后文件原样不动"
        );
        std::fs::remove_file(&p).ok();
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

    #[test]
    fn set_permission_level_writes_three_levels() {
        let p = tmp_path("level-three");
        // ask:裸文件 → default,且无规则块
        std::fs::write(&p, "[model.m1]\nmodel = \"a\"\n").unwrap();
        set_permission_level(&p, "ask").unwrap();
        let s = read_settings(&p).unwrap();
        assert_eq!(s.permission_mode.as_deref(), Some("default"));
        assert!(!s.smart_auto_rules_active);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("allow"), "ask 不留规则块: {text}");
        assert!(text.contains("[ui]\npermission_mode = \"default\""), "{text}");

        // smart:default + 只读规则组
        set_permission_level(&p, "smart").unwrap();
        let s = read_settings(&p).unwrap();
        assert_eq!(s.permission_mode.as_deref(), Some("default"));
        assert!(s.smart_auto_rules_active);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("allow = [\"Read\", \"Grep\", \"Glob\", \"WebSearch\", \"WebFetch\"]"),
            "{text}"
        );

        // trust:always-approve,规则块移除
        set_permission_level(&p, "trust").unwrap();
        let s = read_settings(&p).unwrap();
        assert_eq!(s.permission_mode.as_deref(), Some("always-approve"));
        assert!(!s.smart_auto_rules_active);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("allow"), "trust 移除规则块: {text}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn set_permission_level_is_idempotent() {
        let p = tmp_path("level-idem");
        std::fs::write(&p, "[model.m1]\nmodel = \"a\"\n").unwrap();
        set_permission_level(&p, "smart").unwrap();
        let once = std::fs::read_to_string(&p).unwrap();
        set_permission_level(&p, "smart").unwrap();
        let twice = std::fs::read_to_string(&p).unwrap();
        assert_eq!(once, twice, "同档位重复写入必须逐字节一致");
        set_permission_level(&p, "ask").unwrap();
        set_permission_level(&p, "smart").unwrap();
        let back = std::fs::read_to_string(&p).unwrap();
        assert_eq!(once, back, "切走再切回必须回到同一内容");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn set_permission_level_preserves_other_sections_and_comments() {
        let p = tmp_path("level-keep");
        std::fs::write(
            &p,
            "# 顶部注释\n[models]\ndefault = \"m1\" # 旧默认\n\n[ui]\nposition = \"center\"\n\n[permission]\n# 用户自己的注释\nallow = [\"Bash\"]\n\n[model.m1]\nmodel = \"a\"\n",
        )
        .unwrap();
        set_permission_level(&p, "smart").unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# 顶部注释"), "顶部注释必须保留");
        assert!(text.contains("default = \"m1\" # 旧默认"), "default 行原样: {text}");
        assert!(text.contains("position = \"center\""), "[ui] 其他键原样: {text}");
        assert!(text.contains("# 用户自己的注释"), "[permission] 注释保留: {text}");
        assert!(
            text.contains("allow = [\"Read\", \"Grep\", \"Glob\", \"WebSearch\", \"WebFetch\"]"),
            "{text}"
        );
        assert!(!text.contains("\"Bash\""), "旧 allow 被替换为标准组: {text}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn set_permission_level_rejects_unknown_level_and_bad_sections() {
        // 未知档位:错误信息可读,文件原样不动(校验在读文件之前)
        let p = tmp_path("level-reject");
        let original = "models = \"oops\"\n\n[ui]\nposition = \"c\"\n";
        std::fs::write(&p, original).unwrap();
        let err = set_permission_level(&p, "yolo").unwrap_err();
        assert!(err.contains("未知授权档位"), "{err}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);

        // [permission] 非标准表:拒绝且文件原样
        let p2 = tmp_path("level-nontable");
        std::fs::write(&p2, "permission = \"oops\"\n").unwrap();
        let err = set_permission_level(&p2, "smart").unwrap_err();
        assert!(err.contains("不是标准表"), "{err}");
        assert_eq!(std::fs::read_to_string(&p2).unwrap(), "permission = \"oops\"\n");

        // [ui] 非标准表:拒绝
        let p3 = tmp_path("level-uinontable");
        std::fs::write(&p3, "ui = 3\n").unwrap();
        let err = set_permission_level(&p3, "trust").unwrap_err();
        assert!(err.contains("[ui]"), "{err}");
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(&p2).ok();
        std::fs::remove_file(&p3).ok();
    }

    #[test]
    fn permission_level_mapping_covers_three_levels() {
        assert_eq!(permission_mode_for_level("ask").unwrap(), "default");
        assert_eq!(permission_mode_for_level("smart").unwrap(), "default");
        assert_eq!(permission_mode_for_level("trust").unwrap(), "always-approve");
        assert!(permission_mode_for_level("").is_err());
        assert!(permission_mode_for_level("Ask").is_err());
        assert!(permission_mode_for_level("auto").is_err());
    }

    #[test]
    fn create_model_bootstraps_missing_file_and_sets_default() {
        let p = tmp_path("create-boot");
        create_model(
            &p,
            "deepseek",
            "deepseek-chat",
            "https://api.deepseek.com/v1",
            Some("DeepSeek Chat"),
            Some(" sk-test "),
        )
        .unwrap();
        let s = read_settings(&p).unwrap();
        assert_eq!(s.default_model.as_deref(), Some("deepseek"));
        let m = s.models.iter().find(|m| m.id == "deepseek").unwrap();
        assert_eq!(m.name, "DeepSeek Chat");
        assert_eq!(m.base_url.as_deref(), Some("https://api.deepseek.com/v1"));
        assert!(m.has_api_key, "trim 后的 key 应写入");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("api_key = \"sk-test\""), "{text}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn create_model_preserves_existing_and_does_not_clobber_default() {
        let p = tmp_path("create-keep");
        std::fs::write(&p, "# 用户注释\n[models]\ndefault = \"m1\"\n\n[model.m1]\nmodel = \"a\"\n")
            .unwrap();
        create_model(&p, "m2", "gpt-4o", "https://api.openai.com/v1", None, None).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# 用户注释"), "注释必须保留");
        assert!(text.contains("default = \"m1\""), "已有 default 不被抢改");
        let s = read_settings(&p).unwrap();
        assert!(s.models.iter().any(|m| m.id == "m2"));
        let m2 = s.models.iter().find(|m| m.id == "m2").unwrap();
        assert_eq!(m2.name, "m2", "缺省 name 回落 id");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn create_model_rejects_bad_ids_duplicates_and_corrupt_file() {
        let p = tmp_path("create-reject");
        std::fs::write(&p, "[model.m1]\nmodel = \"a\"\n").unwrap();
        assert!(create_model(&p, "a.b", "x", "https://a/v1", None, None)
            .unwrap_err()
            .contains("只允许"));
        assert!(create_model(&p, "", "x", "https://a/v1", None, None).is_err());
        assert!(create_model(&p, "m1", "x", "https://a/v1", None, None)
            .unwrap_err()
            .contains("已存在"));
        // 非法 id/已存在拒绝后,文件原样
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[model.m1]\nmodel = \"a\"\n");
        std::fs::remove_file(&p).ok();

        let bad = tmp_path("create-badfile");
        std::fs::write(&bad, "这不是 TOML [[[").unwrap();
        assert!(create_model(&bad, "m1", "x", "https://a/v1", None, None).is_err());
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "这不是 TOML [[[");
        std::fs::remove_file(&bad).ok();
    }

    #[test]
    fn create_model_surfaces_second_model_via_read() {
        // 全链验证:从零 bootstrap → 读出 → 选第二个模型保存 default
        let p = tmp_path("create-chain");
        create_model(&p, "a1", "m-a", "https://a/v1", None, None).unwrap();
        create_model(&p, "b2", "m-b", "https://b/v1", None, Some("sk-b")).unwrap();
        save_settings(&p, "b2", None).unwrap();
        let s = read_settings(&p).unwrap();
        assert_eq!(s.default_model.as_deref(), Some("b2"));
        assert_eq!(s.models.len(), 2);
        std::fs::remove_file(&p).ok();
    }
}
