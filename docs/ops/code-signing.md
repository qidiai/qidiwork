# Authenticode 代码签名（SignPath Foundation）

> 工具：`tools/release/signpath-sign.py` · 构建入口：`build_release.ps1`（步骤 4/4，可选）
> API 契约：<https://docs.signpath.io/build-system-integration>（2026-11-22 核对）

## 1. 两种签名,分工不同

| | Authenticode 代码签名（本文） | Ed25519 更新校验（`sign-update.py`） |
|---|---|---|
| 给谁看 | Windows SmartScreen、浏览器、`exe` 属性页的数字签名页签 | 应用内自动更新器（`updater.rs`） |
| 解决什么 | 「未知发布者」蓝色拦截、下载安全风险提示 | 安装包被篡改/调包 |
| 私钥在哪 | SignPath HSM（我们不碰私钥） | 自保管（`G:\qidiwork-secrets\`） |
| 状态 | **已申请**（2026-11-22 提交 SignPath Foundation，审批中） | 0.1.12 起已上线 |

两者都要做，且**顺序固定**：先 Authenticode，再 Ed25519（见 §5）。

## 2. 为什么是 SignPath Foundation

- 对开源项目**免费**，证书签发给我们这个项目（不是个人），私钥在其 HSM 上，
  天然适配「本机构建机 + 手工发布链」——不需要硬件令牌、不需要云签名账号；
- 申请门槛我们基本已满足：公开仓库（GitHub `qidiai/qidiwork`）、Apache 2.0、
  有官网下载页（`https://qidiwork.qidiai.ltd/`）；
- 服务商名录见 <https://signpath.org/projects.html>（Flameshot、Git Extensions 等同侪）。

若申请未过或周期不可接受，备选路线见 §8。

## 3. 申请（需要真人决策，无法代劳）

**申请前置（2026-11-22 已就绪并部署上线）**：

- 下载页 <https://qidiwork.qidiai.ltd/> 已声明「安装包由 SignPath Foundation 提供代码签名」；
- 隐私政策页 <https://www.qidiai.ltd/privacy/> 已上线（应用有 OAuth 登录，属处理用户数据，
  申请表 Privacy Policy URL 填它）。

1. <https://signpath.org/apply> 提交项目信息：仓库地址、许可证、官网、
   项目简介、预计发布节奏；
2. 审批制，周期以对方回复为准；期间对外分发仍按无签名流程
   （`docs/安装指引-SmartScreen.md` 的绕过步骤）；
3. 通过后在 SignPath 控制台配置（ID 都在签名策略详情页可查）：
   - 建一个 **CI 用户**并生成 **API token**（不要用个人交互式账号的 token）；
   - 记下 `organizationId`（GUID）、`projectSlug`、`signingPolicySlug`；
   - artifact configuration 用默认即可，除非控制台另有要求。

**令牌纪律**：token 只走环境变量注入，**绝不入库、不进 `prod/`、不进任何会
被打包/提交的目录**；泄露 → 控制台立即吊销重签。

## 4. 使用

环境变量（`build_release.ps1` 检测到 `SIGNPATH_API_TOKEN` 即自动执行步骤 4/4）：

```powershell
# PowerShell,每次发版会话设一次
$env:SIGNPATH_API_TOKEN          = "<CI 用户 token>"
$env:SIGNPATH_ORGANIZATION_ID    = "<组织 GUID>"
$env:SIGNPATH_PROJECT_SLUG       = "qidiwork"
$env:SIGNPATH_SIGNING_POLICY_SLUG = "<签名策略 slug>"
# 可选:$env:SIGNPATH_ARTIFACT_CONFIG_SLUG / SIGNPATH_TIMEOUT_MINUTES(默认 30) / SIGNPATH_POLL_SECONDS(默认 10)
```

手工单跑（签名产物**就地替换**原文件，`--out` 可改输出路径）：

```bash
python tools/release/signpath-sign.py \
  --file prod/releases/qidiwork-setup-0.1.13.exe \
  --param productVersion=0.1.13 \
  --verify        # 拉回后调 signtool 自验签(自动探测 Windows Kits)
```

输出 stdout 单行 JSON 留档：`{"signingRequestId","statusUrl","file","sha256","size_bytes"}`。

改动脚本后先跑自测（本地 mock，不联网、不需要真 token）：

