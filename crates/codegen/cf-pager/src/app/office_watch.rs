//! Office-artifact manifest watch (QidiWork stage 1).
//!
//! Session <-> task binding mirrors the office-artifact skill (`card.py`):
//! `QIDI_OFFICE_TASK` (trimmed, non-empty) wins, else `"default"`. When
//! `~/.qidi/office-workspaces/<task>/` exists, its `manifest.json` is
//! watched via cf-fsnotify and mirrored into the active agent's scrollback
//! as an [`ArtifactBlock`]: pushed once artifacts appear, replaced in place
//! on change (focused card preserved), refreshed to the "no artifacts" hint
//! when the manifest empties.
//!
//! The event loop drains the broadcast receiver from a 1s timer arm (same
//! pattern as `app::subscription`'s watch) rather than adding a dedicated
//! `select!` branch -- no starvation risk, and a workspace created
//! mid-session is picked up by the lazy re-arm in [`AppView::poll_office_watch`].

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cf_fsnotify::{FsConfig, FsEvent, FsEventSource};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::TryRecvError;

use super::agent_view::AgentView;
use super::app_view::{ActiveView, AppView};
use crate::scrollback::block::RenderBlock;
use crate::scrollback::blocks::ArtifactBlock;

/// Drain cadence for the manifest watch. Events buffer in the broadcast
/// channel between ticks, so this only bounds detection latency.
pub(crate) const OFFICE_WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Pure binding rule, split from [`office_task`] for tests.
pub(crate) fn resolve_task(env: Option<String>) -> String {
    env.map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "default".to_string())
}

/// Session <-> task binding: `QIDI_OFFICE_TASK` or `"default"`.
pub(crate) fn office_task() -> String {
    resolve_task(std::env::var("QIDI_OFFICE_TASK").ok())
}

/// Home directory mirroring the artifact producer (the `office-artifact`
/// skill's card.py resolves `os.path.expanduser("~")`, which checks `$HOME`
/// first on every platform): `$HOME` when set, else the OS profile dir.
/// PTY e2e harnesses sandbox `HOME` to a tempdir, which keeps the watch
/// hermetic without an extra test seam. Shared with
/// [`ArtifactBlock::load_from_manifest`](crate::scrollback::blocks::ArtifactBlock)
/// so the watch arm and the loader resolve the same workspace.
pub(crate) fn office_home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir()
}

/// `<home>/.qidi/office-workspaces/<task>/`, only when it already exists.
/// Pure in `<home>` so tests can drive it with a tempdir.
fn workspace_dir_in(home: &std::path::Path, task: &str) -> Option<PathBuf> {
    let dir = home.join(".qidi").join("office-workspaces").join(task);
    dir.is_dir().then_some(dir)
}

/// `~/.qidi/office-workspaces/<task>/`, only when it already exists.
fn workspace_dir(task: &str) -> Option<PathBuf> {
    workspace_dir_in(&office_home_dir()?, task)
}

/// Live watch on one task workspace. Dropping it releases this consumer's
/// reference to the shared watcher (the cf-fsnotify registry keeps the
/// watcher alive for any other consumers).
pub(crate) struct OfficeWatch {
    task: String,
    _source: Arc<FsEventSource>,
    rx: Receiver<FsEvent>,
    /// Whether the initial mirror has landed in an agent scrollback.
    /// The watch arms during the welcome screen (no agent view yet), so the
    /// arm-time sync is a no-op; without this flag nothing re-triggers it
    /// after the first session is promoted, and pre-registered artifacts
    /// would stay invisible until the manifest changed mid-session.
    mirrored: bool,
}

impl OfficeWatch {
    fn arm(task: &str) -> Option<Self> {
        let dir = workspace_dir(task)?;
        let source = match cf_fsnotify::shared(dir.clone(), FsConfig::default()) {
            Ok(s) => s,
            Err(err) => {
                crate::unified_log::warn(
                    "office watch arm failed",
                    None,
                    Some(serde_json::json!({
                        "task": task,
                        "dir": dir.display().to_string(),
                        "err": err.to_string(),
                    })),
                );
                return None;
            }
        };
        let rx = source.subscribe();
        crate::unified_log::info(
            "office watch armed",
            None,
            Some(serde_json::json!({
                "task": task,
                "dir": dir.display().to_string(),
            })),
        );
        Some(Self {
            task: task.to_string(),
            _source: source,
            rx,
            mirrored: false,
        })
    }
}

