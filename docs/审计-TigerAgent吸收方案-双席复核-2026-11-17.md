# 双席复核：TigerAgent 吸收方案（step-5 × go-deepseek）

**日期**：2026-11-17
**对象**：`docs/TigerAgent-吸收方案-2026-11-16.md`（v2，364 行，原起草：乐团 T1）
**方法**：两席独立分析（架构/风险席 step-5；实现/代码现实席 go-deepseek），指挥官对决定性主张逐条实测复核后裁决。
**结论**：**方案方向成立，但事实层需修正 13 处、优先级需重排；真正值得立刻做的是 3 件，其中 2 件与远程链路无关。**

---

## 一、裁决表

| 项 | 两席分歧 | 指挥官裁决（含实测） |
|---|---|---|
| **P0-1 SPKI pin** | 架构席"先探针"；实现席"可立即" | **可行，但先做半天 spike，不进产品码**。实测 `reqwest-0.12.28/src/async_impl/client.rs:2158` 确有 `use_preconfigured_tls`；`rustls 0.23.45 / tokio-rustls 0.26.5 / sha2 / base64` 均已在 `Cargo.lock`，**真新增只有 `x509-parser`**。但 `x509-parser` 本机缓存缺失 → 离线/内网构建会断，需先确认镜像或改"整证书 DER pin"（实现席建议，零解析） |
| **P0-2 SSE 事件总线 + run_id** | 架构席"暂缓"；实现席"先做本地 adapter" | **暂缓**。实测 `run_id` 在 Rust 侧 **0 命中**（是新概念，不是"上移"）；落点也不该是 `transport.rs`（只搬行），正确 seam 是 `bridge.rs:158` 的 `event_tx`。无第二事件源前抽 dual-mode = 投机抽象 |
| **P0-3 远程审批** | 架构席"立即（仅本地半项）"；实现席"结构已存在，只吸收超时" | **吸收一条：per-request 超时 → deny**。实测 `bridge.rs` 已有 pending 表（:158）、resolve（:626）、cancel（:648）、expire（:666）与前端弹窗；**缺 per-request deadline**。现状只有全局 `PROMPT_TIMEOUT = 30min`（`bridge.rs:31`）且超时≠取消 → 用户忽略弹窗最长挂 30 分钟，是真伤 |
| **P1-4 模型热更新** | 两席均发现方案写错 | **从方案移除**。实测已实现：`commands.rs:474 session_set_model` → `bridge.rs:1115 set_model`（注释原文"不重启内核"）。剩余仅"忙时排队"小增量，降 P2 |
| **P1-5 PDF 批次泵** | 架构席"先探针"；实现席"暂缓（原生依赖最重）" | **先答一个问题再定**：内核 ACP 是否已能直接读 PDF 附件？能 → GUI 泵是重复造轮子。另 `office/` 实测只有 docx 标书导出，`pdf` 仅出现在扩展名白名单（`office/mod.rs:23,30,37`） |
| **P1-6 PTY** | 两席一致 | **不吸收**。无远程终端需求，`mcpServers: []` 硬编码（`bridge.rs:813,849`），且只有客户端半边 |
| **P1-7 决策模式** | 实现席指出落点错 | **暂缓**。`transcript.rs` 是只读回放器，Rust 侧 near-zero（`SessionUpdate.update` 是裸 `Value` 透传），增量在前端 + 内核 schema |
| **P2 组（压缩/判重/撤销/锁）** | 两席一致 | **不吸收本轮**。服务端源码未取到（方案自证），引擎层归 qidicode |
| **Skills 附件化** | 架构席"不吸收"；实现席"立即（≈80–150 行）" | **不吸收**。把技能正文当指令注入上下文，与 qidiwork 现有信任边界（`skills.rs:4-6` 只读 frontmatter、不代执行）相反；现有 zip 安全约束（32 MiB/条目、zip-slip、CRC32）已够 |
| **反面教材 5 条** | 架构席只认 1 条同病；实现席未逐条 | **1 条同病、4 条只是警醒**。同病＝**无签名更新**（`updater.rs` 86 行，仅版本比较 + 返回 URL，`sha/digest/verify/download` 全零命中）——而我们刚发出未签名安装包 |

