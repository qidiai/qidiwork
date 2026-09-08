# QidiWork GUI 办公界面方案 v2

> 版本:v2(2026-09-07)
> 变更:吸收 k3 模型审计结论 + 拓扑修正(单进程多会话)+ 代码级事实核查。
> v1 及审计过程见本文末尾"与 v1 的差异"。

---

## 一、目标

为 QIDI Code 构建原生 GUI 办公界面(QidiWork Workbench):目标用户是不熟命令行的办公人员(标书、周报、文档处理),界面以"任务工作区 + 交付物"为中心,对标豆包侧边工作台 / WorkBuddy。TUI 保留并存,作为开发者模式。

非目标:重写 agent 内核;迁移 TUI 界面代码;改动办公技能生态。

## 二、已验证的代码事实(方案地基)

以下事实全部在源码中核实过,是本方案的前提,不再作为假设:

| # | 事实 | 证据 |
|---|---|---|
| F1 | **一个 qidi.exe 进程原生支持多个会话**:agent 实例拥有唯一 client 连接,下挂 N 个 session actor,各自独立的聊天历史/工具上下文/MCP 状态/持久化通道 | `cf-shell/src/session/acp_session.rs:5-11`;TUI 即此用法(`cf-pager/src/acp/mod.rs:193` 仅 spawn 一次,`AppView.acp_tx` 全 tab 共享) |
| F2 | **`qidi agent --stdio` 独立 ACP agent 进程模式已存在** | `cf-pager-bin/src/main.rs:1171` → `run_stdio_agent`(`cf-shell/src/agent/app.rs:289`) |
| F3 | **cf-acp-lib 双向可用**:side 泛型设计,ClientSide/AgentSide 消息类型齐备;上游已有桌面客户端先例(grok-desktop)驱动同一 stdio 协议 | `cf-acp-lib/src/message.rs:29-38`;`stdin_reader.rs:3` |
| F4 | **权限 100% 经 ACP 透出**:`RequestPermission` 是 client 消息,TUI 自己就是 ACP 消息驱动的权限 UI | `cf-pager/src/app/acp_handler/mod.rs:579`、`permissions.rs` |
| F5 | **`session/load` 存在且带断线游标重放**(`_meta.cursor` + 历史通知重放) | `cf-pager/src/acp/meta.rs:30-64`、`leader_bridge.rs` |
| F6 | **会话崩溃恢复机制已有**:`active_sessions.json` 登记 session_id+pid+cwd,`.tmp` 原子写,崩溃后 `collect_crashed` 清孤儿;会话历史持续落盘 `~/.qidi/sessions/` | `cf-shell/src/active_sessions.rs`;`session/chat_persistence.rs` |
| F7 | **Windows Job Object 有现成实现与测试**(kill-on-close 进程树回收) | `cf-sandbox/src/lib.rs`、`cf-sandbox/tests/windows_job_object.rs`;`cf-mermaid/src/subprocess.rs` 亦用同模式 |
| F8 | **leader 模式跨平台传输为 named pipe**(Win)/UDS(Unix),非 WebSocket;TUI 在用,生产级 | `cf-shell/src/leader/transport.rs:1-52` |
| F9 | **办公数据面已运行**:任务工作区 `~/.qidi/office-workspaces/<task>/manifest.json` 由 card.py 写入;card.py 当前为**非原子写**(直写 `json.dump`) | `~/.qidi/skills/office-artifact/scripts/card.py:55`;`cf-pager/src/app/office_watch.rs` |
| F10 | 配置写盘有 toml_edit 先例;技能清单可直接解析 `~/.qidi/skills/*/SKILL.md` frontmatter | `cf-pager/src/config_toml_edit.rs` |

## 三、架构决策(ADR)

