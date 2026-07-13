use super::super::*;
use super::*;
use std::{
    collections::VecDeque,
    fs,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
fn all_protocol_fixtures_are_valid_json_with_the_expected_message_shape() {
    for (fixture, method, has_id) in [
        (THREAD_START_REQUEST_FIXTURE, "thread/start", true),
        (TURN_START_REQUEST_FIXTURE, "turn/start", true),
        (THREAD_RESUME_REQUEST_FIXTURE, "thread/resume", true),
        (FILE_CHANGE_STARTED_FIXTURE, "item/started", false),
        (
            COMMAND_APPROVAL_REQUEST_FIXTURE,
            "item/commandExecution/requestApproval",
            true,
        ),
        (
            FILE_CHANGE_APPROVAL_REQUEST_FIXTURE,
            "item/fileChange/requestApproval",
            true,
        ),
        (
            PERMISSIONS_APPROVAL_REQUEST_FIXTURE,
            "item/permissions/requestApproval",
            true,
        ),
        (
            TOOL_USER_INPUT_REQUEST_FIXTURE,
            "item/tool/requestUserInput",
            true,
        ),
        (
            MCP_ELICITATION_REQUEST_FIXTURE,
            "mcpServer/elicitation/request",
            true,
        ),
        (
            SERVER_REQUEST_RESOLVED_FIXTURE,
            "serverRequest/resolved",
            false,
        ),
        (
            AUTO_APPROVAL_STARTED_FIXTURE,
            "item/autoApprovalReview/started",
            false,
        ),
        (
            AUTO_APPROVAL_COMPLETED_FIXTURE,
            "item/autoApprovalReview/completed",
            false,
        ),
    ] {
        let message: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(message.get("method").and_then(Value::as_str), Some(method));
        assert!(
            message.get("params").is_some(),
            "{method} must include params"
        );
        assert_eq!(
            message.get("id").is_some(),
            has_id,
            "unexpected id for {method}"
        );
    }
}

#[test]
fn auto_approval_started_and_completed_form_a_visible_lifecycle() {
    let started: Value = serde_json::from_str(AUTO_APPROVAL_STARTED_FIXTURE).unwrap();
    let completed: Value = serde_json::from_str(AUTO_APPROVAL_COMPLETED_FIXTURE).unwrap();

    assert_eq!(
        normalize_codex_notification_at(&started, "2026-07-13T00:00:00Z"),
        vec![AgentEvent::ApprovalReview {
            status: "inProgress".to_string(),
            action: Some("applyPatch".to_string()),
            rationale: None,
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
    assert_eq!(
        normalize_codex_notification_at(&completed, "2026-07-13T00:00:01Z"),
        vec![AgentEvent::ApprovalReview {
            status: "denied".to_string(),
            action: Some("applyPatch".to_string()),
            rationale: Some("The requested path is outside the allowed scope.".to_string()),
            created_at: "2026-07-13T00:00:01Z".to_string(),
        }]
    );
}

#[cfg(unix)]
#[test]
fn unknown_notification_does_not_block_the_next_known_notification() {
    let root = test_root("unknown-continuity");
    fs::create_dir_all(&root).unwrap();
    let log = AgentRunTransportLog::create(
        root.to_string_lossy().as_ref(),
        "unknown-continuity",
        "app_server",
    )
    .unwrap();
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut client = CodexAppServerClient::new(
        child.stdin.take().unwrap(),
        child.stdout.take().unwrap(),
        child.stderr.take().unwrap(),
        log,
        None,
        CodingAgentApprovalsReviewer::AutoReview,
    );

    client
        .route_message(
            json!({ "method": "future/item/progress", "params": { "value": 1 } }),
            "2026-07-13T00:00:00Z".to_string(),
            None,
        )
        .unwrap();
    client
        .route_message(
            json!({
                "method": "turn/started",
                "params": { "turn": { "id": "turn-after-unknown" } }
            }),
            "2026-07-13T00:00:01Z".to_string(),
            None,
        )
        .unwrap();

    let events = client.take_pending_agent_events();
    assert!(matches!(
        events.first(),
        Some(AgentEvent::Diagnostic { method: Some(method), .. })
            if method == "future/item/progress"
    ));
    assert!(matches!(
        events.get(1),
        Some(AgentEvent::TurnStarted { turn_id: Some(turn_id), .. })
            if turn_id == "turn-after-unknown"
    ));

    child.kill().unwrap();
    child.wait().unwrap();
    client.join_readers();
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn stderr_reader_streams_sanitized_lines_to_the_tail_and_transport_log() {
    let root = test_root("stderr-reader");
    fs::create_dir_all(&root).unwrap();
    let log = AgentRunTransportLog::create(
        root.to_string_lossy().as_ref(),
        "stderr-reader",
        "app_server",
    )
    .unwrap();
    let log_path = log.path.clone();
    let mut child = Command::new("sh")
        .args([
            "-c",
            "printf 'first line\\nOPENAI_API_KEY=sk-live-stderr-secret\\n' >&2",
        ])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let reader = spawn_app_server_stderr_reader(child.stderr.take().unwrap(), log, tail.clone());

    assert!(child.wait().unwrap().success());
    reader.join().unwrap();

    let tail = tail.lock().unwrap();
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.front().map(String::as_str), Some("first line"));
    assert!(tail.back().unwrap().contains("[REDACTED_CREDENTIAL_TEXT]"));
    assert!(!tail.back().unwrap().contains("sk-live-stderr-secret"));
    drop(tail);

    let content = fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("\"kind\":\"stderr\""));
    assert!(content.contains("first line"));
    assert!(content.contains("[REDACTED_CREDENTIAL_TEXT]"));
    assert!(!content.contains("sk-live-stderr-secret"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "uses a real Codex account and service to modify a temporary demo workspace"]
fn codex_app_server_demo_generation_smoke_from_env() {
    let root = test_root("demo-generation");
    fs::create_dir_all(&root).unwrap();
    let context = CodingAgentStartContext {
        project_path: root.to_string_lossy().to_string(),
        prompt: r#"Create a minimal Vite-compatible frontend demo in this empty directory.
Use the file editing tool, not shell redirection, to create exactly these files: package.json, index.html, and src/main.js.
The package.json dev script must be `vite --host 127.0.0.1`.
Do not install dependencies and do not start a dev server.
After editing, run exactly: node -e "console.log('voicecoder demo smoke ok')"
Finish only after that command succeeds."#
            .to_string(),
        sandbox: Some(CodingAgentSandboxMode::WorkspaceWrite),
    };

    let result = (|| -> Result<(), String> {
        let mut session = start_codex_app_server_session(context, "m8-demo-smoke", None)?;
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut saw_file_change = false;
        let mut saw_command = false;

        loop {
            if Instant::now() >= deadline {
                let _ = session.cancel();
                return Err("Codex app-server demo smoke test timed out.".to_string());
            }

            let events = session.read_next_agent_events()?;
            for event in &events {
                match event {
                    AgentEvent::ItemStarted { item_type, .. }
                    | AgentEvent::ItemCompleted { item_type, .. } => {
                        saw_file_change |= item_type == "fileChange";
                        saw_command |= item_type == "commandExecution";
                    }
                    AgentEvent::Error {
                        message,
                        terminal: true,
                        ..
                    } => {
                        let _ = session.cancel();
                        return Err(message.clone());
                    }
                    AgentEvent::TurnCompleted { status, .. } => {
                        session.cancel()?;
                        if status != "completed" {
                            return Err(format!("Codex turn ended with status `{status}`."));
                        }
                        if !saw_file_change || !saw_command {
                            return Err(format!(
                                "Missing live feedback: fileChange={saw_file_change}, commandExecution={saw_command}."
                            ));
                        }
                        for relative_path in ["package.json", "index.html", "src/main.js"] {
                            if !root.join(relative_path).is_file() {
                                return Err(format!("Expected generated file `{relative_path}`."));
                            }
                        }
                        return Ok(());
                    }
                    _ => {}
                }
            }
        }
    })();

    let _ = fs::remove_dir_all(root);
    result.unwrap();
}

fn test_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "voicecoder-m8-{label}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}
