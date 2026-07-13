use super::super::*;
use super::*;
use std::{
    fs,
    time::{Duration, Instant},
};

#[test]
fn provider_override_parser_accepts_known_values() {
    assert_eq!(
        CodingAgentProviderRegistry::parse_provider_override("codex_app_server"),
        Some(CodingAgentProviderKind::CodexAppServer)
    );
    assert_eq!(
        CodingAgentProviderRegistry::parse_provider_override(" codex_exec_json "),
        Some(CodingAgentProviderKind::CodexExecJson)
    );
    assert_eq!(
        CodingAgentProviderRegistry::parse_provider_override("auto"),
        None
    );
    assert_eq!(
        CodingAgentProviderRegistry::parse_provider_override("unknown"),
        None
    );
}

#[test]
fn auto_provider_resolves_to_app_server_by_default() {
    assert_eq!(
        CodingAgentProviderRegistry::resolve_provider(CodingAgentProviderKind::Auto),
        CodingAgentProviderKind::CodexAppServer
    );
}

#[test]
fn approval_policy_parser_accepts_current_schema_values() {
    assert_eq!(
        CodingAgentApprovalPolicy::parse("untrusted").unwrap(),
        CodingAgentApprovalPolicy::Untrusted
    );
    assert_eq!(
        CodingAgentApprovalPolicy::parse(" on_request ").unwrap(),
        CodingAgentApprovalPolicy::OnRequest
    );
    assert_eq!(
        CodingAgentApprovalPolicy::parse("never").unwrap(),
        CodingAgentApprovalPolicy::Never
    );
    assert!(CodingAgentApprovalPolicy::parse("on-failure").is_err());
}

#[test]
fn approvals_reviewer_parser_accepts_auto_review_aliases() {
    assert_eq!(
        CodingAgentApprovalsReviewer::parse("auto_review").unwrap(),
        CodingAgentApprovalsReviewer::AutoReview
    );
    assert_eq!(
        CodingAgentApprovalsReviewer::parse("approve-for-me").unwrap(),
        CodingAgentApprovalsReviewer::AutoReview
    );
    assert_eq!(
        CodingAgentApprovalsReviewer::parse("user").unwrap(),
        CodingAgentApprovalsReviewer::User
    );
    assert!(CodingAgentApprovalsReviewer::parse("always_allow").is_err());
}

#[test]
fn thread_and_turn_share_explicit_permission_settings() {
    let permission_settings = CodingAgentPermissionSettings {
        approval_policy: CodingAgentApprovalPolicy::Never,
        approvals_reviewer: CodingAgentApprovalsReviewer::User,
    };
    let thread_params = build_thread_start_params(
        "/tmp/voicecoder-demo",
        CodingAgentSandboxMode::WorkspaceWrite,
        permission_settings,
    );
    let turn_params = build_turn_start_params(
        "thread-1",
        "/tmp/voicecoder-demo",
        CodingAgentSandboxMode::WorkspaceWrite,
        permission_settings,
        "Build the demo",
    );

    for params in [thread_params, turn_params] {
        assert_eq!(
            params.get("approvalPolicy").and_then(Value::as_str),
            Some("never")
        );
        assert_eq!(
            params.get("approvalsReviewer").and_then(Value::as_str),
            Some("user")
        );
    }
}

#[test]
fn provider_diagnostics_include_stable_metadata() {
    let app_server_diagnostic = CodexAppServerProvider.diagnostic();
    let exec_json_diagnostic = CodexExecJsonProvider.diagnostic();

    assert_eq!(
        app_server_diagnostic.provider,
        CodingAgentProviderKind::CodexAppServer
    );
    assert_eq!(
        app_server_diagnostic
            .details
            .get("transport")
            .map(String::as_str),
        Some("stdio")
    );
    assert_eq!(
        app_server_diagnostic
            .details
            .get("command")
            .map(String::as_str),
        Some("codex app-server --stdio")
    );
    assert!(app_server_diagnostic.details.contains_key("approvalPolicy"));
    assert!(app_server_diagnostic
        .details
        .contains_key("approvalsReviewer"));
    assert_eq!(
        app_server_diagnostic
            .details
            .get("defaultSandbox")
            .map(String::as_str),
        Some("workspace-write")
    );
    assert_eq!(
        exec_json_diagnostic.provider,
        CodingAgentProviderKind::CodexExecJson
    );
    assert_eq!(
        exec_json_diagnostic
            .details
            .get("transport")
            .map(String::as_str),
        Some("process-jsonl")
    );
    assert!(app_server_diagnostic.executable.is_some());
    assert!(exec_json_diagnostic.executable.is_some());
}

