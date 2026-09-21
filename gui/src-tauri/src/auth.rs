//! 账号登录入口(方案 P1-3):读 `auth.json` 判断登录态,经 `GET /v1/user`
//! 拉取套餐/今日用量/配额;登录走内核 `login --device-auth` 子进程,把设备码
//! 与验证 URL 经 `auth-event` 事件推给前端;退出走内核 `logout` 子进程。
//!
//! 纪律:
//! - access token 只在 Rust 侧持有(读 auth.json、发 /v1/user),绝不下发前端;
//! - 前端只见脱敏后的 AuthStatus(email/plan/usage/quota);
//! - 网络失败不影响本地登录态判定(token 有效但 /v1/user 挂了 → 仍 logged_in,
//!   usage/quota 留空并在 detail 里说明)。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const DEFAULT_ISSUER: &str = "https://api.qidiai.ltd";
const DEFAULT_CLIENT_ID: &str = "qidi-code";

// ----------------------------------------------------------- 路径与 scope --

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

/// auth.json 顶层 scope key = `{issuer}::{client_id}`(与 cf-shell config.rs 一致)。
fn scope_key() -> String {
    format!("{}::{}", issuer(), client_id())
}

/// 内核可执行文件解析(与 SpawnConfig::default_agent 同一优先级)。
fn resolve_program(app: &AppHandle) -> String {
    if let Ok(p) = std::env::var("QIDIWORK_AGENT_PATH") {
        if !p.trim().is_empty() {
            return p;
        }
    }
    let name = format!("qidiwork{}", std::env::consts::EXE_SUFFIX);
    app.path()
        .resource_dir()
        .ok()
        .map(|d| d.join(&name))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "qidiwork".to_string())
}

// --------------------------------------------------------------- 数据结构 --

/// auth.json 里 GrokAuth 的脱敏投影(只取展示所需字段;绝不 clone key 到响应)。
#[derive(Debug, Clone, Deserialize)]
struct StoredAuth {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    email: Option<String>,
    /// access token(仅 Rust 侧用于发 /v1/user,绝不出 Rust)。
    #[serde(default)]
    key: String,
}

#[derive(Debug, Serialize)]
pub struct UsageToday {
    requests: i64,
    tokens: i64,
}

#[derive(Debug, Serialize)]
pub struct Quota {
    requests_limit: i64,
    tokens_limit: i64,
    requests_left: i64,
    tokens_left: i64,
}

/// 返回给前端的账号状态(不含任何 token)。
#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub plan: Option<String>,
    pub usage_today: Option<UsageToday>,
    pub quota: Option<Quota>,
    /// 补充说明(网络错误等),脱敏后给用户看。
    pub detail: Option<String>,
}

/// 从 auth.json 读出当前 scope 的登录记录。文件缺失/无该 scope → None。
fn read_stored(home: &Path) -> Option<StoredAuth> {
    let raw = std::fs::read_to_string(auth_json_path(home)).ok()?;
    let map: std::collections::BTreeMap<String, StoredAuth> = serde_json::from_str(&raw).ok()?;
    if let Some(a) = map.get(&scope_key()) {
        return Some(a.clone());
    }
    // 兜底:env 覆盖 issuer/client_id 与登录时不一致的老文件。仅当文件里恰好
    // 只有一条记录时才取该项,避免多账号场景下误选他人登录态。
    if map.len() == 1 {
        map.into_values().next()
    } else {
        None
    }
}

// ------------------------------------------------------------- 状态查询 --

