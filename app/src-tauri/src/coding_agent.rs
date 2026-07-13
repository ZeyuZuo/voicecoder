//! Coding-agent provider orchestration and Tauri command entry points.

use crate::env_config::read_local_env;
use chrono::Utc;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::{collections::BTreeMap, process::Command, sync::mpsc::Receiver, thread, time::Duration};
use tauri::{AppHandle, Emitter, State};

mod model;
mod protocol;
mod session;
#[cfg(test)]
mod tests;
mod transport;

pub(crate) use model::CodingAgentRequestState;
use model::*;
use protocol::*;
use session::*;
#[cfg(test)]
use transport::*;

const DEFAULT_CODEX_BIN: &str = "codex";
const CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION: &str = "codex-cli 0.144.1";
const FIRST_APP_SERVER_REQUEST_ID: u64 = 0;
const DEFAULT_CODEX_SANDBOX: CodingAgentSandboxMode = CodingAgentSandboxMode::WorkspaceWrite;
const CODEX_APP_SERVER_TRANSPORT: &str = "stdio";
const CODEX_APPROVAL_POLICY_ENV: &str = "VOICECODER_CODEX_APPROVAL_POLICY";
const CODEX_APPROVALS_REVIEWER_ENV: &str = "VOICECODER_CODEX_APPROVALS_REVIEWER";
const APP_SERVER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const APP_SERVER_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const APP_SERVER_AUTO_REVIEW_TIMEOUT: Duration = Duration::from_secs(120);
const APP_SERVER_USER_DECISION_TIMEOUT: Duration = Duration::from_secs(300);
const APP_SERVER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const APP_SERVER_TRANSPORT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const APP_SERVER_STDERR_TAIL_LINES: usize = 20;
const APP_SERVER_LAST_MESSAGE_LIMIT: usize = 2_000;
const AGENT_UI_TEXT_PREVIEW_CHARS: usize = 1_200;
const AGENT_UI_LONG_TEXT_CHARS: usize = 32_000;
const AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS: usize = 12_000;
const AGENT_UI_STRUCTURED_TOTAL_CHARS: usize = 4_000;
const AGENT_UI_STRUCTURED_MAX_NODES: usize = 160;
const AGENT_UI_STRUCTURED_MAX_DEPTH: usize = 5;
const AGENT_UI_STRUCTURED_MAX_FIELDS: usize = 32;
const AGENT_UI_STRUCTURED_MAX_ITEMS: usize = 24;
const AGENT_UI_REASONING_PARTS: usize = 64;
const AGENT_UI_PLAN_STEPS: usize = 100;
const AGENT_UI_MODEL_STATUS_ITEMS: usize = 32;
const AGENT_UI_MODEL_IDENTIFIER_CHARS: usize = 256;
const AGENT_UI_STATUS_MESSAGE_CHARS: usize = 4_000;
const AGENT_UI_HOOK_ENTRIES: usize = 50;
const AGENT_UI_HOOK_ENTRY_CHARS: usize = 2_000;
const AGENT_UI_HOOK_TOTAL_CHARS: usize = 12_000;
const AGENT_RUN_STARTED_EVENT: &str = "agent://run-started";
const AGENT_EVENT_EVENT: &str = "agent://event";
const AGENT_RUN_COMPLETED_EVENT: &str = "agent://run-completed";
const AGENT_ERROR_EVENT: &str = "agent://error";

pub(crate) struct CodingAgentProviderRegistry;

impl CodingAgentProviderRegistry {
    pub(crate) fn provider_for_kind(
        provider: CodingAgentProviderKind,
    ) -> Result<Box<dyn CodingAgentProvider + Send>, String> {
        match provider {
            CodingAgentProviderKind::Auto => {
                Err("auto coding agent provider must be resolved before use".to_string())
            }
            CodingAgentProviderKind::CodexAppServer => Ok(Box::new(CodexAppServerProvider)),
            CodingAgentProviderKind::CodexExecJson => Ok(Box::new(CodexExecJsonProvider)),
        }
    }

