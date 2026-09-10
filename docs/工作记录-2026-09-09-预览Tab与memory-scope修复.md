# 工作记录 2026-09-09:memory scope 修复 + docx 预览 Tab

> 会话:AtomCode(glm5.3-flash)。本记录覆盖当天两轮工作:①审计后修复 W1/W2;②P1 关键件"中区 docx 预览 Tab"实现。文末含未完成任务清单与架构图。

## 一、做了什么(总览)

| # | 事项 | 产出 | 状态 |
|---|---|---|---|
| 1 | 处理遗留未提交变更 | commit `d7e90c9` | ✅ 已提交 |
| 2 | docx 预览保真度 spike(方案头号风险项) | `gui/spike-docx/` 脚本+结论 | ✅ 已完成 |
| 3 | 子代理审计 `d7e90c9` | 审计报告(无 blocker,2W+4N) | ✅ 已完成 |
| 4 | 修复审计 W1(scope 参与 PartialEq) | commit `237fac6` | ✅ 已提交 |
| 5 | 修复审计 W2(MEMORY_SCOPE.md 过期) | commit `237fac6` | ✅ 已提交 |
| 6 | 中区预览 Tab(P1 关键件) | 6 文件新增/修改,**未提交** | ✅ 实现+静态验证 |
| 7 | deepseek 交叉审计预览 Tab 变更 | 审计报告+3 处修复,见 §六 | ✅ 2026-09-10 补做(直连 222 池) |

## 二、改了哪些文件、为什么

### 2.1 commit `d7e90c9`(遗留变更处置)

| 文件 | 改动 | 为什么 |
|---|---|---|
| `crates/codegen/cf-shell/src/config/mod.rs` | `MemoryConfig` 增加 `scope` 字段+警告 | `[memory] scope` 此前被 serde 静默吞掉,用户写了不生效还不知道;改为显式警告"未实现,已忽略" |
| `.grok/config.toml`(删除) | 删文件 | grok 时代遗留,api_key 为空无泄密,功能已被 `.qidi/` 取代 |
| `.gitignore` | 追加 `.qidi/` | 运行时本地配置目录(可能含 API key),不入库 |

### 2.2 commit `237fac6`(审计修复)

| 文件 | 改动 | 为什么 |
|---|---|---|
| `config/mod.rs` | **移除** `scope` 字段;警告改为查原始 `toml::Value` | W1:`scope` 参与 `PartialEq` 会让热重载把"只改 scope 值"误判为 memory 配置变更,触发无意义 storage 重建+重复警告。字段不存,从源头消除 |
| `MEMORY_SCOPE.md` | 第 12 行改为"解析时警告但忽略" | W2:行为已从"静默忽略"变为"警告但忽略",文档同步 |

### 2.3 预览 Tab 变更(**未提交**,等 deepseek 审计后提交)

| 文件 | 新增/改 | 为什么 |
|---|---|---|
| `gui/src/services/docxPreview.ts` | 新增 | spike 结论落地:docx-preview.js 渲染 + JSZip 数 `<w:drawing>`/`<a:blip>` 探测矢量图形;`useBase64URL` 使图片走 data: URI,契合 CSP `img-src 'self' data:` |
| `gui/src/components/DocxPreview.vue` | 新增 | 预览 Tab 主体:加载→探测→渲染;**矢量图形文档不再渲染空白页**,显示说明+"用系统程序打开"按钮;加载失败同样降级 |
| `gui/src/composables/usePreview.ts` | 新增 | 预览 Tab 注册表(开/关/激活),key 规则 `previewKey(task,name)` 供 MainTabs 反查 |
| `gui/src/components/MainTabs.vue` | 改 | 动态预览 Tab 接入:注册表↔Tab 列表双向同步;关 Tab 同步清注册表(否则重开同一产物被去重跳过);重复点同一产物=激活已有 Tab |
| `gui/src/components/ArtifactPanel.vue` | 改 | 「预览」按钮从 disabled 接通 `openPreview()`;更新头部注释 |
| `gui/src/composables/useOffice.ts` | 改 | **顺带修真 bug**:`openArtifact` 原传 `{path}`,但 Rust `office_open` 签名是 `(task,name)`——「打开」按钮此前必报错;改为按 task+name 调用(走服务端 manifest 重读校验,k3 P1a 审计 P1 的设计意图) |
| `gui/src-tauri/src/commands.rs` | 改 | 新增 `office_read_file` 命令:manifest 已登记→扩展名白名单→canonicalize 根约束→32MB 上限→base64 返回。与 `office_open` 同一道校验链 |
| `gui/src-tauri/src/lib.rs` | 改 | 注册新命令 |
| `gui/src-tauri/Cargo.toml` | 改 | 加 `base64 = "0.22"` |
| `gui/package.json` | 改 | 加 `docx-preview`、`jszip` 依赖(vendor 入 node_modules,符合方案"勿用 CDN") |