/// 组装配号状态:本地读 token → 在线拉 /v1/user。
async fn build_status(home: &Path) -> AuthStatus {
    let stored = match read_stored(home) {
        Some(s) if !s.key.trim().is_empty() => s,
        _ => {
            return AuthStatus {
                logged_in: false,
                email: None,
                user_id: None,
                plan: None,
                usage_today: None,
                quota: None,
                detail: None,
            };
        }
    };
    let url = format!("{}/v1/user", issuer());
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return AuthStatus {
                logged_in: true,
                email: stored.email,
                user_id: Some(stored.user_id),
                plan: None,
                usage_today: None,
                quota: None,
                detail: Some(format!("无法初始化网络客户端: {e}")),
            };
        }
    };
    match client
        .get(&url)
        .bearer_auth(stored.key.trim())
        .send()
        .await
    {
        Err(e) => AuthStatus {
            logged_in: true,
            email: stored.email,
            user_id: Some(stored.user_id),
            plan: None,
            usage_today: None,
            quota: None,
            detail: Some(format!("服务器连接失败:{e}")),
        },
        Ok(resp) if !resp.status().is_success() => {
            let code = resp.status();
            // access token 短 TTL(15 分钟),GUI 绝不自行刷新(刷新轮换 +
            // 重用族吊销会踢掉内核)。GUI 只读取;配额待内核下次启动
            // 续期 auth.json 后再点「刷新」。email/user_id 本地已可用。
            let detail = if code.as_u16() == 401 || code.as_u16() == 403 {
                "登录凭证待自动续期,发送任务后此处配额将可用".to_string()
            } else {
                format!("服务器返回 {code}")
            };
            AuthStatus {
                logged_in: true,
                email: stored.email,
                user_id: Some(stored.user_id),
                plan: None,
                usage_today: None,
                quota: None,
                detail: Some(detail),
            }
        }
        Ok(resp) => {
            let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
            let usage_today = body.get("usage_today").and_then(|u| {
                Some(UsageToday {
                    requests: u.get("requests")?.as_i64()?,
                    tokens: u.get("tokens")?.as_i64()?,
                })
            });
            let quota = body.get("quota").and_then(|q| {
                Some(Quota {
                    requests_limit: q.get("requests_limit")?.as_i64()?,
                    tokens_limit: q.get("tokens_limit")?.as_i64()?,
                    requests_left: q.get("requests_left")?.as_i64()?,
                    tokens_left: q.get("tokens_left")?.as_i64()?,
                })
            });
            AuthStatus {
                logged_in: true,
                email: stored.email,
                user_id: Some(
                    body.get("userId")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(stored.user_id),
                ),
                plan: body
                    .get("plan")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                usage_today,
                quota,
                detail: None,
            }
        }
    }
}

#[tauri::command]
pub async fn auth_status(app: AppHandle) -> Result<AuthStatus, String> {
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    Ok(build_status(&home).await)
}

// ---------------------------------------------------------------- 登录 --

#[derive(Clone, Serialize)]
struct AuthEvent {
    phase: &'static str, // starting | pending | success | error
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// 从验证 URL 的 `user_code=` 查询参数取设备码(服务端下发 complete URL 时)。
fn code_from_uri(uri: &str) -> Option<String> {
    let rest = uri.split_once("user_code=").map(|x| x.1)?;
    let cand = rest.split(['&', '#']).next().unwrap_or("").trim().to_string();
    looks_like_user_code(&cand).then_some(cand)
}

/// 判断一行是否是设备码(分组大写基数字母,含连字符,如 `HYMS-CKKD`)。
/// 至少两组以剔除 `ERROR`/`INFO` 之类的孤立大写词。
fn looks_like_user_code(s: &str) -> bool {
    let t = s.trim();
    let groups: Vec<&str> = t.split('-').collect();
    groups.len() >= 2
        && groups.iter().all(|g| {
            (3..=6).contains(&g.len()) && g.chars().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        })
}

/// 触发设备码登录:后台 spawn 内核 `login --device-auth`,解析 stderr 推事件,
/// 子进程退出后重读 auth.json 推 success/error。命令本身立即返回。
#[tauri::command]
pub async fn auth_login(app: AppHandle) -> Result<(), String> {
    let program = resolve_program(&app);
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let app2 = app.clone();
    tokio::spawn(async move {
        run_device_login(app2, program, home).await;
    });
    Ok(())
}

async fn run_device_login(app: AppHandle, program: String, home: PathBuf) {
    let _ = app.emit(
        "auth-event",
        AuthEvent {
            phase: "starting",
            verification_uri: None,
            user_code: None,
            message: None,
        },
    );

    let mut child = match Command::new(&program)
        .args(["login", "--device-auth"])
        .current_dir(&home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit(
                "auth-event",
                AuthEvent {
                    phase: "error",
                    verification_uri: None,
                    user_code: None,
                    message: Some(format!("无法启动内核: {e}")),
                },
            );
            return;
        }
    };