### D1 框架:Tauri v2 + Vue 3 + TypeScript
文档呈现(docx/xlsx/PDF/图片)是核心负载,webview 是唯一务实路线;iced/egui 需自研富文本排版引擎,否决。Tauri v2 = Rust 后端 + 系统 WebView2,自带 NSIS 打包与 updater。前端选 Vue 3(团队上手成本最低)。
**被否决的替代**:Electron(体积与内存翻倍,无 Rust 侧复用优势);直接演进 office-workbench(Python HTTP 无法做权限弹窗、无安装分发、进程管理弱——退役决定见 D6)。

### D2 拓扑:单 stdio 连接 + 多 session actor(本版核心修正)
Tauri Rust 后端 spawn **一个** `qidi agent --stdio` 子进程,多任务工作区 = 同一连接多次 `session/new`(每会话用 ACP 自带的 cwd 参数做目录隔离)。与 TUI 拓扑同构(F1),资源 = 1 个进程树 + N 个轻量 actor。
- **否决"每工作区一个子进程"**:k3 审计指出的资源失控(每进程再 spawn MCP + Python 孙进程)成立,但结论不是换 leader,而是根本不需要多进程。
- **leader 模式降为 P2+ 可选增强**:唯一剩余价值是"GUI 崩溃后 agent 存活跑完长任务"(named pipe 传输,F8);多会话与断线恢复已由 F1+F5 覆盖,不为它付 P0 复杂度。
- **桥接层仍定义 `Transport` 抽象**(stdio 为第一实现),为 leader/远程沙箱(`cf-shell/src/remote/`)留接口,成本以天计。
- **失败域诚实记录**:单进程崩溃 = 所有会话断开。三层兜底使实际损失收敛为"进行中的 turn 重发":历史持续落盘(F6)+ `active_sessions.json` 崩溃登记(F6)+ 重 spawn 后 `session/load` 游标重放(F5)。GUI 检测进程退出 → 自动重 spawn + 逐会话 load → UI 明示"会话已恢复"。

### D3 数据契约:零改动 + GUI 对 manifest 只读
manifest.json schema、card.py、office-artifact / office-tools / office-canvas / bid-* 技能全部不动。GUI 对 manifest **只读**(读 + fsnotify 监听),一切写操作经 agent/card.py 完成——消除双写冲突(k3 P0,card.py 直写已核实,F9)。
**配套小项(P1)**:card.py 改原子写(临时文件 + `os.replace`,十行内),保护 TUI/GUI 之外的第三方读取方。

### D4 安全基线(P0 落地,不是增强)
agent 产出的文档来自互联网/标书输入/模型输出,预览渲染按不可信输入处理:
1. 文档预览渲染进 `<iframe sandbox>`(或独立 webview 窗口,P0 定型二选一),与主窗口 DOM 隔离;
2. `tauri.conf.json` 严格 CSP:`default-src 'self'`,禁远程源;
3. docx→HTML 结果经 DOMPurify 消毒;禁用 WebView2 任意导航与 window.open;
4. Tauri IPC 白名单:fs 命令 canonicalize 后必须落在 `~/.qidi/office-workspaces/` 内(沿用项目 `confine_fs` 模式);"系统打开"只接受路径参数 + 扩展名白名单,不向前端暴露通用 exec;
5. 子进程管理:spawn 即挂 Windows Job Object(KILL_ON_JOB_CLOSE,照抄 F7 现成代码),GUI 死亡整树回收。

### D5 批注路线:GUI 批注 = 给 agent 的指令载体
GUI 预览内划选文字→写批注→生成结构化整改清单发给 agent;UI 明示"批注不写入文档"。office-canvas 的 WPS 原生批注路线保留不动,二者语义不冲突。回写 `w:comment`(OOXML)作为后续可选增强(office-tools 已有 OOXML 修补能力),不在本期承诺。

### D6 office-workbench 退役:冻结 → 双轨 → 删除
理由(显式记录):Python HTTP 版无法实现权限审批弹窗、无安装分发与自动更新、进程管理弱、双前端分裂体验。步骤:P2 冻结新功能 → 与 GUI 双轨一个版本并更新《操作手册》→ 确认无引用后删除。

