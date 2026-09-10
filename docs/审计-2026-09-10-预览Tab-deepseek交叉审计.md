# qidiwork 预览 Tab 变更交叉审计报告

- 模型路由: http://192.168.1.222:8765/v1/chat/completions / model=codely-flash (实际服务: codely-flash)
- 生成时间: 2026-09-10T01-57-19

---

# 审计报告：qidiwork GUI「中区 docx 预览 Tab」未提交变更交叉审计

## 1. 总评

变更主体符合设计意图：Rust 侧 `office_read_file` 与 `office_open` 共用同一校验链，前端 CSP `img-src 'self' data:` 与 `useBase64URL` 匹配，矢量图形降级链路完整；但存在 **1 个必须修复的 Blocker**（预览注册表 `activePreview` 残留导致重开同一产物 Tab 不激活），修复后即可提交。  
**结论：GO with fixes**

---

## 2. Blocker（必须修复才能提交）

### B1/C3：预览 Tab 关闭后 `activePreview` 残留，重开同一产物不激活

- **文件**：`gui/src/composables/usePreview.ts`、`gui/src/components/MainTabs.vue`
- **位置**：`closePreview()` / `watch(activePreview)`
- **问题**：
  1. 用户打开产物 X → `activePreview = 'X'`，激活 Tab；
  2. 关闭该 Tab → `closePreview('X')` 从 `openPreviews` 移除 X，但 **`activePreview` 仍为 `'X'`**；
  3. 再次点击同一产物 → `openPreview()` 因 `openPreviews` 已无 X 而重新 push，并执行 `activePreview.value = tab.key`。由于值与当前 `'X'` 相同，**Vue ref 不触发 watch**，`MainTabs` 的 `watch(activePreview)` 不执行，`activeTab` 仍留在 chat/其他 Tab。
  4. 结果：Tab 重新出现但**未激活**，用户看不到预览内容，必须手动点击一次，与设计意图“重复点同一产物=激活已有 Tab”冲突。
- **附带问题**：用户手动点击其他预览 Tab 后，`activePreview` 不会同步跟随；此时再点回原产物卡片，同样可能因 `activePreview` 相同值而不激活。
- **建议修复**：
  ```ts
  // usePreview.ts
  export function closePreview(key: string): void {
    const idx = openPreviews.value.findIndex((t) => t.key === key);
    if (idx >= 0) openPreviews.value.splice(idx, 1);
    if (activePreview.value === key) activePreview.value = ""; // 必须清理
  }
  ```
  同时在 `MainTabs.vue` 的手动点击 Tab 逻辑中同步 `activePreview`，例如：
  ```ts
  function selectTab(key: TabKey) {
    activeTab.value = key;
    if (key.startsWith("preview:")) activePreview.value = key.slice("preview:".length);
  }
  ```

---

## 3. Warning（建议修复，不阻塞）

### W1：32MB 上限存在 TOCTOU，且全量读入 + base64 使内存峰值翻倍

- **文件**：`gui/src-tauri/src/commands.rs`（`office_read_file`）
- **问题**：
  - 大小检查用 `metadata().len()` 在 `fs::read` **之前**执行，两者之间文件可能被替换膨胀，导致实际读取超过上限；
  - `fs::read` 全量读入 32MB + base64 编码生成 ~44MB 字符串，内存峰值约 2.5~3 倍文件大小；
  - IPC 传大 base64 payload 在 WebView 端 `atob` 解码也会阻塞 UI 线程。
- **建议**：用 `File::open` + `Take(PREVIEW_MAX_BYTES + 1)` 严格限制读取量并在读取后校验；如业务上标书 1~2MB，可考虑将上限降到 8~16MB；同时将阻塞 IO/编码放入 `spawn_blocking`。

### W2：生产 CSP `style-src 'self'` 可能拦截 docx-preview 注入的内联样式

