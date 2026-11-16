//! GUI 新版本提示(R1 的 GUI 侧)+ 安装包下载与完整性校验(U 片)。
//!
//! 两部分:
//!
//! 1. **版本检查**:拉取自托管版本清单
//!    `https://qidiwork.qidiai.ltd/releases/latest.json`,与自身
//!    `CARGO_PKG_VERSION` 比较;仅当远端 gui 版本**严格大于**自身时返回版本与
//!    下载地址,否则返回 None。
//!
//! 2. **下载 + 校验**(施工规格 2026-11-17 §U):流式下载安装包到
//!    `%TEMP%\qidiwork-update\`,边写边算 sha256,并与清单里的 `sig`
//!    (Ed25519,签名对象 = sha256 的 32 字节**原始摘要**)核对。
//!
//! 纪律:更新提示是尽力而为的锦上添花,**任何**网络/解析/比较失败都静默返回
//! None,绝不打扰用户、绝不阻塞启动。下载失败则**删除临时文件**并 Err——
//! 宁可让用户重来,也不留半截安装包。
//!
//! 降级语义(§U.2):编译期 `QIDIWORK_UPDATER_PUBKEY` 为空 → 跳过验签,
//! `verified=false`(当前发布链尚未配密钥,属过渡态);非空 → 签名缺失或不匹配
//! 即**硬失败**,不留文件。

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// 自托管版本清单(nginx 已配 no-cache + CORS *)。
const LATEST_JSON_URL: &str = "https://qidiwork.qidiai.ltd/releases/latest.json";

/// 下载落地目录(位于系统临时目录下,与 `%TEMP%` 一致)。
const UPDATE_DIR_NAME: &str = "qidiwork-update";

/// 安装包大小上限:512 MiB。超限即中止并删除,防磁盘打满。
const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;

/// 连接超时 / 整体超时(整包 600 s,慢网也不至于挂死 IPC)。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(600);

/// 编译期公钥(base64 的 32 字节 Ed25519 公钥)。未配置 → None。
const UPDATER_PUBKEY: Option<&str> = option_env!("QIDIWORK_UPDATER_PUBKEY");

/// 下载成功的安装包描述(前端据此显示"已验证/未验证")。
#[derive(Debug, Clone, Serialize)]
pub struct DownloadedUpdate {
    /// 落地绝对路径(`%TEMP%\qidiwork-update\<文件名>`)。
    pub path: String,
    /// 实际字节数。
    pub bytes: u64,
    /// 实际落盘内容的 sha256(小写十六进制)。
    pub sha256: String,
    /// 是否通过 Ed25519 验签(无公钥或无清单 sha256 时为 false)。
    pub verified: bool,
}

// ---------------------------------------------------------------- 版本比较

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

/// 检查 GUI 是否有新版本。
///
/// 有 → `Some({"version","url","sha256","sig"})`;无/失败 → None。
/// `sha256`/`sig` 缺失时为 null(清单可只给 url,此时下载后 `verified=false`)。
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

    // 清单结构:{"gui":{"version":"0.1.1","url":"…","sha256":"…","sig":"…"},"kernel":{…}}
    let gui = body.get("gui")?;
    let latest_raw = gui.get("version")?.as_str()?;
    let url = gui.get("url")?.as_str()?;

    let latest = parse_version(latest_raw)?;
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;

    if latest > current {
        Some(json!({
            "version": latest_raw,
            "url": url,
            "sha256": gui.get("sha256").and_then(Value::as_str),
            "sig": gui.get("sig").and_then(Value::as_str),
        }))
    } else {
        None
    }
}

// ---------------------------------------------------------------- 校验原语

