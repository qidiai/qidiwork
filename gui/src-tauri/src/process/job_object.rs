//! Windows Job Object 进程树管理(方案 v2 D4 安全基线)。
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`:GUI 崩溃/被杀时,内核关闭 Job
//! 句柄即整树回收 agent 及其孙进程(MCP server、shell 命令),杜绝孤儿
//! agent 继续写文件。参照主工程 cf-sandbox 的成熟写法(windows 0.62)。

#![cfg(windows)]

use std::mem::{size_of, zeroed};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::core::PCWSTR;

/// 持有 Job 句柄的守卫。句柄存活期间挂入的进程受 kill-on-close 约束;
/// drop(或显式 [`JobGuard::terminate`])即整树回收。
pub struct JobGuard {
    handle: HANDLE,
}

// SAFETY: HANDLE 是可平凡复制的裸句柄;Win32 Job Object API 线程安全,
// JobGuard 的全部方法只做句柄级系统调用,跨线程使用没有数据竞争。
#[allow(unsafe_code)]
unsafe impl Send for JobGuard {}
#[allow(unsafe_code)]
unsafe impl Sync for JobGuard {}

impl JobGuard {
    /// 创建带 `KILL_ON_JOB_CLOSE` 限制的匿名 Job Object。
    #[allow(unsafe_code)]
    pub fn new() -> Result<Self, String> {
        // SAFETY: 参数为空名称与默认安全属性,无指针生命周期风险。
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|e| format!("CreateJobObjectW failed: {e}"))?;

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: handle 刚由 CreateJobObjectW 返回;指针与长度指向匹配的
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION,info_class 与之对应。
        let result = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(e) = result {
            // SAFETY: 同上,handle 归属本函数,失败路径立即关闭。
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(format!("SetInformationJobObject failed: {e}"));
        }
        Ok(Self { handle })
    }

    /// 把已启动子进程挂入 Job。此后该进程及其全部后代受 kill-on-close 约束。
    ///
    /// 已知竞态:assign 前子进程可能已 spawn 自己的孙子进程;Windows 8+ 的
    /// Job 语义(子进程继承 Job)保证 assign 之后产生的后代全部入 Job,
    /// assign 前的窗口期由 spawn 流程尽量缩短,风险可接受(方案 v2 §D4)。
    #[allow(unsafe_code)]
    pub fn assign(&self, child_handle: HANDLE) -> Result<(), String> {
        // SAFETY: 两个句柄均有效——self.handle 由 new 创建,child_handle
        // 来自 tokio Child::raw_handle(),进程尚在运行。
        unsafe { AssignProcessToJobObject(self.handle, child_handle) }
            .map_err(|e| format!("AssignProcessToJobObject failed: {e}"))
    }

    /// 立即终止 Job 内全部进程(整树)。
    #[allow(unsafe_code)]
    pub fn terminate(&self) -> Result<(), String> {
        // SAFETY: handle 归属 self,进程组整体终止正是本模块的职责。
        unsafe { TerminateJobObject(self.handle, 1) }
            .map_err(|e| format!("TerminateJobObject failed: {e}"))
    }
}

impl Drop for JobGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // drop = 关闭 Job 句柄 → KILL_ON_JOB_CLOSE 由内核完成整树回收,
        // 无需显式 Terminate;CloseHandle 失败仅记日志(进程退出路径)。
        // SAFETY: handle 归属 self,drop 是唯一关闭点。
        unsafe {
            if let Err(e) = CloseHandle(self.handle) {
                tracing::warn!("JobGuard CloseHandle failed: {e}");
            }
        }
    }
}