#[test]
fn default_permission_settings_use_silent_auto_review() {
    let settings = CodingAgentPermissionSettings::default();

    assert_eq!(
        settings.approval_policy,
        CodingAgentApprovalPolicy::OnRequest
    );
    assert_eq!(
        settings.approvals_reviewer,
        CodingAgentApprovalsReviewer::AutoReview
    );
}

#[test]
fn builds_initialize_request_with_voicecoder_client_info() {
    let request = build_initialize_request();

    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("initialize")
    );
    assert_eq!(
        request.get("id").and_then(Value::as_u64),
        Some(FIRST_APP_SERVER_REQUEST_ID)
    );
    assert_eq!(
        request
            .pointer("/params/clientInfo/name")
            .and_then(Value::as_str),
        Some("voicecoder")
    );
    assert_eq!(
        request
            .pointer("/params/capabilities/experimentalApi")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        request
            .pointer("/params/capabilities/requestAttestation")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn validate_initialize_response_accepts_result() {
    let response = json!({
        "id": FIRST_APP_SERVER_REQUEST_ID,
        "result": {
            "platformFamily": "macos"
        }
    });

    assert!(validate_initialize_response(response).is_ok());
}

#[test]
fn validate_initialize_response_rejects_json_rpc_error() {
    let response = json!({
        "id": FIRST_APP_SERVER_REQUEST_ID,
        "error": {
            "code": -32000,
            "message": "Not initialized"
        }
    });

    assert_eq!(
        validate_initialize_response(response).unwrap_err(),
        "Codex app-server initialize 失败：Not initialized"
    );
}

#[test]
fn builds_json_rpc_request_with_supplied_id_method_and_params() {
    let request = build_json_rpc_request(7, "thread/start", json!({ "cwd": "/tmp/project" }));

    assert_eq!(request.get("id").and_then(Value::as_u64), Some(7));
    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("thread/start")
    );
    assert_eq!(
        request.pointer("/params/cwd").and_then(Value::as_str),
        Some("/tmp/project")
    );
}

#[test]
fn classifies_response_notification_server_request_and_unknown_messages() {
    assert_eq!(
        classify_json_rpc_message(&json!({ "id": 7, "result": {} })),
        JsonRpcInboundKind::Response { request_id: 7 }
    );
    assert_eq!(
        classify_json_rpc_message(&json!({
            "method": "turn/started",
            "params": { "turn": { "id": "turn-1" } }
        })),
        JsonRpcInboundKind::Notification {
            method: "turn/started".to_string()
        }
    );
    assert_eq!(
        classify_json_rpc_message(&json!({
            "id": "approval-1",
            "method": "item/fileChange/requestApproval",
            "params": {}
        })),
        JsonRpcInboundKind::ServerRequest {
            request_id: Value::String("approval-1".to_string()),
            method: "item/fileChange/requestApproval".to_string()
        }
    );
    assert!(matches!(
        classify_json_rpc_message(&json!({ "unexpected": true })),
        JsonRpcInboundKind::Unknown { .. }
    ));
}

#[test]
fn builds_json_rpc_error_response_for_expired_server_request() {
    assert_eq!(
        build_json_rpc_error_response(Value::String("approval-1".to_string()), -32002, "Timed out"),
        json!({
            "id": "approval-1",
            "error": {
                "code": -32002,
                "message": "Timed out"
            }
        })
    );
}