### D7 TUI 并存
TUI 不迁移、不冻结,继续作为开发者模式。GUI 是新增呈现层,二者共享 agent 内核与 `~/.qidi` 数据面。注意:GUI 独占它 spawn 的 agent 进程的 stdio,与 TUI 的 in-process 模式天然不冲突。

## 四、系统架构

```
┌───────────────────────────────────────────────────────┐
│ QidiWork GUI (Tauri v2)                               │
│ ┌───────────────────────────────────────────────────┐ │
│ │ 前端 Vue3:三区布局 / 对话 / 预览 Tab(sandbox) / 卡片│ │
│ └──────────────────┬────────────────────────────────┘ │
│ │ Tauri IPC(白名单命令 + event)                      │
│ ┌──────────────────┴────────────────────────────────┐ │
│ │ Rust 后端 (crates/gui/src-tauri)                   │ │
│ │ · Transport trait → StdioTransport(第一实现)       │ │
│ │ · ACP client 桥(cf-acp-lib,照 spawn.rs 模式)      │ │
│ │ · 多 session 管理(session/new per 工作区)          │ │
│ │ · manifest watch(cf-fsnotify)→ event             │ │
│ │ · Job Object 进程树管理 · 系统打开(白名单)          │ │
│ └──────────────────┬────────────────────────────────┘ │
└─────────────────────┼─────────────────────────────────┘
            ACP JSON-RPC over stdio(单进程)
┌─────────────────────┴─────────────────────────────────┐
│ qidi.exe agent --stdio(F2,零改动)                    │
│ 1 client connection → N session actor(F1)            │
│ cf-shell 运行时 + cf-tools + cf-sampler + MCP + 技能   │
└───────────────────────────────────────────────────────┘
          数据面(已存在):~/.qidi/office-workspaces/(GUI 只读)
                        ~/.qidi/sessions/(崩溃恢复)
```

## 五、界面设计

```
┌──────────────────────────────────────────────────────────────┐
│ QIDI 办公工作台          任务: [投标A ▾]          ─  □  ×    │
├───────────┬──────────────────────────────┬───────────────────┤
│ 任务工作区 │   中间区(多 Tab)              │  产物面板          │
│───────────│──────────────────────────────│───────────────────│
│ + 新建任务 │ ┌─ 对话 ─┬─ 投标函.docx ─┐    │ 📄 投标函.docx     │
│ ● 投标A   │ │ 用户: 按大纲生成投标函        │ │   38KB 11:02      │
│ ○ 周报汇总 │ │ QIDI: 已生成 5 章…           │ │   [预览] [打开]    │
│ ○ default │ │ ⚙ 调用 office-tools…         │ │ 🖼 平面布置图.png  │
│───────────│ │ ✓ 产物已登记 2 份 → 卡片已推送 │ │   [预览] [打开]    │
│ 常用技能   │ └─────────────────────────────┘ │───────────────────│
│ [标书][周报]│ ┌─────────────────────────────┐ │ 点卡片 → 中间区    │
│ [图纸][审阅]│ │ 输入任务…  (Ctrl+Enter 发送) │ │ 开预览 Tab         │
└───────────┴─┴─────────────────────────────┴─┴───────────────────┘
```

- 左区:工作区列表(= session,切换即切换上下文)+ 常用技能按钮(解析 SKILL.md,免记命令);
- 中区:对话 Tab + 文档预览 Tab(点产物卡片开新 Tab,预览运行在 sandbox 内);
- 右区:产物卡片(manifest 直映:名称/大小/时间/来源技能),fsnotify 实时刷新;
- 全中文、大字号、少快捷键;权限审批用对话框(选项来自 ACP RequestPermission 的 options,不自造语义)。

## 六、复用清单(v2 修订)