    let stderr = child.stderr.take().expect("已声明 piped stderr");
    let mut lines = BufReader::new(stderr).lines();
    let mut uri: Option<String> = None;
    let mut code: Option<String> = None;
    let mut emitted_pending = false;

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let t = line.trim();
                if uri.is_none() && (t.starts_with("http://") || t.starts_with("https://")) {
                    uri = Some(t.to_string());
                    // 完整验证 URL 常以 ?user_code=XXXX-YYYY 预填设备码,优先从中取。
                    if code.is_none() {
                        code = code_from_uri(t);
                    }
                } else if code.is_none() && looks_like_user_code(t) {
                    code = Some(t.to_string());
                }
                if !emitted_pending && uri.is_some() {
                    emitted_pending = true;
                    let _ = app.emit(
                        "auth-event",
                        AuthEvent {
                            phase: "pending",
                            verification_uri: uri.clone(),
                            user_code: code.clone(),
                            message: None,
                        },
                    );
                }
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!(error = %e, "读取 login stderr 失败");
                break;
            }
        }
    }

    let status = child.wait().await;
    match status {
        Ok(s) if s.success() => {
            // 补发一次带 code 的 pending(若 code 比 uri 晚到),再报成功。
            let _ = app.emit(
                "auth-event",
                AuthEvent {
                    phase: "success",
                    verification_uri: uri,
                    user_code: code,
                    message: None,
                },
            );
        }
        Ok(s) => {
            let _ = app.emit(
                "auth-event",
                AuthEvent {
                    phase: "error",
                    verification_uri: None,
                    user_code: None,
                    message: Some(format!("登录未完成(内核退出码 {:?})", s.code())),
                },
            );
        }
        Err(e) => {
            let _ = app.emit(
                "auth-event",
                AuthEvent {
                    phase: "error",
                    verification_uri: None,
                    user_code: None,
                    message: Some(format!("等待登录进程失败: {e}")),
                },
            );
        }
    }
}

// ---------------------------------------------------------------- 退出 --

/// 登出:spawn 内核 `logout` 删除本地 token,成功后返回脱敏的未登录状态。
#[tauri::command]
pub async fn auth_logout(app: AppHandle) -> Result<AuthStatus, String> {
    let program = resolve_program(&app);
    let home = app.path().home_dir().ok().ok_or("无法解析主目录")?;
    let out = Command::new(&program)
        .arg("logout")
        .current_dir(&home)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("无法启动内核: {e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        tracing::warn!(code = ?out.status.code(), stderr = %msg, "logout 非零退出");
    }
    Ok(build_status(&home).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真机 `qidiwork login --device-auth` 抓到的 stderr 逐行(设备码可能纯字母)。
    #[test]
    fn code_shape_accepts_letters_only_and_rejects_noise() {
        assert!(looks_like_user_code("HYMS-CKKD"));
        assert!(looks_like_user_code("  AB23-4CD5  "));
        assert!(!looks_like_user_code("ERROR")); // 孤立大写词无连字符
        assert!(!looks_like_user_code("Confirm this code in your browser:"));
        assert!(!looks_like_user_code("Waiting for authorization..."));
        assert!(!looks_like_user_code(""));
    }

    #[test]
    fn code_prefers_url_param() {
        assert_eq!(
            code_from_uri("https://api.qidiai.ltd/activate?user_code=HYMS-CKKD").as_deref(),
            Some("HYMS-CKKD")
        );
        // 参数后有 & 续接也能截断
        assert_eq!(
            code_from_uri("https://x/activate?foo=1&user_code=AB23-4CD5&bar=2").as_deref(),
            Some("AB23-4CD5")
        );
        assert_eq!(code_from_uri("https://api.qidiai.ltd/activate"), None);
    }

    #[test]
    fn stored_auth_parses_real_file_shape() {
        // 与真实 auth.json 顶层结构一致:scope key → {user_id,email,key,...}
        let json = r#"{"https://api.qidiai.ltd::qidi-code":{"key":"tok","auth_mode":"oidc","user_id":"u_c28c715233aa35f6","email":"test@qidiai.ltd","expires_at":"2026-09-19T21:14:00.701751700Z"}}"#;
        let map: std::collections::BTreeMap<String, StoredAuth> =
            serde_json::from_str(json).expect("应能解析");
        let got = map.get("https://api.qidiai.ltd::qidi-code").expect("scope 命中");
        assert_eq!(got.user_id, "u_c28c715233aa35f6");
        assert_eq!(got.email.as_deref(), Some("test@qidiai.ltd"));
        assert_eq!(got.key, "tok");
    }
}