- **文件**：`gui/src-tauri/tauri.conf.json`、`gui/src/services/docxPreview.ts`
- **问题**：生产 CSP 为 `style-src 'self'`，没有 `'unsafe-inline'`；docx-preview 渲染时通常会在容器内注入 `<style>` 元素或元素内联 `style` 属性，若被 CSP 拦截会导致样式缺失、排版错乱。材料中的 spike 目录未提供验证内容，无法确认实际表现。
- **建议**：用生产构建（非 devCsp）跑一次 spike 样本确认渲染样式完整；如确实需要内联样式，调整 CSP（例如为注入 style 加 nonce/hash），或显式引入 docx-preview 的 CSS 产物。

### W3：打开/降级按钮的错误路径未捕获，失败无 UI 反馈

- **文件**：`gui/src/components/DocxPreview.vue`（`openInSystem`）、`gui/src/composables/useOffice.ts`（`openArtifact`）
- **问题**：`openPreviewArtifact` / `openArtifact` 的 `invoke` 没有 `try/catch`；当 `office_open` 因 manifest 过期、文件被删等原因拒绝时，会变成 unhandled promise rejection，用户点击无任何反馈。
- **建议**：在按钮处理器中 `catch` 并显示错误；`openArtifact` 同理。

### W4：`open_allowed` 内部实现不在审计材料内，路径比较方式无法独立验证

- **文件**：`gui/src-tauri/src/commands.rs`（依赖 `office.rs` 的 `open_allowed`）
- **问题**：`office_read_file` 没有直接做 canonicalize 根比较，而是复用 `open_allowed(&card.path, &root)`；其内部比较方式（是否处理 `G:\foo` 与 `G:\foobar` 前缀陷阱、symlink 等）在本次材料中不可见。
- **建议**：补充/确认 `open_allowed` 已通过 P1a 审计；如未审计应补查。

### W5：`vendor 入 node_modules` 的说法与 `.gitignore` 矛盾

- **文件**：`gui/package.json`、`.gitignore`
- **问题**：
  - `docx-preview: ^0.4.0`、`jszip: ^3.10.2` 均为 semver 范围，仅由 `package-lock.json` 锁定；
  - `.gitignore` 明确忽略 `gui/node_modules/`，因此**并未 vendor**，发布构建仍依赖 npm registry 重新安装。
- **建议**：若发布环境可联网，保留 lockfile + `npm ci` 即可（当前可接受）；若目标环境离线或要求可重复构建，需改为真正的 vendor 策略或内部镜像，并修正文档说法。

### W6：async 命令内执行同步阻塞 IO

- **文件**：`gui/src-tauri/src/commands.rs`（`office_read_file`）
- **问题**：`#[tauri::command] pub async fn` 内直接执行 `fs::metadata` / `fs::read` / base64 编码（最大 32MB），阻塞 tokio worker，可能拉高其他 IPC 命令（agent/session）延迟。
- **建议**：将阻塞段放入 `tauri::async_runtime::spawn_blocking`。

---

## 4. Note（工程债/风格）

### N1：`DocxPreview.vue` 的 props watch 冗余

- 父级使用 `:key="activeTab"`，组件随 activeTab 切换而销毁重建，`props.task/name` 在组件生命周期内不会变化；`watch(() => [props.task, props.name], load)` 实际不会在有效场景触发，可删除。

### N2：`previewKey` 使用裸 `/` 拼接，含 `/` 时 key 冲突

- `previewKey(task, name)` 返回 `` `${task}/${name}` ``；若 task 或 name 含 `/`（如任务名含路径分隔符），会与另一组 task/name 产生相同 key。中文/空格无碍。建议改用 `encodeURIComponent` 或数组 JSON 序列化。另注释“以产物绝对路径派生”与实现不符。

### N3：`gui/spike-docx/` 未跟踪目录需处置

- 当前为 untracked 目录，既未加入版本控制也未 ignore。若 spike 产物需要保留回归样本，应明确纳入并整理；否则加入 `.gitignore`，避免误提交实验文件。

