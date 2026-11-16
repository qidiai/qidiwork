# 权限请求超时（permission timeout → 拒绝）

agent 通过 ACP `session/request_permission` 请求用户审批时，桥层为**每一条**在途
请求挂一个截止时间。到期仍无人回填 → 自动以**拒绝**回应 agent，并通知前端撤掉
弹窗。**绝不自动 allow。**

代码位置：`gui/src-tauri/src/acp/bridge.rs`（`DEFAULT_PERMISSION_TIMEOUT`、
`PermissionEntry`、`spawn_permission_timeout`）。

## 默认值与可注入性

| 项 | 值 | 说明 |
|---|---|---|
| 默认超时 | **120 s** | `DEFAULT_PERMISSION_TIMEOUT` |
| 测试注入 | `new_with_timeout(transport, timeout)` | `#[cfg(test)]`，公开 `attach(...)` 签名不变 |
| 起点 | 请求落表时 | 不是回合开始、也不是写队列时刻 |
| 并发 | 与 `resolve_permission` / `cancel_permission` 共用 `permission_pending` 锁 | 先到者生效，后到者 no-op |

## 到期后的行为

1. 加锁复核：该 `request_id` 仍在表内**且** `deadline <= now` 才继续（否则说明
   用户已回填、会话已删或桥已断，直接静默退出）。
2. 移除条目（此后 `resolve_permission` 会返回「不存在已响应」）。
3. 向 agent 回一条 `result.outcome`：

| 请求的 `options` 形态 | 回给 agent | 前端事件 |
|---|---|---|
| 含 `kind` 带 `reject` 的选项（`reject_once` / `reject_always`） | `{"outcome":"selected","optionId":"<reject 选项 id>"}` | `permission_cancelled{reason:"timeout"}` |
| 无 reject 选项（例如只有 `allow_once`） | `{"outcome":"cancelled"}`（不带 optionId） | 同上 |

4. 日志：`tracing::warn!(request_id, session_id, "权限请求超时，按拒绝处理")`。

**安全不变量**：超时路径只会落到 reject 选项或 `cancelled`，任何情况下都不会
选择 `allow_once` / `allow_always`。

**前端清场语义（2026-11-17 修正）**：超时事件带 `request_id`，前端只移除**该条**权限项，
且**不解除**会话 `busy`（回合仍在跑，兄弟请求可能还没答）；只有不带 `request_id` 的
`permission_cancelled`（`turn_ended` / `bridge_restart` / `session_removed`）才清整会话并解 `busy`。

## 与 30 分钟 `PROMPT_TIMEOUT` 的区别

| | 权限超时（本文件） | `PROMPT_TIMEOUT` |
|---|---|---|
| 值 | 120 s | 30 min（`bridge.rs` 常量） |
| 作用对象 | 单条在途权限请求 | 一次 `session/prompt` RPC 的应答等待 |
| 到期动作 | 对 agent **回填拒绝/取消**（协议内闭环），并广播 `permission_cancelled` | 只发 `TurnCompleted` 错误事件；**不是**取消 |
| agent 侧状态 | 收到明确应答，可继续/结束 | 仍在跑，真正中断需配合 `session/cancel` |

两者互不替代：权限超时是「用户不答就按拒绝」的产品语义；`PROMPT_TIMEOUT` 只是
防前端无限挂起。

## 相关清理路径（都不是超时）

- `cancel_permission`：用户关弹窗 → 回 `cancelled`，事件 `reason` 由调用方给。
- `expire_permissions_for_session`：回合已结束 → 只清表、只发事件，**不再回 agent**
  （turn 已亡，kernel 自会取消）。
- `on_exit` / `forget_session`：断连或删会话 → 回 `cancelled`，事件
  `reason = "bridge_restart"` / `"session_removed"`。

## 测试

`gui/src-tauri/src/acp/bridge.rs` 测试模块（注入 20 ms）：

- `permission_timeout_rejects` — 超时 → `selected` + reject optionId，表清空，事件 `timeout`
- `permission_resolved_before_timeout_no_second_response` — 先 resolve，超时后无第二条应答
- `permission_timeout_without_reject_option_cancels` — 只有 allow → `cancelled`
- `timeout_never_allows` — 所有超时路径下发的 optionId 均不属于 allow 集合

```bash
cd G:/qidiwork/gui/src-tauri
cargo test --lib acp::bridge
```
