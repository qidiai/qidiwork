# 施工规格：updater 下载校验 + 权限超时（并行两片）

**日期**：2026-11-17
**来源**：`docs/审计-TigerAgent吸收方案-双席复核-2026-11-17.md` Top-3 的第 1、2 项。
**并行纪律**：两片文件零交集（见 §0）。

---

## 0. 文件所有权

| 片 | 独占文件 |
|---|---|
| **U（updater）** | `gui/src-tauri/src/updater.rs`、`src/commands.rs`、`src/lib.rs`、`gui/src/components/SettingsModal.vue`、`gui/src-tauri/Cargo.toml`、`docs/ops/updater.md` |
| **P（permission）** | `gui/src-tauri/src/acp/bridge.rs`、`docs/ops/permission-timeout.md` |

**共同禁令**：不改 `fill.rs`/`verify.rs`/`markdown.rs`/`office/**`/`tauri.conf.json`/`cli/**`；不改对方的文件；不 commit。

---

## U. updater：下载 + 完整性校验（Ed25519）

### U.1 清单契约（只在 `latest.json` 的 `gui` 对象内扩展，保持兼容）

```json
{"gui":{"version":"0.1.12","url":"https://…/QidiWork_0.1.12_x64-setup.exe",
        "sha256":"<64 hex>","sig":"<base64 ed25519 签名>"}}
```

- `sha256`/`sig` 缺失时：**下载仍可做，但必须视为"未验证"**（返回 `verified:false`，UI 明确提示）。
- `sig` 的签名对象 = **sha256 摘要的 32 字节原始值**（不是十六进制字符串）。

### U.2 公钥与降级

- 编译期环境变量 `QIDIWORK_UPDATER_PUBKEY`（base64，32 字节 Ed25519 公钥）。
- 若为空：跳过签名校验，`verified=false`（当前发布链尚未配密钥，属过渡态）。
- 若非空：**签名缺失或不匹配 → 硬失败**，删除已下载文件，不得提示可运行。

### U.3 API（`updater.rs`）

```rust
pub struct DownloadedUpdate { pub path: String, pub bytes: u64, pub sha256: String, pub verified: bool }

/// 下载到 %TEMP%\qidiwork-update\<文件名>；流式写盘、边写边算 sha256；
/// 上限 512 MiB；超限/校验失败即删除临时文件并 Err。
pub async fn download_update(url: &str, sha256: &str, sig: Option<&str>) -> Result<DownloadedUpdate, String>;
```
- 用 `reqwest::Client`（10 s 连接超时 / 600 s 总超时），`Response::chunk()` 循环写文件（不整包进内存）。
- sha256 用常量时间比较；导入 `sha2`。
- 签名校验用 `ed25519-dalek = "2"`（**已在本机 cargo 缓存：2.2.0**），payload=`[u8;32]` 摘要。
- 文件名从 URL 末段取，白名单字符 `[A-Za-z0-9._-]`，否则用 `update-<version>.exe`。
- 下载前删除同名旧文件；失败清理。

命令层（`commands.rs` 末尾）：
```rust
#[tauri::command] pub async fn download_gui_update(version: String, url: String, sha256: String, sig: Option<String>) -> Result<DownloadedUpdate, String>;
#[tauri::command] pub fn run_downloaded_update(path: String) -> Result<(), String>; // 仅允许 %TEMP%\qidiwork-update\ 下、扩展名 .exe，然后 opener::open
```
`lib.rs` 注册两条命令。

### U.4 前端（`SettingsModal.vue`）

- 「检查更新」现有逻辑保留；有新版时显示：版本号、`已验证/未验证` 徽标、按钮「下载」。
- 下载中禁用按钮并显示"下载中…"；完成后显示「运行安装程序」（调用 `run_downloaded_update`）。
- 未验证（`verified=false`）时必须显示醒目提示"此安装包未通过签名校验"。

### U.5 测试（写在 `updater.rs`）