### N4：docs 中文文件名提交风险

- `docs/工作记录-2026-09-09-预览Tab与memory-scope修复.md` 在 Windows/CI 脚本中可能有编码/路径兼容性问题；建议改为 ASCII 文件名（如 `docs/worklog-2026-09-09-preview-tab.md`）。

### N5：矢量图形判定是启发式

- `drawings > blips` 可覆盖“纯位图”“纯矢量”边界，但**混合文档**（同时含矢量形状和位图且数量相等）可能漏判。建议保留 spike 样本作为回归测试，并在代码注释中注明判定边界。

### N6：模块级注册表不随任务切换清理

- `usePreview.ts` 的 `openPreviews` / `activePreview` 为模块级单例，切换任务后旧任务的预览 Tab 仍保留，加载时会报错并走降级 UI。当前可接受，后续可在切换任务时清理。

---

## 5. 核对表

| 项 | 结论 | 证据 |
|---|---|---|
| A1 | ✅ | `office_read_file` 与 `office_open` 同样依次调用 `read_manifest` → `manifest.iter().find` → `open_allowed`，无绕过；校验链完整一致。 |
| A2 | ⚠️ | 大小上限在 `metadata().len()`（读取前）检查，但存在 TOCTOU；`fs::read` 全量读入 + base64 编码使内存峰值翻倍，IPC 大 payload 有性能风险。 |
| A3 | ⚠️ | 复用 `open_allowed(&card.path, &root)`，本命令代码未直接做根比较；`open_allowed` 内部实现不在材料中，前缀陷阱/符号链接处理无法独立验证。 |
| A4 | ✅ | 所有错误经 `Err(String)` → Tauri reject → `DocxPreview.vue` catch → `phase = error` 显示降级 UI + “用系统程序打开”按钮。 |
| B1 | ❌ | 关闭 Tab 会调用 `closePreview(key)` 清理 `openPreviews`，但 `activePreview` 未重置，重开同一产物时 watch 不触发，Tab 不激活（Blocker）。 |
| B2 | ⚠️ | 组件卸载依赖 `:key` 重建销毁 DOM，无显式 `onUnmounted` 清理；`containerRef` 判空可防大部分卸载后写入，未发现重复渲染堆叠。 |
| B3 | ⚠️ | `previewKey(task,name)` 与 MainTabs 的 `preview:${task}/${name}` 反查一致；但裸 `/` 拼接在 task/name 含 `/` 时 key 不唯一；中文/空格无碍。 |
| B4 | ✅ | 前端 invoke 键名 `task`/`name` 与 Rust 形参严格一致，无 camelCase 陷阱；错误未捕获问题见 Warning W3。 |
| B5 | ✅ | `probeDocx` 在 `renderDocx` 前顺序 await，非并行；probe、render、invoke 错误均被 catch 进入 error 降级 UI。 |
| B6 | ⚠️ | package.json 用 `^` 范围 + lockfile 锁定；`.gitignore` 忽略 `gui/node_modules/`，与“vendor 入 node_modules”说法矛盾，发布构建依赖 npm registry。 |
| C1 | ✅ | `lib.rs` 的 `generate_handler![...]` 已包含 `commands::office_read_file`；`Cargo.toml` 中 `base64 = "0.22"` 明确。 |
| C2 | ✅ | Rust 返回 `Result<String, String>`（base64）；前端 `invoke<string>` 后 `base64ToBytes` 用 `atob` → `Uint8Array`，匹配且可正确处理二进制。 |
| C3 | ❌ | 关闭流程调用 `closePreview` 清理注册表 ✅；去重 push 逻辑正确 ✅；但 `activePreview` 残留导致重开不激活，与 B1 同根。 |
| D1 | Note | `gui/spike-docx/` 未跟踪目录需处置；docs 中文文件名在 Windows/CI 有兼容性风险；均不阻塞当前提交。 |