#[test]
fn approval_requests_are_routed_to_auto_review_by_default() {
    let request: Value = serde_json::from_str(COMMAND_APPROVAL_REQUEST_FIXTURE).unwrap();
    let params = request.get("params").unwrap();

    assert_eq!(
        classify_server_request_handling(
            "item/commandExecution/requestApproval",
            params,
            CodingAgentApprovalsReviewer::AutoReview,
        ),
        ServerRequestHandling::AutoReview
    );
    assert_eq!(
        classify_server_request_handling(
            "item/commandExecution/requestApproval",
            params,
            CodingAgentApprovalsReviewer::User,
        ),
        ServerRequestHandling::UserDecision
    );

    let event = build_server_request_event(
        request.get("id").unwrap().clone(),
        "item/commandExecution/requestApproval".to_string(),
        params.clone(),
        ServerRequestHandling::AutoReview,
        "2026-07-13T00:02:00Z".to_string(),
        "2026-07-13T00:00:00Z".to_string(),
    );
    assert!(matches!(
        event,
        AgentEvent::ServerRequest {
            kind,
            status,
            requires_user_input: false,
            auto_review: true,
            ..
        } if kind == "command_approval" && status == "auto_reviewing"
    ));
}

#[test]
fn server_request_fixtures_cover_approval_input_elicitation_and_resolution() {
    for (fixture, method) in [
        (
            COMMAND_APPROVAL_REQUEST_FIXTURE,
            "item/commandExecution/requestApproval",
        ),
        (
            FILE_CHANGE_APPROVAL_REQUEST_FIXTURE,
            "item/fileChange/requestApproval",
        ),
        (
            PERMISSIONS_APPROVAL_REQUEST_FIXTURE,
            "item/permissions/requestApproval",
        ),
        (
            TOOL_USER_INPUT_REQUEST_FIXTURE,
            "item/tool/requestUserInput",
        ),
        (
            MCP_ELICITATION_REQUEST_FIXTURE,
            "mcpServer/elicitation/request",
        ),
    ] {
        let request: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(request.get("method").and_then(Value::as_str), Some(method));
        assert!(validate_request_id(request.get("id").unwrap()).is_ok());
    }

    let notification: Value = serde_json::from_str(SERVER_REQUEST_RESOLVED_FIXTURE).unwrap();
    assert_eq!(
        normalize_codex_notification_at(&notification, "2026-07-13T00:00:03Z"),
        vec![AgentEvent::ServerRequestResolved {
            request_id: json!(9002),
            request_key: "number:9002".to_string(),
            status: "resolved".to_string(),
            resolution: Some("server".to_string()),
            message: Some("Codex 已完成该请求的处理".to_string()),
            created_at: "2026-07-13T00:00:03Z".to_string(),
        }]
    );
}

#[test]
fn typed_approval_responses_match_current_app_server_schema() {
    let command = pending_request_from_fixture(
        COMMAND_APPROVAL_REQUEST_FIXTURE,
        ServerRequestHandling::UserDecision,
    );
    let accepted = build_server_request_response(
        &command,
        &test_resolution(command.request_id.clone(), "accept"),
    )
    .unwrap();
    assert_eq!(
        accepted.response.pointer("/result/decision"),
        Some(&json!("accept"))
    );
    assert_eq!(accepted.log_payload, accepted.response);

    let file = pending_request_from_fixture(
        FILE_CHANGE_APPROVAL_REQUEST_FIXTURE,
        ServerRequestHandling::UserDecision,
    );
    let accepted_for_session = build_server_request_response(
        &file,
        &test_resolution(file.request_id.clone(), "acceptForSession"),
    )
    .unwrap();
    assert_eq!(
        accepted_for_session.response.pointer("/result/decision"),
        Some(&json!("acceptForSession"))
    );
}

#[test]
fn permissions_response_grants_only_requested_profile_or_denies_with_empty_profile() {
    let request = pending_request_from_fixture(
        PERMISSIONS_APPROVAL_REQUEST_FIXTURE,
        ServerRequestHandling::UserDecision,
    );
    let mut accept = test_resolution(request.request_id.clone(), "acceptForSession");
    accept.scope = Some("session".to_string());
    let granted = build_server_request_response(&request, &accept).unwrap();
    assert_eq!(
        granted.response.pointer("/result/scope"),
        Some(&json!("session"))
    );
    assert_eq!(
        granted
            .response
            .pointer("/result/permissions/network/enabled"),
        Some(&json!(true))
    );
    assert_eq!(
        granted
            .response
            .pointer("/result/permissions/fileSystem/write/0"),
        Some(&json!("/tmp/voicecoder-demo/generated"))
    );

    let denied = build_server_request_response(
        &request,
        &test_resolution(request.request_id.clone(), "decline"),
    )
    .unwrap();
    assert_eq!(
        denied.response.pointer("/result/permissions"),
        Some(&json!({}))
    );
    assert_eq!(
        denied.response.pointer("/result/scope"),
        Some(&json!("turn"))
    );
}