```bash
python tools/release/test_signpath_sign.py     # 期望 7/7 passed
```

行为说明：

- 提交 = `POST /SigningRequests/SubmitWithArtifact`（multipart 直传，安装包
  无需先挂到公网）；
- 轮询 `/Status` 直到 `isFinalStatus`；审批等待很慢属正常，调
  `SIGNPATH_TIMEOUT_MINUTES`，超时**不撤请求**，可按 `statusUrl` 继续跟进；
- `status` 为 `Failed`/`Denied`/`Canceled` → 退出码 1 并给出请求链接；
- 只有 `Completed` 才拉回 `/SignedArtifact`，先写 `.part` 再原子替换，
  不会留下半截文件。

## 5. 发版顺序铁律（顺序反了会出事）

```
① 构建安装包（build_release.ps1 / npm run tauri build）
② SignPath Authenticode 签名        ← 本文档,改变文件字节
③ sha256 + Ed25519 sig              ← sign-update.py,必须签②的产物
④ 写 latest.json 并上传              ← sha256/sig 对应③的输出
```

Authenticode 签名会重写 PE 结构，文件哈希**必然变化**。若先 ③ 后 ②，
`latest.json` 里的 `sha256`/`sig` 与分发文件不符 → 已配置公钥的客户端
**硬失败**（`sha256 校验失败` + 删文件）。同理，任何「重打包/重命名后再传」
的操作都在 ③ 之后做，且必须重跑 ③。

## 6. 效果与预期管理

- 签名版：`exe` 属性 → 数字签名页签显示发布者 **SignPath Foundation**；
  「未知发布者」类警告消失；
- SmartScreen / Chrome·Edge 的「不常下载，可能有危险」提示看的是文件与站点
  **声誉**，低下载量的新文件仍可能有短期提示，随下载量积累自然消退——
  签名不能瞬间清零这类提示，EV 证书才可以（见 §8）；
- 已装旧版（无签名）的用户走应用内更新时，下载的是签名版，体验更好；
  但 `docs/安装指引-SmartScreen.md` 的绕过步骤要保留到旧版本自然淘汰。

## 7. 排障

| 现象 | 原因 / 处理 |
|---|---|
| `缺少环境变量 SIGNPATH_*` | 按 §4 注入；`build_release.ps1` 未设 token 时跳过签名是**预期行为** |
| 提交 4xx（如 400/403） | 脚本不重试 4xx 并打出响应体；常见是 token 无权限 / slug 拼错，对照控制台 |
| 长时间 `WaitingForApproval` | 审批流未点批；去控制台批，或调大 `SIGNPATH_TIMEOUT_MINUTES` |
| 轮询超时但想恢复 | 请求还在：拿 stdout/日志里的 `statusUrl` 手动查状态，完成后用同一 URL 走 `SignedArtifact` 下载 |
| `signtool 验签失败` | 拉回的产物损坏或网络中断，重提一次；仍失败看 SignPath 审计日志 |
| 客户端「sha256 校验失败」 | 踩了 §5 的顺序坑：对签名后的文件重跑 `sign-update.py` 并更新清单 |

## 8. 备选路线（Foundation 申请未过时）

| 路线 | 成本 | 说明 |
|---|---|---|
| Azure Trusted Signing | 约 $10/月 | 微软官方，EV 级信任（SmartScreen 立即免提示），纯软件无需硬件令牌；需 Azure 订阅 + 身份验证 |
| 购买 EV 代码签名证书 | 约 $400–700/年 | SmartScreen 立即信任；通常需企业资质 + 硬件令牌/云 HSM |
| 购买 OV 证书 | 约 $200–500/年 | 去掉「未知发布者」，SmartScreen 仍需声誉积累 |
| SignPath Foundation（当前选择） | 免费 | 审批制；证书归属项目、HSM 托管私钥 |

## 9. 未来可选项

- **GitHub Actions 化发布**：`windows-latest` runner 上构建 + SignPath
  GitHub Action（`signpath/github-action-submit-signing-request`）自动
  提交签名，配合 Trusted Build System 还能开启 origin verification
  （把「这个二进制出自哪个 commit」也签进去）。当前手工链不上 CI 是因为
  页数实测依赖本机 WPS COM，迁移需先解掉这个依赖。
- **内核也签名**：`target/release/qidiwork.exe` 目前随包分发、未单独签名。
  若要做，同一策略加一个提交即可。
