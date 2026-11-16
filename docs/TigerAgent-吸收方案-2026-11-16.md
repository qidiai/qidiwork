> **2026-11-17 复核**：本文经 step-5 × go-deepseek 双席复核，事实层有 13 处需修正、批次优先级需重排；**在按 §五 派工前先读 `docs/审计-TigerAgent吸收方案-双席复核-2026-11-17.md`**。

# TigerAgent 机制吸收方案 —— qidiwork 落地版
> 日期 2026-11-16 · 状态 **v2（双席对抗审计后返工稿）** · 起草：乐团编码手 T1 · v1 经 qwen 席（23 攻点）+ step-5 席（14 攻点）独立审计，指挥官合并为返工单 R2，本文按单逐项修订
>
> **证据基线声明（三档，定义与立项规则见 §1.2）**：**源码实证**＝tiger 桌面客户端 Rust/JS 代码可立项；**文档级**＝仅有 tiger 仓库内 `doc/` 声称、Go 服务端未复核，不得直接立项；**未找到**＝当期克隆无实现可引，只留问题不立项。
>
> **唯一事实来源**：`G:\qidicode\target\tmp\codearts-analysis\汇编-CodeArts已产出分析内容.md:21-300`（下文简称「汇编」）。tiger 克隆根 `C:\Users\ASUS\tiger-src`（=`github.com/litai686/tiger`，349 跟踪文件 / 1 提交 / 无 `.go` / 无 submodule）；**Go 服务端在另一仓库 `litai686/TigerAgent`，本轮未取到**（`README.md:3-9`；汇编「审计结论」节）。
>
> **qidiwork 侧事实**：指挥官已核实 F1–F7，引用时逐条保留来源标注。**全文落地点路径一律写绝对路径 `G:\qidiwork\...`**（step5#9：当轮实测 `G:\qidicode` 下 `find -name transport.rs` 零命中，真实位置 `G:\qidiwork\gui\src-tauri\`；相对路径派工有找错仓库之险）。

---

## 〇、v1→v2 修订记录

> 逐攻点处置。**来源席#**：`qwen#N` = `attack-qwen.md` 第 N 攻点；`step5#N` = `attack-step5.md` 第 N 攻点。"不改"项给理由。

