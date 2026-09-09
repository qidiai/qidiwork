//! agent 子进程管理:spawn `qidi agent --stdio`、行传输、Job Object 整树
//! 约束、退出检测(方案 v2 D2/D4)。
//!
//! 可靠性模型(k3 M2 审计定稿):入站行走有界 mpsc 单消费者可靠队列
//! (协议帧不丢,慢消费背压,消费者经 `take_line_receiver` 唯一取走);
//! 退出通知走 broadcast(一次性事件);写入走有界队列,满则报
//! `Backpressure`(agent 假死信号)。进程崩溃不影响 GUI(F1 单进程多
//! 会话的失败域集中风险由 session/load 游标重放兜底,见方案 §D2)。

pub mod job_object;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc};

use crate::transport::{AgentTransport, ExitInfo, TransportError};

const INBOUND_CAPACITY: usize = 1024;
const EXIT_CAPACITY: usize = 4;
const WRITER_CAPACITY: usize = 1024;
/// stderr 单行截断上限(仅日志用途,无需与协议同限)。
const STDERR_LINE_CAP: usize = 64 * 1024;

/// spawn 配置。program 可执行文件按 PATH 解析(`qidi` → `qidi.exe`)。
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl SpawnConfig {
    /// 默认:PATH 上 `qidi agent --stdio`,cwd = 用户主目录。
    /// 环境变量 `QIDIWORK_AGENT_PATH` 覆盖程序路径(测试/灰度用;
    /// 信任边界与本机权限等价,不应暴露给前端)。
    pub fn default_agent(cwd: Option<PathBuf>) -> Self {
        Self {
            program: std::env::var("QIDIWORK_AGENT_PATH")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "qidi".to_string()),
            // 参数同样可覆盖(联调指向本二进制 --mock-agent 时使用)
            args: std::env::var("QIDIWORK_AGENT_ARGS")
                .ok()
                .map(|a| a.split_whitespace().map(String::from).collect())
                .unwrap_or_else(|| vec!["agent".into(), "--stdio".into()]),
            cwd,
        }
    }
}

/// 运行中的 agent 子进程(stdio 传输的第一实现)。
pub struct AgentProcess {
    cfg: SpawnConfig,
    pid: Option<u32>,
    started_at: std::time::Instant,
    running: Arc<AtomicBool>,
    to_writer: mpsc::Sender<String>,
    /// 入站行可靠队列的唯一接收端;`take_line_receiver` 一次性取走。
    line_rx: Mutex<Option<mpsc::Receiver<String>>>,
    exit: broadcast::Sender<ExitInfo>,
    /// 无 Job 平台(unix)的兜底终止信号;Windows 上 Job terminate 为主,
    /// 此信号仍保留(waiter 侧 select),保证单一路径语义一致。
    kill: mpsc::Sender<()>,
    #[cfg(windows)]
    _job: Option<job_object::JobGuard>,
}

impl AgentProcess {
    /// spawn 子进程并挂入 Job Object(windows),启动读/写/等待三个任务。
    /// 返回前子进程已就绪,但尚无任何往返——协议握手属于 M3 的 ACP 桥。
    pub async fn spawn(cfg: SpawnConfig) -> Result<Arc<Self>, TransportError> {
        // 带路径分隔符的 program 提前校验存在性,给出明确错误
        // (PATH 上的裸名交由系统解析);解析结果进日志便于排障。
        if cfg.program.contains(['/', '\\']) && !Path::new(&cfg.program).exists() {
            return Err(TransportError::Other(format!(
                "agent 程序不存在: {}",
                cfg.program
            )));
        }

        let mut command = Command::new(&cfg.program);
        command
            .args(&cfg.args)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        {
            // GUI 模式下隐藏 qidi.exe 的控制台窗口
            command.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
        }
        if let Some(dir) = &cfg.cwd {
            command.current_dir(dir);
        }

        let mut child = command.spawn().map_err(TransportError::Io)?;
        let pid = child.id();

        // SAFETY: raw_handle 返回子进程的有效 Win32 句柄;句柄仅用于
        // AssignProcessToJobObject,不关闭(所有权在 tokio Child)。
        #[cfg(windows)]
        let job = {
            let guard = job_object::JobGuard::new().map_err(TransportError::Other)?;
            if let Some(raw) = child.raw_handle() {
                guard
                    .assign(windows::Win32::Foundation::HANDLE(raw as _))
                    .map_err(TransportError::Other)?;
            }
            Some(guard)
        };

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Io(std::io::Error::other("child stdin 不可用")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Io(std::io::Error::other("child stdout 不可用")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| TransportError::Io(std::io::Error::other("child stderr 不可用")))?;

        let (line_tx, line_rx) = mpsc::channel(INBOUND_CAPACITY);
        let (exit_tx, _) = broadcast::channel(EXIT_CAPACITY);
        let (write_tx, write_rx) = mpsc::channel(WRITER_CAPACITY);
        let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
        let running = Arc::new(AtomicBool::new(true));

        spawn_writer_task(stdin, write_rx, running.clone(), exit_tx.clone());
        spawn_reader_task(stdout, line_tx, running.clone());
        spawn_stderr_task(stderr);
        // Child 的所有权整体交给 waiter 任务:wait/kill 都在单一任务内,
        // 避免跨任务共享 &mut(结构体只保留 pid / 句柄式信息)。
        let waiter_running = Arc::clone(&running);
        let waiter_exit = exit_tx.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                _ = kill_rx.recv() => {
                    child.start_kill().ok();
                    child.wait().await
                }
            };
            let (code, success) = match status {
                Ok(s) => (s.code(), s.success()),
                Err(e) => {
                    tracing::error!("agent wait 失败(按异常退出处理): {e}");
                    (None, false)
                }
            };
            tracing::info!(?code, success, "agent 进程退出");
            let _ = waiter_exit.send(ExitInfo { code, success });
            waiter_running.store(false, Ordering::Relaxed);
        });

        let process = Arc::new(Self {
            cfg,
            pid,
            started_at: std::time::Instant::now(),
            running,
            to_writer: write_tx,
            line_rx: Mutex::new(Some(line_rx)),
            exit: exit_tx,
            kill: kill_tx,
            #[cfg(windows)]
            _job: job,
        });
        tracing::info!(pid = pid, program = %process.cfg.program, "agent 子进程已启动");
        Ok(process)
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn uptime(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
}