| 资产 | 处置 | 依据 |
|---|---|---|
| cf-shell / cf-tools / cf-sampler / cf-models / MCP | 直接复用(stdio 子进程) | F2 |
| **cf-acp-lib(client 侧)** | **直接复用** | F3 + grok-desktop 先例(v1 此处为"待验证",已证实) |
| `session/load` + 游标重放 + `active_sessions.json` | 直接复用(P2 会话恢复 = 接线) | F5 F6(v1 为"存疑",已证实) |
| cf-sandbox 的 Job Object | 抄用(进程树管理) | F7 |
| office-workspaces 契约 + card.py | 直接复用;card.py 加原子写 | F9 |
| office-artifact / office-tools / office-canvas / bid-* 技能 | 直接复用 | — |
| 配置写盘 / 技能发现 | 照 config_toml_edit.rs / SKILL.md frontmatter | F10 |
| cf-office-preview | 降级兜底保留(纯文本预览) | — |
| office-workbench | 吸收布局与预览思路后退役(D6) | — |
| cf-pager(TUI) | 保留并存,不迁移 | D7 |

## 七、分阶段计划(总计约 11 周)

### 前置行政项(立即启动,与 P0 并行)
- Authenticode 代码签名证书采购(行政周期以周计;无签名 = SmartScreen 红屏,对目标用户等于安装失败)。

### P0 地基(2.5 周)
**第 1 周**:
- Tauri v2 工程(`crates/gui/`:`src-tauri/` + Vue3 前端)+ 三区静态布局;
- ACP 桥:spawn stdio 子进程 + Job Object 挂树(抄 F7)+ Transport trait;cf-acp-lib client 接入,`session/new` / `session/prompt` / 流式 `session/update` 转 Tauri event(照 `spawn.rs` 与 `leader_bridge.rs` 模式,半天冒烟验证即可——非 v1 的"二选一 spike");
- **docx 保真度 spike(唯一保留的 spike)**:3-5 份真实 bid 产物跑 docx-preview.js,定义可接受标准;不达标格式降级"系统打开";评估 LibreOffice headless → PDF 高保真备选(office-tools 依赖清单已含 LibreOffice)。
**第 1.5–2.5 周**:
- 权限审批对话框(`RequestPermission` client 消息处理,交互语义对齐 `acp_handler/permissions.rs`);
- D4 安全基线全部落地(CSP / sandbox / 消毒 / IPC 白名单);
- 前端对话流:流式 Markdown、工具调用折叠条目、agent 崩溃检测 + 自动重 spawn + `session/load` 恢复。

**验收**:GUI 发自然语言任务,流式回复与工具调用可见;权限对话框可批准/拒绝;任务管理器强杀 GUI 后无孤儿 qidi 进程;杀 agent 进程后 UI 显示明确状态并恢复会话历史(不卡死、不白屏)。

### P1 办公工作台(2.5 周)
- Rust 侧:扫描 `office-workspaces` 列工作区、cf-fsnotify 监听 manifest 推 event、系统打开(扩展名白名单);
- 前端:左区工作区列表 + 新建/切换(每工作区一个 session,cwd 绑定);右区产物卡片(实时刷新);中区预览 Tab(docx 按 spike 结论选 docx-preview.js 或 LibreOffice PDF;xlsx 用 SheetJS(vendor 引入,勿用 CDN)分 sheet;PDF/图片原生内嵌);
- **技能按钮/菜单**(办公用户核心交互,从 P2 提前):解析 SKILL.md frontmatter,office-* / bid-* 可点触发;
- card.py 原子写改造(独立小项)。

**验收**:GUI 内点技能按钮发一个 bid-write 任务 → 产物卡片自动出现在右区 → 点击预览 → "打开"唤起 WPS;全程无需键盘输入命令。

### P2 任务体验(2 周)
- 会话恢复接线(`session/load` + active_sessions,已是现成机制);
- 最小设置页:模型选择 / API key(复用 toml_edit 写 `config.toml` 先例)——发布阻塞项;
- 任务完成/需审批系统通知(Tauri notification);会话标题与历史列表;
- 长对话虚拟列表(500+ 条不卡)、深浅色主题、字号;
- leader 模式评估(长任务存活性增强,可选,不承诺);
- office-workbench 冻结新功能,进入双轨。