| 攻点(来源席#) | 处置摘要 |
|---|---|
| qwen#1 / #4 / #6 / #7 / #10 / #11 / #12 / #15 | **不改**：§2.1 证据三组行号（汇编 `:183-185`）、`transport.rs:1-15` 注释原句、§2.2 全部数字与事件族、§2.3 "只吸收结构"、§2.5 PDF 参数、§2.6 PTY 分档、§2.7 decision 参数、§三 五条反面教材行号——两席逐字比对汇编成立。 |
| qwen#2 | §2.1 加脚注：tiger 原文"TLS 1.2/1.3 握手签名仍由 rustls/ring 验证"（汇编 `:178`）与同段"只允许 TLS 1.3"（`:176`）的张力**归 tiger 侧原文**。 |
| qwen#3 + step5#6 | §2.1 改动面重写：删"rustls 侧不新增依赖"（低估），按汇编 `:188` 列五件套直依赖；风险栏加"reqwest 0.12 注入自定义 verifier 的 feature/connector 路径"为**实施首日探针**（tiger 直连 rustls 不经 reqwest）。 |
| qwen#5 | `Cargo.lock x509-parser=0` 来源标签由"F4"改**"编码手 2026-11-16 双法实测"**（F4 原文不含 x509 项）。 |
| qwen#8 / #9 / #13 | **不改**：§2.4 证据行号、PDF 原始参数、P0/P1 未混入文档级证据——均复核成立。 |
| qwen#14 | §2.8 表头重写：删"四项全部仅有 doc/ 声称"（与同表桌面源码实证列自相矛盾），改为"**服务端算法**仅有 doc/ 声称；**桌面契约**有源码可证——P2 的对象是算法，不是契约"。 |
| qwen#16 | updater 行域全文统一为 **F5 口径 `updater.rs:1-20`**（v1 §三/§6.2 写 `1-12`）。 |
| qwen#17 | **不改**：结构验收成立（六节主结构保留，v2 前置 §〇，行数见文末自测）。 |
| qwen#18 | §1.3 与 §2.8 把"判重/假完成/事实块是 qidicode 的对照补缺点"降为**"待 qidicode 侧核实"**（证据包无 qidicode 内容，跨仓断言不外流）。 |
| qwen#19 + step5#2 | §5.4 重写：不再宣称"三条反面教材各有 rg 检索式"，逐条标「rg」/「测试」/「无判据」；§5.3 批次 A 删 Go/OpenSSL 风味 rg（rustls 侧恒零命中），主门禁改"错误 pin 硬失败"单测，rg 改检 `set_certificate_verifier` 调用点 + 人工白名单。 |
| qwen#20 | §二 末补 **§2.9 未吸收机制对账表**（汇编 §6 LSP / §8 Skills / §13.1 后台作业），其中 **§8 Skills 注入单列升级为「复核候选」**（不入批次，立项留下一轮）。 |
| qwen#21 | §4.2/§4.4 引用改 `G:\qidiwork\docs\<文件名>` 实路径（三份当轮实测在位）；§4.1 远程接入方案路径未实测在位，**不补路径**，仍挂 F6。 |
| qwen#22 | §4.3"私有库"改**代码仓 `G:\qidiwork\docs\服务端-登录与模型控制方案-v1.md`**（qwen 席当轮实测在位；文档私有库另有同主题文档，勿混）；§2.4 同源引用一并改。 |
| qwen#23 | §2.2 风险栏加：`run_id` 迟到过滤语义由 tiger 前端 JS（`store.js:376-464`）**上移 Rust 事件总线**，作用域需重定义。 |
| step5#1 | §2.1 改动面补 **`AgentTransport` 契约映射四子项**；并补显式有利论据：trait 是行级而非 AsyncRead/AsyncWrite。 |
| step5#3 | §2.2 补事件源 **dual-mode**（local line-stream adapter / remote SSE adapter）+ 验收⑥"两模式同路径行为一致"。 |
| step5#4 | §2.3 验收③补 `acp/bridge.rs` 行号级映射表；§六 新增 §6.6 本地弹窗与远程 IM 审批并存的仲裁与决策回传通道。 |
| step5#5 | §6.2 前置"下载侧现状盘点"（`updater.rs` 无下载代码）。 |
| step5#6（后半） | **主动声明缺口**：该攻点后半"§六增 token 存储选型"未列入返工单 R2 条目，v2 未新增 §六 条目，保留 §2.1 风险栏"落盘权限与脱敏"原表述；若需补请下轮明确。 |
| step5#7 | §5.3 占位包名 `<tauri-crate>` 全替为实测包名 **`qidiwork-gui`**（`G:\qidiwork\gui\src-tauri\Cargo.toml` `[package].name`，version 0.1.11），并写真实 cwd。 |
| step5#8 | §2.3 加注：`permission.js:1-12` 头注释自证"审批模式已移除、弹窗与排队逻辑已删除、保留空实现"——按**"已删除的现状"**表述。 |
| step5#9 | 全文落地点改绝对路径（见文首声明）。 |
| step5#10 | §六 新增 §6.7：P0 端到端验收前置时序依赖，并明示 v2 只交单测级 + 假服务端 fixture。 |
| step5#11 | **采纳（可选项）**：§5.1 批次 E 拆 **E1（PDF 批次泵）/ E2（PTY 客户端协议）**。 |
| step5#12 / #13 / #14 | **不改**：#12/#13 判定"不成立"（依赖关系、行号抽查属实）；#14 已闭环——指挥官当轮读私有库该文档头 25 行确认在位，§4.3 按 qwen#22 口径写。 |

---

## 一、框架判断（地基）

### 1.1 同构性

| 维度 | tiger（TigerAgent） | qidiwork | 同构判断 |
|---|---|---|---|
| 客户端 | Tauri 桌面端（Rust `src-tauri/` + 前端 `src/`） | Tauri GUI（Rust `gui/src-tauri/` + 前端） | **高**：同框架、同语言、同"前端↔进程"分层 |
| 引擎连接 | 远程 HTTP/SSE + 自签 TLS（连接串 `tigerAgent://host:port?fp=...&token=...`，汇编 §9） | 本地 stdio 子进程（`G:\qidiwork\gui\src-tauri\src\transport.rs:1-15`，F3） | **中**：`AgentTransport` trait 注释已预留"leader(named pipe)或远程沙箱"新实现 |
| 服务端 | Go 服务端（仓库未取到） | 规划中（F7：服务端控模型/额度/上游，80% 工作量在服务端） | **中**：形态不同，价值在"远程客户端所 afford 的契约" |
| 产品场景 | 编码助手 | 办公工作台（F2 有 `office/` 模块） | **低**：PDF 批次泵、决策模式等可跨场景借用 |

**结论**：可直接吸收的是**产品层 / 传输层 / 客户端层**机制——tiger 在这一段有真实 Rust 源码可核（汇编 §9–§13）。引擎层（回合循环、压缩、判重、撤销）在 tiger 侧只有文档与前端终态契约，归属 qidicode（§1.3）。

### 1.2 证据等级纪律

| 档位 | 定义 | 判据 | 立项规则 |
|---|---|---|---|
| **源码实证** | tiger 桌面客户端 Rust/JS/前端源码行号可直接引用 | 汇编给出 `文件:行号` 且标注"代码证实" | 可进 P0/P1，可直接派工 |
| **文档级** | 仅 `doc/*.md` 声称，Go 服务端未复核 | 汇编标注"文档线索，未源码核实" | **不得直接立项**，只能进 P2，且必须带"待取 `litai686/TigerAgent` 服务端源码复核" |
| **未找到** | 当期克隆无该实现 | 汇编标注"未找到"/"无法确认" | 不立项，只写入 §六 开放问题 |

**纪律三条**：① 任一吸收项的证据栏若给不出 `文件:行号`，直接降档或剔除；② 文档级项被要求升级为 P0/P1 时，前置条件是先取得服务端源码，不得以"文档写得很细"代替取证；③ 本文引用 qidiwork 事实一律带 F 编号来源，不引入未核声称。

### 1.3 归属裁决

| 层 | 归属 | 本文处置 |
|---|---|---|
| 产品层（决策模式、PDF 批次泵、审批 UX、终端视图） | **qidiwork** | 进 P1 |
| 传输/客户端层（SPKI pin + Bearer、SSE 事件契约、run_id 迟到过滤、断线重连、PTY 客户端协议） | **qidiwork** | 进 P0/P1 |
| 引擎层（回合状态机与判重、假完成检测、确定性事实块、上下文压缩、Git 快照撤销、keyed 文件锁） | **qidicode** | 只留指针（§2.8 末栏 + §三），不展开设计 |

引擎层中，**判重（重复工具调用检测）、假完成检测、确定性事实块**三项是否构成本仓库（qidicode）的**对照补缺点**，**待 qidicode 侧核实**——本证据包（汇编 `:21-300`）与 F1–F7 均不含 qidicode 源码，跨仓断言不外流为既成事实（qwen#18）。若 qidicode 侧核实成立，另行立项。

---

## 二、吸收清单

> 每项六栏：机制描述 / 证据（tiger 文件:行号）/ qidiwork 落地点（**绝对路径**）/ 改动面 / 验收标准 / 风险。

### 2.1 P0-1 SPKI pin + Bearer 自签远程传输

| 栏 | 内容 |
|---|---|
| **机制描述** | 客户端只允许 TLS 1.3，固定 SNI `tigerAgent.local`，不出示客户端证书；自定义 verifier 解析 X.509、对 DER `SubjectPublicKeyInfo` 做 SHA-256，编码为 `sha256:<base64url-nopad>` 后比对，不匹配**硬失败**，不回退明文、不 TOFU 自动接受新 key；握手签名由 rustls/ring 验证（**脚注①**），CA/SAN/有效期**不是**本处身份判据；hyper 包装层对每个请求统一注入 `Authorization: Bearer`；连接串 `tigerAgent://host:port?fp=...&token=...`。 |
| **证据** | `src-tauri/src/common/agent_http.rs:63-123,175-205,250-303`；`src-tauri/src/common/server_mode.rs:51-113`；`src-tauri/src/pages/settings.rs:151-178`（汇编 §9，**源码实证**） |
| **qidiwork 落地点** | `G:\qidiwork\gui\src-tauri\src\transport.rs` 新增 `AgentTransport` 的**第二个实现**（远程传输）+ 新 `G:\qidiwork\gui\src-tauri\src\tls\` 子模块。F3 已明写"将来切换 leader(named pipe)或远程沙箱时提供新的实现即可,协议层不动"（`G:\qidiwork\gui\src-tauri\src\transport.rs:1-15`，2026-11-16 实测复核），本项正是该扩展点；同时承接 F6《QidiWork-远程接入方案-IM与语音-v1.md》链路上的 `transport.rs` 段。 |
| **改动面** | ① **直依赖五件套（汇编 `:188` 移植要点逐件，qwen#3）**：`rustls` + `tokio-rustls` + `x509-parser` + `sha2` + `base64`——**v1"rustls 侧不新增依赖（复用 reqwest 0.12 + rustls-tls）"为低估，v2 删除该判断**；`G:\qidiwork\gui\src-tauri\Cargo.lock` 当前 `x509-parser` 计数 0（**编码手 2026-11-16 双法实测**：`grep -c "x509"` = 0 且 `grep -o "x509[a-z-]*"` 输出空）。② **`AgentTransport` 契约映射四子项（step5#1）**：(a) **ExitInfo 进程退出码语义的远程重映射**——本地 `subscribe_exit` 是一次性 broadcast 的进程码（`transport.rs:62-83`），远程侧对应"运行结束事件 + 退出原因"，**不得**把 HTTP 断连当退出码；(b) **重连期间 `send` 的入队-vs-失败语义**——本地 `send` 尽力即返，远程须显式定义"未建连时入队待发"还是"立即返回错误"，二选一并写进 trait 文档与测试；(c) **64 MiB 单行上限在新实现保留**（`transport.rs:19` `MAX_LINE_BYTES`＝64 MiB），SSE 单事件超限按**协议错误**处理而非静默截断；(d) **行流断线自愈归属层**——重连只放在事件源 adapter（§2.2），`transport` 层不加隐式重试，避免双重重试叠加。③ `G:\qidiwork\gui\src-tauri\src\settings.rs` 增 fp/token 配置项与持久化。④ 新增 `tls/` 与 remote transport 约 2–3 文件。 |
| **验收标准** | ① 单测：给定 DER 证书算出的 pin 与控制端脚本输出逐字一致（`sha256:<base64url-nopad>` 格式断言）；② **错误 pin 连接硬失败**——经 `dangerous().set_certificate_verifier(...)` 注入 pin 不匹配的 fixture 证书，断言握手阶段返回 `Err`，且**无**明文回退分支（← **批次 A 主门禁**，见 §5.3）；③ 新代码 `rg -n "set_certificate_verifier"` 命中点逐处人工白名单登记（预期仅 `tls/` 下 1 处，多出一处即拒绝合入）；④ token 不出现在日志/前端 IPC 载荷（检索 transport 层日志调用点 + 前端字段）。 |
| **风险** | ① **reqwest 0.12 注入自定义 verifier 的 feature/connector 路径未验证＝实施首日探针（qwen#3 + step5#6）**：需 `rustls-tls-manual-roots` 一类 feature 或手搓 `ClientConfig`/connector，而 tiger 是**直连 rustls 不经 reqwest**（`agent_http.rs:63-70`，tiger 自身即 `impl ServerCertVerifier`），该路径 qidiwork 侧无先例；**探针不过则回落"rustls + tokio-rustls 自建 connector"，不走 reqwest**（这也是五件套里 `tokio-rustls` 必须在列的原因）。② pin 分发与轮换未定（换密钥＝改客户端配置，§6.3）。③ Bearer token 是明文凭据，落盘权限与脱敏必须做。④ rustls verifier API 跨版本稳定性。⑤ pin 只证明"连到了预期服务端"，**不**代表服务端内容可信。 |

> **脚注①（qwen#2）**：tiger 原文为"**TLS 1.2/1.3** 握手签名仍由 rustls/ring 验证"（汇编 `:178`），同段另称"只允许 TLS 1.3"（汇编 `:176`）。**该张力归 tiger 侧原文**，本文不代为消解；v1 转述时删掉了"1.2"限定语，v2 回到"由 rustls/ring 验证"并在此明示原文两说。
>
> **有利论据（step5#1，显式陈述）**：`AgentTransport` 是**行级 trait**——`send`（尽力即返）/ `take_line_receiver`（单消费者行流）/ `subscribe_exit`（一次性 broadcast）/ `terminate`（`transport.rs:62-83`），**不是** `AsyncRead` / `AsyncWrite`。因此 TLS 实现只需"read-loop + 分行器 → `mpsc`"喂给既有 receiver，**不存在字节流桥接鸿沟**；`Closed`/`Backpressure`/`Io` 三态（`transport.rs:33-41`）可由 mpsc 关闭与背压直接表达。这是本项判 P0 且改动面可控的根据。

### 2.2 P0-2 SSE 事件契约 + run_id 迟到过滤 + 断线重连

| 栏 | 内容 |
|---|---|
| **机制描述** | 事件流路径 `/v1/workspaces/{id}/events?client_id=<uuid>`；信封 `{type, payload:{type:created\|updated\|deleted, payload:<数据>}}`；事件族 `message/session/run_complete/permission_request/permission_notification/config_changed/agent_event/file/skills_event/mcp_event/lsp_event`，终端另有 `terminal_event`；建连失败每 **3 s** 重试、连续 **5 次**失败发 `agent-connection-lost`；已建立流断开则立即重连；`run_complete.text` 转发前截到 **200,000** 字符；丢失完成事件时前端每 **15 s** 查一次、最多 **4 次**；`run_id` 不匹配的旧 `run_complete` 被忽略。 |
| **证据** | 事件与重连 `src-tauri/src/pages/agent.rs:2324-2445`；前端处理 `src/views/agent/sse.js:34-135,376-398,594-625,709-722`；迟到过滤 `src/views/agent/store.js:376-464`（汇编 §10、§4，**源码实证**） |
| **qidiwork 落地点** | 远程接入事件总线：在 `G:\qidiwork\gui\src-tauri\src\transport.rs` 之上抽"事件源"抽象，Rust 侧新增 `G:\qidiwork\gui\src-tauri\src\events\` 模块（连接状态机 + 游标 + 去重），前端做事件分发。承接 F6 远程接入方案中 `Vue→Tauri IPC→AcpBridge/AgentProcess→transport.rs→引擎` 同一链路（来源：F6）。 |
| **改动面** | ① **事件源抽象必须 dual-mode（step5#3）**：`local line-stream adapter`（复用现行 stdio 行流，`transport.rs:1-5`、`:62-72`）与 `remote SSE adapter`（tiger 契约）**两个实现**，前端只认统一事件类型——不得让"本地一套、远程一套"成为两套分发真相（v1 风险栏自己反对"第二套真相"却未堵此口）。② SSE 完整 parser——**不得**照抄 tiger 简化实现（§三 第 3 条）。③ 前端加事件分发与幂等去重。④ `run_id` 需贯穿请求、事件、取消与迟到过滤。⑤ 去重键建议 `run_id+seq`。 |
| **验收标准** | ① 重连单测：伪造连续 5 次失败 → 恰好 1 次 `agent-connection-lost`，第 6 次成功建连并恢复订阅；② 迟到过滤单测：run_id=A 运行中注入 run_id=B 的 `run_complete`，状态不变；③ 截断边界测试（199,999 / 200,000 / 200,001 字符）；④ 丢失完成事件 → 15 s×4 兜底查询后落终态（可注入计时器，**不**加 sleep）；⑤ SSE parser 覆盖 CRLF、多行 `data:`、带 `event:` id 用例；⑥ **两模式同路径行为一致（step5#3）**：同一事件序列（`message`/`run_complete`/`permission_request`）分别经 local line-stream adapter 与 remote SSE adapter 注入，前端状态机终态与去重结果逐字一致。 |
| **风险** | ① 事件契约需与 `acp/bridge.rs` 既有 permission 状态语义对齐，避免第二套真相（§2.3 已给行号级映射）。② 轮询兜底与 SSE 重复投递需幂等。③ 3 s×5 在移动网络下可能过短，取值应可配。④ **`run_id` 语义搬家（qwen#23）**：tiger 的迟到过滤在前端 JS（`store.js:376-464`），"当前 run"是**单页面视图态**；上移到 Rust 事件总线后变成**多订阅者共享态**，作用域须重定义为 "workspace+session+run_id" 三元组，否则多窗口/多订阅者会互相过滤掉对方的事件。 |

### 2.3 P0-3 permission_request / permission_notification 事件结构

| 栏 | 内容 |
|---|---|
| **机制描述** | 权限相关在事件契约里是**两个独立事件类型**：`permission_request`（请求审批，带待决动作）与 `permission_notification`（状态通知）。tiger 桌面端的**行为**是反面教材（固定 `skip:true`、竞态到达的请求直接 `action:"allow"`——**但见下"证据"栏加注：该实现已删除，只剩空实现**），本项**只吸收结构**，不吸收其策略，也不把已删除代码当现存行为。 |
| **证据** | 事件族 `src/views/agent/sse.js:34-135`、`src-tauri/src/pages/agent.rs:2324-2445`；反面行为 `src/views/agent/permission.js:1-29`——其中 `:1-12` 头注释原文自证"审批模式已移除……弹窗与排队逻辑已删除，保留空实现以兼容各调用点""服务端 skip=true，不产生权限请求"（汇编 §5、§10，**源码实证**；按"**已删除的现状**"表述，step5#8） |
| **qidiwork 落地点** | 远程审批流，与 F6 远程接入方案的"权限审批"目标对齐；状态源落在 `G:\qidiwork\gui\src-tauri\src\acp\bridge.rs`（F6：含 permission 状态），新增事件契约 + 决策回传通道（仲裁见 §6.6）。 |
| **改动面** | 新增审批状态机（pending / approved / denied / timeout）+ 事件结构体 + 前端审批 UI 接线；仅吸收结构，显式**不**引入 YOLO 直通。 |
| **验收标准** | ① 无决策时超时 → 默认 **deny**（测试断言，且断言结果不是 allow）；② 端到端：`permission_request` → UI 决策 → 引擎收到 decision 的往返测试；③ 事件字段与 `acp/bridge.rs` 现有 permission 状态**一对一映射**、不回退为布尔——**行号级映射基准见下表**；④ `rg -n 'skip.*true\|action.*"allow"'` 在 `G:/qidiwork/gui/src-tauri/src` 零命中。 |
| **风险** | 超时策略直接影响可用性（过短＝误拒，过长＝挂起）；`permission_request` 与 `permission_notification` 混用会导致误授权；远程通道上"审批人是谁"未定（§6.4）；本地弹窗与远程审批并存的仲裁未定（§6.6）。 |

**验收③的行号级映射基准（step5#4；step-5 席 2026-11-16 当轮实测 `G:\qidiwork\gui\src-tauri\src\acp\bridge.rs`，全文 1607 行）**：

| 契约元素 | `acp/bridge.rs` 位置 | 映射纪律 |
|---|---|---|
| `PermissionRequest` / `PermissionCancelled` 事件定义 | `:48-64` | 新契约逐字段映射到这两者，不新增同义变体 |
| `permission_pending: HashMap<u64, PermissionEntry>` | `:142`、`:158`（两处） | 待决表即"一对一映射"的状态源；新事件须填满 `PermissionEntry` 且无残留字段 |
| permission 事件消费点 | `:349-350` | 远程事件必须经此处落状态，**不得**旁路写表 |
| 决策回传入口 `resolve_permission` / `cancel_permission` | 同文件（当轮实测**已在用**；精确行号未取） | 远程决策复用既有入口，不新造决策 API |

### 2.4 P1-4 模型热更新链路

| 栏 | 内容 |
|---|---|
| **机制描述** | 切换模型先 `POST /config/model`，再 `POST /agent/update` 重建 agent；工作区忙时置 `pendingModelReload`，待运行结束或下次发送前重试；新增 provider 后检查 `/providers` 快照，未出现仍提示"重启 Agent 后生效"。provider 字段：`type:"openai-compat"`、`base_url`、`api_key`、模型 ID、`ctx_size/context_window`、视觉与 reasoning 元数据；服务端设置只回掩码或空 key，编辑留空表示保留原 key；Rust 直连 `{base_url}/models`（接受 `{data:[{id}]}` 或数组）校验地址并取模型列表。 |
| **证据** | `src/views/agent/model.js:9-79`；`src/views/models.js:647-680`；`src-tauri/src/pages/agent.rs:1084-1171,1202-1262,1325-1369,1757-1820`（汇编 §11，**源码实证**） |
| **qidiwork 落地点** | 承接 F7（服务端控模型/额度/上游 + auto 路由；客户端已具备 OAuth2 issuer、设备码 RFC8628、PKCE 回环、模型目录 URL 下发，来源：**代码仓** `G:\qidiwork\docs\服务端-登录与模型控制方案-v1.md`）。本项补的正是 F7 客户端侧的缺口：**目录或模型变更后如何在不重启前提下重建引擎侧模型配置**。 |
| **改动面** | `G:\qidiwork\gui\src-tauri\src\settings.rs` / `G:\qidiwork\gui\src-tauri\src\commands.rs` 增配置 revision + `pendingModelReload` 状态；`config_changed` 事件接入（依赖 §2.2 事件总线）；`/models` 探测在 Rust 侧执行（token 不出 Rust）。 |
| **验收标准** | ① 忙时切模型 → `pendingModelReload=true` 且当前 run 不中断，run 结束后自动生效（测试）；② 空 `api_key` 编辑保存后原 key 仍在（断言不回写空串/掩码）；③ `/models` 探测三态测试（`{data}` 成功 / 裸数组 / 非预期 JSON 报错）；④ 前端与 IPC 载荷 `rg` 不到明文 key。 |
| **风险** | 与 F7 服务端下发的模型目录构成双源（建议定服务端目录为唯一真值源，本地 UI 只读）；重建时机选错会打断运行（必须在空闲边界）；"所有云端修改必定无重启"在 tiger 侧也不由代码保证（汇编 §11 明示），**不要**继承该宣传口径。 |

### 2.5 P1-5 PDF 批次泵

| 栏 | 内容 |
|---|---|
| **机制描述** | 单附件上限 20 MiB；图片 > 1 MiB 或最长边 > 2048 才压缩；PDF 视觉模型每批 **10 页**、文本模型每批 **5 页**并扣除其他图片配额；> 50 页需确认，最多 100 批；页图最长边 2048、scale ≤ 3、JPEG **0.85**；**单页失败隔离**；批次由 `run_id` 驱动，出错/取消/切会话即停。 |
| **证据** | `src/views/agent/pdf_batch.js:18-24,54-88,167-193`；`src/views/agent/pdf.js:18-21,103-175`（汇编 §13.2，**源码实证**） |
| **qidiwork 落地点** | 办公场景（qidiwork = Tauri 办公工作台）：PDF → 模型输入的流水线，落 `G:\qidiwork\gui\src-tauri\src\office\`（F2 已有该模块）。 |
| **改动面** | 新增 Rust 侧 pdf 批次泵；渲染用 `pdfium/poppler` 替代浏览器 pdf.js（汇编「移植要点」明示）；run_id 接入现有运行登记与取消路径。 |
| **验收标准** | ① 分批函数单测：10/5 页切批、> 50 页触发确认、100 批上限；② 坏页 fixture → 该页记 failed、其余页继续（失败隔离断言）；③ 取消/切会话 → 泵停止且无残留后台任务；④ 页图参数（最长边 2048、JPEG 0.85）在产出元数据中可验。 |
| **风险** | 引入 pdfium/poppler 原生依赖（打包体积、三平台差异）；办公文档敏感，页图落盘需清理策略；长合同页数与 100 批上限的真实匹配度需实测。 |

### 2.6 P1-6 PTY 客户端协议

| 栏 | 内容 |
|---|---|
| **机制描述** | 创建时传 `kind:"pty"`、`target_id`、初始尺寸 `120×32`；输出采用**序号游标** + SSE 增量 + 轮询去重；增量拉取 **256 KiB**、首次回填 **1 MiB**。**服务端 PTY/SSH 实现未找到**（仅客户端协议可核）。 |
| **证据** | `src/views/terminal/store.js:397-412`；`src/views/terminal/pty.js:337-420,447-495`（汇编 §13.3，客户端**源码实证**；服务端档位＝**未找到**） |
| **qidiwork 落地点** | 远程接入中的终端/命令执行视图（F6 远程指挥场景）；游标与回填协议挂在 §2.2 事件总线上。 |
| **改动面** | Rust PTY 用 `portable-pty`（汇编移植要点），SSH 用 `russh`（若确需）；游标协议 + 增量/首次回填拉取。 |
| **验收标准** | ① 游标去重单测：重叠区间只追加一次、缺口不跳过；② 1 MiB 首次回填 / 256 KiB 增量边界断言；③ 断线重连后按游标续拉，不重复不丢（模拟测试）。 |
| **风险** | 只有客户端半边 → 服务端契约需自定，故列 P1 而非 P0；终端回显含敏感信息，日志与遥测需脱敏。 |

### 2.7 P1-7 决策模式 choice / bool / score

| 栏 | 内容 |
|---|---|
| **机制描述** | 请求体带 `{"mode":"auto\|choice\|bool\|score","tool_policy":"none","structured_output":"auto"}`；决策轮不走普通对话折叠插入，而是排队成**独立 run**；前端同时支持专用 `decision` ContentPart 与旧 `<agent_decision_result>` 文本协议；choice 候选**最多 12** 个，bool 必须归一为布尔，score 默认区间 **[0,10]**，前端夹取并计算 normalized。服务端结构化输出实现**未找到**（文档级线索：独立旁路、不注册工具、一次定向重试、`response_format=json_schema` 被拒后移除该字段重试并按 `baseURL+model` 缓存能力）。 |
| **证据** | `src/views/agent/decision_mode.js:132-141,159-211,374-417`；`src/views/agent/send.js:147-168,319-329`（汇编 §12，客户端**源码实证**；服务端部分**文档级**） |
| **qidiwork 落地点** | 办公工作台的结构化选择场景（审批选项、表单字段建议、分支决策）；要求 `decision` part **贯穿存储 / REST / SSE / 展示**四层，落 `G:\qidiwork\gui\src-tauri\src\transcript.rs`（F2 已有模块）。 |
| **改动面** | 新增 decision part 类型（转写存储）+ REST 往返 + SSE 事件 + 前端渲染；服务端校验用 `jsonschema` 严格验证，不把前端 clamp 当校验。 |
| **验收标准** | ① 序列化往返测试：decision part 经 存储→REST→SSE 后字段零丢失（汇编点名的失败模式）；② 服务端校验：score=12 被拒或夹取并记录，**不依赖**前端；③ choice 候选 13 个 → 拒绝；④ bool 非布尔输入 → 归一或拒绝，行为明确。 |
| **风险** | 前端 clamp 冒充服务端校验（tiger 原型的真实缺口）；两套协议（part + 文本）并存会长期腐化，建议只留 part；与 F7 auto 路由的交互未定。 |

### 2.8 P2 组（服务端算法：文档级 · 未复核 · **不得直接立项**）

> **四项的服务端「算法实现」均仅有 tiger `doc/` 声称**（Go 服务端 `litai686/TigerAgent` 未取到，未复核，**不得直接立项**）；**桌面侧「契约」有源码可证**——P2 的对象是**算法**，不是契约（qwen#14：v1 表头"四项全部仅有 doc/ 声称"与同表桌面源码实证列自相矛盾，v2 删除该表述）。前置条件：取得服务端源码并复核算法。

| 序 | 机制 | 证据（档位） | 桌面可证部分（契约） | 归属指针 |
|---|---|---|---|---|
| P2-1 | **上下文压缩 + 确定性事实块 + `#N` recall** | `doc/agent-context-compaction-flow.md:40-64,72-96,122-141,164-190,236-249`；手动压缩 `src/views/agent/compact.js:22-34,158-205,272-279`（**服务端算法：文档级·未复核**） | 手动压缩按钮 + `summary_message_id` 变化确认 + 5 分钟 UI 兜底 | 引擎层 → qidicode；**事实块**（`[Session Facts]` 确定性抽取、非 LLM 生成）**待 qidicode 侧核实**是否为补缺点 |
| P2-2 | **回合恢复与重复工具检测 / 假完成检测** | `doc/agent-server.md:9-11`；前端终态 `src/views/agent/sse.js:512-518,589-612,778-806`；`doc/frontend-agent-conventions.md:14`（与现行代码不符）（**服务端算法：文档级·未复核**） | 空输出只提示不重试；`runStats` 仍统计但已无 `detectFakeCompletion` | 引擎层 → qidicode；**重复调用判重**与**假完成检测**均**待 qidicode 侧核实**（qwen#18） |
| P2-3 | **Git 对象快照 + anchor 撤销** | `doc/agent-parallel-sessions-file-guards.md:11-25`；`doc/agent-undo-anchor-plan.md:57-73`；`doc/agent-server.md:28,37`；桌面 `src/views/agent/undo.js:35-45,88-110,113-152`（**服务端算法：文档级·未复核**） | 撤销带 `anchor=<userMessageId>`、运行中拒绝撤销、非 Git 目录提示文件不回退 | 引擎层 → qidicode |
| P2-4 | **keyed 文件锁 + 读后写守卫 + `run_id` 并发模型** | `doc/agent-parallel-sessions-file-guards.md:27-65`；桌面 `src/views/agent/send.js:49-57,167-186`、`src/views/agent/store.js:376-464`（**服务端算法：文档级·未复核**） | 并行默认关闭、每 workspace 最多 3 个活动会话、run_id 不匹配的旧事件被忽略 | `run_id` 并发模型已被 §2.2 以源码实证方式吸收；锁实现归引擎层 → qidicode |

**P2 通用立项规则**：四项在取得服务端源码并逐项复核**算法**前，**不得**进入 P0/P1，也不得写入实施批次（§五）。

### 2.9 未吸收机制对账表（汇编 §6 / §8 / §13.1，qwen#20）

> v1 的漏洞：汇编里还有三项机制**既未进 §二 也未进 §三／§六**，清单完整性无对账。v2 逐项给处置与理由。

| # | 汇编机制 | 汇编位置 | 档位 | v2 处置 | 理由 |
|---|---|---|---|---|---|
| A-1 | **LSP 8 工具与按符号修复** | §6（汇编 `:120-136`）；`src/views/agent/lsp_fix.js:15-29,86-167` | **未找到**（8 个 LSP 工具与语言服务器客户端实现均未找到，仅 `lsp_replace_symbol`/`lsp_rename` 两个名称被桌面端当副作用工具统计） | **不吸收** | 算法与工具定义无源码可核；可证部分只剩"安装并启动语言服务器"的运维流程（1.5 s 轮询 / 客户端最长等 12 分钟 / 自动修复冷却 30 分钟、普通提示冷却 10 分钟），与办公工作台无直接收益，不满足 §1.2 立项规则 |
| A-2 | **Skills 加载与系统提示注入** | §8（汇编 `:150-166`）；`src/views/agent/skill_selector.js:54-91,125-185,274-335,378-391`；`src-tauri/src/pages/skills.rs:23-29,159-220` | 自动加载/系统提示注入**未找到**；"手动选择 → base64 → `text/markdown` 附件（名前缀 `skill:`）"**源码实证**；安装 ZIP 上限 50 MiB、请求超时 300 s **源码实证** | **升级为「复核候选」**（仍**不入批次**） | 见下方 A-2 专项 |
| A-3 | **后台作业列表 / 读尾 / 删除** | §13.1（汇编 `:267`）；`src/views/terminal/api.js:74-85`；`src/views/terminal/store.js:8-12,263-290,548-583` | 客户端契约**源码实证**（列作业、读最后 64 KiB、删除、列表 1.5 s 刷新）；`job_output` 工具本体**未找到** | **不吸收**（留指针） | 纯终端运维面；qidiwork 无远程长任务队列需求，且工具本体未找到，立项即需自造整套服务端契约（成本/收益不成立） |

**A-2 专项（复核候选，非立项——是否立项留给下一轮）**：

- **为什么它是本方案最该复核的漏项**：qidiwork 侧 `G:\qidiwork\gui\src-tauri\src\skills.rs` 与 `G:\qidiwork\gui\src-tauri\src\skills_package.rs`（F2 实测存在）与 tiger 的技能读取/打包面**直接对口**——这是全清单里唯一"两侧同层且都有代码"的机制，其余或只有客户端半边、或服务端未找到。
- **落地点（只做现状盘点，v2 不改动任何代码）**：`G:\qidiwork\gui\src-tauri\src\skills.rs`、`G:\qidiwork\gui\src-tauri\src\skills_package.rs`。
- **复核判据（四条，均可用源码/配置直接答）**：① qidiwork 是否已有明确的"内置 → 用户级 → 仓库级"优先级与覆盖规则（tiger 侧汇编移植要点点名此三档）；② 技能正文是否被当作**不可信指令数据**处理，**自动注入与显式调用是否分离**；③ 系统提示中的技能目录是否设**独立 token 预算**（避免技能清单吃掉上下文）；④ 打包/安装是否有明确上限与超时（tiger：ZIP 50 MiB / 300 s）。
- **结论归属**：**是否立项留给下一轮（指挥官裁决）**；v2 只登记候选、落地点与判据，不写入 §五 批次，也不改变 §1.3 归属裁决。

---

## 三、明确不吸收项（反面教材表）

| 序 | 反面做法 | 证据 | 本方案处置 |
|---|---|---|---|
| 1 | **YOLO 权限直通**：固定发 `skip:true`，竞态到达的 `permission_request` 也直接 `action:"allow"` | `src/views/agent/permission.js:1-29`（汇编 §5，源码实证；**该实现已删除，仅剩空实现**，见 §2.3） | 拒绝。§2.3 只吸收事件结构；超时默认 **deny**（§2.3 验收①），并以 §5.4 判据防复现 |
| 2 | **桌面更新无签名无哈希**：`update.json` 取版本与 URL、`.part + Range` 续传，装包前不校验签名/SHA-256 | `src-tauri/src/pages/index.rs:41-68,214-325`；`src-tauri/src/common/utils/download.rs:16-120`（汇编 §13.5）。对照：tiger 服务端安装脚本**有** checksums：`install-server.sh:180-203` | 拒绝照抄，但**如实标注：qidiwork GUI 现状同病**——`G:\qidiwork\gui\src-tauri\src\updater.rs:1-20`（F5 口径）实测只做版本严格比较（任何失败静默 None），下载侧无 hash/digest/verify/sha 校验。故这是**两侧共同改进点**（不是 tiger 劣势、qidiwork 优势），列入 §6.2 独立跟进，不从 tiger 借方案 |
| 3 | **SSE parser 简化实现**：只认 `\n\n` 分隔、单行 `data:`、精确 `"event: "` 前缀，不处理 CRLF / 多行 data / event id | `src-tauri/src/pages/agent.rs:2324-2445`（汇编 §10 明示"不宜原样照抄"） | 拒绝照抄。§2.2 改动面明确"自写完整 parser"，验收⑤ 列 CRLF / 多行 data / event id 用例 |
| 4 | **MCP"重启"假象**：UI 文案称需重启，但"重启"只断开重连桌面 SSE，**不重启**用户自行运行的服务端进程 | `src/views/agent/mcp_dialog.js:3-7,160-188`；`src/views/agent.js:438-450`（汇编 §7，源码实证） | 拒绝。qidiwork 若做 MCP 配置变更提示，文案与行为必须一致：要么真重建、要么明说"重连"，不得让用户以为已生效 |
| 5 | **空输出不自动重试**：`run_complete.empty_output` 只弹错误提示，`runStats` 统计未形成判定 | `src/views/agent/sse.js:512-518,589-612,778-806`（汇编 §1，源码实证） | 拒绝其终态策略。空整轮应进 §2.2 事件终态、重试预算交引擎层（qidicode）；**不作为** qidiwork 侧自动重试逻辑的实现依据 |

---

## 四、与既有规划衔接

### 4.1 《QidiWork-远程接入方案-IM与语音-v1.md》（F6）

- 该方案已定链路：`Vue→Tauri IPC→AcpBridge/AgentProcess→transport.rs→引擎`；目标含"手机 IM/语音/屏幕共享远程指挥 + 权限审批"（来源：F6）。
- 本方案三项 P0 全部嵌在该链路上，**不新增并行通道**：P0-1 是 `transport.rs` 的第二个 `AgentTransport` 实现（F3 注释已预留扩展点）；P0-2 是 transport 之上的事件总线，正对该方案"事件下行"段；P0-3 复用 `acp/bridge.rs` 既有 permission 状态作状态源（F6）。
- **缺口（该方案未定义，本方案补齐）**：200,000 字符截断与 15 s×4 兜底查询（§2.2）；3 s×5 断线判定并发 `agent-connection-lost`；`run_id` 不匹配一律忽略。
- **冲突点**：无结构冲突；本方案**不**定义 IM/语音通道本身，只定义其下面的传输与事件层。建议定稿后把 §2.2 事件契约回写该方案，v2 不擅改既有规划文件。

### 4.2 《QidiWork-GUI下一阶段规划与框架方案-v3.md》（F6）

- 文档实路径：`G:\qidiwork\docs\QidiWork-GUI下一阶段规划与框架方案-v3.md`（qwen 席当轮实测在位，qwen#21）。
- 本方案改动全部落在既有模块（F2 实测存在的清单）：`G:\qidiwork\gui\src-tauri\src\transport.rs`（P0-1、P0-2 底座）、`...\src\acp\bridge.rs`（P0-3 状态源）、`...\src\settings.rs`（P0-1 fp/token、P1-4 模型配置）、`...\src\office\`（P1-5）、`...\src\transcript.rs`（P1-7）。
- **不新造顶层目录**，与 v3 的"沿用现有模块边界"取向一致；P1-5 / P1-7 属办公产品层增量，需在 v3 功能清单里排位，本方案只给机制与验收判据。
- **不冲突点**：不改动 v3 的导航/视图分层，不动 `onboarding.rs`、`updater.rs`、`diag.rs`。

### 4.3 《服务端-登录与模型控制方案-v1》（F7）

- 文档实路径：**代码仓** `G:\qidiwork\docs\服务端-登录与模型控制方案-v1.md`（qwen 席当轮实测在位；**文档私有库 `G:\qidiwork-docs-working-tree\` 另有同主题文档**，两者勿混，qwen#22 + step5#14）。
- F7 已定：服务端控模型/额度/上游 + auto 路由；客户端已具备 OAuth2 issuer 下发、设备码授权 RFC8628、PKCE 回环、模型目录 URL 下发。
- P1-4 补的是 F7 的**客户端补片**：目录/模型变更后如何在不重启下重建引擎侧配置。
- **真值源裁决（本方案建议）**：模型目录以服务端下发为唯一真值源，本地 UI 只读（否则双源冲突，见 §2.4 风险）。
- **边界**：额度/计费/上游路由本身归 F7，本方案不重复设计。

### 4.4 其余既有文档的定位

- `G:\qidiwork\docs\功能清单-2026-09-21-登录与模型控制品牌迁移-未提交变更集.md`（F6）：仅作现状参照，与本方案无交叉改动。
- `G:\qidiwork\docs\DEEPSEEK-HARNESS-吸收分析-2026-08-23.md`（F6）：文体先例——沿用其"逐项证据 + 处置结论"结构，**不引用其结论**。

---

## 五、实施批次与门禁

### 5.1 批次划分

| 批次 | 内容 | 依赖 | 关键判据 |
|---|---|---|---|
| **A** | P0-1 SPKI pin 远程传输 | 无（但**依赖首日探针结论**：reqwest connector 路径 vs 自建 rustls connector，见 §2.1 风险①） | 错误 pin 硬失败、无明文回退 |
| **B** | P0-2 SSE 事件总线 + 重连 + 迟到过滤（含 dual-mode 两 adapter） | A | `run_id` 贯穿、兜底查询落终态 |
| **C** | P0-3 远程审批流 | B | 超时默认 deny |
| **D** | P1-4 模型热更新 | B（用 `config_changed` 事件） | 忙时不打断、空 key 保留 |
| **E1** | P1-5 PDF 批次泵 | 独立（可与 D 并行） | 单页失败隔离 |
| **E2** | P1-6 PTY 客户端协议 | 独立（可与 D 并行；服务端契约须自定） | 游标去重 |
| **F** | P1-7 决策模式 | 独立（可与 D 并行） | decision part 四层零丢失 |

### 5.2 依赖关系

```
A ──► B ──► C
          └──► D
E1（独立，可与 D 并行）
E2（独立，可与 D 并行）
F （独立，可与 D 并行）
P2-1..P2-4：不进批次（服务端源码未取到）；§2.9 A-2（Skills 复核候选）亦不进批次
```

### 5.3 每批门禁命令（真实包名 + 真实 cwd；v2 未执行任何实施命令）

```bash
# 全部命令 cwd = G:\qidiwork\gui\src-tauri（包名实测 qidiwork-gui，step5#7）
# 批次 A —— 主门禁是"错误 pin 硬失败"单测，不是 Go/OpenSSL 风味 rg（见 §5.4）
cargo test -p qidiwork-gui tls::pin -- --nocapture          # 验收②：错误 pin 硬失败（主门禁）
cargo test -p qidiwork-gui tls::pin_format -- --nocapture   # 验收①：pin 编码逐字一致
rg -n "set_certificate_verifier" G:/qidiwork/gui/src-tauri/src   # 验收③：命中点逐处人工白名单（预期 1 处）
cargo check -p qidiwork-gui

# 批次 B
cargo test -p qidiwork-gui events:: -- --nocapture
cargo test -p qidiwork-gui sse_parser -- --nocapture
cargo test -p qidiwork-gui events::dual_mode -- --nocapture   # 验收⑥：两模式同路径行为一致

# 批次 C
cargo test -p qidiwork-gui permission:: -- --nocapture        # 断言超时=deny
rg -n 'skip.*true|action.*"allow"' G:/qidiwork/gui/src-tauri/src   # 期望零命中

# 批次 D / E1 / E2 / F
cargo test -p qidiwork-gui model_reload -- --nocapture
cargo test -p qidiwork-gui pdf_batch -- --nocapture
cargo test -p qidiwork-gui pty_cursor -- --nocapture
cargo test -p qidiwork-gui decision -- --nocapture
```

### 5.4 交批次纪律与可验证判据（qwen#19 + step5#2 重写）

v1 的两处错述已删：① 批次 A 的 `rg -n "danger_accept|accept_invalid_certs|InsecureSkip"`——三标识符是 **Go/OpenSSL 风味**，在 rustls 侧**恒零命中**，通过也证明不了任何事（虚假安全感）；② "§三 第 1/3/5 条各对应一个 rg 检索式"**不实**（原文只有 2 条 rg，且第 5 条无任何判据）。v2 改为逐条标真实判据：

| 反面教材 / 机制 | 可机器复核判据 | 类型 |
|---|---|---|
| §三 第 1 条 YOLO 直通 | `rg -n 'skip.*true\|action.*"allow"' G:/qidiwork/gui/src-tauri/src` 期望零命中 | rg |
| §三 第 3 条 SSE parser 简化 | `cargo test -p qidiwork-gui sse_parser`（CRLF / 多行 data / event id 用例） | 测试 |
| §三 第 5 条 空输出不重试 | **无判据**——qidiwork 侧无对应实现，只约束"不实现自动重试"，由 §2.2 终态测试间接覆盖 | 无（如实标注） |
| P0-1 证书校验（§2.1 验收②③） | **主门禁＝错误 pin 硬失败单测**（经 `dangerous().set_certificate_verifier(...)` 注入不匹配 fixture，断言握手 `Err`、无明文回退）；**辅助＝**`rg -n "set_certificate_verifier"` 检调用点 + 人工白名单（rustls 定制 verifier 必经 `dangerous().set_certificate_verifier`，tiger 自身即 `impl ServerCertVerifier`，`agent_http.rs:63-70`） | 测试 + rg + 人工 |

- 每批合入前由审计席按可核证据面复核：复核对象是 §二 的 tiger `文件:行号`，不是实现者的自述。
- 任一批次若发现证据与代码不符（例：tiger 行号处代码与描述不一致），**停手报 SPEC GAP**，不得自行改口径。
- P2 组（§2.8）与 §2.9 A-2 复核候选**不得**进入任何批次；其前置条件是 §6.1 有结论（A-2 为下一轮裁决）。
- 与 §三 第 2 条（更新校验同病）相关的工作**不属于本方案批次**，按 §6.2 单独立项。

---

## 六、开放问题

### 6.1 TigerAgent 服务端源码获取路径

- **现状**：`litai686/TigerAgent` 未取到——`gh` 未认证，匿名 HTTP 被沙箱网络策略阻断（汇编「审计结论」节）。
- **候选路径**：① 授权后 `gh auth login` 再克隆；② 对方提供源码包（受控渠道，不进公开仓库）；③ 从文档 + SaaS 行为黑盒反推（成本最高、证据等级最低）。
- **影响**：在此之前 §2.8 四项只能停在 P2；§2.9 A-1「LSP」与 A-3「后台作业」的工具本体也在该仓库，同样受限。

### 6.2 qidiwork updater 校验落地（前置：下载侧现状盘点，step5#5）

- **下载侧现状盘点（必须先做，否则无派工落点）**：`G:\qidiwork\gui\src-tauri\src\updater.rs`（F5 口径 `:1-20`）**根本没有下载代码**——只有 `parse_version` + `CARGO_PKG_VERSION` 严格比较、返回 version 与 url，`sha/digest/hash/verify` 三法零命中。**故"下载长度 / Content-Range 校验"不能直接挂在 `updater.rs`。**
- **待盘点的三条候选真实下载路径**：① Tauri updater 插件（若在 `tauri.conf.json` / 插件注册处启用）；② 前端自实现下载；③ 系统浏览器交给用户手工下载安装。**先确认哪条在用，再定校验落点**。
- **可借模式**：tiger 服务端安装脚本 `install-server.sh:180-203` 有 checksums（**仅脚本级证据，非 Rust 实现**）。
- **待决策**：清单内嵌 SHA-256 + 签名体系选型 + 下载长度 / Content-Range 校验。
- **定性**：两侧共同缺口（§三 第 2 条），**不属 tiger 吸收项**，建议单独立项。

### 6.3 SPKI pin 的密钥分发方式

- **事实**：pin 是公开身份，不需保密；tiger 经连接串 `?fp=` 传入（汇编 §9）。
- **候选**：① 客户端内置默认 pin（换密钥需发版）；② 连接配置手工录入（同 tiger）；③ 已认证服务端下发（引出自举信任问题）。
- **建议**：内置 + 手工覆盖双通道；**永不** TOFU 自动接受新 key（与 tiger 一致的纪律）。

### 6.4 远程通道上的审批人身份

- Bearer token 只证明通道可达，**不证明"谁在批"**。待定：审批人鉴权方式、超时时长阈值、审批记录是否留痕审计；与 F6 IM/语音通道的绑定方式未定。

### 6.5 模型目录真值源裁决

- 服务端下发目录（F7）与本地 provider 编辑（§2.4）谁为准？若以服务端为准：本地"新增 provider"路径是否保留、保留时如何不与目录冲突。建议指挥官在 F7 与 v3 之间裁决后回写 §2.4 风险栏。

### 6.6 本地弹窗与远程 IM 审批并存的仲裁与决策回传通道（step5#4）

- **并存是默认情形**：本地用户在 GUI 前、同时又经 IM/语音远程指挥时，同一 `permission_request` 会有两个潜在审批人。
- **待定①（仲裁）**：先到先得（首个 decision 生效、另一侧收到 `PermissionCancelled` 并 UI 置灰）／本地优先／远程优先。**建议先到先得**，因为它与 `acp/bridge.rs` 现有 `permission_pending` 表语义天然一致（`resolve_permission` 已在使用，见 §2.3 映射表）。
- **待定②（决策回传通道）**：复用 `transport.send` 的行流（JSON-RPC 请求）还是另开通道？**建议复用行流**，避免第二条真相通道。
- **判据**：两条决策并发到达时，`permission_pending` 只能被消费一次（断言第二次 `resolve` 返回"已解决"而非二次授权），且另一侧必收 `PermissionCancelled`。

### 6.7 P0 端到端验收的前置时序依赖（step5#10）

- **依赖事实**：远程链路对端服务端**不存在**——F7 服务端仍在规划中（80% 工作量在服务端，见 §1.1 / §4.3）。
- **后果**：§2.1/§2.2/§2.3 的验收全部是**单测级**；任何"真机端到端"验收都必须等 F7 服务端可用，时序上晚于批次 A/B/C 的合入。
- **本方案口径（明确）**：v2 只交**单测级 + 假服务端 fixture**——A 用 fixture 证书跑 pin 校验、B/C 用假 SSE 服务端注入事件序列；**真端到端验收不在本方案范围**，等 F7 落地后另立验收轮。

---

> **v2 边界声明**：本文只做机制吸收与判据设计，不含实现代码、不含任何凭据；所有 tiger 证据均可在汇编 `:21-300` 逐条回溯；所有 qidiwork 事实均带 F 编号来源或标注当轮实测席次；返工处置见 §〇 逐攻点表。
