// Per-test-case module for the `pty_e2e` integration test crate.
#[allow(unused_imports)]
use super::common::*;

/// QidiWork office-artifact watch e2e (P2/P3) — DIAGNOSTIC MODE.
///
/// Drives the real binary through the full office flow and writes a screen
/// snapshot to C:\e2e-logs after EVERY step (soft waits: a timeout records
/// the miss and continues), so one run yields the complete visual trail:
/// welcome → turn → initial cards → live update → Tab → k x3 → Enter →
/// Esc → o → [ ] → quit.
///
/// Auth is injected via `QIDI_AUTH` (the highest-priority, file-free,
/// network-free path in AuthManager) with a far-future expiry: deterministic
/// welcome, no auth.x.ai call, no login screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn office_artifact_watch_e2e() {
    let content = ContentController::start().await.expect("start content");
    content.set_response(format!("{MOCK_RESPONSE_SENTINEL} office watch turn."));
    let binary = pager_binary().expect("resolve pager binary");

    let task = "pty-office-watch";
    let ws = content
        .home()
        .join(".qidi")
        .join("office-workspaces")
        .join(task);
    std::fs::create_dir_all(&ws).expect("create office workspace");

    let artifact_name = "office-e2e-report.txt";
    let artifact_path = ws.join(artifact_name);
    std::fs::write(&artifact_path, "office artifact fixture\n").expect("write artifact");
    let manifest = serde_json::json!({
        "task": task,
        "artifacts": [{
            "path": artifact_path.to_string_lossy(),
            "name": artifact_name,
            "size": 25u64,
            "mtime": 1788163298.6f64,
            "registered_at": 1788163299.5f64,
            "skill": "office-tools",
            "note": "e2e",
        }]
    });
    let manifest_path = ws.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest).unwrap())
        .expect("write manifest");

    let open_log = content.home().join("office-open.log");
    let shim = content.home().join("logopen.cmd");
    std::fs::write(
        &shim,
        format!("@echo off\r\necho %1 >> \"{}\"\r\n", open_log.to_string_lossy()),
    )
    .expect("write open shim");

    // Env: oauth base (drops XAI_API_KEY) + inline auth token + probe guard
    // + task binding + open seam.
    let mut env = oauth_env_for_pager(&content);
    let inline_auth = serde_json::json!({
        "key": "pty-office-e2e-token",
        "auth_mode": "oidc",
        "create_time": "2026-01-01T00:00:00Z",
        "user_id": "office-watch-e2e",
        "email": "office-watch-e2e@test.invalid",
        "expires_at": "2030-01-01T00:00:00Z",
        "refresh_token": "pty-office-e2e-refresh",
        "oidc_issuer": "http://localhost:22255",
        "oidc_client_id": "qidi-code"
    })
    .to_string();
    env.push(("QIDI_AUTH".into(), inline_auth));
    env.push(("QIDI_OFFICE_TASK".into(), task.into()));
    env.push(("QIDI_SKIP_TERMINAL_PROBE".into(), "1".into()));
    env.push((
        "QIDI_OFFICE_OPEN_CMD".into(),
        format!("cmd /c {}", shim.to_string_lossy()),
    ));
    let env_refs: Vec<(&str, &str)> =
        env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    let mut harness =
        PtyHarness::new(&binary, DEFAULT_ROWS, DEFAULT_COLS, &[], &env_refs)
            .expect("spawn pager");
    harness.set_respond_to_queries(true);

    let _ = std::fs::remove_dir_all(r"C:\e2e-logs");
    let _ = std::fs::create_dir_all(r"C:\e2e-logs");
    let mut summary = String::new();

    // Soft wait: pump the screen, record hit/miss, never fail.
    macro_rules! soft {
        ($marker:expr, $timeout:expr, $step:expr) => {{
            let hit = harness
                .wait_for_text($marker, Duration::from_secs($timeout))
                .is_ok();
            summary.push_str(&format!(
                "{} {} {}\n",
                $step,
                if hit { "OK" } else { "MISS" },
                $marker
            ));
            let _ = std::fs::write(
                format!(r"C:\e2e-logs\{}.txt", $step),
                harness.screen_contents(),
            );
            hit
        }};
    }
    macro_rules! tap {
        ($keys:expr, $step:expr) => {{
            harness.inject_keys($keys).expect("inject");
            harness.update(Duration::from_millis(600));
            let _ = std::fs::write(
                format!(r"C:\e2e-logs\{}.txt", $step),
                harness.screen_contents(),
            );
        }};
    }

    soft!(WELCOME_SCREEN_SENTINEL, 25, "01-welcome");

    // Turn
    harness
        .inject_keys(format!("{PROMPT}\r").as_bytes())
        .expect("submit prompt");
    soft!(MOCK_RESPONSE_SENTINEL, 30, "02-turn");

    // Initial mirror
    soft!(artifact_name, 20, "03-initial-cards");

    // Live update
    let second_name = "office-e2e-second.docx";
    std::fs::write(ws.join(second_name), b"second fixture").expect("write second artifact");
    let manifest2 = serde_json::json!({
        "task": task,
        "artifacts": [
            manifest["artifacts"][0].clone(),
            {
                "path": ws.join(second_name).to_string_lossy(),
                "name": second_name,
                "size": 14u64,
                "mtime": 1788163400.0f64,
                "registered_at": 1788163401.0f64,
                "skill": "bid-write",
                "note": "",
            }
        ]
    });
    std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest2).unwrap())
        .expect("rewrite manifest");
    soft!(second_name, 20, "04-live-second");

    // Interaction chain with per-step snapshots
    // Bare letters (k/j) are suppressed in default mode and forwarded to
    // the prompt; arrows are always active. Tab auto-selects the latest
    // user message. The scrollback entry order varies per run (artifact
    // block vs response vs turn-status interleave), so instead of a fixed
    // number of Down presses, retry: Down -> Enter -> check for the
    // preview panel; if another block's viewer opened, Esc and continue.
    tap!(b"\t", "05-tab");
    soft!("Enter", 3, "09-enter-pre"); // footer hints if any
    // Navigate to the artifact block and open its preview. The viewer
    // draw is tick-scheduled, so give each attempt a few seconds.
    let mut preview_hit = false;
    let mut attempts = 0;
    while attempts < 8 {
        attempts += 1;
        harness.inject_keys(b"\x1b[B").expect("down");
        harness.update(Duration::from_millis(500));
        harness.inject_keys(b"\r").expect("enter");
        if harness
            .wait_for_text("暂不支持终端预览", Duration::from_secs(4))
            .is_ok()
        {
            preview_hit = true;
            break;
        }
        // Some other block's viewer opened (e.g. the response's markdown
        // viewer) — close it and keep walking.
        harness.inject_keys(keys::ESC).expect("esc");
        harness.update(Duration::from_millis(500));
    }
    summary.push_str(&format!(
        "10-enter-preview {} (attempts: {})\n",
        if preview_hit { "OK" } else { "MISS" },
        attempts
    ));
    let _ = std::fs::write(
        r"C:\e2e-logs\10-enter.txt",
        harness.screen_contents(),
    );
    // Close the viewer (either the artifact preview or a leftover).
    tap!(keys::ESC, "11-esc");
    tap!(b"o", "12-o");
    tap!(b"]", "13-bracket-r");
    tap!(b"[", "14-bracket-l");

    // o shim result
    let logged = std::fs::read_to_string(&open_log).unwrap_or_default();
    summary.push_str(&format!("open-log: {}\n", logged.trim()));
    // Preserve the pager's unified log (office watch telemetry) for triage.
    let pager_log = content
        .home()
        .join(".grok")
        .join("logs")
        .join("unified.jsonl");
    if let Ok(s) = std::fs::read_to_string(&pager_log) {
        let _ = std::fs::write(r"C:\e2e-logs\pager-log.jsonl", s);
    }
    let _ = std::fs::write(r"C:\e2e-logs\SUMMARY.txt", &summary);
    println!("{summary}");

    assert!(
        !harness.contains_text("panicked"),
        "pager panicked\n{}",
        harness.screen_contents()
    );
    harness.quit().expect("clean quit");
}
