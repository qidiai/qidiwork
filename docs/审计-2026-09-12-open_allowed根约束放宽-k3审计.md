# open_allowed 根约束放宽 kimi-k3 聚焦审计报告

- 审计方: **kimi-k3(经 ai-bridge 网关 127.0.0.1:9800,tools/_k3_audit_dispatch2.py 直调)**
- 生成时间: 2026-09-12
- 审计对象: gui/src-tauri/src/office/mod.rs(open_allowed)+ commands.rs(office_open / office_read_file / office_delete_workspace)
- 触发: 冒烟测试发现**全部真实产物预览/打开被拒**——bid-* 技能把交付物写在用户项目目录(E:\合肥方案\…)并在 manifest 登记原路径,k3 M1 审计加的"canonicalize 落在 workspaces 根内"约束将其全数拒绝(报"路径越界")。读取/打开类操作放宽为"manifest 调解 + 扩展名白名单",根约束仅保留给删除类破坏性操作。

---

## k3 原始结论

总评:**GO with fixes**(放宽方向成立;W1/W2 为必修)

对攻击面三问:
1. 被攻陷 webview **不能**借道读任意文件——无参数可注入裸路径,只能枚举已登记 name;
2. "用户点开=看自己的文件"对 office_open 成立,对 office_read_file 不完全成立(base64 字节进 webview,与 W3 叠加成外泄链);
3. 根外 + opener 组合有新风险:**UNC 路径点击 → ShellExecute/SMB 发起 NTLM 认证 → NTLMv2 哈希外泄**(原根约束恰好挡住,本次放宽新引入)。

Blocker: 无

Warning:
1. **W1(必修)符号链接绕过白名单** — 扩展名取自 canonicalize **之前**的路径:prompt 注入 → agent 建 `交付物.docx` 符号链接指向 `payload.exe` 并登记 → 扩展名检查放行 → canonicalize 解析出 exe → ShellExecute 执行,GUI 成为绕过 agent exec 审批的 confused deputy。修复:canonicalize 后对最终路径重查白名单。
2. **W2(必修)UNC/设备/相对路径** — `\\attacker\share\x.docx` 通过全部检查,点击即 NTLMv2 哈希外泄;相对路径按 GUI 进程 CWD 解析不可预测。修复:canonicalize 后断言本地盘符绝对路径(Disk/VerbatimDisk),相对路径提前拒绝。
3. **W3 read 与 open 风险不对等,html 是放大器** — office_read_file 把字节 base64 送进 webview;白名单含 html,若被渲染为活页面即 agent 内容在特权 webview 执行。修复:read 链剔除 html(仅系统打开保留)。

Note:
1. N1 exists→canonicalize→open 的 TOCTOU 窗口毫秒级且利用者需本地写权限,W1 修复后收益更低,不单独修。
2. N2 commands.rs 两处注释仍写"根约束",与实现矛盾(已同步修正)。
3. N3 read_manifest 的 task 校验与 office_delete_workspace 不一致(未拒 `:`/`.`),只读危害低,建议抽统一校验函数(遗留)。
4. N4 删除链确认原样未动;测试与实现一一对应无漂移。

---

## 修复处置(审计当日全部落地,复检 cargo test 40/40 通过)

| 项 | 处置 |
|---|---|
| W1 符号链接绕过 | **已修**:open_allowed 改为 canonicalize **先**、扩展名白名单对 canonical 路径检查 |
| W2 UNC/设备/相对 | **已修**:相对路径入口即拒;新增 `is_local_absolute`(Windows: Disk/VerbatimDisk 前缀;Unix: RootDir),canonical 后断言 |
| W3 html 进 webview | **已修**:新增 `PREVIEW_READ_EXTENSIONS`(剔除 html)与 `open_allowed_for_read`,office_read_file 改用;html 仍可系统打开 |
| N2 注释失实 | **已修**:office_open / office_read_file 文档注释同步 |
| N3 task 校验不一致 | **存照**(遗留):建议抽统一校验函数,三处共用 |
| N4 | 确认项,无需动作 |

新增测试:open_allowed_enforces_extension_and_allows_registered_out_of_root(根外 docx 放行/zip 拒/不存在拒)、open_allowed_rejects_relative_and_read_rejects_html(相对拒/html read 拒而 open 放/md 双链放行)。