**为什么矢量图形要降级而不是硬渲染**:spike 实测(5 份真实标书),附表五施工总平面图 49 个 DrawingML 矢量形状、0 个位图,docx-preview.js 完全丢弃这类内容,渲染结果空白——这是库的能力边界,配置调不好。降级路径恰好符合方案 v2 预设的"不达标格式降级系统打开"。LibreOffice PDF 备选路线**正式放弃**(需装 300MB 外部软件,违背自主可控原则;WPS COM 转 PDF 记为 P2 后可选增强)。

### 2.4 验证记录

- `cargo check -p cf-shell`(主工程,3m26s)✅
- `cargo check`(gui/src-tauri,34s)✅
- `npx vue-tsc --noEmit`(gui)✅
- **未做**:Tauri 窗口端到端实测(点真实标书产物走一遍预览/降级/打开)

## 三、spike 结论(留档)

| 样本 | XML 事实 | docx-preview 渲染 | 判定 |
|---|---|---|---|
| 施工组织设计技术暗标(1.1MB) | 252 表格/2.2万段 | 252 表格全渲染 | ✅ 通过 |
| 暗标副本(2)(994KB) | 253 表格/49 图形 | 253 表格全渲染 | ✅ 通过 |
| 修改意见(609KB) | 9 张嵌入位图 | 9 张全渲染 | ✅ 通过 |
| 附表四横道图 | 1 表格 | 1 表格 287 单元格 | ✅ 通过 |
| 附表五施工总平面图 | 49 矢量图形/0 位图 | **0 个渲染** | ❌ 降级系统打开 |

判定规则:`<w:drawing>` 数 > `<a:blip>` 数 ⇒ 存在矢量图形 ⇒ 降级。

## 四、未完成任务(按方案 v2 P1→P4)

### P1 剩余
- [ ] **技能按钮/菜单**(办公核心交互):解析 SKILL.md frontmatter,office-*/bid-* 可点触发
- [ ] card.py 原子写改造(独立小项)
- [ ] xlsx/PDF 预览(方案 P1 清单内,SheetJS vendor 引入;当前预览 Tab 仅 docx)
- [ ] 预览 Tab 端到端实测(Tauri 窗口点真实产物)

### P2 任务体验
- [ ] 会话恢复接线(`session/load` + active_sessions)
- [ ] 最小设置页(模型选择/API key,复用 toml_edit 先例)——**发布阻塞项**
- [ ] 系统通知、会话标题/历史、长对话虚拟列表、深浅色主题、字号
- [ ] (可选)WPS COM 转 PDF 增强:矢量图形文档"预览级"呈现

### P3a 批注与 diff → P3b 打包分发 → P4 试用收尾
- 全部未开始(批注、NSIS 安装包+签名、WebView2 兜底、中文化、2 周真实试用)

### 工程债
- [x] 预览 Tab 变更提交(等 deepseek 审计)+ spike 目录处置(已 ignore `gui/spike-docx/`)——2026-09-10 完成,见 §六
- [ ] AtomGit 推送:本地压着 7 个 commit 未推
- [ ] CI 修复(protoc)后从未实际跑过 runner(GitHub 推不上去,CI 实际无效)

## 五、架构图

### 5.1 预览 Tab 数据流(本次新增部分)

```
┌──────────── 右区 ArtifactPanel ────────────┐
│ 产物卡片 [预览] [打开]                      │
└──────┬─────────────────────┬──────────────┘
       │ openPreview()        │ openArtifact()
       │ (usePreview)         │ (useOffice)
       ▼                      ▼
┌─ 中区 MainTabs ─┐    Tauri IPC invoke
│ Tab: 对话|函.docx│   ┌────────────────────────┐
│  ┌────────────┐ │   │ office_open(task,name) │
│  │DocxPreview │ │   │  manifest 校验(登记表)  │
│  └─────┬──────┘ │   │  扩展名白名单           │
│        │ invoke  │   │  canonicalize 根约束   │
│        ▼         │   └──────────┬─────────────┘
│  office_read_file│              ▼
│  (32MB 上限,     │   tauri-plugin-opener
│   base64 返回)   │   → WPS/系统默认程序
└───────┬─────────┘
        ▼
┌─ 前端 services/docxPreview.ts ─┐
│ 1. base64 → Uint8Array         │
│ 2. JSZip 解包数 drawing/blip   │
│    vectorGraphics? ──是──→ 降级提示+[用系统程序打开]
│    否 ↓                        │
│ 3. docx-preview renderAsync    │
│    (useBase64URL→CSP data:)    │
└────────────────────────────────┘
```