/// What a drain found, collapsed for the caller.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(crate) enum DrainOutcome {
    /// No relevant events.
    Idle,
    /// `manifest.json` changed, or events were lost (Lagged -> reload to
    /// be safe).
    ManifestChanged,
    /// Channel closed (watcher dropped): re-arm next tick.
    Closed,
}

/// Non-blocking drain of pending watch events. Split out for tests.
pub(crate) fn drain_events(rx: &mut Receiver<FsEvent>) -> DrainOutcome {
    let mut outcome = DrainOutcome::Idle;
    loop {
        match rx.try_recv() {
            Ok(FsEvent::FilesChanged { paths, .. }) => {
                if paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|n| n == "manifest.json"))
                {
                    outcome = DrainOutcome::ManifestChanged;
                }
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Lagged(_)) => outcome = DrainOutcome::ManifestChanged,
            Err(TryRecvError::Closed) => {
                outcome = DrainOutcome::Closed;
                break;
            }
        }
    }
    outcome
}

/// Open a file with the OS default handler (fire-and-forget).
///
/// `QIDI_OFFICE_OPEN_CMD` overrides the platform opener: the value is
/// whitespace-split into program + leading args and the path is appended as
/// the final argument. PTY e2e uses this to log opens instead of launching
/// real applications.
///
/// Errors are logged, not surfaced: the TUI must stay responsive even when
/// no handler is registered for the file type.
pub(crate) fn open_in_system(path: &str) {
    let override_cmd = std::env::var("QIDI_OFFICE_OPEN_CMD")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let spawned = match override_cmd {
        Some(cmdline) => {
            let mut parts = cmdline.split_whitespace();
            match parts.next() {
                Some(prog) => std::process::Command::new(prog)
                    .args(parts)
                    .arg(path)
                    .spawn(),
                None => return,
            }
        }
        None => platform_open(path),
    };
    if let Err(err) = spawned {
        tracing::warn!("office artifact system-open failed: {path}: {err}");
    }
}

#[cfg(target_os = "windows")]
fn platform_open(path: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", path])
        .spawn()
}

