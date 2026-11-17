# GUI 更新：清单契约 / 下载 / Ed25519 校验（updater）

> 施工规格：`docs/施工规格-updater+perm-2026-11-17.md` §U
> 代码：`gui/src-tauri/src/updater.rs`、`gui/src-tauri/src/commands.rs`（末尾两条命令）、
> `gui/src/components/SettingsModal.vue`（更新提示条）

## 1. 用途

设置页的「检查更新 → 下载 → 运行安装程序」闭环：

1. 拉取自托管清单 `https://qidiwork.qidiai.ltd/releases/latest.json`，
   与自身 `CARGO_PKG_VERSION` 做 `major.minor.patch` 严格比较；
2. 远端 **严格大于** 自身时，前端出现更新提示条（版本号 + 校验徽标 + 「下载」）；
3. 「下载」走 Rust 侧流式下载 + 完整性校验，落地到 `%TEMP%\qidiwork-update\`；
4. 下载完成后按钮变为「运行安装程序」，由 tauri-plugin-opener
   （ShellExecuteW 语义，不经过 `cmd.exe`）拉起安装包。

检查更新的任何失败（网络/解析/版本格式）都**静默返回 None**，不打扰用户、不阻塞启动。

## 2. 清单契约

只在 `latest.json` 的 `gui` 对象内扩展，`kernel` 等其它键不受影响，旧客户端忽略新字段仍可工作：

```json
{
  "gui": {
    "version": "0.1.12",
    "url": "https://qidiwork.qidiai.ltd/releases/QidiWork_0.1.12_x64-setup.exe",
    "sha256": "9f2c…（64 位十六进制，大小写不敏感）",
    "sig": "base64(Ed25519 签名，64 字节)"
  },
  "kernel": { "version": "…", "url": "…" }
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `version` | 是 | `major.minor.patch` 纯数字三段；带后缀（`0.1.2-beta`）视为解析失败，不提示 |
| `url` | 是 | 安装包直链，必须是 `http(s)://` |
| `sha256` | 否 | 安装包摘要。缺失 → 下载仍可做，但结果**必然** `verified=false` |
| `sig` | 否 | Ed25519 签名，**签名对象 = sha256 的 32 字节原始摘要**（不是十六进制字符串） |

签名对象示例（发布脚本侧）：

```bash
# digest = sha256 的 32 字节二进制，不是 64 字符 hex 文本
SIG=$(openssl pkeyutl -sign -inkey updater.key -rawin -in digest.bin | base64 -w0)
```

## 3. 公钥配置（编译期）

```bash
# 32 字节 Ed25519 公钥的 base64，构建 GUI 时注入
export QIDIWORK_UPDATER_PUBKEY="$(base64 -w0 updater.pub)"
cargo build --release   # 或 npm run tauri build
```

- 走 `option_env!("QIDIWORK_UPDATER_PUBKEY")`：**编译期读取**，改公钥必须重新构建。
- 空白值等价于未配置。
- 当前发布链尚未配密钥，属**过渡态**：未注入 → 一切下载结果都是 `verified=false`。

## 4. 降级语义（重要）

| 公钥 | 清单 `sha256` | 清单 `sig` | 结果 |
|---|---|---|---|
| 未配置 | 有，且与落盘一致 | 有/无 | 下载成功，`verified=false`（UI 显示「未验证」+ 醒目提示） |
| 未配置 | 无 | — | 下载成功，`verified=false` |
| 已配置 | 有，且一致 | 有效 | 下载成功，`verified=true`（「已验证」） |
| 已配置 | 有，但**不一致** | 任意 | **硬失败** + 删除文件 |
| 已配置 | — | **缺失/空** | **硬失败** + 删除文件 |
| 已配置 | — | 无效（改 1 字节 / 换密钥） | **硬失败** + 删除文件 |

铁律：**未验证的安装包不得自动运行**——`verified=false` 时前端只做醒目提示，
是否运行由用户显式点「运行安装程序」决定；Rust 侧不提供任何自动运行路径。

## 5. 下载实现约束

| 项 | 取值 |
|---|---|
| 落地目录 | `%TEMP%\qidiwork-update\`（`std::env::temp_dir().join("qidiwork-update")`） |
| 文件名 | 取 URL 末段的 `[A-Za-z0-9._-]`，其余 ASCII 字符替换为 `_`、非 ASCII 丢弃，去前导 `.`；无内容 → `update-<version>.exe`；缺扩展名补 `.exe` |
| 大小上限 | 512 MiB（`content_length` 预检 + 边写边累计双重把关） |
| 超时 | 连接 10 s / 整体 600 s |
| 写盘 | `Response::chunk()` 循环 → `tokio::fs::File`，不整包进内存；边写边喂 `Sha256` |
| 摘要比较 | 先解析成 32 字节再**常量时间**比较 |
| 失败处理 | 下载前删同名旧文件；超限 / 摘要不符 / 验签失败 → 删除已写文件并 `Err` |
| 运行前置校验 | `canonicalize` 后必须**直接位于**更新目录下且扩展名为 `.exe` |

`run_downloaded_update` 刻意做「直接位于其下」（`parent() == 更新目录`）而非 `starts_with`，
以拒绝 `%TEMP%\qidiwork-update\sub\evil.exe` 这类子目录投放。

## 6. IPC 契约

```rust
// 返回 { path, bytes, sha256, verified }
#[tauri::command] download_gui_update(version: String, url: String, sha256: String, sig: Option<String>)
                   -> Result<DownloadedUpdate, String>

// 仅允许更新目录下的 .exe，成功后交给系统外壳启动
#[tauri::command] run_downloaded_update(path: String) -> Result<(), String>
```

前端调用形态：

```ts
await invoke("download_gui_update", {
  version: u.version,
  url: u.url,
  sha256: u.sha256 ?? "",   // 缺失传空串 → 不比对摘要，verified=false
  sig: u.sig ?? null,       // null = 无签名
});
await invoke("run_downloaded_update", { path: res.path });
```

## 7. 前端展示

- 有新版：`📦 发现新版本 <版本>` + 徽标（下载后出现：`已验证` / `未验证`）+ 「下载」。
- 下载中：按钮禁用并显示「下载中…」。
- 下载完成：按钮变「运行安装程序」；`未验证` 时横幅下方显示
  「⚠ 此安装包未通过签名校验，运行前请自行确认来源可信。」并把提示写入消息流。
- 失败：错误文本内联显示（不弹窗），可再次点「下载」重试。
- 始终保留「用浏览器下载」后备入口（opener 打开清单里的直链）。

## 8. 测试

```bash
cd gui/src-tauri
cargo test --lib updater
```

覆盖：`sha256_matches` / `sha256_mismatch_rejected`（含 1 bit 差异）、
`ed25519_signature_roundtrip`、`bad_signature_rejected`（改签名 / 改摘要 / 换密钥）、
`unsigned_with_pubkey_rejected`（配公钥但无签名 → Err；无公钥 → Ok(false)）、
`filename_sanitized`（`../../evil.exe` → `evil.exe`）、
`download_rejects_bad_args_before_network`（空地址 / 非 http(s) / 非法 sha256 在联网前拒）、
`run_update_rejects_outside_dir`。**不测真实网络。**

## 9. 排障

| 现象 | 原因 |
|---|---|
| 一直「未验证」 | 构建时未注入 `QIDIWORK_UPDATER_PUBKEY`（过渡态，符合预期） |
| 「签名校验失败:清单缺少 sig…」 | 已注入公钥但清单没写 `sig` → 按铁律硬失败 |
| 「sha256 校验失败」 | 清单摘要与安装包不符（发布流程需重算） |
| 「安装包超过 512 MiB 上限」 | 体积超限，需拆分或调整 `MAX_UPDATE_BYTES` |
| 「只允许运行更新目录下的安装包」 | 传入的路径不在 `%TEMP%\qidiwork-update\` 直接下层 |
| 「更新目录不可用」 | `%TEMP%` 不可写或目录被清理工具删掉了 |

## 10. 清理

安装包留在 `%TEMP%\qidiwork-update\`，由系统临时目录清理策略回收；同版本重复下载会先删旧文件。

## 11. 发版签名（工具链）

工具在 `tools/release/`（说明见 `tools/release/README.md`），替代 §2 里手写的 openssl 片段，
输出与 `verify_ed25519` / `check_signature` 逐字节对齐的清单字段。

| 脚本 | 作用 |
|---|---|
| `gen-update-key.py` | 生成 `update-signing-key.pem`（PKCS8 Ed25519），打印 `PRIVATE_KEY=` 与 `PUBLIC_KEY_B64=`（32 字节 raw 的 base64）；已存在则拒绝覆盖（`--force` 才覆盖） |
| `sign-update.py` | 算 `sha256`（hex + 32 字节原始摘要）→ Ed25519 签名 → 输出可粘贴的 `{version,file,sha256,size_bytes,sig}`；`--verify` 用导出公钥自验签并打印 `SIGN OK` |

发版三步（公钥先注入，再出包，再签名）：

```bash
set QIDIWORK_UPDATER_PUBKEY=<PUBLIC_KEY_B64>      # 编译期注入，改钥必须重新构建
npm run tauri build
python tools/release/sign-update.py --key G:/qidiwork-secrets/update-signing-key.pem \
  --file prod/releases/qidiwork-setup-0.1.12.exe --version 0.1.12 --verify
```

- 私钥**绝不入库**，固定放仓库外 `G:\qidiwork-secrets\` 并做离线备份。
- 清单 `version` 必须是纯 `major.minor.patch`（§2 版本解析规则），所以文件名里的重打包后缀
  `-r4` 不能写进 `version`；`sign-update.py` 省略 `--version` 时会自动剥掉 `-rN` 并从文件名推导，
  推导结果打到 stderr，务必人工核对。
- **换钥 = 一次迁移**：旧客户端内嵌旧公钥，新钥签的新包会被它硬拒。必须先发一次"带新公钥"的版本
  （该版用旧钥签，或在不注入公钥的过渡态发布），等用户装上来之后，再正式启用新钥。
