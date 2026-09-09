//! ACP 会话桥(GUI 作为 ACP client,方案 v2 P0)。
//!
//! 协议面仅 5 方法:initialize / session/new / session/prompt /
//! session/cancel / session/load;入站只处理 session/update 通知与
//! session/request_permission 请求。fs / terminal 能力不声明、不实现
//! (agent 的文件操作走其自有工具,缩小 GUI 攻击面,方案 v2 §D4)。

pub mod bridge;
pub mod jsonrpc;

pub use bridge::{AcpBridge, BridgeEvent, reconnect};