    pub(crate) fn diagnostics() -> Vec<CodingAgentProviderDiagnostic> {
        vec![
            CodexAppServerProvider.diagnostic(),
            CodexExecJsonProvider.diagnostic(),
        ]
    }

    pub(crate) fn resolve_provider(provider: CodingAgentProviderKind) -> CodingAgentProviderKind {
        match provider {
            CodingAgentProviderKind::Auto => Self::provider_override_from_env()
                .unwrap_or(CodingAgentProviderKind::CodexAppServer),
            explicit_provider => explicit_provider,
        }
    }

    pub(crate) fn provider_override_from_env() -> Option<CodingAgentProviderKind> {
        read_local_env("VOICECODER_CODING_AGENT_PROVIDER")
            .and_then(|value| Self::parse_provider_override(&value))
    }

    fn parse_provider_override(value: &str) -> Option<CodingAgentProviderKind> {
        match value.trim().to_lowercase().as_str() {
            "codex_app_server" => Some(CodingAgentProviderKind::CodexAppServer),
            "codex_exec_json" => Some(CodingAgentProviderKind::CodexExecJson),
            "auto" => None,
            _ => None,
        }
    }
}

pub(crate) struct CodexAppServerProvider;

impl CodingAgentProvider for CodexAppServerProvider {
    fn kind(&self) -> CodingAgentProviderKind {
        CodingAgentProviderKind::CodexAppServer
    }

    fn validate_start(&self) -> Result<(), String> {
        validate_codex_executable()?;
        resolve_coding_agent_permission_settings()?;
        Ok(())
    }

    fn diagnostic(&self) -> CodingAgentProviderDiagnostic {
        let mut diagnostic = codex_diagnostic(
            self.kind(),
            [
                ("transport", CODEX_APP_SERVER_TRANSPORT),
                ("command", "codex app-server --stdio"),
                ("threadMode", "persistent"),
                (
                    "defaultSandbox",
                    DEFAULT_CODEX_SANDBOX.app_server_thread_sandbox(),
                ),
            ],
        );

        apply_permission_settings_to_diagnostic(&mut diagnostic);
        diagnostic.details.insert(
            "responseTimeoutSeconds".to_string(),
            APP_SERVER_RESPONSE_TIMEOUT.as_secs().to_string(),
        );
        diagnostic.details.insert(
            "serverRequestTimeoutSeconds".to_string(),
            APP_SERVER_SERVER_REQUEST_TIMEOUT.as_secs().to_string(),
        );
        diagnostic.details.insert(
            "heartbeatIntervalSeconds".to_string(),
            APP_SERVER_HEARTBEAT_INTERVAL.as_secs().to_string(),
        );
        diagnostic
    }

    fn start_session(
        &self,
        context: CodingAgentStartContext,
    ) -> Result<Box<dyn CodingAgentSession + Send>, String> {
        self.validate_start()?;
        let log_id = format!("provider-{}", Utc::now().timestamp_millis());
        Ok(Box::new(start_codex_app_server_session(
            context, &log_id, None,
        )?))
    }
}

pub(crate) struct CodexExecJsonProvider;

impl CodingAgentProvider for CodexExecJsonProvider {
    fn kind(&self) -> CodingAgentProviderKind {
        CodingAgentProviderKind::CodexExecJson
    }

    fn validate_start(&self) -> Result<(), String> {
        validate_codex_executable()?;
        resolve_coding_agent_permission_settings()?;
        Ok(())
    }

    fn diagnostic(&self) -> CodingAgentProviderDiagnostic {
        let mut diagnostic = codex_diagnostic(
            self.kind(),
            [
                ("transport", "process-jsonl"),
                (
                    "command",
                    "codex --ask-for-approval on-request exec --json --sandbox workspace-write --cd <project> <prompt>",
                ),
                ("threadMode", "single-run"),
                (
                    "defaultSandbox",
                    DEFAULT_CODEX_SANDBOX.app_server_thread_sandbox(),
                ),
            ],
        );

        apply_permission_settings_to_diagnostic(&mut diagnostic);
        diagnostic
    }

