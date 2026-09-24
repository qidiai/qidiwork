//! 首次启动引导(onboarding)的后端支撑(R10 / K3 审计修正版)。
//!
//! 职责:
//! - `server_models`:用已存 JWT 拉取 `{issuer}/v1/models`(OpenAI 格式)的模型
//!   列表。token 只在 Rust 侧,前端经本命令取列表——GUI 的 CSP `connect-src`
//!   仅放行 ipc,前端无法直连网络,故必须由 Rust 代取。
//! - `check_onboarding_auth`:读 `auth.json` 判定本地登录态(轻量、不走网络),
//!   供引导 Step 2 轮询「授权是否完成」。
//! - `onboarding_marker_read` / `onboarding_marker_write`:完成标记文件
//!   `<app_data_dir>/onboarding_done`。前端 localStorage(`qidi.onboarded`)与
//!   本文件**双写**;`shouldShowOnboarding()` 两处任一缺失即视为未完成。
//!
//! 纪律:`auth.rs` 为审计冻结文件(禁改),其 `read_stored` / scope-key 解析
//! 为私有实现。本模块按**同源规则复刻**该逻辑(issuer/client_id/grok_home 的
//! env 优先级、scope key 形状、单条记录兜底),不修改 `auth.rs`;access token
//! 绝不下发前端。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const DEFAULT_ISSUER: &str = "https://api.qidiai.ltd";
const DEFAULT_CLIENT_ID: &str = "qidi-code";

// ------------------------------------------- 路径与 scope(复刻 auth.rs) --

fn issuer() -> String {
    std::env::var("QIDI_OAUTH2_ISSUER")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ISSUER.to_string())
        .trim_end_matches('/')
        .to_string()
}

fn client_id() -> String {
    std::env::var("QIDI_OAUTH2_CLIENT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string())
}

/// grok_home:与内核一致,`$QIDI_HOME` 优先,否则 `~/.qidi`。
fn grok_home(home: &Path) -> PathBuf {
    std::env::var_os("QIDI_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".qidi"))
}

fn auth_json_path(home: &Path) -> PathBuf {
    grok_home(home).join("auth.json")
}

/// auth.json 顶层 scope key = `{issuer}::{client_id}`(与 auth.rs / cf-shell 一致)。
fn scope_key() -> String {
    format!("{}::{}", issuer(), client_id())
}

/// auth.json 里当前 scope 记录的脱敏投影(仅取登录判定/取 token 所需字段)。
/// 未知字段被 serde 默认忽略(与真实 auth.json 的其余字段兼容)。
#[derive(Debug, Clone, Deserialize)]
struct StoredAuth {
    /// access token(仅 Rust 侧用于发 /v1/models,绝不出 Rust)。
    #[serde(default)]
    key: String,
}

/// 从给定 auth.json 路径读当前 scope 的登录记录。文件缺失/无该 scope → None。
/// 兜底:仅当文件里恰好只有一条记录时取该项(与 auth.rs 同规则,避免多账号误选)。
fn read_auth_json(path: &Path) -> Option<StoredAuth> {
    let raw = std::fs::read_to_string(path).ok()?;
    let map: std::collections::BTreeMap<String, StoredAuth> = serde_json::from_str(&raw).ok()?;
    if let Some(a) = map.get(&scope_key()) {
        return Some(a.clone());
    }
    if map.len() == 1 {
        map.into_values().next()
    } else {
        None
    }
}

/// 读取用户主目录下的 auth.json。
fn read_stored(home: &Path) -> Option<StoredAuth> {
    read_auth_json(&auth_json_path(home))
}

/// 本地登录态:存在当前 scope 记录且 token 非空白。
fn is_logged_in(home: &Path) -> bool {
    read_stored(home).is_some_and(|s| !s.key.trim().is_empty())
}

// ------------------------------------------------------------- 模型列表 --

/// 服务端可用模型(OpenAI `/v1/models` 的 `data[]` 投影)。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelInfo {
    /// 模型 id(OpenAI 条目 `id`)。
    pub id: String,
    /// 展示名(条目 `name` 缺省回落 id)。
    pub name: String,
    /// 描述(条目 `description`,缺省 None)。
    pub description: Option<String>,
}

/// 解析 OpenAI 格式 `{"data":[{...}]}` 为模型列表。纯函数,便于单测。
/// 缺失/类型不符的条目与空 id 一律跳过(宽容解析,不让单个坏条目拖垮列表)。
fn parse_models(body: &serde_json::Value) -> Vec<ModelInfo> {
    let Some(items) = body.get("data").and_then(|d| d.as_array()) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(id)
            .to_string();
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        models.push(ModelInfo {
            id: id.to_string(),
            name,
            description,
        });
    }
    models
}

