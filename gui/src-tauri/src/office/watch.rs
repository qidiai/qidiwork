//! manifest 变更监听:notify 递归监听 office-workspaces 根,防抖后按
//! task 回调(调用方重读 manifest 并推送前端,事件名 `office-event`)。
//! GUI 只读;写方是 card.py(已原子写),故读到的一定是完整 JSON。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// 防抖窗口(绝对时长,不随事件滑动):窗口到点即强制 flush,
/// 持续事件流下回调最多延迟一个窗口,不会饿死(k3 P1a 复核确认)。
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(300);
const POLL_IDLE: Duration = Duration::from_millis(200);

/// 运行中的监听器:drop 时建议先 `stop()`;即使不调用,守护线程也会在
/// 进程退出时随之消亡。
pub struct ManifestWatch {
    _watcher: RecommendedWatcher,
    stop_flag: Arc<AtomicBool>,
}

impl ManifestWatch {
    /// 监听 root 下所有 manifest.json;防抖后按受影响 task 回调
    /// `on_change(task)`(在独立线程上执行)。
    pub fn start<F>(root: PathBuf, on_change: F) -> notify::Result<Self>
    where
        F: Fn(String) + Send + 'static,
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop_flag);
        std::thread::spawn(move || {
            debounce_loop(&root, rx, stopped, on_change);
        });

        Ok(Self {
            _watcher: watcher,
            stop_flag,
        })
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

fn stopped(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}

/// 防抖主循环:首个事件起开 300ms 窗口,窗口内事件按 task 合并,
/// 溢出/乱序一律全量重读(读的是变更后的 manifest,无需增量)。
fn debounce_loop(
    root: &PathBuf,
    rx: std::sync::mpsc::Receiver<notify::Event>,
    stop: Arc<AtomicBool>,
    on_change: impl Fn(String),
) {
    loop {
        if stopped(&stop) {
            return;
        }
        // 等首个事件(200ms 粒度轮询 stop 标志)
        let first = match rx.recv_timeout(POLL_IDLE) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut touched = tasks_from(&first.paths, root);
        // 防抖窗口:合并同窗内所有事件
        let deadline = std::time::Instant::now() + DEBOUNCE_WINDOW;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(DEBOUNCE_WINDOW.min(POLL_IDLE)) {
                Ok(event) => touched.extend(tasks_from(&event.paths, root)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if stopped(&stop) {
                return;
            }
        }
        touched.sort();
        touched.dedup();
        for task in touched {
            on_change(task);
        }
    }
}

/// 从事件路径提取受影响 task 名(只关心 manifest.json)。
fn tasks_from(paths: &[PathBuf], root: &PathBuf) -> Vec<String> {
    paths
        .iter()
        .filter(|p| {
            p.file_name().is_some_and(|n| n == "manifest.json")
                && p.parent().is_some_and(|d| d.starts_with(root))
        })
        .filter_map(|p| {
            p.parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::fs;

    #[test]
    fn debounce_fires_under_continuous_events() {
        // 持续 1.5s 高频写 5 个 manifest:窗口不断被新事件推迟,
        // 但 MAX_MERGE_WINDOW 强制 flush,每个 task 至少回调一次。
        let root = std::env::temp_dir().join(format!("qidi-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for i in 0..5 {
            let d = root.join(format!("t{i}"));
            fs::create_dir_all(&d).unwrap();
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let watch = ManifestWatch::start(root.clone(), move |task| {
            tx.send(task).unwrap();
        })
        .unwrap();

        let writer_root = root.clone();
        let writer = std::thread::spawn(move || {
            for round in 0..30 {
                for i in 0..5 {
                    let p = writer_root.join(format!("t{i}")).join("manifest.json");
                    fs::write(&p, format!("{{\"artifacts\":[{round}]}}")).unwrap();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        writer.join().unwrap();

        // 收集回调(6s 兜底),5 个 task 必须都触发
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        let mut seen = std::collections::HashSet::new();
        while std::time::Instant::now() < deadline && seen.len() < 5 {
            if let Ok(task) = rx.recv_timeout(Duration::from_millis(300)) {
                seen.insert(task);
            }
        }
        watch.stop();
        assert_eq!(seen.len(), 5, "持续事件下所有 task 最终都应回调");
        let _ = fs::remove_dir_all(&root);
    }
}