---

## 二、必须修正的事实（两席核实 + 指挥官复测）

| # | 方案原文 | 实测 |
|---|---|---|
| 1 | `AgentTransport` 四方法 | **6 个**：`send` / `take_line_receiver` / `subscribe_exit` / `is_running` / `terminate` / `shutdown`（`transport.rs:62-83`）；漏掉的两个恰是远程实现必须重定义的 |
| 2 | `TransportError` 三态 | **4 变体**，含 `Other(String)`（`transport.rs:33-41`），远程映射未定义 |
| 3 | "run_id 由前端上移" | Rust 侧 `run_id` **0 命中**；属全新概念，作用域须定义 |
| 4 | P0-2 落点 `transport.rs` 之上 | 应为 `bridge.rs:158 permission_pending` 同层的 `event_tx` 与 `commands.rs` 的 `emit("acp-event")` |
| 5 | "新增审批状态机" | 状态机/事件/命令/前端弹窗全在（`bridge.rs:558-690`），只缺超时 |
| 6 | "P1-4 缺口：不重启重建模型配置" | 已实现（`commands.rs:474` → `bridge.rs:1115`） |
| 7 | "P1-7 落 transcript.rs" | `transcript.rs` 只读回放（`:1-12`），Rust 侧无需改动 |
| 8 | "reqwest 注入 verifier 路径未验证（首日探针）" | **已验证可行**：`use_preconfigured_tls`（reqwest 0.12.28）；剩余坑是 ALPN 需自设 |
| 9 | "五件套直依赖" | 实为 **1 件**（`x509-parser`）；其余已在 lock（reqwest 传递依赖）；另 `tokio` 缺 `net` feature |
| 10 | "x509-parser 不在 lock" | 成立；且**本机 registry 缓存也没有**（只有 `x509-cert`），离线构建会断 |
| 11 | updater "无下载代码" | 成立，且更强：**连校验也没有**；`lib.rs:89` 只注册 `check_gui_update`，前端用 `plugin-opener` 打开 URL |
| 12 | `MAX_LINE_BYTES` 行号 `:19` | 实为 `:17`（轻微） |
| 13 | "permission 消费点 :349-350" | 该处是 session close 清理；落表+发事件在 `:558-600` |

---

## 三、裁决：Top-3 与顺序

| 顺序 | 事项 | 理由 | 代价 |
|---|---|---|---|
| **1** | **updater 下载 + 校验落地**（先盘下载侧：Tauri updater 插件 / 自建 GET） | 唯一正在伤害真实用户的安全债；我们刚发未签名安装包 | 取决于选型；若走插件，近配置级 |
| **2** | **permission per-request 超时 → deny** | 纯本地、复用现有入口、消除 30 分钟挂起 | 60–120 行 + 单测 |
| **3** | **P0-1 半天 spike**（pin 编码/校验 + reqwest connector + 错 pin 硬失败；不写产品码） | 决定 A 批是否可行；同时逼出"禁新增依赖"豁免流程与离线镜像问题 | 半天 |

**明确不做**：P0-2 全量、P1-6、Skills 附件化、P2 组；P1-5 先探针；P1-7 等内核 schema。

---

## 四、两席分歧的最终处置

| 分歧 | 裁决 |
|---|---|
| P0 是否倒序工程 | **是**。对端不存在时写远程链路 = 给假契约写死代码；先做本地真缺口 + spike |
| 依赖纪律 | 现有"禁新增依赖"（`skills_package.rs:3-4`）是局部约束，但新增 TLS 栈须走**显式豁免 + 离线镜像验证**，不得默认放行 |
| pin 语义 | 首版可用**整证书 DER sha256**（零解析、离线可建），SPKI 作为对齐 tiger 的后续项；两者都要"错即硬失败、无明文回退" |
| P0-2 落点 | `bridge/commands` 事件层，不是 `transport` |
| Skills | 不吸收 |

---

## 五、对方案文档的处置

- 原件**保留并提交**（作为 v2 记录），但在头部加一行指向本复核；
- 本复核落地后，方案的 P0/P1 批次表需按 §三 重排；在重排前**不按原 §5.1 派工**。