    fn start_session(
        &self,
        context: CodingAgentStartContext,
    ) -> Result<Box<dyn CodingAgentSession + Send>, String> {
        self.validate_start()?;
        let log_id = format!("provider-{}", Utc::now().timestamp_millis());
        Ok(Box::new(start_codex_exec_json_session(context, &log_id)?))
    }
}

#[tauri::command]
pub fn get_coding_agent_provider_status() -> CodingAgentProviderStatus {
    let provider_override = CodingAgentProviderRegistry::provider_override_from_env();
    let auto_provider =
        CodingAgentProviderRegistry::resolve_provider(CodingAgentProviderKind::Auto);
    let active_provider_error = CodingAgentProviderRegistry::provider_for_kind(auto_provider)
        .and_then(|provider| provider.validate_start())
        .err();

    CodingAgentProviderStatus {
        auto_provider,
        provider_override,
        active_provider_configured: active_provider_error.is_none(),
        active_provider_error,
        diagnostics: CodingAgentProviderRegistry::diagnostics(),
    }
}

#[tauri::command]
pub fn start_initial_demo_run(
    app: AppHandle,
    request_state: State<'_, CodingAgentRequestState>,
    request: StartInitialDemoRunRequest,
) -> Result<(), String> {
    validate_start_initial_demo_run_request(&request)?;
    let request_receiver = request_state.register(&request.run_id)?;
    let request_state = request_state.inner().clone();

    thread::spawn(move || {
        let demo_session_id = request.demo_session_id.clone();
        let run_id = request.run_id.clone();
        let result = run_initial_demo_agent(app.clone(), request, request_receiver);
        request_state.unregister(&run_id);
        if let Err(error) = result {
            emit_agent_error(&app, Some(demo_session_id), Some(run_id), error);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn resolve_coding_agent_server_request(
    request_state: State<'_, CodingAgentRequestState>,
    request: ResolveCodingAgentServerRequestRequest,
) -> Result<(), String> {
    validate_request_id(&request.request_id)?;
    validate_server_request_action(&request.action)?;
    request_state.resolve(
        &request.run_id,
        ServerRequestResolution {
            request_id: request.request_id,
            action: request.action,
            answers: request.answers,
            content: request.content,
            scope: request.scope,
        },
    )
}

#[tauri::command]
pub fn is_coding_agent_run_active(
    request_state: State<'_, CodingAgentRequestState>,
    run_id: String,
) -> Result<bool, String> {
    request_state.is_active(&run_id)
}

fn validate_start_initial_demo_run_request(
    request: &StartInitialDemoRunRequest,
) -> Result<(), String> {
    if request.demo_session_id.trim().is_empty() {
        return Err("启动 demo 生成失败：DemoSession id 不能为空。".to_string());
    }
    if request.run_id.trim().is_empty() {
        return Err("启动 demo 生成失败：AgentRun id 不能为空。".to_string());
    }
    validate_coding_agent_start_context(&CodingAgentStartContext {
        project_path: request.project_path.clone(),
        prompt: request.prompt.clone(),
        sandbox: request.sandbox,
    })
}

fn validate_request_id(request_id: &Value) -> Result<(), String> {
    if request_id.is_string() || request_id.as_i64().is_some() || request_id.as_u64().is_some() {
        return Ok(());
    }
    Err("响应 app-server 请求失败：requestId 必须是字符串或整数。".to_string())
}

fn validate_server_request_action(action: &str) -> Result<(), String> {
    if matches!(
        action,
        "accept" | "acceptForSession" | "decline" | "cancel" | "submit"
    ) {
        return Ok(());
    }
    Err(format!("响应 app-server 请求失败：不支持操作 `{action}`。"))
}

fn run_initial_demo_agent(
    app: AppHandle,
    request: StartInitialDemoRunRequest,
    request_receiver: Receiver<ServerRequestResolution>,
) -> Result<(), String> {
    let requested_provider = request.provider.unwrap_or(CodingAgentProviderKind::Auto);
    let provider = CodingAgentProviderRegistry::resolve_provider(requested_provider);

    match provider {
        CodingAgentProviderKind::CodexAppServer => {
            resolve_coding_agent_permission_settings()?;
            let result =
                run_app_server_demo_agent(app.clone(), request.clone(), provider, request_receiver);
            if result.is_ok() || !can_fallback_to_codex_exec_json(requested_provider) {
                return result;
            }

            run_codex_exec_json_demo_agent(app, request, result.err())
        }
        CodingAgentProviderKind::CodexExecJson => {
            run_codex_exec_json_demo_agent(app, request, None)
        }
        CodingAgentProviderKind::Auto => {
            Err("auto coding agent provider must be resolved before use".to_string())
        }
    }
}

fn can_fallback_to_codex_exec_json(requested_provider: CodingAgentProviderKind) -> bool {
    requested_provider == CodingAgentProviderKind::Auto
        && CodingAgentProviderRegistry::provider_override_from_env().is_none()
        && CodexExecJsonProvider.validate_start().is_ok()
}

fn run_app_server_demo_agent(
    app: AppHandle,
    request: StartInitialDemoRunRequest,
    provider: CodingAgentProviderKind,
    request_receiver: Receiver<ServerRequestResolution>,
) -> Result<(), String> {
    let context = CodingAgentStartContext {
        project_path: request.project_path.clone(),
        prompt: request.prompt.clone(),
        sandbox: request.sandbox,
    };
    let mut session =
        start_codex_app_server_session(context, &request.run_id, Some(request_receiver))?;

    emit_agent_run_started(
        &app,
        AgentRunStartedEvent {
            demo_session_id: request.demo_session_id.clone(),
            run_id: request.run_id.clone(),
            project_path: request.project_path.clone(),
            provider,
            codex_thread_id: session.codex_thread_id.clone(),
            codex_turn_id: session.initial_turn_id.clone(),
            runtime: CodingAgentRuntimeMetadata {
                provider,
                version: session.codex_version.clone(),
                transport: CODEX_APP_SERVER_TRANSPORT.to_string(),
                sandbox: session.sandbox.app_server_thread_sandbox().to_string(),
                approval_policy: Some(
                    session
                        .permission_settings
                        .approval_policy
                        .as_str()
                        .to_string(),
                ),
                approvals_reviewer: Some(
                    session
                        .permission_settings
                        .approvals_reviewer
                        .as_str()
                        .to_string(),
                ),
                transport_log_path: Some(session.client.log_path().to_string()),
                protocol_baseline_version: CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION.to_string(),
                protocol_compatibility: codex_protocol_compatibility(&session.codex_version)
                    .to_string(),
            },
            started_at: current_agent_event_timestamp(),
        },
    )?;

    let mut summary = AgentRunEventSummary::default();
    let pending_events = session.take_pending_agent_events();
    match emit_agent_events(
        &app,
        &request.demo_session_id,
        &request.run_id,
        pending_events,
        &mut summary,
    ) {
        Ok(true) => return finish_agent_run(app, request, session, summary),
        Ok(false) => {}
        Err(error) => {
            let _ = session.cancel();
            return Err(error);
        }
    }

    loop {
        let events = match session.read_next_agent_events() {
            Ok(events) => events,
            Err(error) => {
                let _ = session.cancel();
                return Err(error);
            }
        };
        match emit_agent_events(
            &app,
            &request.demo_session_id,
            &request.run_id,
            events,
            &mut summary,
        ) {
            Ok(true) => return finish_agent_run(app, request, session, summary),
            Ok(false) => {}
            Err(error) => {
                let _ = session.cancel();
                return Err(error);
            }
        }
    }
}

fn run_codex_exec_json_demo_agent(
    app: AppHandle,
    request: StartInitialDemoRunRequest,
    fallback_reason: Option<String>,
) -> Result<(), String> {
    let context = CodingAgentStartContext {
        project_path: request.project_path.clone(),
        prompt: request.prompt.clone(),
        sandbox: request.sandbox,
    };
    let mut session = start_codex_exec_json_session(context, &request.run_id)?;

    emit_agent_run_started(
        &app,
        AgentRunStartedEvent {
            demo_session_id: request.demo_session_id.clone(),
            run_id: request.run_id.clone(),
            project_path: request.project_path.clone(),
            provider: CodingAgentProviderKind::CodexExecJson,
            codex_thread_id: format!("exec-json:{}", request.run_id),
            codex_turn_id: format!("exec-json-turn:{}", request.run_id),
            runtime: CodingAgentRuntimeMetadata {
                provider: CodingAgentProviderKind::CodexExecJson,
                version: session.codex_version.clone(),
                transport: "process-jsonl".to_string(),
                sandbox: session.sandbox.app_server_thread_sandbox().to_string(),
                approval_policy: Some(
                    session
                        .permission_settings
                        .approval_policy
                        .as_str()
                        .to_string(),
                ),
                approvals_reviewer: Some(
                    session
                        .permission_settings
                        .approvals_reviewer
                        .as_str()
                        .to_string(),
                ),
                transport_log_path: Some(session.transport_log.path.clone()),
                protocol_baseline_version: CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION.to_string(),
                protocol_compatibility: codex_protocol_compatibility(&session.codex_version)
                    .to_string(),
            },
            started_at: current_agent_event_timestamp(),
        },
    )?;

    if let Some(reason) = fallback_reason {
        emit_agent_event(
            &app,
            AgentEventEnvelope {
                demo_session_id: request.demo_session_id.clone(),
                run_id: request.run_id.clone(),
                event: AgentEvent::PlanUpdate {
                    text: format!(
                        "codex app-server 不可用，已切换到 codex exec --json 后备路径：{reason}"
                    ),
                    created_at: current_agent_event_timestamp(),
                },
            },
        )?;
    }

    let mut summary = AgentRunEventSummary::default();
    loop {
        let events = session.read_next_agent_events()?;
        if emit_agent_events(
            &app,
            &request.demo_session_id,
            &request.run_id,
            events,
            &mut summary,
        )? {
            return finish_codex_exec_json_agent_run(app, request, session, summary);
        }
    }
}

fn finish_codex_exec_json_agent_run(
    app: AppHandle,
    request: StartInitialDemoRunRequest,
    mut session: CodexExecJsonSession,
    summary: AgentRunEventSummary,
) -> Result<(), String> {
    let _ = session.cancel();
    let status = summary.completion_status();

    emit_agent_run_completed(
        &app,
        AgentRunCompletedEvent {
            demo_session_id: request.demo_session_id,
            run_id: request.run_id,
            final_message: summary.final_message,
            changed_files: summary.changed_files,
            status,
            error: summary.error_message,
            completed_at: current_agent_event_timestamp(),
        },
    )
}

fn finish_agent_run(
    app: AppHandle,
    request: StartInitialDemoRunRequest,
    mut session: CodexAppServerSession,
    summary: AgentRunEventSummary,
) -> Result<(), String> {
    let _ = session.cancel();
    let status = summary.completion_status();

    emit_agent_run_completed(
        &app,
        AgentRunCompletedEvent {
            demo_session_id: request.demo_session_id,
            run_id: request.run_id,
            final_message: summary.final_message,
            changed_files: summary.changed_files,
            status,
            error: summary.error_message,
            completed_at: current_agent_event_timestamp(),
        },
    )
}

fn codex_diagnostic<const N: usize>(
    provider: CodingAgentProviderKind,
    details: [(&str, &str); N],
) -> CodingAgentProviderDiagnostic {
    let executable = codex_executable();
    let version_result = validate_codex_executable();
    let version = version_result.clone().ok();
    let missing_dependencies = if version_result.is_ok() {
        Vec::new()
    } else {
        vec![executable.clone()]
    };
    let mut detail_map = details
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<BTreeMap<_, _>>();
    detail_map.insert(
        "executableSource".to_string(),
        if read_local_env("VOICECODER_CODEX_BIN").is_some() {
            "VOICECODER_CODEX_BIN".to_string()
        } else {
            "PATH".to_string()
        },
    );
    detail_map.insert(
        "protocolBaselineVersion".to_string(),
        CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION.to_string(),
    );
    detail_map.insert(
        "protocolCompatibility".to_string(),
        version
            .as_deref()
            .map(codex_protocol_compatibility)
            .unwrap_or("unavailable")
            .to_string(),
    );

    CodingAgentProviderDiagnostic {
        provider,
        configured: version_result.is_ok(),
        missing_dependencies,
        executable: Some(executable),
        version,
        details: detail_map,
        error: version_result.err(),
    }
}

fn validate_codex_executable() -> Result<String, String> {
    let executable = codex_executable();
    let output = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|error| format!("无法执行 Codex CLI `{executable}`：{error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Codex CLI `{executable}` 返回非零退出码。")
        } else {
            format!("Codex CLI `{executable}` 检查失败：{stderr}")
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Err(format!("Codex CLI `{executable}` 没有返回版本信息。"));
    }

    Ok(stdout)
}

fn codex_protocol_compatibility(version: &str) -> &'static str {
    if version.trim() == CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION {
        "verified"
    } else {
        "version-mismatch"
    }
}

fn codex_protocol_version_warning(version: &str) -> Option<String> {
    (codex_protocol_compatibility(version) != "verified").then(|| {
        format!(
            "当前 Codex CLI 为 `{version}`，VoiceCoder app-server 协议基线为 `{CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION}`。未知事件会降级记录；请重新生成 schema 并运行协议兼容检查。"
        )
    })
}

fn codex_executable() -> String {
    read_local_env("VOICECODER_CODEX_BIN").unwrap_or_else(|| DEFAULT_CODEX_BIN.to_string())
}

fn resolve_coding_agent_permission_settings() -> Result<CodingAgentPermissionSettings, String> {
    let defaults = CodingAgentPermissionSettings::default();
    let approval_policy = read_local_env(CODEX_APPROVAL_POLICY_ENV)
        .as_deref()
        .map(CodingAgentApprovalPolicy::parse)
        .transpose()?
        .unwrap_or(defaults.approval_policy);
    let approvals_reviewer = read_local_env(CODEX_APPROVALS_REVIEWER_ENV)
        .as_deref()
        .map(CodingAgentApprovalsReviewer::parse)
        .transpose()?
        .unwrap_or(defaults.approvals_reviewer);

    Ok(CodingAgentPermissionSettings {
        approval_policy,
        approvals_reviewer,
    })
}

fn apply_permission_settings_to_diagnostic(diagnostic: &mut CodingAgentProviderDiagnostic) {
    match resolve_coding_agent_permission_settings() {
        Ok(settings) => {
            diagnostic.details.insert(
                "approvalPolicy".to_string(),
                settings.approval_policy.as_str().to_string(),
            );
            diagnostic.details.insert(
                "approvalsReviewer".to_string(),
                settings.approvals_reviewer.as_str().to_string(),
            );
        }
        Err(error) => {
            diagnostic.configured = false;
            diagnostic.error = Some(match diagnostic.error.take() {
                Some(existing_error) => format!("{existing_error} {error}"),
                None => error,
            });
        }
    }
}

#[derive(Default)]
struct AgentRunEventSummary {
    final_message: Option<String>,
    changed_files: Vec<String>,
    error_message: Option<String>,
    turn_status: Option<String>,
    terminal: bool,
}

impl AgentRunEventSummary {
    fn completion_status(&self) -> String {
        self.turn_status.clone().unwrap_or_else(|| {
            if self.error_message.is_some() {
                "failed".to_string()
            } else {
                "completed".to_string()
            }
        })
    }
}

fn emit_agent_events(
    app: &AppHandle,
    demo_session_id: &str,
    run_id: &str,
    events: Vec<AgentEvent>,
    summary: &mut AgentRunEventSummary,
) -> Result<bool, String> {
    for event in events {
        update_agent_run_summary(summary, &event);
        emit_agent_event(
            app,
            AgentEventEnvelope {
                demo_session_id: demo_session_id.to_string(),
                run_id: run_id.to_string(),
                event,
            },
        )?;
    }

    Ok(summary.terminal)
}

fn update_agent_run_summary(summary: &mut AgentRunEventSummary, event: &AgentEvent) {
    match event {
        AgentEvent::FileChange { path, .. } => append_unique(&mut summary.changed_files, path),
        AgentEvent::ItemStarted { item, .. } | AgentEvent::ItemCompleted { item, .. } => {
            append_item_changed_files(&mut summary.changed_files, item);
            if item.get("type").and_then(Value::as_str) == Some("agentMessage")
                && item.get("phase").and_then(Value::as_str) == Some("final_answer")
            {
                summary.final_message = extract_string(item, "/text");
            }
        }
        AgentEvent::ItemDelta { method, delta, .. } if method == "item/fileChange/patchUpdated" => {
            append_changes_changed_files(&mut summary.changed_files, delta);
        }
        AgentEvent::TurnCompleted {
            status,
            final_message,
            ..
        } => {
            if final_message.is_some() {
                summary.final_message = final_message.clone();
            }
            summary.turn_status = Some(status.clone());
            summary.terminal = status != "inProgress";
        }
        AgentEvent::Error {
            message, terminal, ..
        } => {
            if *terminal {
                summary.error_message = Some(message.clone());
                summary.turn_status = Some("failed".to_string());
                summary.terminal = true;
            }
        }
        _ => {}
    }
}

fn append_item_changed_files(changed_files: &mut Vec<String>, item: &Value) {
    if item.get("type").and_then(Value::as_str) == Some("fileChange") {
        append_changes_changed_files(changed_files, item.get("changes").unwrap_or(&Value::Null));
    }
}

fn append_changes_changed_files(changed_files: &mut Vec<String>, changes: &Value) {
    if let Some(changes) = changes.as_array() {
        for change in changes {
            if let Some(path) = extract_string(change, "/path") {
                append_unique(changed_files, &path);
            }
        }
    }
}

fn append_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|candidate| candidate == value) {
        values.push(value.to_string());
    }
}

fn emit_agent_run_started(app: &AppHandle, payload: AgentRunStartedEvent) -> Result<(), String> {
    app.emit(AGENT_RUN_STARTED_EVENT, payload)
        .map_err(|error| format!("Failed to emit agent run started event: {error}"))
}

fn emit_agent_event(app: &AppHandle, payload: AgentEventEnvelope) -> Result<(), String> {
    app.emit(AGENT_EVENT_EVENT, payload)
        .map_err(|error| format!("Failed to emit agent event: {error}"))
}

fn emit_agent_run_completed(
    app: &AppHandle,
    payload: AgentRunCompletedEvent,
) -> Result<(), String> {
    app.emit(AGENT_RUN_COMPLETED_EVENT, payload)
        .map_err(|error| format!("Failed to emit agent run completed event: {error}"))
}

fn emit_agent_error(
    app: &AppHandle,
    demo_session_id: Option<String>,
    run_id: Option<String>,
    message: String,
) {
    let _ = app.emit(
        AGENT_ERROR_EVENT,
        AgentErrorEvent {
            demo_session_id,
            run_id,
            message,
            occurred_at: current_agent_event_timestamp(),
        },
    );
}