1. `sha256_matches` / `sha256_mismatch_rejected`（纯函数）
2. `ed25519_signature_roundtrip`：测试内生成 keypair，签 32 字节摘要，验签通过
3. `bad_signature_rejected`：改 1 字节 → Err
4. `unsigned_with_pubkey_rejected`：设公钥但 sig=None → Err
5. `filename_sanitized`：`../../evil.exe` → 落盘名安全

> 不测真实网络；下载函数只测"校验/命名/错误"纯逻辑（把下载体抽成 `verify_and_store(bytes, …)` 或类似内部函数）。

### U.6 验收

- `cargo check --all-targets` 0 error；`cargo test --lib office::bid` 不退化；新增测试全绿
- `cargo test --lib updater` 全绿（新测试）
- `npm run build` 通过
- 文档 `docs/ops/updater.md`：清单字段、公钥配置、降级语义（未验证不得自动运行）

---

## P. permission 请求 per-request 超时 → 拒绝

### P.1 现状（已实测）

`bridge.rs`：`permission_pending` 表（:158）、`handle_agent_request` 落表并发 `PermissionRequest`（:558-600）、`resolve_permission`（:626）、`cancel_permission`（:648）、`expire_permissions_for_session`（:666）。**无 per-request 截止时间**；只有全局 `PROMPT_TIMEOUT = 30 min` 且超时≠取消。

### P.2 目标

- 每条 pending 请求带截止时间（默认 **120 s**，`Bridge` 构造时可注入，便于测试）。
- 到期仍 pending → 自动以 **拒绝** 选项回应 agent，并向 UI 发 `PermissionCancelled{reason:"timeout"}`；**绝不允许自动 allow**。
- 选择"拒绝选项"的规则：在该请求的 `options` 里找 `kind` 含 `reject`（`reject_once`/`reject_always`）的 option id；找不到则走 `cancelled` 路径。

### P.3 实现要点

1. `PermissionEntry` 增字段：`reject_option: Option<String>`（从 params.options 解析，见 `option_ids_of` 旁新增 `reject_option_of`）。
2. `Bridge` 增字段 `permission_timeout: Duration`（默认 120s；`new(...)` 保持不变，内部用默认值；测试用 `#[cfg(test)] fn set_permission_timeout` 或 `new_with_timeout`）。
3. 落表后 `tokio::spawn` 一个超时任务：sleep 到截止 → 加锁检查该 id 仍在表内且 `deadline <= now` → 移除 → 发送拒绝/取消 → 发事件。**不要**为每次请求保存 JoinHandle（任务自行 no-op）。
4. 事件与日志：`tracing::warn!(request_id, session_id, "权限请求超时，按拒绝处理")`。
5. 并发安全：与 `resolve_permission`/`cancel_permission` 共用同一把锁；resolve 先到则超时任务查到已移除 → 直接返回。

### P.4 测试（写在 `bridge.rs` 测试模块，参照既有 mock transport 测试）

1. `permission_timeout_rejects`：注入超时 20ms，发出请求 → 等待 → agent 收到 `selected` + reject optionId，pending 清空，事件含 `reason="timeout"`。
2. `permission_resolved_before_timeout_no_second_response`：先 resolve，再等超过超时 → 无第二条响应。
3. `permission_timeout_without_reject_option_cancels`：options 只有 allow → 到期走 `cancelled`，且**不是** allow。
4. `timeout_never_allows`：断言所有超时路径下发给 agent 的 optionId 均不属于 allow 集合。

### P.5 验收

- `cargo test --lib office::bid` 不退化；`cargo test --lib acp::bridge` 全绿
- `cargo clippy --all-targets` 中 `acp/bridge.rs` 0 新增告警
- 文档 `docs/ops/permission-timeout.md`：默认 120s、拒绝语义、与 30 分钟 `PROMPT_TIMEOUT` 的区别

---

## 共同回执

改动摘要（文件+行数+关键片段）、命令与原始输出、主动声明未验证项，结尾 `CODER RESULT: DONE`。禁止 `git add/commit`、`cargo clean`、`npm run tauri build`。
