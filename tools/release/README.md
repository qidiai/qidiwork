# 更新包签名工具链（tools/release）

给安装包生成 `sha256` + Ed25519 `sig`，与客户端
[`gui/src-tauri/src/updater.rs`](../../gui/src-tauri/src/updater.rs) 的
`verify_ed25519` / `check_signature` **逐字节对齐**。

## 0. 契约（先读这一段，改工具前必须复读）

| 项目 | 客户端要求 | 本工具做法 |
|---|---|---|
| 签名对象 | `sha256` 的 **32 字节原始摘要**（不是 hex 字符串） | `hashlib.sha256(...).digest()` 直接喂给 Ed25519 |
| 签名格式 | Ed25519，**base64**（解码后 64 字节） | `base64.b64encode(sig)` |
| 公钥格式 | **base64**（解码后 32 字节 raw），编译期 `option_env!("QIDIWORK_UPDATER_PUBKEY")` | `Encoding.Raw` + base64 |
| 验签算法 | `ed25519_dalek::VerifyingKey::verify_strict` | 标准 Ed25519（RFC 8032），与 strict 模式一致 |
| 摘要比较 | 64 位小写十六进制、常量时间比较 | `hexdigest()` 即小写 64 位 |
| 清单字段 | `{"version","url","sha256","sig"}` | `sign-update.py` 输出 `version/file/sha256/size_bytes/sig`，`url` 发版时补 |

> 低于 512 MiB 上限：客户端 `MAX_UPDATE_BYTES` 为 512 MiB，超限会被拒。

## 1. 密钥保管（最重要的纪律）

- **私钥绝不入库**（不加 `git add`，不进 `prod/`，不进任何会被打包的目录）。
- 建议固定存放：`G:\qidiwork-secrets\update-signing-key.pem`（仓库外），并另做离线备份
  （U 盘 / 密码管理器）——私钥丢了就只能走第 5 节的换钥流程。
- 私钥文件名固定 `update-signing-key.pem`（PKCS8，无口令加密；靠目录权限保护）。
- 本机生成工具依赖 `cryptography`（当前 48.0.1）：`pip install cryptography`。

生成密钥对（**当前目录**落盘 `update-signing-key.pem`，`--out-dir` 可改，`--force` 才覆盖）：

```bash
python tools/release/gen-update-key.py
# PRIVATE_KEY=G:\qidiwork-secrets\update-signing-key.pem
# PUBLIC_KEY_B64=BEIDI/hLq5LZ5RfFzoLm/+LyIrhie91tMy/fh9GXH3A=
```

`PUBLIC_KEY_B64` 就是注入客户端的公钥（32 字节 raw 的 base64）。

## 2. 发版四步

```bash
# ① 出包（MSI/NSIS 由 tauri 产出，落到 prod/releases/）
#    注意：先注入公钥再构建，见第 3 节
npm run tauri build

# ② 算摘要 + 签名（输出可直接粘进 latest-*.json 的 gui 段）
python tools/release/sign-update.py \
  --key G:/qidiwork-secrets/update-signing-key.pem \
  --file prod/releases/qidiwork-setup-0.1.12.exe \
  --version 0.1.12 --verify

# ③ 把输出 JSON 里的 sha256 / size_bytes / sig 填进 prod/releases/latest-<ver>.json，
#    并补 url / notes / published_at（file 字段用安装包文件名）
#    同时更新 *.sha256 文本（见下方一行命令）

# ④ 上传安装包 + 清单到 https://qidiwork.qidiai.ltd/releases/ ，并本地留档
sha256sum prod/releases/qidiwork-setup-0.1.12.exe | cut -d' ' -f1 > prod/releases/qidiwork-setup-0.1.12.sha256
```

`--version` 可省略：从文件名推导（`qidiwork-setup-0.1.11-r4.exe` → `0.1.11`，
重打包后缀 `-rN` 会被剥掉；`-alpha.6` 之类语义化预发布标签保留）。推导结果会打到 stderr，**务必核对**。

输出示例（stdout，`--compact` 输出单行）：