/// 模型列表 HTTP 错误 → 用户可行动的中文文案。
///
/// 401/403 单独成句:auth.json 里的 access token 可能已过期(内核自己会刷新,
/// 但本命令直接用存量 key)。此时引导第三步必须把用户送回登录,而不是停在
/// "服务器返回 401" 这种无从下手的报错上。
fn models_error(status: u16) -> String {
    match status {
        401 | 403 => "登录已失效:请重新登录后再选择模型".to_string(),
        other => format!("获取模型列表失败:服务器返回 {other}"),
    }
}

/// 用已存 JWT 拉取服务端模型列表。未登录/请求失败 → 中文 Err。
#[tauri::command]
pub async fn server_models(app: AppHandle) -> Result<Vec<ModelInfo>, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let stored = read_stored(&home)
        .filter(|s| !s.key.trim().is_empty())
        .ok_or_else(|| "未登录:请先登录 QIDI 账号后再选择模型".to_string())?;

    let url = format!("{}/v1/models", issuer());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("无法初始化网络客户端: {e}"))?;
    let resp = client
        .get(&url)
        .bearer_auth(stored.key.trim())
        .send()
        .await
        .map_err(|e| format!("获取模型列表失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(models_error(resp.status().as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析模型列表失败: {e}"))?;
    let models = parse_models(&body);
    if models.is_empty() {
        return Err("服务器未返回任何可用模型".to_string());
    }
    Ok(models)
}

/// 引导用登录态检测:读 auth.json 判断是否已登录(不走网络)。
#[tauri::command]
pub async fn check_onboarding_auth(app: AppHandle) -> Result<bool, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    Ok(is_logged_in(&home))
}

// --------------------------------------------------------- 完成标记文件 --

/// 完成标记文件名(与前端 localStorage 键 `qidi.onboarded` 双写)。
const MARKER_FILE: &str = "onboarding_done";

fn marker_path(dir: &Path) -> PathBuf {
    dir.join(MARKER_FILE)
}

/// 标记是否存在(目录缺失/文件缺失均视为未完成)。
fn marker_read(dir: &Path) -> bool {
    marker_path(dir).is_file()
}

/// 写标记(幂等;目录不存在则创建)。
fn marker_write(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(marker_path(dir), b"1")
}

/// 读完成标记文件(app_data_dir/onboarding_done)。
#[tauri::command]
pub async fn onboarding_marker_read(app: AppHandle) -> Result<bool, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法解析数据目录: {e}"))?;
    Ok(marker_read(&dir))
}