/// 小写十六进制编码。
fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 解析 64 位十六进制 sha256(容忍大小写与首尾空白)。
fn parse_sha256_hex(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for i in 0..32 {
        let hi = (bytes[i * 2] as char).to_digit(16)?;
        let lo = (bytes[i * 2 + 1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// 常量时间字节比较(不因首个不同字节的位置而提前返回)。
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 期望 sha256 是否等于实际 sha256(常量时间;任一非 64 位十六进制 → false)。
pub(crate) fn sha256_matches(expected_hex: &str, actual_hex: &str) -> bool {
    match (parse_sha256_hex(expected_hex), parse_sha256_hex(actual_hex)) {
        (Some(e), Some(a)) => ct_eq(&e, &a),
        _ => false,
    }
}

/// 编译期公钥(空白视为未配置)。
fn configured_pubkey() -> Option<&'static str> {
    UPDATER_PUBKEY.map(str::trim).filter(|s| !s.is_empty())
}

/// 用给定公钥验证 `digest`(32 字节 sha256 原始摘要)的 Ed25519 签名。
fn verify_ed25519(pubkey_b64: &str, digest: &[u8; 32], sig_b64: &str) -> Result<(), String> {
    let pk_raw = B64
        .decode(pubkey_b64.trim().as_bytes())
        .map_err(|e| format!("公钥 base64 解码失败: {e}"))?;
    let pk_arr: [u8; 32] = pk_raw
        .try_into()
        .map_err(|_| "公钥长度不是 32 字节".to_string())?;
    let vk = VerifyingKey::from_bytes(&pk_arr).map_err(|e| format!("公钥无效: {e}"))?;

    let sig_raw = B64
        .decode(sig_b64.trim().as_bytes())
        .map_err(|e| format!("签名 base64 解码失败: {e}"))?;
    let sig_arr: [u8; 64] = sig_raw
        .try_into()
        .map_err(|_| "签名长度不是 64 字节".to_string())?;
    let sig = Signature::from_bytes(&sig_arr);

    vk.verify_strict(digest, &sig)
        .map_err(|e| format!("Ed25519 验签不通过: {e}"))
}

/// 验签总闸:公钥为空 → Ok(false)(未验证,不报错);公钥非空但签名缺失/不匹配 → Err。
fn check_signature(
    pubkey_b64: Option<&str>,
    digest: &[u8; 32],
    sig_b64: Option<&str>,
) -> Result<bool, String> {
    let Some(pk) = pubkey_b64.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(false); // 过渡态:链上还没配公钥
    };
    let Some(sig) = sig_b64.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err("清单缺少 sig,而本版本已配置更新公钥".to_string());
    };
    verify_ed25519(pk, digest, sig)?;
    Ok(true)
}

// ---------------------------------------------------------------- 文件名收敛

/// 从 URL / 原始文件名收敛出**安全**的落地名。
///
/// 规则(§U.3):取最后一个路径段,只保留 `[A-Za-z0-9._-]`,其余 ASCII 字符
/// 替换为 `_`、非 ASCII 一律丢弃;去掉前导 `.`(杜绝 `.`/`..`);无内容则退化为
/// `update-<version>.exe`;最终强制 `.exe` 后缀。
pub(crate) fn safe_filename(raw_or_url: &str, version: &str) -> String {
    // 先砍 query / fragment,再取最后一段(同时兼容 Windows 反斜杠)
    let no_query = raw_or_url.split(['?', '#']).next().unwrap_or(raw_or_url);
    let last = no_query.rsplit(['/', '\\']).next().unwrap_or(no_query);

    let mut kept = String::with_capacity(last.len());
    for c in last.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            kept.push(c);
        } else if c.is_ascii() {
            kept.push('_');
        }
    }
    let kept = kept.trim_start_matches('.');

    let base = if kept.is_empty() {
        fallback_name(version)
    } else {
        kept.to_string()
    };
    if base.to_ascii_lowercase().ends_with(".exe") {
        base
    } else {
        format!("{base}.exe")
    }
}

/// 无可用文件名时的退化名:`update-<version>.exe`(version 也做白名单过滤)。
fn fallback_name(version: &str) -> String {
    let v: String = version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    let v = v.trim_matches('.');
    if v.is_empty() {
        "update.exe".to_string()
    } else {
        format!("update-{v}.exe")
    }
}