#[test]
fn user_input_response_is_typed_redacted_and_auto_resolves_to_recommended_option() {
    let request = pending_request_from_fixture(
        TOOL_USER_INPUT_REQUEST_FIXTURE,
        ServerRequestHandling::UserInput { auto_resolve: true },
    );
    let mut resolution = test_resolution(request.request_id.clone(), "submit");
    resolution
        .answers
        .insert("theme".to_string(), vec!["深色".to_string()]);
    let answered = build_server_request_response(&request, &resolution).unwrap();
    assert_eq!(
        answered.response.pointer("/result/answers/theme/answers/0"),
        Some(&json!("深色"))
    );
    assert_eq!(
        answered.log_payload.pointer("/result"),
        Some(&json!("[REDACTED_USER_INPUT]"))
    );

    let timed_out = build_server_request_timeout_response(&request).unwrap();
    assert_eq!(timed_out.status, "auto_resolved");
    assert_eq!(
        timed_out
            .response
            .pointer("/result/answers/theme/answers/0"),
        Some(&json!("浅色"))
    );
}

#[test]
fn mcp_elicitation_response_keeps_content_out_of_transport_log() {
    let request = pending_request_from_fixture(
        MCP_ELICITATION_REQUEST_FIXTURE,
        ServerRequestHandling::McpElicitation,
    );
    let mut resolution = test_resolution(request.request_id.clone(), "accept");
    resolution.content = Some(json!({ "environment": "staging" }));
    let accepted = build_server_request_response(&request, &resolution).unwrap();
    assert_eq!(
        accepted.response.pointer("/result/action"),
        Some(&json!("accept"))
    );
    assert_eq!(
        accepted.response.pointer("/result/content/environment"),
        Some(&json!("staging"))
    );
    assert_eq!(
        accepted.log_payload.pointer("/result"),
        Some(&json!("[REDACTED_USER_INPUT]"))
    );
}

#[test]
fn request_registry_routes_resolutions_and_removes_finished_runs() {
    let state = CodingAgentRequestState::default();
    let receiver = state.register("run-request-test").unwrap();
    state
        .resolve("run-request-test", test_resolution(json!(42), "decline"))
        .unwrap();
    assert_eq!(receiver.recv().unwrap().request_id, json!(42));
    state.unregister("run-request-test");
    assert!(state
        .resolve("run-request-test", test_resolution(json!(42), "decline"),)
        .is_err());
}

