#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(unsafe_code)]
//! T1 验收测试(指挥官编写,锁定禁改):GUI 拉起内核子进程前必须注入
//! QIDI_OAUTH2_ISSUER / QIDI_OAUTH2_CLIENT_ID,使内核与 GUI 认同一
//! OAuth scope(修复登录连 auth.x.ai 超时、退出找不到会话的根因)。
//!
//! SAFETY:edition 2024 下 set_var/remove_var 为 unsafe。本文件所有
//! env 读写经 ENV_LOCK 串行化,且每个用例结束前恢复原值;本测试二进制
//! 内无其他并发 env 使用者。

use std::ffi::OsStr;
use std::sync::Mutex;

use qidiwork_gui::auth::inject_oauth_env;
use tokio::process::Command;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const ISSUER_KEY: &str = "QIDI_OAUTH2_ISSUER";
const CLIENT_KEY: &str = "QIDI_OAUTH2_CLIENT_ID";

struct EnvGuard {
    saved: [(&'static str, Option<std::ffi::OsString>); 2],
}

impl EnvGuard {
    fn capture() -> Self {
        Self {
            saved: [
                (ISSUER_KEY, std::env::var_os(ISSUER_KEY)),
                (CLIENT_KEY, std::env::var_os(CLIENT_KEY)),
            ],
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            // SAFETY:ENV_LOCK 持有期间串行执行,见文件头注释。
            unsafe {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}

fn env_value<'a>(cmd: &'a Command, key: &str) -> Option<&'a OsStr> {
    cmd.as_std()
        .get_envs()
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v)
}

#[test]
fn injects_default_issuer_and_client_when_env_unset() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::capture();
    // SAFETY:ENV_LOCK 串行化,见文件头注释。
    unsafe {
        std::env::remove_var(ISSUER_KEY);
        std::env::remove_var(CLIENT_KEY);
    }
    let mut cmd = Command::new("qidiwork");
    inject_oauth_env(&mut cmd);
    assert_eq!(
        env_value(&cmd, ISSUER_KEY).and_then(|v| v.to_str()),
        Some("https://api.qidiai.ltd")
    );
    assert_eq!(
        env_value(&cmd, CLIENT_KEY).and_then(|v| v.to_str()),
        Some("qidi-code")
    );
}

#[test]
fn honors_env_override_and_trims_trailing_slash() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::capture();
    // SAFETY:ENV_LOCK 串行化,见文件头注释。
    unsafe {
        std::env::set_var(ISSUER_KEY, "https://sso.example.test/");
        std::env::set_var(CLIENT_KEY, "custom-client");
    }
    let mut cmd = Command::new("qidiwork");
    inject_oauth_env(&mut cmd);
    assert_eq!(
        env_value(&cmd, ISSUER_KEY).and_then(|v| v.to_str()),
        Some("https://sso.example.test")
    );
    assert_eq!(
        env_value(&cmd, CLIENT_KEY).and_then(|v| v.to_str()),
        Some("custom-client")
    );
}

#[test]
fn whitespace_env_falls_back_to_defaults() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::capture();
    // SAFETY:ENV_LOCK 串行化,见文件头注释。
    unsafe {
        std::env::set_var(ISSUER_KEY, "   ");
        std::env::set_var(CLIENT_KEY, "");
    }
    let mut cmd = Command::new("qidiwork");
    inject_oauth_env(&mut cmd);
    assert_eq!(
        env_value(&cmd, ISSUER_KEY).and_then(|v| v.to_str()),
        Some("https://api.qidiai.ltd")
    );
    assert_eq!(
        env_value(&cmd, CLIENT_KEY).and_then(|v| v.to_str()),
        Some("qidi-code")
    );
}

#[test]
fn preserves_unrelated_env_and_overrides_stale_oauth_values() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::capture();
    // SAFETY:ENV_LOCK 串行化,见文件头注释。
    unsafe {
        std::env::remove_var(ISSUER_KEY);
        std::env::remove_var(CLIENT_KEY);
    }
    let mut cmd = Command::new("qidiwork");
    cmd.env("QIDI_HOME", "C:/some/home");
    cmd.env(ISSUER_KEY, "https://stale.example");
    cmd.env(CLIENT_KEY, "stale-client");
    inject_oauth_env(&mut cmd);
    assert_eq!(
        env_value(&cmd, "QIDI_HOME").and_then(|v| v.to_str()),
        Some("C:/some/home")
    );
    assert_eq!(
        env_value(&cmd, ISSUER_KEY).and_then(|v| v.to_str()),
        Some("https://api.qidiai.ltd")
    );
    assert_eq!(
        env_value(&cmd, CLIENT_KEY).and_then(|v| v.to_str()),
        Some("qidi-code")
    );
}