#[cfg(target_os = "macos")]
fn platform_open(path: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("open").arg(path).spawn()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_open(path: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open").arg(path).spawn()
}

/// Insert or refresh the artifact block in a scrollback.
///
/// - No block + non-empty manifest -> push.
/// - No block + empty manifest -> stay silent (never push an empty card).
/// - Existing block -> replace in place, preserving the focused card;
///   committed (minimal-mode) entries can't be mutated, so they are
///   removed and re-pushed, mirroring `acp_handler::permissions`.
pub(crate) fn upsert_artifact_block(agent: &mut AgentView, mut block: ArtifactBlock) {
    let mut found = None;
    for i in (0..agent.scrollback.len()).rev() {
        if let Some(e) = agent.scrollback.entry(i)
            && matches!(e.block, RenderBlock::Artifact(_))
        {
            found = Some(e.id);
            break;
        }
    }
    match found {
        Some(eid) => {
            // Preserve the focused card across reloads (clamped to the new
            // card count; a zero-card block clamps to 0).
            if let Some(e) = agent.scrollback.get_by_id_mut(eid)
                && let RenderBlock::Artifact(old) = &e.block
            {
                block.focused_idx = old.focused_idx.min(block.cards.len().saturating_sub(1));
            }
            if agent.scrollback.is_committed(eid) {
                agent.scrollback.remove_entry(eid);
                agent.scrollback.push_block(RenderBlock::Artifact(block));
            } else if let Some(e) = agent.scrollback.get_by_id_mut(eid) {
                e.block = RenderBlock::Artifact(block);
                // The card count (and thus the block's height) just changed;
                // drop the entry's cached output/height so the next draw
                // re-projects it. Without this, an update that lands while
                // the scrollback is not yet committed renders the block into
                // its old, shorter box and clips the new cards (seen live in
                // the PTY e2e telemetry: upsert(cards:2) succeeded on screen
                // only after commit-based re-push).
                e.invalidate_cache();
            }
        }
        None if !block.cards.is_empty() => {
            agent.scrollback.push_block(RenderBlock::Artifact(block));
        }
        None => {}
    }
}

impl AppView {
    /// Whether the watch tick is wanted: already armed, or the bound
    /// workspace now exists (the latter covers workspaces created
    /// mid-session). Re-evaluated per tick; cheap (one stat syscall).
    pub(crate) fn office_watch_wanted(&self) -> bool {
        self.office_watch.is_some() || workspace_dir(&office_task()).is_some()
    }

    /// Timer tick: lazy-arm, drain, sync. Never blocks the event loop.
    /// Poll the manifest watch. Returns true iff the scrollback was mutated
    /// (block pushed/replaced), so the caller can redraw on change only.
    pub(crate) fn poll_office_watch(&mut self) -> bool {
        if self.office_watch.is_none() {
            self.office_watch = OfficeWatch::arm(&office_task());
        }
        let outcome = match &mut self.office_watch {
            Some(w) => drain_events(&mut w.rx),
            None => DrainOutcome::Idle,
        };
        let mut changed = false;
        if matches!(outcome, DrainOutcome::ManifestChanged) {
            crate::unified_log::info("office watch manifest changed", None, None);
        }
        match outcome {
            DrainOutcome::Idle => {}
            DrainOutcome::ManifestChanged => changed = self.sync_office_artifacts(),
            // Watcher died (workspace removed, etc.); next tick re-arms.
            DrainOutcome::Closed => self.office_watch = None,
        }
        // Self-healing mirror: if the artifact block is absent from the
        // active agent's scrollback — never landed, or wiped by a
        // session-create / overdue-turn-reconcile rebuild — re-upsert it.
        // The presence scan is a few dozen map lookups per tick; the
        // manifest is only re-read while the block is actually missing.
        // `mirrored` then records the first landing that verifiably stuck
        // (informational; fs events keep driving updates afterwards).
        if !self.artifact_block_present() && self.sync_office_artifacts() {
            if self.artifact_block_present() {
                if let Some(w) = &mut self.office_watch {
                    if !w.mirrored {
                        w.mirrored = true;
                        crate::unified_log::info("office watch mirror settled", None, None);
                    }
                }
            }
            changed = true;
        }
        changed
    }

    /// Whether the active agent's scrollback currently holds an artifact
    /// block (any card count). False on the welcome screen / when no agent
    /// view exists — callers treat that as "needs (re)push" and rely on
    /// [`sync_office_artifacts`] no-op'ing until a session appears.
    fn artifact_block_present(&self) -> bool {
        let ActiveView::Agent(id) = self.active_view else {
            return false;
        };
        let Some(agent) = self.agents.get(&id) else {
            return false;
        };
        (0..agent.scrollback.len()).any(|i| {
            agent
                .scrollback
                .entry(i)
                .map(|e| matches!(e.block, RenderBlock::Artifact(_)))
                .unwrap_or(false)
        })
    }

    /// Load the bound task's manifest and upsert it into the root active
    /// agent's scrollback. Returns true iff a block was actually upserted
    /// (used by the initial-mirror retry: arm-time sync runs on the welcome
    /// screen where no agent exists yet). Parse errors and non-agent views
    /// are skipped.
    fn sync_office_artifacts(&mut self) -> bool {
        let Some(w) = &self.office_watch else {
            return false;
        };
        let block = match ArtifactBlock::load_from_manifest(&w.task) {
            Ok(b) => b,
            Err(err) => {
                crate::unified_log::warn(
                    "office watch manifest load failed",
                    None,
                    Some(serde_json::json!({
                        "task": w.task,
                        "err": err.to_string(),
                    })),
                );
                return false;
            }
        };
        let ActiveView::Agent(id) = self.active_view else {
            return false;
        };
        let Some(agent) = self.agents.get_mut(&id) else {
            return false;
        };
        // Gate on ACP session readiness: a block pushed while
        // `session.create` is still in flight joins the scrollback entries
        // but is excluded from the turns projection that create.done
        // finalizes — present-but-invisible (seen in PTY e2e telemetry:
        // upsert settled, screen never painted it). `session_id` is only
        // assigned once the ACP session/new reply lands, so deferring to
        // the first tick after that keeps the push (and every later
        // manifest-change upsert) on the projected path.
        if agent.session.session_id.is_none() {
            return false;
        }
        crate::unified_log::info(
            "office watch upsert",
            None,
            Some(serde_json::json!({"cards": block.cards.len()})),
        );
        upsert_artifact_block(agent, block);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::agent_view::test_agent_view;
    use cf_fsnotify::FsEventKind;
    use tokio::sync::broadcast;

    fn card(name: &str) -> crate::scrollback::blocks::ArtifactCard {
        serde_json::from_value(serde_json::json!({
            "path": format!("C:\\\\ws\\\\{name}"),
            "name": name,
            "size": 12,
            "mtime": 1.0,
            "registered_at": 2.0,
            "skill": "t",
            "note": "",
        }))
        .unwrap()
    }

    fn block_with(names: &[&str]) -> ArtifactBlock {
        ArtifactBlock {
            cards: names.iter().map(|n| card(n)).collect(),
            focused_idx: 0,
        }
    }

    fn manifest_event() -> FsEvent {
        FsEvent::FilesChanged {
            paths: vec![PathBuf::from("ws").join("manifest.json")],
            kind: FsEventKind::Modified,
        }
    }

    #[test]
    fn workspace_dir_in_requires_existing_dir() {
        let home = tempfile::tempdir().expect("temp home");
        let task_dir = home
            .path()
            .join(".qidi")
            .join("office-workspaces")
            .join("demo");
        assert!(workspace_dir_in(home.path(), "demo").is_none());
        std::fs::create_dir_all(&task_dir).expect("mkdir");
        assert_eq!(workspace_dir_in(home.path(), "demo"), Some(task_dir));
        // A file at the task path does not count as a workspace.
        let file_home = tempfile::tempdir().expect("temp home 2");
        let ws = file_home.path().join(".qidi").join("office-workspaces");
        std::fs::create_dir_all(&ws).expect("mkdir ws");
        std::fs::write(ws.join("as-file"), b"x").expect("write file");
        assert!(workspace_dir_in(file_home.path(), "as-file").is_none());
    }

    #[test]
    fn resolve_task_matrix() {
        assert_eq!(resolve_task(None), "default");
        assert_eq!(resolve_task(Some("  ".into())), "default");
        assert_eq!(resolve_task(Some(" bid-2501 ".into())), "bid-2501");
    }

    #[test]
    fn drain_detects_manifest_change_and_close() {
        let (tx, mut rx) = broadcast::channel(8);
        assert_eq!(drain_events(&mut rx), DrainOutcome::Idle);

        tx.send(FsEvent::FilesChanged {
            paths: vec![PathBuf::from("other.txt")],
            kind: FsEventKind::Modified,
        })
        .unwrap();
        assert_eq!(drain_events(&mut rx), DrainOutcome::Idle);

        tx.send(manifest_event()).unwrap();
        assert_eq!(drain_events(&mut rx), DrainOutcome::ManifestChanged);

        drop(tx);
        assert_eq!(drain_events(&mut rx), DrainOutcome::Closed);
    }

    #[test]
    fn drain_treats_lagged_as_changed() {
        let (tx, mut rx) = broadcast::channel(1);
        tx.send(manifest_event()).unwrap();
        tx.send(manifest_event()).unwrap();
        // First recv is Lagged(1) -> reload to be safe.
        assert_eq!(drain_events(&mut rx), DrainOutcome::ManifestChanged);
    }

    #[test]
    fn upsert_pushes_only_when_nonempty() {
        let mut agent = test_agent_view(None, PathBuf::from("/tmp"));
        upsert_artifact_block(&mut agent, block_with(&[]));
        assert_eq!(agent.scrollback.len(), 0, "empty manifest never pushes");

        upsert_artifact_block(&mut agent, block_with(&["a.docx"]));
        assert_eq!(agent.scrollback.len(), 1);
        assert!(matches!(
            agent.scrollback.entry(0).unwrap().block,
            RenderBlock::Artifact(_)
        ));
    }

    #[test]
    fn upsert_replaces_in_place_and_preserves_focus() {
        let mut agent = test_agent_view(None, PathBuf::from("/tmp"));
        upsert_artifact_block(&mut agent, block_with(&["a.docx", "b.xlsx"]));
        // Simulate the user having focused the second card.
        if let Some(e) = agent.scrollback.entry_mut(0)
            && let RenderBlock::Artifact(b) = &mut e.block
        {
            b.focused_idx = 1;
        }
        upsert_artifact_block(&mut agent, block_with(&["a.docx", "b.xlsx", "c.pdf"]));
        assert_eq!(agent.scrollback.len(), 1, "refresh must not duplicate");
        let RenderBlock::Artifact(b) = &agent.scrollback.entry(0).unwrap().block else {
            panic!("still an artifact block");
        };
        assert_eq!(b.cards.len(), 3);
        assert_eq!(b.focused_idx, 1, "focus survives reload");
    }

    #[test]
    fn upsert_clamps_focus_when_cards_shrink() {
        let mut agent = test_agent_view(None, PathBuf::from("/tmp"));
        upsert_artifact_block(&mut agent, block_with(&["a", "b", "c"]));
        if let Some(e) = agent.scrollback.entry_mut(0)
            && let RenderBlock::Artifact(b) = &mut e.block
        {
            b.focused_idx = 2;
        }
        upsert_artifact_block(&mut agent, block_with(&["a"]));
        let RenderBlock::Artifact(b) = &agent.scrollback.entry(0).unwrap().block else {
            panic!();
        };
        assert_eq!(b.focused_idx, 0, "focus clamps to the shrunk card list");
    }
}