### 5.2 GUI 三区结构(P1 现状)

```
┌─────────────────────────────────────────────────────┐
│ QIDI 办公工作台 (Tauri v2 + Vue3)                    │
├───────────┬──────────────────────────┬──────────────┤
│ 左区       │ 中区 MainTabs            │ 右区          │
│ Workspace │  ┌────────┬───────────┐  │ ArtifactPanel│
│ Sidebar   │  │ 对话    │ 函.docx ✕│  │ 产物卡片列表  │
│ 工作区列表 │  │ ChatView│DocxPreview│  │ [预览][打开]  │
│ (office_  │  └────────┴───────────┘  │              │
│  scan)    │  ACP 桥(stdio 子进程)    │ manifest 监听 │
│           │  Job Object 挂树防孤儿   │ (office-event)│
├───────────┴──────────────────────────┴──────────────┤
│ StatusBar                                           │
└─────────────────────────────────────────────────────┘
        数据源: ~/.qidi/office-workspaces/<task>/manifest.json
        (card.py 写,GUI 只读 —— 方案 D3)
```

### 5.3 office 数据面命令层(Rust)

```
commands.rs (IPC 面)
├─ office_scan        → office::list_workspaces     左区工作区
├─ office_artifacts   → office::read_manifest       右区卡片
├─ office_open        → read_manifest+open_allowed  系统打开
├─ office_read_file   → read_manifest+open_allowed  预览字节(新)
│        + 32MB 上限 + base64
└─ office_watch_start → ManifestWatch(fsnotify)     office-event 推送
        共同校验链(office::open_allowed):
        manifest 已登记 → 扩展名白名单 → canonicalize 根约束
```

## 六、2026-09-10 补记:deepseek 交叉审计与修复(Codely 会话)

ai-bridge MCP 连不上(其入口 mcp-server.js 已不存在),改由 Codely 直连
`http://192.168.1.222:8765` 模型池的 deepseek 路由(codely-flash)完成本审计:
41k 字符变更材料 → 审计报告(usage 33.5k tokens),留档
`docs/审计-2026-09-10-预览Tab-deepseek交叉审计.md`。

**结论:GO with fixes**(1 Blocker + 6 Warning + 6 Note)。审计后当场修复:

| 级别 | 项 | 修复 |
|---|---|---|
| Blocker B1/C3 | `closePreview` 未清 `activePreview` 残留 → 重开同一产物 Tab 重新出现但不激活(Vue ref 同值不触发 watch) | `closePreview` 清残留;`MainTabs` 手动点 Tab 改走 `selectTab` 同步 `activePreview` |
| Warning W1 | `office_read_file` 32MB 预检与读取间存在 TOCTOU 窗口 | 改 `File::open`+`take(MAX+1)` 限制读取量,读后再验 |
| Warning W3 | `DocxPreview`「用系统程序打开」无 catch,office_open reject 时无反馈 | catch → `pushSystem`(与 ArtifactPanel 同模式) |

**审计但未改(有意保留)**:
- W2 生产 CSP `style-src 'self'` 或拦 docx-preview 内联样式 → 并入"预览 Tab 端到端实测"项,须生产构建验证
- W4 `open_allowed` 路径比较疑虑 → 已人工核实:`Path::starts_with` 为**组件级**比较,无 `G:\foo` vs `G:\foobar` 前缀陷阱,且有单测覆盖,安全
- W5「vendor 入 node_modules」说法与 .gitignore 矛盾 → 实为 lockfile 锁定 + npm ci 策略(非真 vendor);离线构建需求出现时再改
- W6 async 命令内同步 IO(最大 32MB) → 后续 `spawn_blocking`
- N1 props watch 冗余 / N2 `previewKey` 裸 `/` 拼接(name 为文件名不含 `/`,task 为目录名) / N6 模块级注册表跨任务残留 → 低风险,记待办

验证:`cargo check`(47s)✅ `npx vue-tsc --noEmit` ✅。以上修复随本记录一并提交。

---
*记录人:AtomCode (glm5.3-flash),2026-09-09。§六补记:Codely,2026-09-10。上一次全局评审见 `docs/PROJECT_REVIEW_2026-07-25.md`。*