/// 下载落地目录:`%TEMP%\qidiwork-update`(与 `run_downloaded_update` 同一白名单)。
pub fn update_dir() -> PathBuf {
    std::env::temp_dir().join(UPDATE_DIR_NAME)
}

// ---------------------------------------------------------------- 下载

/// 下载安装包到 `%TEMP%\qidiwork-update\<文件名>`。
///
/// - 流式写盘(`Response::chunk()` 循环),不整包进内存;边写边算 sha256。
/// - 上限 512 MiB;超限 / sha256 不符 / 验签失败 → 删除临时文件并 Err。
/// - `sha256` 为空串:跳过摘要比对,但结果**必定** `verified=false`。
/// - 文件名取 URL 末段(白名单过滤);取不到时退化为 `update-<version>.exe`。
///   本包装器不带版本号,故退化名为 `update.exe`;命令层用
///   [`download_update_versioned`] 传入清单版本,退化名带版本号。
pub async fn download_update(
    url: &str,
    sha256: &str,
    sig: Option<&str>,
) -> Result<DownloadedUpdate, String> {
    download_update_versioned(url, sha256, sig, "").await
}

/// [`download_update`] 的带版本号版本(命令层传入清单 `version`,用于退化文件名)。
pub async fn download_update_versioned(
    url: &str,
    sha256: &str,
    sig: Option<&str>,
    version: &str,
) -> Result<DownloadedUpdate, String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("下载地址为空".to_string());
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(format!("下载地址必须是 http(s): {url}"));
    }
    let expected = sha256.trim();
    if !expected.is_empty() && parse_sha256_hex(expected).is_none() {
        return Err("清单里的 sha256 不是 64 位十六进制".to_string());
    }

    let dir = update_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("创建更新目录失败 {}: {e}", dir.display()))?;
    let target = dir.join(safe_filename(url, version));
    // 下载前删除同名旧文件,避免把上次的半截包当成新包
    let _ = std::fs::remove_file(&target);

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败:HTTP {}", resp.status()));
    }
    // 不用 let-chain:本项目 MSRV 1.85(Cargo.toml rust-version),链式 if 需 1.88+
    if let Some(len) = resp.content_length() {
        if len > MAX_UPDATE_BYTES {
            return Err(format!(
                "安装包 {len} 字节超过 {} MiB 上限",
                MAX_UPDATE_BYTES / (1024 * 1024)
            ));
        }
    }

    let mut hasher = Sha256::new();
    let mut total: u64 = 0;

    let written: Result<(), String> = async {
        let mut file = tokio::fs::File::create(&target)
            .await
            .map_err(|e| format!("创建文件失败 {}: {e}", target.display()))?;
        loop {
            let chunk = resp.chunk().await.map_err(|e| format!("下载中断: {e}"))?;
            let Some(chunk) = chunk else { break };
            total += chunk.len() as u64;
            if total > MAX_UPDATE_BYTES {
                return Err(format!(
                    "安装包超过 {} MiB 上限,已中止",
                    MAX_UPDATE_BYTES / (1024 * 1024)
                ));
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("写盘失败: {e}"))?;
        }
        file.flush().await.map_err(|e| format!("刷盘失败: {e}"))?;
        Ok(())
    }
    .await;

    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&target).await;
        return Err(e);
    }

    let finalized = hasher.finalize();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&finalized);
    let actual_hex = hex_of(&digest);

    if !expected.is_empty() && !sha256_matches(expected, &actual_hex) {
        let _ = tokio::fs::remove_file(&target).await;
        return Err(format!("sha256 校验失败:期望 {expected},实际 {actual_hex}"));
    }

    let verified = match check_signature(configured_pubkey(), &digest, sig) {
        Ok(ok) => ok && !expected.is_empty(),
        Err(e) => {
            let _ = tokio::fs::remove_file(&target).await;
            return Err(format!("签名校验失败:{e}"));
        }
    };

    Ok(DownloadedUpdate {
        path: target.to_string_lossy().into_owned(),
        bytes: total,
        sha256: actual_hex,
        verified,
    })
}

// ---------------------------------------------------------------- 运行安装包