/// 写完成标记文件(app_data_dir/onboarding_done)。
#[tauri::command]
pub async fn onboarding_marker_write(app: AppHandle) -> Result<(), String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法解析数据目录: {e}"))?;
    marker_write(&dir).map_err(|e| format!("写入引导标记失败: {e}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "qidiwork-onboarding-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// server_models 的解析核心:标准 OpenAI 形状 → 模型列表;
    /// name 缺省回落 id,description 缺省 None。
    #[test]
    fn parse_models_reads_openai_shape() {
        let body = serde_json::json!({
            "object": "list",
            "data": [
                {"id": "qidi-large", "object": "model", "name": "QIDI Large", "description": "旗舰推理模型"},
                {"id": "qidi-fast", "object": "model"}
            ]
        });
        let models = parse_models(&body);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "qidi-large");
        assert_eq!(models[0].name, "QIDI Large");
        assert_eq!(models[0].description.as_deref(), Some("旗舰推理模型"));
        assert_eq!(models[1].id, "qidi-fast");
        assert_eq!(models[1].name, "qidi-fast", "缺省 name 回落 id");
        assert_eq!(models[1].description, None);
    }

    /// 宽容解析:缺 data / data 非数组 / 空数组 / 条目缺 id 或 id 空 → 跳过。
    #[test]
    fn parse_models_tolerates_malformed_and_empty() {
        assert!(parse_models(&serde_json::json!({})).is_empty());
        assert!(parse_models(&serde_json::json!({"data": []})).is_empty());
        assert!(parse_models(&serde_json::json!({"data": "oops"})).is_empty());
        assert!(parse_models(&serde_json::Value::Null).is_empty());
        let body = serde_json::json!({
            "data": [{"name": "无 id"}, {"id": "  "}, {"id": "ok"}]
        });
        let models = parse_models(&body);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "ok");
    }

    /// check_onboarding_auth 的核心:按当前 scope key 命中登录记录;
    /// 空 token 视为未登录;文件缺失 → None。
    #[test]
    fn read_auth_json_detects_login_by_scope_key() {
        let dir = temp_dir("auth");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        let key = scope_key();

        std::fs::write(
            &path,
            format!(r#"{{"{key}":{{"key":"tok","user_id":"u1","email":"a@b.c"}}}}"#),
        )
        .unwrap();
        let stored = read_auth_json(&path).expect("应命中 scope");
        assert_eq!(stored.key, "tok");

        // 空 key → 视为未登录(调用方据此判定)
        std::fs::write(&path, format!(r#"{{"{key}":{{"key":"  "}}}}"#)).unwrap();
        assert!(read_auth_json(&path).is_some_and(|s| s.key.trim().is_empty()));

        // 文件缺失 → None
        assert!(read_auth_json(&dir.join("nope.json")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 单条记录兜底:env 覆盖 issuer/client_id 与登录时不一致的老文件仍可命中。
    #[test]
    fn read_auth_json_single_entry_fallback() {
        let dir = temp_dir("auth-single");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        std::fs::write(&path, r#"{"https://other.example::cli":{"key":"tok2"}}"#).unwrap();
        let stored = read_auth_json(&path).expect("单条记录兜底命中");
        assert_eq!(stored.key, "tok2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 门控契约(Rust 侧):完成标记文件是 shouldShowOnboarding 的两个输入
    /// 之一——未写 → 未完成;写入后幂等 → 已完成。
    #[test]
    fn onboarding_gate_file_marker_contract() {
        let dir = temp_dir("gate");
        assert!(!marker_read(&dir), "未写标记:视为未完成");
        marker_write(&dir).unwrap();
        assert!(marker_read(&dir), "写后:视为已完成");
        marker_write(&dir).unwrap(); // 幂等
        assert!(marker_read(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 401/403 必须翻成「重新登录」,其他状态码保留服务器状态便于排查。
    #[test]
    fn models_error_maps_auth_failures_to_relogin() {
        assert!(models_error(401).contains("重新登录"));
        assert!(models_error(403).contains("重新登录"));
        assert!(models_error(500).contains("500"));
    }

    /// scope key 形状:`{issuer}::{client_id}`(与 auth.rs 一致)。
    #[test]
    fn scope_key_shape() {
        assert_eq!(scope_key(), format!("{}::{}", issuer(), client_id()));
        assert!(scope_key().contains("::"));
    }
}