### P3a 批注与 diff(3 周)
- GUI 内批注:D5 路线(划选→批注→结构化整改清单→agent 落地→验证响应);
- 产物基线 diff(docx 先转文本再 diff,转换管线新建);
- 与 office-canvas 的联动语义对齐(设计任务,预留时间)。

### P3b 打包分发(1 周)
- NSIS 安装包 + qidi.exe sidecar(外置,不内嵌)+ updater;
- sidecar 哈希/签名校验;ACP `initialize` 协议版本协商,不兼容给明确提示;
- WebView2:检测缺失装 bootstrapper;企业策略受限环境提供 fixed-version 打包选项(+约 40MB)或 TUI 兜底文档。

### P4 试用与收尾(3 周)
- 中文化全面校对、崩溃报告对接 cf-crash-handler、TUI/GUI 并存文档;
- **2 周真实办公用户试用 + 修复**(办公软件的 bug 大半在试用后暴露);
- 试用确认后删除 office-workbench。

## 八、风险登记簿

| 风险 | 等级 | 对策 | 状态 |
|---|---|---|---|
| docx 预览保真度不达办公标准 | 高 | P0 spike 前置;不达标格式降级"系统打开";LibreOffice PDF 备选 | 待 spike |
| 单进程失败域集中(agent 崩溃全断) | 中 | F5/F6 三层兜底,损失收敛为进行中 turn 重发;leader 增强可选 | 已有兜底 |
| WebView2 被企业策略禁用/锁版本 | 中 | fixed-version 打包或 TUI 兜底 | 已列入 P3b |
| 杀软误报(NSIS+sidecar+spawn+文件监控画像) | 中 | 签名缓解 + 厂商白名单申诉流程 | 已签名证书前置 |
| 依赖供应链(SheetJS 已撤出 npm) | 中 | 全部依赖 vendor 进仓库,离线可构建 | 已列入 P1 |
| `~/.qidi/skills` 用户态文件与 GUI 版本漂移 | 低 | 版本检查 + 引导更新 | P4 |
| GUI↔agent 协议版本不兼容 | 低 | initialize 协商 + 明确提示 | 已列入 P3b |
| 前端工程能力 | 低 | Vue3 + 组件规范先行;P0 骨架定型后为业务组件堆叠 | — |

## 九、与 v1 的差异(审计吸收记录)

| 项 | v1 | v2 | 原因 |
|---|---|---|---|
| 默认拓扑 | 每工作区一个 stdio 子进程(P2 再议 leader) | **单连接多 session actor** | 用户指正 + F1 代码证实;资源最优且与 TUI 同构 |
| leader 模式 | 未涉及 → 审计建议作默认 | P2+ 可选增强 | 多会话/恢复已被原生机制覆盖,仅存"GUI 崩溃后任务存活"价值 |
| cf-acp-lib 复用 | 待 spike 二选一 | 直接复用 | F3 证实(grok-desktop 先例) |
| session/load | 存疑,P2 可能悬空 | 直接复用 | F5 证实(游标重放) |
| 权限透出 | "待盘点是否 100% ACP" | 已证实,保留边角盘点 | F4 |
| Job Object | 审计新增 | 采纳,降为一两天工程活 | F7 现成实现+测试 |
| 安全基线 | 未提 | P0 全量落地(D4) | k3 P0 成立 |
| manifest 写权 | "GUI 直接读写" | GUI 只读 + card.py 原子写 | k3 P0 成立(F9 核实直写) |
| 批注 | "吸收 office-canvas 语义"(含糊) | 拍板:指令载体,不回写文档(D5) | k3 指出的双批注系统冲突 |
| 技能按钮 | P2 未提 | 提前至 P1 | 办公用户核心交互 |
| 设置页 | 未提 | P2,发布阻塞项 | k3 P1 成立 |
| 签名证书 | 未提 | 前置行政项,立即启动 | SmartScreen 拦截 |
| 周期 | 10 周 | **11 周**(P0 2.5 + P1 2.5 + P2 2 + P3a 3 + P3b 1,试用并入 P4) | k3 建议的 12 周中,spike 缩水与现成机制消解约 1 周 |