/// 运行已下载的安装程序。
///
/// 只允许 `%TEMP%\qidiwork-update\` **直接位于**其下、扩展名 `.exe` 的真实文件
/// (canonicalize 后校验,防符号链接/`..` 逃逸),然后交给 tauri-plugin-opener
/// (ShellExecuteW 语义,**不经过 cmd.exe**,避免命令注入)。
///
/// 用 `dunce::canonicalize` 而非 `Path::canonicalize`(工程约定:避免 Windows
/// `\\?\` 前缀在系统外壳/日志里显示成怪路径)。
pub fn run_downloaded_update(path: &str) -> Result<(), String> {
    let dir = dunce::canonicalize(update_dir()).map_err(|e| format!("更新目录不可用: {e}"))?;
    let canon =
        dunce::canonicalize(path).map_err(|e| format!("安装包不存在或不可访问 {path}: {e}"))?;
    if canon.parent() != Some(dir.as_path()) {
        return Err(format!("只允许运行更新目录下的安装包: {}", canon.display()));
    }
    let is_exe = canon
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("exe"))
        .unwrap_or(false);
    if !is_exe {
        return Err("只允许运行 .exe 安装程序".to_string());
    }
    let s = canon
        .to_str()
        .ok_or_else(|| "安装包路径非 UTF-8".to_string())?;
    tauri_plugin_opener::open_path(s, None::<&str>).map_err(|e| format!("启动安装程序失败: {e}"))
}