```json
{
  "version": "0.1.11",
  "file": "qidiwork-setup-0.1.11-r4.exe",
  "sha256": "970c0215d32f1211943174f47295b4ff02278a5aeade6c90093b765d81351bd0",
  "size_bytes": 38840414,
  "sig": "2uH0geUTvvtThWsuJrUkV9x8vdGqKhjumtjXfTSMq9kIEVROgTEuhI5sHOGohS+w7DCxFrvPOnoo0mLxEdRQDQ=="
}
```

`--verify` 用私钥导出公钥自验签一次，成功打印 `SIGN OK`（失败 `SIGN FAIL: ...` 且退出码 1）。
`--sig <b64>` 配合 `--verify` 可验任意给定签名，用于篡改自检：

```bash
# 把上面 sig 改一个字符后必须失败
python tools/release/sign-update.py --key <pem> --file <exe> \
  --sig 3uH0geUT...== --verify   # → SIGN FAIL，退出码 1
```

## 3. 公钥注入

公钥是**编译期**常量，改公钥必须重新构建客户端：

```bash
# Windows cmd
set QIDIWORK_UPDATER_PUBKEY=BEIDI/hLq5LZ5RfFzoLm/+LyIrhie91tMy/fh9GXH3A=
npm run tauri build

# PowerShell
$env:QIDIWORK_UPDATER_PUBKEY="BEIDI/hLq5LZ5RfFzoLm/+LyIrhie91tMy/fh9GXH3A="
npm run tauri build

# bash / Git Bash
export QIDIWORK_UPDATER_PUBKEY=BEIDI/hLq5LZ5RfFzoLm/+LyIrhie91tMy/fh9GXH3A=
npm run tauri build
```

- 未注入（或注入空白）→ 客户端走**过渡态**：跳过验签，下载结果标记 `verified=false`，只显示「未验证」提示，不硬失败。
- 已注入 → 清单**必须有 `sig`**，缺失或验签失败一律硬报错（`清单缺少 sig,而本版本已配置更新公钥`）。
- 注入后先做一次真实发版回归：装旧版客户端 → 点检查更新 → 必须出现「已验证」徽标，才算链路通。

## 4. 排障

| 现象 | 原因 |
|---|---|
| 客户端「未验证」 | 构建时没注入 `QIDIWORK_UPDATER_PUBKEY`（过渡态） |
| `SIGN FAIL` 但字节看起来一样 | 签名对象被换成了 hex 字符串 → 必须签 32 字节原始摘要 |
| `公钥长度不是 32 字节` | 注入的是 PEM 或带换行的 base64；应是 32 字节 raw 的 base64（44 字符，含 `=`） |
| `签名长度不是 64 字节` | 签成了 hex，或对文件本体签名（大文件签名 64 字节仍对，但是摘要错误） |
| `sha256 校验失败` | 安装包被替换/重打包后没重跑 `sign-update.py` |

## 5. 失钥处置（换钥 = 一次迁移）

换钥会让**旧客户端无法验证新包**（旧客户端内嵌旧公钥，新包用新钥签 → 硬失败）。所以：

1. 永远**先发一次"带新公钥"的版本**：旧公钥照旧签这一版（老用户能装上），只把公钥换成新的；
   若旧私钥已丢到无法签这一版，则这一版只能不注入公钥（过渡态、`verified=false`）或用旧私钥签。
2. 该版本上线并被大多数用户安装后，再正式改用新私钥签名后续版本。
3. 期间清单里可以同时写旧包（旧钥签）与新包（新钥签），由版本号决定客户端下哪个。
4. 记录换钥日期与两把公钥的 base64，落到发版记录里，避免下次再猜。

## 6. 自测（不碰 prod 文件）

```bash
mkdir -p /tmp/sign-test && cd /tmp/sign-test
python G:/qidiwork/tools/release/gen-update-key.py
python G:/qidiwork/tools/release/sign-update.py --key update-signing-key.pem \
  --file "G:/qidiwork/prod/releases/qidiwork-setup-0.1.11-r4.exe" --version 0.1.11 --verify
```

预期：`SIGN OK` + JSON（`sha256` 与 `prod/releases/latest-0.1.11.json` 一致，可当交叉校验）。