impl AgentTransport for AgentProcess {
    fn send(&self, line: String) -> Result<(), TransportError> {
        self.to_writer.try_send(line).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => TransportError::Backpressure,
            mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
        })
    }

    fn take_line_receiver(&self) -> Result<mpsc::Receiver<String>, TransportError> {
        self.line_rx
            .lock()
            .map_err(|_| TransportError::Other("line_rx 锁中毒".into()))?
            .take()
            .ok_or_else(|| TransportError::Other("line receiver 已被取用".into()))
    }

    fn subscribe_exit(&self) -> broadcast::Receiver<ExitInfo> {
        self.exit.subscribe()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn terminate(&self) {
        // 顺序:先 Job 整树(Windows 主路径),再发 kill 信号兜底
        // (waiter 任务 select 到信号后 start_kill)。两条路径幂等。
        #[cfg(windows)]
        if let Some(job) = &self._job
            && let Err(e) = job.terminate()
        {
            tracing::warn!("Job terminate failed, 回退 start_kill: {e}");
        }
        self.kill.try_send(()).ok();
    }

    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        // 先订阅后终止:否则快速退出的 ExitInfo 会在 subscribe 之前发出,
        // 被错过,导致本函数空等 10s 超时(k3 M2 审计 P0-3)。
        Box::pin(async move {
            let mut rx = self.exit.subscribe();
            self.terminate();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                match tokio::time::timeout_at(deadline, rx.recv()).await {
                    Ok(Ok(_)) => return,
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return,
                    Err(_) => {
                        tracing::warn!("agent 退出等待超时(10s),继续");
                        return;
                    }
                }
            }
        })
    }
}

/// 写入任务:有界队列 → stdin,逐行 flush。写失败(管道破裂/agent 死亡)
/// 时主动标记 running=false 并广播退出信号,让发送方即时感知,
/// 不必等 wait 任务确认进程状态(k3 M2 审计 P1-5)。
fn spawn_writer_task(
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::Receiver<String>,
    running: Arc<AtomicBool>,
    exit: broadcast::Sender<ExitInfo>,
) {
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            // 队列里的行由上层保证不含换行;补 \n 定界并逐行 flush
            // (交互式协议要求立即送达)。
            let mut buf = line;
            buf.push('\n');
            if stdin.write_all(buf.as_bytes()).await.is_err() {
                transport_dead(&running, &exit, "写入失败");
                break;
            }
            if stdin.flush().await.is_err() {
                transport_dead(&running, &exit, "flush 失败");
                break;
            }
        }
        // rx 关闭(进程对象被弃)→ stdin 随 kill_on_drop 关闭。

        fn transport_dead(
            running: &Arc<AtomicBool>,
            exit: &broadcast::Sender<ExitInfo>,
            why: &str,
        ) {
            tracing::error!("agent stdin {why},传输关闭");
            running.store(false, Ordering::Relaxed);
            let _ = exit.send(ExitInfo {
                code: None,
                success: false,
            });
        }
    });
}