#[cfg(test)]
mod tests {
    // 测试模块内允许 unwrap/expect/panic(非测试代码仍全禁)
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex_of(&h.finalize())
    }

    // ---------------------------------------------------------- 版本比较

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

    // ---------------------------------------------------------- sha256

    #[test]
    fn sha256_matches() {
        // 测试函数与工具函数同名,须用 `super::` 消歧
        let hex = sha256_hex(b"qidiwork-update-payload");
        assert!(super::sha256_matches(&hex, &hex));
        // 大小写与首尾空白容忍
        assert!(super::sha256_matches(
            &format!(" {} ", hex.to_ascii_uppercase()),
            &hex
        ));
        assert!(parse_sha256_hex(&hex).is_some());
    }

    #[test]
    fn sha256_mismatch_rejected() {
        let a = sha256_hex(b"payload-a");
        let b = sha256_hex(b"payload-b");
        assert!(!super::sha256_matches(&a, &b));
        // 只差 1 bit 也必须拒绝
        let mut bytes = parse_sha256_hex(&a).unwrap();
        bytes[31] ^= 0x01;
        assert!(!super::sha256_matches(&hex_of(&bytes), &a));
        // 非 64 位十六进制一律 false
        assert!(!super::sha256_matches("", &b));
        assert!(!super::sha256_matches("zz", &b));
        assert!(!super::sha256_matches(&a, ""));
    }

    // ---------------------------------------------------------- Ed25519

    #[test]
    fn ed25519_signature_roundtrip() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_b64 = B64.encode(sk.verifying_key().to_bytes());
        let digest = [9u8; 32];
        let sig_b64 = B64.encode(sk.sign(&digest).to_bytes());

        assert!(verify_ed25519(&pubkey_b64, &digest, &sig_b64).is_ok());
        assert_eq!(
            check_signature(Some(&pubkey_b64), &digest, Some(&sig_b64)),
            Ok(true)
        );
        // 空白包裹也应容忍
        assert!(
            verify_ed25519(&format!(" {pubkey_b64} "), &digest, &format!("{sig_b64}\n")).is_ok()
        );
    }

    #[test]
    fn bad_signature_rejected() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_b64 = B64.encode(sk.verifying_key().to_bytes());
        let digest = [9u8; 32];
        let mut sig_bytes = sk.sign(&digest).to_bytes();
        sig_bytes[0] ^= 0x01; // 改 1 字节
        let bad_sig_b64 = B64.encode(sig_bytes);
        assert!(check_signature(Some(&pubkey_b64), &digest, Some(&bad_sig_b64)).is_err());

        // 签名对但摘要被改 1 字节 → 也必须拒绝
        let good_sig_b64 = B64.encode(sk.sign(&digest).to_bytes());
        let mut other_digest = digest;
        other_digest[0] ^= 0xff;
        assert!(check_signature(Some(&pubkey_b64), &other_digest, Some(&good_sig_b64)).is_err());

        // 用另一把密钥签 → 拒绝
        let other = SigningKey::from_bytes(&[8u8; 32]);
        let other_sig = B64.encode(other.sign(&digest).to_bytes());
        assert!(check_signature(Some(&pubkey_b64), &digest, Some(&other_sig)).is_err());
    }

    #[test]
    fn unsigned_with_pubkey_rejected() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_b64 = B64.encode(sk.verifying_key().to_bytes());
        let digest = [1u8; 32];

        // 配了公钥但没有签名 / 空签名 → Err(硬失败)
        assert!(check_signature(Some(&pubkey_b64), &digest, None).is_err());
        assert!(check_signature(Some(&pubkey_b64), &digest, Some("   ")).is_err());
        // 公钥本身非法 → Err
        assert!(check_signature(Some("not-base64!!"), &digest, Some("x")).is_err());

        // 没配公钥 → 未验证但不报错(过渡态)
        assert_eq!(check_signature(None, &digest, None), Ok(false));
        assert_eq!(check_signature(Some("  "), &digest, None), Ok(false));
    }

    // ---------------------------------------------------------- 文件名

    #[test]
    fn filename_sanitized() {
        // 路径穿越:URL 末段与裸文件名两种形态都只能落一个纯文件名
        assert_eq!(
            safe_filename("https://x.example/../../evil.exe", ""),
            "evil.exe"
        );
        assert_eq!(safe_filename("../../evil.exe", ""), "evil.exe");
        assert_eq!(safe_filename("..\\..\\evil.exe", ""), "evil.exe");

        // 落地目标必须直接位于更新目录下
        let dir = update_dir();
        let target = dir.join(safe_filename(
            "https://x.example/..%2f..%2fevil.exe?x=1#y",
            "",
        ));
        assert_eq!(target.parent(), Some(dir.as_path()));
        assert_eq!(
            target
                .file_name()
                .and_then(|f| f.to_str())
                .map(|f| f.contains('/') || f.contains('\\')),
            Some(false)
        );

        // 正常发布名保留
        assert_eq!(
            safe_filename(
                "https://qidiwork.qidiai.ltd/releases/QidiWork_0.1.12_x64-setup.exe",
                "0.1.12"
            ),
            "QidiWork_0.1.12_x64-setup.exe"
        );
        // 非法字符替换、缺扩展名补 .exe、无末段/`..` 走退化名
        assert_eq!(
            safe_filename("https://x/setup tool.exe", ""),
            "setup_tool.exe"
        );
        assert_eq!(safe_filename("https://x/setup tool", ""), "setup_tool.exe");
        assert_eq!(safe_filename("https://x/", "0.1.12"), "update-0.1.12.exe");
        assert_eq!(safe_filename("https://x/..", "0.1.12"), "update-0.1.12.exe");
        assert_eq!(safe_filename("https://x/", ""), "update.exe");
    }

    // ---------------------------------------------------------- 参数校验

    #[test]
    fn download_rejects_bad_args_before_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // 空地址 / 非 http(s) / 非法 sha256 都在发起网络请求前被拒
        assert!(rt.block_on(download_update("", "", None)).is_err());
        assert!(
            rt.block_on(download_update("file:///c:/evil.exe", "", None))
                .is_err()
        );
        assert!(
            rt.block_on(download_update("https://x/a.exe", "not-a-digest", None))
                .is_err()
        );
    }

    #[test]
    fn run_update_rejects_outside_dir() {
        // 更新目录外 / 不存在的路径一律拒绝
        assert!(run_downloaded_update("C:\\Windows\\System32\\cmd.exe").is_err());
        let missing = update_dir().join("definitely-not-here.exe");
        assert!(run_downloaded_update(&missing.to_string_lossy()).is_err());
    }
}