fn pending_request_from_fixture(
    fixture: &str,
    handling: ServerRequestHandling,
) -> PendingServerRequest {
    let request: Value = serde_json::from_str(fixture).unwrap();
    PendingServerRequest {
        request_id: request.get("id").unwrap().clone(),
        method: request
            .get("method")
            .and_then(Value::as_str)
            .unwrap()
            .to_string(),
        params: request.get("params").unwrap().clone(),
        handling,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn test_resolution(request_id: Value, action: &str) -> ServerRequestResolution {
    ServerRequestResolution {
        request_id,
        action: action.to_string(),
        answers: BTreeMap::new(),
        content: None,
        scope: None,
    }
}

#[test]
fn transport_log_records_jsonl_and_sanitizes_run_id() {
    let root = std::env::temp_dir().join(format!(
        "voicecoder-transport-log-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&root).unwrap();
    let log =
        AppServerTransportLog::create(root.to_string_lossy().as_ref(), "run/with unsafe chars")
            .unwrap();
    let log_path = log.path.clone();

    log.record("inbound", "message", json!({ "method": "turn/started" }))
        .unwrap();
    let content = fs::read_to_string(&log_path).unwrap();

    assert!(log_path.ends_with("agent_run_run_with_unsafe_chars_app_server.jsonl"));
    assert!(content.contains("\"direction\":\"inbound\""));
    assert!(content.contains("\"method\":\"turn/started\""));

    drop(log);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn heartbeat_tracks_last_message_and_effective_progress() {
    let heartbeat = SharedAppServerHeartbeat::default();
    heartbeat.record_message(
        &json!({ "method": "item/started", "params": {} }),
        "2026-07-13T00:00:00Z",
    );
    heartbeat.record_progress("2026-07-13T00:00:01Z");
    let snapshot = heartbeat.snapshot();

    assert_eq!(
        snapshot.last_message_at.as_deref(),
        Some("2026-07-13T00:00:00Z")
    );
    assert_eq!(snapshot.last_method.as_deref(), Some("item/started"));
    assert_eq!(
        snapshot.last_progress_at.as_deref(),
        Some("2026-07-13T00:00:01Z")
    );
}

#[test]
#[ignore = "uses a real Codex account, service, and app-server process"]
fn codex_app_server_transport_smoke_from_env() {
    let root = std::env::temp_dir().join(format!(
        "voicecoder-app-server-smoke-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&root).unwrap();
    let context = CodingAgentStartContext {
        project_path: root.to_string_lossy().to_string(),
        prompt: "Do not modify files. Reply with exactly: transport ok".to_string(),
        sandbox: Some(CodingAgentSandboxMode::ReadOnly),
    };
    let result = (|| -> Result<(), String> {
        let mut session = start_codex_app_server_session(context, "smoke", None)?;
        let deadline = Instant::now() + Duration::from_secs(120);

        loop {
            if Instant::now() >= deadline {
                let _ = session.cancel();
                return Err("Codex app-server transport smoke test timed out.".to_string());
            }

            let events = session.read_next_agent_events()?;
            if events
                .iter()
                .any(|event| matches!(event, AgentEvent::TurnCompleted { .. }))
            {
                session.cancel()?;
                return Ok(());
            }
            if let Some(message) = events.iter().find_map(|event| match event {
                AgentEvent::Error { message, .. } => Some(message.clone()),
                _ => None,
            }) {
                let _ = session.cancel();
                return Err(message);
            }
        }
    })();

    let _ = fs::remove_dir_all(root);
    result.unwrap();
}

#[test]
fn thread_start_request_matches_codex_0_144_1_fixture() {
    let actual = build_json_rpc_request(
        1,
        "thread/start",
        build_thread_start_params(
            "/tmp/voicecoder-demo",
            CodingAgentSandboxMode::WorkspaceWrite,
            CodingAgentPermissionSettings::default(),
        ),
    );
    let expected: Value = serde_json::from_str(THREAD_START_REQUEST_FIXTURE).unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn turn_start_request_matches_codex_0_144_1_fixture() {
    let actual = build_json_rpc_request(
        2,
        "turn/start",
        build_turn_start_params(
            "thread-fixture",
            "/tmp/voicecoder-demo",
            CodingAgentSandboxMode::WorkspaceWrite,
            CodingAgentPermissionSettings::default(),
            "Build the demo",
        ),
    );
    let expected: Value = serde_json::from_str(TURN_START_REQUEST_FIXTURE).unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn file_change_notification_fixture_normalizes_for_the_frontend() {
    let notification: Value = serde_json::from_str(FILE_CHANGE_STARTED_FIXTURE).unwrap();
    let events = normalize_codex_notification_at(&notification, "2026-07-13T00:00:00Z");
    let item = notification.pointer("/params/item").unwrap().clone();

    assert_eq!(
        events,
        vec![AgentEvent::ItemStarted {
            thread_id: "thread-fixture".to_string(),
            turn_id: "turn-fixture".to_string(),
            item_id: "item-file-change-fixture".to_string(),
            item_type: "fileChange".to_string(),
            lifecycle: "in_progress".to_string(),
            status: Some("inProgress".to_string()),
            started_at: "2026-07-13T00:00:00+00:00".to_string(),
            item,
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn file_change_approval_fixture_locks_server_request_shape() {
    let request: Value = serde_json::from_str(FILE_CHANGE_APPROVAL_REQUEST_FIXTURE).unwrap();

    assert_eq!(request.get("id").and_then(Value::as_u64), Some(9001));
    assert_eq!(
        request.get("method").and_then(Value::as_str),
        Some("item/fileChange/requestApproval")
    );
    assert_eq!(
        request.pointer("/params/itemId").and_then(Value::as_str),
        Some("item-file-change-fixture")
    );
    assert_eq!(
        request
            .pointer("/params/startedAtMs")
            .and_then(Value::as_u64),
        Some(1_783_900_800_000)
    );
}

#[test]
fn auto_approval_rejection_fixture_becomes_visible_agent_event() {
    let notification: Value = serde_json::from_str(AUTO_APPROVAL_COMPLETED_FIXTURE).unwrap();
    let events = normalize_codex_notification_at(&notification, "2026-07-13T00:00:00Z");

    assert_eq!(
        events,
        vec![AgentEvent::ApprovalReview {
            status: "denied".to_string(),
            action: Some("applyPatch".to_string()),
            rationale: Some("The requested path is outside the allowed scope.".to_string()),
            created_at: "2026-07-13T00:00:00Z".to_string(),
        }]
    );
}

#[test]
fn builds_thread_start_params_with_cwd_and_workspace_sandbox() {
    let params = build_thread_start_params(
        "/tmp/voicecoder-demo",
        CodingAgentSandboxMode::WorkspaceWrite,
        CodingAgentPermissionSettings::default(),
    );

    assert_eq!(
        params.get("cwd").and_then(Value::as_str),
        Some("/tmp/voicecoder-demo")
    );
    assert_eq!(
        params.get("sandbox").and_then(Value::as_str),
        Some("workspace-write")
    );
    assert_eq!(
        params.get("approvalPolicy").and_then(Value::as_str),
        Some("on-request")
    );
    assert_eq!(
        params.get("approvalsReviewer").and_then(Value::as_str),
        Some("auto_review")
    );
    assert_eq!(
        params
            .pointer("/runtimeWorkspaceRoots/0")
            .and_then(Value::as_str),
        Some("/tmp/voicecoder-demo")
    );
    assert_eq!(
        params.get("threadSource").and_then(Value::as_str),
        Some("user")
    );
}

#[test]
fn builds_turn_start_params_with_thread_cwd_prompt_and_sandbox_policy() {
    let params = build_turn_start_params(
        "thread-1",
        "/tmp/voicecoder-demo",
        CodingAgentSandboxMode::WorkspaceWrite,
        CodingAgentPermissionSettings::default(),
        "Build the demo",
    );

    assert_eq!(
        params.get("threadId").and_then(Value::as_str),
        Some("thread-1")
    );
    assert_eq!(
        params.get("cwd").and_then(Value::as_str),
        Some("/tmp/voicecoder-demo")
    );
    assert_eq!(
        params.pointer("/input/0/type").and_then(Value::as_str),
        Some("text")
    );
    assert_eq!(
        params.pointer("/input/0/text").and_then(Value::as_str),
        Some("Build the demo")
    );
    assert_eq!(
        params.get("approvalPolicy").and_then(Value::as_str),
        Some("on-request")
    );
    assert_eq!(
        params.get("approvalsReviewer").and_then(Value::as_str),
        Some("auto_review")
    );
    assert_eq!(
        params
            .pointer("/sandboxPolicy/type")
            .and_then(Value::as_str),
        Some("workspaceWrite")
    );
    assert_eq!(
        params
            .pointer("/sandboxPolicy/writableRoots/0")
            .and_then(Value::as_str),
        Some("/tmp/voicecoder-demo")
    );
}

#[test]
fn maps_danger_full_access_sandbox_for_thread_and_turn_protocols() {
    let thread_params = build_thread_start_params(
        "/tmp/voicecoder-demo",
        CodingAgentSandboxMode::DangerFullAccess,
        CodingAgentPermissionSettings::default(),
    );
    let turn_params = build_turn_start_params(
        "thread-1",
        "/tmp/voicecoder-demo",
        CodingAgentSandboxMode::DangerFullAccess,
        CodingAgentPermissionSettings::default(),
        "Build the demo",
    );

    assert_eq!(
        thread_params.get("sandbox").and_then(Value::as_str),
        Some("danger-full-access")
    );
    assert_eq!(
        turn_params
            .pointer("/sandboxPolicy/type")
            .and_then(Value::as_str),
        Some("dangerFullAccess")
    );
}

#[test]
fn builds_codex_exec_json_args_with_project_sandbox_and_prompt() {
    let args = build_codex_exec_json_args(
        &CodingAgentStartContext {
            project_path: "/tmp/voicecoder-demo".to_string(),
            prompt: "Build the demo".to_string(),
            sandbox: Some(CodingAgentSandboxMode::WorkspaceWrite),
        },
        CodingAgentPermissionSettings::default(),
    );

    assert_eq!(
        args,
        vec![
            "--ask-for-approval",
            "on-request",
            "--config",
            "approvals_reviewer=\"auto_review\"",
            "exec",
            "--json",
            "--sandbox",
            "workspace-write",
            "--cd",
            "/tmp/voicecoder-demo",
            "Build the demo"
        ]
    );
}

#[test]
fn validates_coding_agent_start_context_requires_project_path_and_prompt() {
    assert!(
        validate_coding_agent_start_context(&CodingAgentStartContext {
            project_path: "/tmp/project".to_string(),
            prompt: "Build it".to_string(),
            sandbox: None,
        })
        .is_ok()
    );

    assert_eq!(
        validate_coding_agent_start_context(&CodingAgentStartContext {
            project_path: " ".to_string(),
            prompt: "Build it".to_string(),
            sandbox: None,
        })
        .unwrap_err(),
        "Coding Agent 启动失败：项目路径不能为空。"
    );
    assert_eq!(
        validate_coding_agent_start_context(&CodingAgentStartContext {
            project_path: "/tmp/project".to_string(),
            prompt: " ".to_string(),
            sandbox: None,
        })
        .unwrap_err(),
        "Coding Agent 启动失败：prompt 不能为空。"
    );
}

#[test]
fn extracts_thread_and_turn_ids_from_app_server_responses() {
    let thread_response = json!({
        "id": 1,
        "result": {
            "thread": {
                "id": "thread-1"
            }
        }
    });
    let turn_response = json!({
        "id": 2,
        "result": {
            "turn": {
                "id": "turn-1"
            }
        }
    });

    assert_eq!(
        extract_json_pointer_string(&thread_response, "/result/thread/id", "missing").unwrap(),
        "thread-1"
    );
    assert_eq!(
        extract_json_pointer_string(&turn_response, "/result/turn/id", "missing").unwrap(),
        "turn-1"
    );
    assert_eq!(
        extract_json_pointer_string(&turn_response, "/result/thread/id", "missing").unwrap_err(),
        "missing"
    );
}

#[test]
fn builds_json_rpc_notification_without_request_id() {
    let notification = build_initialized_notification();

    assert_eq!(
        notification.get("method").and_then(Value::as_str),
        Some("initialized")
    );
    assert!(notification.get("id").is_none());
    assert_eq!(notification.get("params"), Some(&json!({})));
}

#[test]
fn validate_json_rpc_response_accepts_result_message() {
    let response = json!({
        "id": 2,
        "result": {
            "threadId": "thread-1"
        }
    });

    assert_eq!(
        validate_json_rpc_response(response)
            .unwrap()
            .pointer("/result/threadId")
            .and_then(Value::as_str),
        Some("thread-1")
    );
}

#[test]
fn validate_json_rpc_response_rejects_error_message() {
    let response = json!({
        "id": 3,
        "error": {
            "code": -32001,
            "message": "boom"
        }
    });

    assert_eq!(
        validate_json_rpc_response(response).unwrap_err(),
        "Codex app-server request 失败：boom"
    );
}

#[test]
fn validate_json_rpc_response_formats_error_objects_without_message() {
    let response = json!({
        "id": 3,
        "error": {
            "code": -32603,
            "data": {
                "reason": "internal"
            }
        }
    });

    assert_eq!(
        validate_json_rpc_response(response).unwrap_err(),
        "Codex app-server request 失败：{\"code\":-32603,\"data\":{\"reason\":\"internal\"}}"
    );
}

#[test]
fn validate_json_rpc_response_rejects_message_without_result() {
    let response = json!({
        "method": "turn/started",
        "params": {
            "turnId": "turn-1"
        }
    });

    assert_eq!(
        validate_json_rpc_response(response).unwrap_err(),
        "Codex app-server 响应缺少 result。"
    );
}