/// 一次有界读行的结果。
enum LineRead {
    /// EOF 且无残留字节。
    Eof,
    /// 得到一行(已剥离行尾 \r\n);`truncated` 表示超过 cap、超出部分
    /// 已丢弃,buf 保留截断前缀(供 stderr 日志),且**不含**真实行尾
    /// ——截断后必须继续读到真正的 \n,不能用 ends_with 判断(截断处
    /// 可能恰好是 \n 字节,误判会把余下内容当下一行投递)。
    Line { truncated: bool },
}

/// 分块有界读行:`read_until` 会为超长行完整分配内存(防 OOM 不彻底),
/// 这里按块读入,超过 cap 后剩余字节只计数、不入缓冲,直到真实行尾。
/// 行尾的 `\r\n`/`\n` 在未截断时剥离——JSON-RPC over newline 协议下行
/// 内容本身不含裸换行,剥离不损伤载荷。
async fn read_bounded_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<LineRead> {
    buf.clear();
    let mut overflow = 0usize;
    loop {
        let before = buf.len();
        match reader.read_until(b'\n', buf).await {
            Ok(0) => {
                if before == 0 {
                    return Ok(LineRead::Eof); // EOF 且无残留
                }
                break; // EOF,处理无换行结尾的最后一行
            }
            Ok(_) => {
                if buf.len() > cap {
                    if overflow == 0 {
                        overflow += before; // 行首到截断点也一并丢弃
                    }
                    overflow += buf.len() - before;
                    buf.truncate(before);
                } else if buf.ends_with(b"\n") {
                    break;
                }
            }
            Err(e) => return Err(e),
        }
    }
    if overflow > 0 {
        tracing::error!(
            dropped = overflow,
            "agent 输出单行超过上限,超出部分丢弃(防 OOM)"
        );
        return Ok(LineRead::Line { truncated: true });
    }
    if buf.ends_with(b"\n") {
        buf.pop();
        if buf.ends_with(b"\r") {
            buf.pop();
        }
    }
    Ok(LineRead::Line { truncated: false })
}

fn spawn_reader_task(
    stdout: impl AsyncRead + Unpin + Send + 'static,
    tx: mpsc::Sender<String>,
    running: Arc<AtomicBool>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        loop {
            match read_bounded_line(&mut reader, &mut buf, crate::transport::MAX_LINE_BYTES).await {
                Ok(LineRead::Eof) => break,
                Ok(LineRead::Line { truncated: true }) => continue, // error 已记,整行丢弃
                Ok(LineRead::Line { truncated: false }) => {
                    if buf.is_empty() {
                        continue; // 空行:协议上无意义
                    }
                    let line = String::from_utf8_lossy(&buf).into_owned();
                    // 可靠队列:消费者未取走前帧滞留(背压),不丢帧。
                    if tx.send(line).await.is_err() {
                        break; // 消费方消失(正常关停路径)
                    }
                }
                Err(e) => {
                    tracing::error!("agent stdout 读取失败: {e}");
                    break;
                }
            }
        }
        running.store(false, Ordering::Relaxed);
        tracing::info!("agent stdout 已关闭(EOF)");
    });
}

fn spawn_stderr_task(stderr: impl AsyncRead + Unpin + Send + 'static) {
    // 必须持续排空 stderr:不读则管道写满会阻塞 agent 本身(经典死锁)。
    // 与 stdout 共用限长读取,超限行记录截断前缀(k3 M2 审计 P1-1)。
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buf: Vec<u8> = Vec::with_capacity(1024);
        loop {
            match read_bounded_line(&mut reader, &mut buf, STDERR_LINE_CAP).await {
                Ok(LineRead::Eof) => break,
                Ok(LineRead::Line { truncated: true }) => {
                    tracing::warn!(target: "agent-stderr", "[截断] {}", String::from_utf8_lossy(&buf));
                }
                Ok(LineRead::Line { truncated: false }) => {
                    tracing::warn!(target: "agent-stderr", "{}", String::from_utf8_lossy(&buf));
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::job_object;

    use tokio::process::Command;

    // JobGuard 纯单测:真实 Win32 句柄 + cmd 长驻子进程,验证
    // terminate() 即时整树回收(方案 v2 §D4 验收项)。
    #[cfg(windows)]
    #[tokio::test]
    async fn job_guard_terminate_kills_child() {
        let guard = job_object::JobGuard::new().unwrap();
        let mut child = Command::new("cmd")
            .args(["/c", "ping -n 60 127.0.0.1 > nul"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        // SAFETY: child 刚 spawn,raw_handle 有效且归本测试所有。
        let raw = child.raw_handle().expect("tokio Child 句柄应可用");
        guard
            .assign(windows::Win32::Foundation::HANDLE(raw as _))
            .unwrap();

        guard.terminate().unwrap();
        let started = std::time::Instant::now();
        let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
            .await
            .expect("Job 终止后子进程应在 10s 内退出")
            .unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "Job 终止应近乎即时"
        );
        assert!(!status.success(), "TerminateJobObject 的退出码不应为成功");
    }
}
