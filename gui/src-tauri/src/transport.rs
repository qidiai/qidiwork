//! Agent 传输层抽象:与 agent 子进程之间 newline-delimited JSON-RPC 行流。
//!
//! M3 的 ACP 桥只依赖 [`AgentTransport`] trait;stdio 子进程是第一实现
//! (`process::AgentProcess`)。将来切换 leader(named pipe)或远程沙箱时
//! 提供新的实现即可,协议层不动。
//!
//! # 可靠性契约(k3 M2 审计 P0-2)
//!
//! 入站行走**有界 mpsc 可靠队列**:协议帧不丢,消费者慢时生产侧背压;
//! broadcast 只用于退出通知(一次性事件,可容忍 Lagged)。前端展示流
//! 由消费方(现 M2 事件转发器,M3 起为 ACP 桥)自行派生。

use std::fmt;

use serde::Serialize;
use tokio::sync::{broadcast, mpsc};

/// 单行上限(64 MiB)。超限行被丢弃并记日志,防止 agent 异常时 OOM GUI。
pub const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// agent 进程退出信息。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExitInfo {
    /// 进程退出码;被信号/Job 强杀或等待出错时为 None。
    pub code: Option<i32>,
    /// 进程是否正常退出(成功码)。M3 据此区分"任务完成"与"崩溃/强杀",
    /// 决定是否走 session/load 恢复(方案 v2 §D2)。
    pub success: bool,
}

#[derive(Debug)]
pub enum TransportError {
    /// agent 侧已关闭(进程退出或管道断开),无法再发送。
    Closed,
    /// 写入队列已满:agent 长时间未消费 stdin(假死/满载),M3 据此判定。
    Backpressure,
    /// 底层 IO 错误。
    Io(std::io::Error),
    /// 平台层错误(Job Object 等,文本已含上下文)。
    Other(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "agent 传输已关闭"),
            Self::Backpressure => write!(f, "agent 写入队列已满(疑似假死)"),
            Self::Io(e) => write!(f, "agent 传输 IO 错误: {e}"),
            Self::Other(e) => write!(f, "agent 进程管理错误: {e}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// 行传输 trait。实现必须保证:
/// - `send` 尽力即返(入队);队列满时返回 [`TransportError::Backpressure`],
///   进程死亡后返回 [`TransportError::Closed`]——**不承诺送达**;
/// - `take_line_receiver` 只能成功一次:入站行是协议帧,单消费者可靠队列,
///   不允许第二个消费者分走帧(展示流由消费者自行派生);
/// - 进程树整体终止由实现负责(Job Object 兜底 + 主动清理)。
pub trait AgentTransport: Send + Sync + 'static {
    /// 入队一行(不带换行符,实现负责补 `\n` 并 flush)。
    fn send(&self, line: String) -> Result<(), TransportError>;
    /// 取走入站行队列的唯一消费者(协议帧不丢)。
    fn take_line_receiver(&self) -> Result<mpsc::Receiver<String>, TransportError>;
    /// 订阅进程退出通知(一次性事件,broadcast)。
    fn subscribe_exit(&self) -> broadcast::Receiver<ExitInfo>;
    /// 进程/传输是否仍在运行(stdout EOF 或退出即 false)。
    fn is_running(&self) -> bool;
    /// 同步快速终止整树(TerminateJobObject / start_kill)。
    /// 退出通知仍会经 `subscribe_exit` 送达。
    fn terminate(&self);
    /// 终止并等待进程完全退出。
    /// 装箱 Future(RPITIT 不可 dyn,桥需要 `Arc<dyn AgentTransport>`)。
    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;
}
