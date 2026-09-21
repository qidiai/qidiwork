//! GUI 新版本提示(R1 的 GUI 侧):拉取自托管版本清单
//! `https://qidiwork.qidiai.ltd/releases/latest.json`,与自身
//! `CARGO_PKG_VERSION` 比较;仅当远端 gui 版本**严格大于**自身时返回版本与下载
//! 地址,否则返回 None。
//!
//! 纪律:更新提示是尽力而为的锦上添花,**任何**网络/解析/比较失败都静默返回
//! None,绝不打扰用户、绝不阻塞启动。

use std::time::Duration;

use serde_json::{Value, json};

/// 自托管版本清单(nginx 已配 no-cache + CORS *)。
const LATEST_JSON_URL: &str = "https://qidiwork.qidiai.ltd/releases/latest.json";

/// 把 `major.minor.patch` 解析为三元组。
///
/// 段数不足三段、任一段非纯数字(如 `0.1.2-beta`)、或段数多于三段
/// (如 `0.1.2.3`)一律视为解析失败 → None(宁可不提示,不误提示)。
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let mut it = s.split('.');
    let major = it.next()?.parse::<u64>().ok()?;
    let minor = it.next()?.parse::<u64>().ok()?;
    let patch = it.next()?.parse::<u64>().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// 检查 GUI 是否有新版本。有 → `Some({"version","url"})`;无/失败 → None。
#[tauri::command]
pub async fn check_gui_update() -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;

    let resp = client.get(LATEST_JSON_URL).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;

    // 清单结构:{"gui":{"version":"0.1.1","url":"..."},"kernel":{...}}
    let gui = body.get("gui")?;
    let latest_raw = gui.get("version")?.as_str()?;
    let url = gui.get("url")?.as_str()?;

    let latest = parse_version(latest_raw)?;
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;

    if latest > current {
        Some(json!({ "version": latest_raw, "url": url }))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parses_three_numeric_parts() {
        assert_eq!(parse_version("0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_version(" 1.20.300 "), Some((1, 20, 300)));
    }

    #[test]
    fn version_rejects_non_three_part() {
        assert_eq!(parse_version("0.1"), None); // 段数不足
        assert_eq!(parse_version("0.1.2-beta"), None); // 带后缀
        assert_eq!(parse_version("0.1.2.3"), None); // 段数过多
        assert_eq!(parse_version("a.b.c"), None); // 非数字
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn version_strictly_greater() {
        assert!((1, 0, 0) > (0, 9, 9));
        assert!((0, 1, 2) > (0, 1, 1));
        assert!(!((0, 1, 1) > (0, 1, 1)));
    }
}
