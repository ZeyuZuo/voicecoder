use crate::env_config::read_local_env;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

const DEFAULT_CODEX_BIN: &str = "codex";
const FIRST_APP_SERVER_REQUEST_ID: u64 = 0;
const DEFAULT_CODEX_SANDBOX: CodingAgentSandboxMode = CodingAgentSandboxMode::WorkspaceWrite;
const CODEX_APP_SERVER_TRANSPORT: &str = "stdio";
const CODEX_APPROVAL_POLICY_ENV: &str = "VOICECODER_CODEX_APPROVAL_POLICY";
const CODEX_APPROVALS_REVIEWER_ENV: &str = "VOICECODER_CODEX_APPROVALS_REVIEWER";
const APP_SERVER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const APP_SERVER_SERVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const APP_SERVER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const APP_SERVER_TRANSPORT_POLL_INTERVAL: Duration = Duration::from_millis(250);
const APP_SERVER_STDERR_TAIL_LINES: usize = 20;
const APP_SERVER_LAST_MESSAGE_LIMIT: usize = 2_000;
const AGENT_RUN_STARTED_EVENT: &str = "agent://run-started";
const AGENT_EVENT_EVENT: &str = "agent://event";
const AGENT_RUN_COMPLETED_EVENT: &str = "agent://run-completed";
const AGENT_ERROR_EVENT: &str = "agent://error";

#[allow(dead_code)]
pub(crate) trait CodingAgentSession {
    fn cancel(&mut self) -> Result<(), String>;
}

#[allow(dead_code)]
pub(crate) trait CodingAgentProvider {
    fn kind(&self) -> CodingAgentProviderKind;
    fn validate_start(&self) -> Result<(), String>;
    fn diagnostic(&self) -> CodingAgentProviderDiagnostic;
    fn start_session(
        &self,
        context: CodingAgentStartContext,
    ) -> Result<Box<dyn CodingAgentSession + Send>, String> {
        let _ = context;
        Err("Coding Agent session start is not implemented yet.".to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentProviderKind {
    Auto,
    CodexAppServer,
    CodexExecJson,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentProviderStatus {
    auto_provider: CodingAgentProviderKind,
    provider_override: Option<CodingAgentProviderKind>,
    active_provider_configured: bool,
    active_provider_error: Option<String>,
    diagnostics: Vec<CodingAgentProviderDiagnostic>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentProviderDiagnostic {
    provider: CodingAgentProviderKind,
    configured: bool,
    missing_dependencies: Vec<String>,
    executable: Option<String>,
    version: Option<String>,
    details: BTreeMap<String, String>,
    error: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub(crate) struct CodingAgentStartContext {
    pub project_path: String,
    pub prompt: String,
    #[serde(default)]
    pub sandbox: Option<CodingAgentSandboxMode>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodingAgentSandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CodingAgentApprovalPolicy {
    Untrusted,
    OnRequest,
    Never,
}

impl CodingAgentApprovalPolicy {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_lowercase().as_str() {
            "untrusted" => Ok(Self::Untrusted),
            "on-request" | "on_request" => Ok(Self::OnRequest),
            "never" => Ok(Self::Never),
            _ => Err(format!(
                "{CODEX_APPROVAL_POLICY_ENV} 只支持 untrusted、on-request 或 never。"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodingAgentApprovalsReviewer {
    User,
    AutoReview,
    GuardianSubagent,
}

impl CodingAgentApprovalsReviewer {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_lowercase().as_str() {
            "user" => Ok(Self::User),
            "auto_review" | "auto-review" | "approve_for_me" | "approve-for-me" => {
                Ok(Self::AutoReview)
            }
            "guardian_subagent" | "guardian-subagent" => Ok(Self::GuardianSubagent),
            _ => Err(format!(
                "{CODEX_APPROVALS_REVIEWER_ENV} 只支持 user、auto_review 或 guardian_subagent。"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::AutoReview => "auto_review",
            Self::GuardianSubagent => "guardian_subagent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CodingAgentPermissionSettings {
    approval_policy: CodingAgentApprovalPolicy,
    approvals_reviewer: CodingAgentApprovalsReviewer,
}

impl Default for CodingAgentPermissionSettings {
    fn default() -> Self {
        Self {
            approval_policy: CodingAgentApprovalPolicy::OnRequest,
            approvals_reviewer: CodingAgentApprovalsReviewer::AutoReview,
        }
    }
}

impl CodingAgentSandboxMode {
    fn app_server_thread_sandbox(self) -> &'static str {
        match self {
            CodingAgentSandboxMode::ReadOnly => "read-only",
            CodingAgentSandboxMode::WorkspaceWrite => "workspace-write",
            CodingAgentSandboxMode::DangerFullAccess => "danger-full-access",
        }
    }

    fn app_server_turn_sandbox_policy(self, project_path: &str) -> Value {
        match self {
            CodingAgentSandboxMode::ReadOnly => json!({
                "type": "readOnly",
                "networkAccess": false
            }),
            CodingAgentSandboxMode::WorkspaceWrite => json!({
                "type": "workspaceWrite",
                "writableRoots": [project_path],
                "networkAccess": false
            }),
            CodingAgentSandboxMode::DangerFullAccess => json!({
                "type": "dangerFullAccess"
            }),
        }
    }

    fn codex_exec_sandbox_arg(self) -> &'static str {
        self.app_server_thread_sandbox()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentEvent {
    ThreadStarted {
        thread_id: String,
        created_at: String,
    },
    TurnStarted {
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        created_at: String,
    },
    AgentMessage {
        text: String,
        created_at: String,
    },
    PlanUpdate {
        text: String,
        created_at: String,
    },
    ApprovalReview {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        action: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rationale: Option<String>,
        created_at: String,
    },
    Diagnostic {
        level: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        method: Option<String>,
        created_at: String,
    },
    Command {
        command: String,
        status: String,
        created_at: String,
    },
    FileChange {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        change_type: Option<String>,
        created_at: String,
    },
    TurnCompleted {
        #[serde(skip_serializing_if = "Option::is_none")]
        final_message: Option<String>,
        created_at: String,
    },
    Error {
        message: String,
        created_at: String,
    },
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartInitialDemoRunRequest {
    demo_session_id: String,
    run_id: String,
    project_path: String,
    prompt: String,
    #[serde(default)]
    sandbox: Option<CodingAgentSandboxMode>,
    #[serde(default)]
    provider: Option<CodingAgentProviderKind>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunStartedEvent {
    demo_session_id: String,
    run_id: String,
    project_path: String,
    provider: CodingAgentProviderKind,
    codex_thread_id: String,
    codex_turn_id: String,
    runtime: CodingAgentRuntimeMetadata,
    started_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodingAgentRuntimeMetadata {
    provider: CodingAgentProviderKind,
    version: String,
    transport: String,
    sandbox: String,
    approval_policy: Option<String>,
    approvals_reviewer: Option<String>,
    transport_log_path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventEnvelope {
    demo_session_id: String,
    run_id: String,
    event: AgentEvent,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunCompletedEvent {
    demo_session_id: String,
    run_id: String,
    final_message: Option<String>,
    changed_files: Vec<String>,
    completed_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorEvent {
    demo_session_id: Option<String>,
    run_id: Option<String>,
    message: String,
    occurred_at: String,
}

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
        Ok(Box::new(start_codex_app_server_session(context, &log_id)?))
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
        Ok(Box::new(start_codex_exec_json_session(context)?))
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
    request: StartInitialDemoRunRequest,
) -> Result<(), String> {
    validate_start_initial_demo_run_request(&request)?;

    thread::spawn(move || {
        let demo_session_id = request.demo_session_id.clone();
        let run_id = request.run_id.clone();
        if let Err(error) = run_initial_demo_agent(app.clone(), request) {
            emit_agent_error(&app, Some(demo_session_id), Some(run_id), error);
        }
    });

    Ok(())
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

fn run_initial_demo_agent(
    app: AppHandle,
    request: StartInitialDemoRunRequest,
) -> Result<(), String> {
    let requested_provider = request.provider.unwrap_or(CodingAgentProviderKind::Auto);
    let provider = CodingAgentProviderRegistry::resolve_provider(requested_provider);

    match provider {
        CodingAgentProviderKind::CodexAppServer => {
            resolve_coding_agent_permission_settings()?;
            let result = run_app_server_demo_agent(app.clone(), request.clone(), provider);
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
) -> Result<(), String> {
    let context = CodingAgentStartContext {
        project_path: request.project_path.clone(),
        prompt: request.prompt.clone(),
        sandbox: request.sandbox,
    };
    let mut session = start_codex_app_server_session(context, &request.run_id)?;

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
            },
            started_at: current_agent_event_timestamp(),
        },
    )?;

    let mut summary = AgentRunEventSummary::default();
    let pending_events = session.take_pending_agent_events();
    if emit_agent_events(
        &app,
        &request.demo_session_id,
        &request.run_id,
        pending_events,
        &mut summary,
    )? {
        return finish_agent_run(app, request, session, summary);
    }

    loop {
        let events = session.read_next_agent_events()?;
        if emit_agent_events(
            &app,
            &request.demo_session_id,
            &request.run_id,
            events,
            &mut summary,
        )? {
            return finish_agent_run(app, request, session, summary);
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
    let mut session = start_codex_exec_json_session(context)?;

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
                transport_log_path: None,
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
    if let Some(message) = summary.error_message {
        emit_agent_error(
            &app,
            Some(request.demo_session_id),
            Some(request.run_id),
            message,
        );
        return Ok(());
    }

    emit_agent_run_completed(
        &app,
        AgentRunCompletedEvent {
            demo_session_id: request.demo_session_id,
            run_id: request.run_id,
            final_message: summary.final_message,
            changed_files: summary.changed_files,
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
    if let Some(message) = summary.error_message {
        emit_agent_error(
            &app,
            Some(request.demo_session_id),
            Some(request.run_id),
            message,
        );
        return Ok(());
    }

    emit_agent_run_completed(
        &app,
        AgentRunCompletedEvent {
            demo_session_id: request.demo_session_id,
            run_id: request.run_id,
            final_message: summary.final_message,
            changed_files: summary.changed_files,
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

struct CodexExecJsonSession {
    child: Child,
    stdout: BufReader<ChildStdout>,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
    codex_version: String,
}

impl CodingAgentSession for CodexExecJsonSession {
    fn cancel(&mut self) -> Result<(), String> {
        if self
            .child
            .try_wait()
            .map_err(|error| format!("检查 Codex exec --json 子进程状态失败：{error}"))?
            .is_none()
        {
            self.child
                .kill()
                .map_err(|error| format!("停止 Codex exec --json 失败：{error}"))?;
        }
        let _ = self.child.wait();
        Ok(())
    }
}

impl CodexExecJsonSession {
    fn read_next_agent_events(&mut self) -> Result<Vec<AgentEvent>, String> {
        loop {
            let mut line = String::new();
            let read_bytes = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("读取 Codex exec --json stdout 失败：{error}"))?;
            if read_bytes == 0 {
                let status = self
                    .child
                    .wait()
                    .map_err(|error| format!("等待 Codex exec --json 退出失败：{error}"))?;
                if status.success() {
                    return Ok(vec![AgentEvent::TurnCompleted {
                        final_message: None,
                        created_at: current_agent_event_timestamp(),
                    }]);
                }

                return Err(format!(
                    "Codex exec --json 已退出，退出码：{}。",
                    format_exit_status(status)
                ));
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let message = serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("Codex exec --json 输出不是合法 JSON：{error}"))?;
            let events = normalize_codex_exec_json_event(&message);
            if !events.is_empty() {
                return Ok(events);
            }
        }
    }
}

fn start_codex_exec_json_session(
    context: CodingAgentStartContext,
) -> Result<CodexExecJsonSession, String> {
    validate_coding_agent_start_context(&context)?;
    let codex_version = validate_codex_executable()?;
    let permission_settings = resolve_coding_agent_permission_settings()?;
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);

    let executable = codex_executable();
    let args = build_codex_exec_json_args(&context, permission_settings);
    let mut child = Command::new(&executable)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动 Codex exec --json 失败：{error}"))?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err("Codex exec --json stdout 不可用。".to_string());
    };

    Ok(CodexExecJsonSession {
        child,
        stdout: BufReader::new(stdout),
        sandbox,
        permission_settings,
        codex_version,
    })
}

fn build_codex_exec_json_args(
    context: &CodingAgentStartContext,
    permission_settings: CodingAgentPermissionSettings,
) -> Vec<String> {
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);
    vec![
        "--ask-for-approval".to_string(),
        permission_settings.approval_policy.as_str().to_string(),
        "--config".to_string(),
        format!(
            "approvals_reviewer=\"{}\"",
            permission_settings.approvals_reviewer.as_str()
        ),
        "exec".to_string(),
        "--json".to_string(),
        "--sandbox".to_string(),
        sandbox.codex_exec_sandbox_arg().to_string(),
        "--cd".to_string(),
        context.project_path.clone(),
        context.prompt.clone(),
    ]
}

#[allow(dead_code)]
struct CodexAppServerSession {
    child: Child,
    client: CodexAppServerClient,
    project_path: String,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
    codex_version: String,
    codex_thread_id: String,
    initial_turn_id: String,
    initial_prompt: String,
}

impl CodingAgentSession for CodexAppServerSession {
    fn cancel(&mut self) -> Result<(), String> {
        let stop_result = if self
            .child
            .try_wait()
            .map_err(|error| format!("检查 Codex app-server 子进程状态失败：{error}"))?
            .is_none()
        {
            self.child
                .kill()
                .map_err(|error| format!("停止 Codex app-server 失败：{error}"))
        } else {
            Ok(())
        };
        let _ = self.child.wait();
        self.client.join_readers();
        stop_result
    }
}

impl CodexAppServerSession {
    fn take_pending_agent_events(&mut self) -> Vec<AgentEvent> {
        self.client.take_pending_agent_events()
    }

    fn read_next_agent_events(&mut self) -> Result<Vec<AgentEvent>, String> {
        self.client.read_next_agent_events(&mut self.child)
    }
}

#[derive(Default)]
struct AgentRunEventSummary {
    final_message: Option<String>,
    changed_files: Vec<String>,
    error_message: Option<String>,
    terminal: bool,
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
        AgentEvent::TurnCompleted { final_message, .. } => {
            if final_message.is_some() {
                summary.final_message = final_message.clone();
            }
            summary.terminal = true;
        }
        AgentEvent::Error { message, .. } => {
            summary.error_message = Some(message.clone());
            summary.terminal = true;
        }
        _ => {}
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

fn start_codex_app_server_session(
    context: CodingAgentStartContext,
    run_id: &str,
) -> Result<CodexAppServerSession, String> {
    validate_coding_agent_start_context(&context)?;
    let codex_version = validate_codex_executable()?;
    let permission_settings = resolve_coding_agent_permission_settings()?;
    let transport_log = AppServerTransportLog::create(&context.project_path, run_id)?;

    let executable = codex_executable();
    let mut child = Command::new(&executable)
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("启动 Codex app-server 失败：{error}"))?;

    let Some(stdin) = child.stdin.take() else {
        let _ = child.kill();
        return Err("Codex app-server stdin 不可用。".to_string());
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Err("Codex app-server stdout 不可用。".to_string());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        return Err("Codex app-server stderr 不可用。".to_string());
    };
    let mut client = CodexAppServerClient::new(stdin, stdout, stderr, transport_log);
    if let Err(error) = initialize_codex_app_server(&mut child, &mut client) {
        let error = cleanup_child_with_error(&mut child, error);
        client.join_readers();
        return Err(error);
    }
    let run_handles =
        match start_initial_codex_turn(&mut child, &mut client, &context, permission_settings) {
            Ok(run_handles) => run_handles,
            Err(error) => {
                let error = cleanup_child_with_error(&mut child, error);
                client.join_readers();
                return Err(error);
            }
        };
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);

    Ok(CodexAppServerSession {
        child,
        client,
        project_path: context.project_path,
        sandbox,
        permission_settings,
        codex_version,
        codex_thread_id: run_handles.thread_id,
        initial_turn_id: run_handles.turn_id,
        initial_prompt: context.prompt,
    })
}

fn validate_coding_agent_start_context(context: &CodingAgentStartContext) -> Result<(), String> {
    if context.project_path.trim().is_empty() {
        return Err("Coding Agent 启动失败：项目路径不能为空。".to_string());
    }
    if context.prompt.trim().is_empty() {
        return Err("Coding Agent 启动失败：prompt 不能为空。".to_string());
    }
    Ok(())
}

fn cleanup_child_with_error(child: &mut Child, error: String) -> String {
    let _ = child.kill();
    let _ = child.wait();
    error
}

fn initialize_codex_app_server(
    child: &mut Child,
    client: &mut CodexAppServerClient,
) -> Result<(), String> {
    client.send_request(child, "initialize", initialize_params())?;
    client.send_notification("initialized", json!({}))?;
    Ok(())
}

fn initialize_params() -> Value {
    json!({
        "clientInfo": {
            "name": "voicecoder",
            "title": "VoiceCoder",
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "experimentalApi": true,
            "requestAttestation": false
        }
    })
}

struct CodexAppServerRunHandles {
    thread_id: String,
    turn_id: String,
}

fn start_initial_codex_turn(
    child: &mut Child,
    client: &mut CodexAppServerClient,
    context: &CodingAgentStartContext,
    permission_settings: CodingAgentPermissionSettings,
) -> Result<CodexAppServerRunHandles, String> {
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);
    let thread_response = client.send_request(
        child,
        "thread/start",
        build_thread_start_params(&context.project_path, sandbox, permission_settings),
    )?;
    let thread_id = extract_json_pointer_string(
        &thread_response,
        "/result/thread/id",
        "Codex app-server thread/start 响应缺少 thread.id。",
    )?;

    let turn_response = client.send_request(
        child,
        "turn/start",
        build_turn_start_params(
            &thread_id,
            &context.project_path,
            sandbox,
            permission_settings,
            &context.prompt,
        ),
    )?;
    let turn_id = extract_json_pointer_string(
        &turn_response,
        "/result/turn/id",
        "Codex app-server turn/start 响应缺少 turn.id。",
    )?;

    Ok(CodexAppServerRunHandles { thread_id, turn_id })
}

fn build_thread_start_params(
    project_path: &str,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
) -> Value {
    json!({
        "cwd": project_path,
        "runtimeWorkspaceRoots": [project_path],
        "approvalPolicy": permission_settings.approval_policy.as_str(),
        "approvalsReviewer": permission_settings.approvals_reviewer.as_str(),
        "sandbox": sandbox.app_server_thread_sandbox(),
        "threadSource": "user"
    })
}

fn build_turn_start_params(
    thread_id: &str,
    project_path: &str,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
    prompt: &str,
) -> Value {
    json!({
        "threadId": thread_id,
        "cwd": project_path,
        "runtimeWorkspaceRoots": [project_path],
        "sandboxPolicy": sandbox.app_server_turn_sandbox_policy(project_path),
        "approvalPolicy": permission_settings.approval_policy.as_str(),
        "approvalsReviewer": permission_settings.approvals_reviewer.as_str(),
        "input": [
            {
                "type": "text",
                "text": prompt
            }
        ]
    })
}

#[allow(dead_code)]
fn normalize_codex_notification(notification: &Value) -> Vec<AgentEvent> {
    normalize_codex_notification_at(notification, &current_agent_event_timestamp())
}

fn normalize_codex_notification_at(notification: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(method) = notification.get("method").and_then(Value::as_str) else {
        return Vec::new();
    };
    let params = notification.get("params").unwrap_or(&Value::Null);

    match method {
        "thread/started" => extract_string(params, "/thread/id")
            .map(|thread_id| {
                vec![AgentEvent::ThreadStarted {
                    thread_id,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "turn/started" => vec![AgentEvent::TurnStarted {
            turn_id: extract_string(params, "/turn/id"),
            created_at: created_at.to_string(),
        }],
        "item/agentMessage/delta" => extract_string(params, "/delta")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::AgentMessage {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "item/plan/delta" => extract_string(params, "/delta")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::PlanUpdate {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "turn/plan/updated" => format_turn_plan_update(params)
            .map(|text| {
                vec![AgentEvent::PlanUpdate {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            normalize_auto_approval_review(params, created_at)
        }
        "item/started" | "item/completed" => normalize_codex_item(params.get("item"), created_at),
        "item/fileChange/patchUpdated" => {
            normalize_codex_file_changes(params.get("changes"), created_at)
        }
        "turn/completed" => normalize_codex_turn_completed(params, created_at),
        "error" | "thread/realtime/error" => vec![AgentEvent::Error {
            message: format_codex_error(params.get("error").unwrap_or(params)),
            created_at: created_at.to_string(),
        }],
        _ => Vec::new(),
    }
}

fn normalize_codex_exec_json_event(event: &Value) -> Vec<AgentEvent> {
    normalize_codex_exec_json_event_at(event, &current_agent_event_timestamp())
}

fn normalize_codex_exec_json_event_at(event: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(event_type) = event.get("type").and_then(Value::as_str) else {
        return Vec::new();
    };

    match event_type {
        "thread.started" => extract_string(event, "/thread_id")
            .or_else(|| extract_string(event, "/thread/id"))
            .map(|thread_id| {
                vec![AgentEvent::ThreadStarted {
                    thread_id,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "turn.started" => vec![AgentEvent::TurnStarted {
            turn_id: extract_string(event, "/turn_id")
                .or_else(|| extract_string(event, "/turn/id")),
            created_at: created_at.to_string(),
        }],
        "turn.completed" => vec![AgentEvent::TurnCompleted {
            final_message: extract_exec_final_message(event),
            created_at: created_at.to_string(),
        }],
        "turn.failed" => vec![AgentEvent::Error {
            message: extract_string(event, "/error")
                .or_else(|| extract_string(event, "/message"))
                .or_else(|| {
                    event
                        .get("error")
                        .map(format_codex_error)
                        .filter(|message| !message.is_empty())
                })
                .unwrap_or_else(|| "Codex exec --json turn failed.".to_string()),
            created_at: created_at.to_string(),
        }],
        "item.started" | "item.completed" => {
            normalize_codex_exec_json_item(event.get("item"), created_at)
        }
        "item.agent_message.delta" | "item.agentMessage.delta" => extract_string(event, "/delta")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::AgentMessage {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "error" => vec![AgentEvent::Error {
            message: event
                .get("error")
                .map(format_codex_error)
                .unwrap_or_else(|| format_codex_error(event)),
            created_at: created_at.to_string(),
        }],
        _ => Vec::new(),
    }
}

fn normalize_codex_item(item: Option<&Value>, created_at: &str) -> Vec<AgentEvent> {
    let Some(item) = item else {
        return Vec::new();
    };

    match item.get("type").and_then(Value::as_str) {
        Some("agentMessage") => extract_string(item, "/text")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::AgentMessage {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("plan") => extract_string(item, "/text")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::PlanUpdate {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("commandExecution") => {
            let Some(command) = extract_string(item, "/command") else {
                return Vec::new();
            };
            vec![AgentEvent::Command {
                command,
                status: extract_string(item, "/status").unwrap_or_else(|| "unknown".to_string()),
                created_at: created_at.to_string(),
            }]
        }
        Some("fileChange") => normalize_codex_file_changes(item.get("changes"), created_at),
        _ => Vec::new(),
    }
}

fn normalize_codex_exec_json_item(item: Option<&Value>, created_at: &str) -> Vec<AgentEvent> {
    let Some(item) = item else {
        return Vec::new();
    };

    match item.get("type").and_then(Value::as_str) {
        Some("agent_message") | Some("agentMessage") => extract_string(item, "/text")
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![AgentEvent::AgentMessage {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("plan_update") | Some("plan") => format_exec_plan_update(item)
            .map(|text| {
                vec![AgentEvent::PlanUpdate {
                    text,
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        Some("command_execution") | Some("commandExecution") => {
            let Some(command) = extract_string(item, "/command") else {
                return Vec::new();
            };
            vec![AgentEvent::Command {
                command,
                status: extract_string(item, "/status").unwrap_or_else(|| "unknown".to_string()),
                created_at: created_at.to_string(),
            }]
        }
        Some("file_change") | Some("fileChange") => {
            normalize_codex_exec_json_file_change(item, created_at)
        }
        _ => Vec::new(),
    }
}

fn normalize_codex_file_changes(changes: Option<&Value>, created_at: &str) -> Vec<AgentEvent> {
    changes
        .and_then(Value::as_array)
        .map(|changes| {
            changes
                .iter()
                .filter_map(|change| {
                    extract_string(change, "/path").map(|path| AgentEvent::FileChange {
                        path,
                        change_type: extract_string(change, "/kind/type"),
                        created_at: created_at.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_auto_approval_review(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(status) = extract_string(params, "/review/status") else {
        return Vec::new();
    };

    vec![AgentEvent::ApprovalReview {
        status,
        action: extract_string(params, "/action/type"),
        rationale: extract_string(params, "/review/rationale").filter(|value| !value.is_empty()),
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_exec_json_file_change(item: &Value, created_at: &str) -> Vec<AgentEvent> {
    let mut events = normalize_codex_file_changes(item.get("changes"), created_at);
    if events.is_empty() {
        if let Some(path) = extract_string(item, "/path") {
            events.push(AgentEvent::FileChange {
                path,
                change_type: extract_string(item, "/kind/type")
                    .or_else(|| extract_string(item, "/change_type"))
                    .or_else(|| extract_string(item, "/changeType")),
                created_at: created_at.to_string(),
            });
        }
    }

    events
}

fn normalize_codex_turn_completed(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let mut events = vec![AgentEvent::TurnCompleted {
        final_message: extract_final_agent_message(params),
        created_at: created_at.to_string(),
    }];

    if let Some(error) = params
        .pointer("/turn/error")
        .filter(|error| !error.is_null())
    {
        events.push(AgentEvent::Error {
            message: format_codex_error(error),
            created_at: created_at.to_string(),
        });
    }

    events
}

fn extract_final_agent_message(params: &Value) -> Option<String> {
    params
        .pointer("/turn/items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().rev().find_map(|item| {
                if item.get("type").and_then(Value::as_str) == Some("agentMessage") {
                    extract_string(item, "/text").filter(|text| !text.is_empty())
                } else {
                    None
                }
            })
        })
}

fn extract_exec_final_message(event: &Value) -> Option<String> {
    extract_string(event, "/final_message")
        .or_else(|| extract_string(event, "/finalMessage"))
        .or_else(|| extract_string(event, "/message"))
        .or_else(|| extract_final_agent_message(event))
}

fn format_turn_plan_update(params: &Value) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(explanation) =
        extract_string(params, "/explanation").filter(|text| !text.is_empty())
    {
        lines.push(explanation);
    }

    if let Some(plan) = params.get("plan").and_then(Value::as_array) {
        lines.extend(plan.iter().filter_map(|step| {
            let step_text = extract_string(step, "/step")?;
            let status = extract_string(step, "/status").unwrap_or_else(|| "pending".to_string());
            Some(format!("[{status}] {step_text}"))
        }));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn format_exec_plan_update(item: &Value) -> Option<String> {
    extract_string(item, "/text")
        .or_else(|| extract_string(item, "/message"))
        .filter(|text| !text.is_empty())
        .or_else(|| format_turn_plan_update(item))
}

fn format_codex_error(error: &Value) -> String {
    extract_string(error, "/message")
        .or_else(|| extract_string(error, "/error/message"))
        .or_else(|| extract_string(error, "/message/text"))
        .unwrap_or_else(|| error.to_string())
}

fn extract_string(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn current_agent_event_timestamp() -> String {
    Utc::now().to_rfc3339()
}

#[derive(Clone, Debug, PartialEq)]
enum JsonRpcInboundKind {
    Response { request_id: u64 },
    Notification { method: String },
    ServerRequest { request_id: Value, method: String },
    Unknown { reason: String },
}

enum AppServerReaderEvent {
    Message {
        message: Value,
        received_at: String,
    },
    InvalidJson {
        line: String,
        error: String,
        received_at: String,
    },
    Closed,
    Failed(String),
}

struct PendingServerRequest {
    request_id: Value,
    method: String,
    deadline: Instant,
}

#[derive(Clone, Default)]
struct AppServerHeartbeatSnapshot {
    last_message_at: Option<String>,
    last_progress_at: Option<String>,
    last_method: Option<String>,
    last_message: Option<String>,
}

#[derive(Clone, Default)]
struct SharedAppServerHeartbeat(Arc<Mutex<AppServerHeartbeatSnapshot>>);

impl SharedAppServerHeartbeat {
    fn record_message(&self, message: &Value, received_at: &str) {
        if let Ok(mut heartbeat) = self.0.lock() {
            heartbeat.last_message_at = Some(received_at.to_string());
            heartbeat.last_method = message
                .get("method")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            heartbeat.last_message = Some(truncate_transport_text(
                &message.to_string(),
                APP_SERVER_LAST_MESSAGE_LIMIT,
            ));
        }
    }

    fn record_progress(&self, occurred_at: &str) {
        if let Ok(mut heartbeat) = self.0.lock() {
            heartbeat.last_progress_at = Some(occurred_at.to_string());
        }
    }

    fn snapshot(&self) -> AppServerHeartbeatSnapshot {
        self.0.lock().map(|value| value.clone()).unwrap_or_default()
    }
}

#[derive(Clone)]
struct AppServerTransportLog {
    path: String,
    file: Arc<Mutex<File>>,
}

impl AppServerTransportLog {
    fn create(project_path: &str, run_id: &str) -> Result<Self, String> {
        let voicecoder_dir = Path::new(project_path).join(".voicecoder");
        fs::create_dir_all(&voicecoder_dir)
            .map_err(|error| format!("创建 app-server 诊断目录失败：{error}"))?;
        let file_name = format!(
            "agent_run_{}_app_server.jsonl",
            sanitize_transport_log_stem(run_id)
        );
        let path = voicecoder_dir.join(file_name);
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .map_err(|error| format!("创建 app-server 原始日志失败：{error}"))?;

        Ok(Self {
            path: path.to_string_lossy().to_string(),
            file: Arc::new(Mutex::new(file)),
        })
    }

    fn record(&self, direction: &str, kind: &str, payload: Value) -> Result<(), String> {
        let record = json!({
            "recordedAt": current_agent_event_timestamp(),
            "direction": direction,
            "kind": kind,
            "payload": payload
        });
        let line = serde_json::to_string(&record)
            .map_err(|error| format!("序列化 app-server 原始日志失败：{error}"))?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| "app-server 原始日志锁已损坏。".to_string())?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.flush())
            .map_err(|error| format!("写入 app-server 原始日志失败：{error}"))
    }
}

#[allow(dead_code)]
struct CodexAppServerClient {
    stdin: ChildStdin,
    receiver: Receiver<AppServerReaderEvent>,
    stdout_reader: Option<thread::JoinHandle<()>>,
    stderr_reader: Option<thread::JoinHandle<()>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    heartbeat: SharedAppServerHeartbeat,
    transport_log: AppServerTransportLog,
    next_request_id: u64,
    pending_responses: BTreeMap<u64, Value>,
    pending_agent_events: VecDeque<AgentEvent>,
    pending_server_requests: VecDeque<PendingServerRequest>,
    last_heartbeat_notice: Instant,
}

impl CodexAppServerClient {
    fn new(
        stdin: ChildStdin,
        stdout: ChildStdout,
        stderr: ChildStderr,
        transport_log: AppServerTransportLog,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let heartbeat = SharedAppServerHeartbeat::default();
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let stdout_reader = spawn_app_server_stdout_reader(
            stdout,
            sender,
            transport_log.clone(),
            heartbeat.clone(),
        );
        let stderr_reader =
            spawn_app_server_stderr_reader(stderr, transport_log.clone(), stderr_tail.clone());

        Self {
            stdin,
            receiver,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            stderr_tail,
            heartbeat,
            transport_log,
            next_request_id: FIRST_APP_SERVER_REQUEST_ID,
            pending_responses: BTreeMap::new(),
            pending_agent_events: VecDeque::new(),
            pending_server_requests: VecDeque::new(),
            last_heartbeat_notice: Instant::now(),
        }
    }

    fn log_path(&self) -> &str {
        &self.transport_log.path
    }

    fn next_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    fn send_request(
        &mut self,
        child: &mut Child,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let request_id = self.next_request_id();
        let request = build_json_rpc_request(request_id, method, params);
        self.write_json_line(&request, "request")?;
        self.read_response(child, request_id)
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_json_line(&build_json_rpc_notification(method, params), "notification")
    }

    fn take_pending_agent_events(&mut self) -> Vec<AgentEvent> {
        self.pending_agent_events.drain(..).collect()
    }

    fn read_response(&mut self, child: &mut Child, request_id: u64) -> Result<Value, String> {
        if let Some(response) = self.pending_responses.remove(&request_id) {
            return validate_json_rpc_response(response);
        }

        let deadline = Instant::now() + APP_SERVER_RESPONSE_TIMEOUT;
        loop {
            self.resolve_expired_server_requests()?;
            if Instant::now() >= deadline {
                return Err(self.transport_error_context(
                    child,
                    &format!("等待 Codex app-server request {request_id} 响应超时。"),
                ));
            }

            match self
                .receiver
                .recv_timeout(APP_SERVER_TRANSPORT_POLL_INTERVAL)
            {
                Ok(event) => {
                    if let Some(response) =
                        self.process_reader_event(child, event, Some(request_id))?
                    {
                        return validate_json_rpc_response(response);
                    }
                }
                Err(RecvTimeoutError::Timeout) => self.ensure_child_is_running(child)?,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.transport_error_context(
                        child,
                        "Codex app-server stdout reader 已断开。",
                    ));
                }
            }
        }
    }

    fn read_next_agent_events(&mut self, child: &mut Child) -> Result<Vec<AgentEvent>, String> {
        loop {
            if !self.pending_agent_events.is_empty() {
                return Ok(self.take_pending_agent_events());
            }

            self.resolve_expired_server_requests()?;
            if !self.pending_agent_events.is_empty() {
                return Ok(self.take_pending_agent_events());
            }

            match self
                .receiver
                .recv_timeout(APP_SERVER_TRANSPORT_POLL_INTERVAL)
            {
                Ok(event) => {
                    let _ = self.process_reader_event(child, event, None)?;
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.ensure_child_is_running(child)?;
                    if self.last_heartbeat_notice.elapsed() >= APP_SERVER_HEARTBEAT_INTERVAL {
                        self.last_heartbeat_notice = Instant::now();
                        return Ok(vec![self.heartbeat_diagnostic_event()]);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.transport_error_context(
                        child,
                        "Codex app-server stdout reader 已断开。",
                    ));
                }
            }
        }
    }

    fn process_reader_event(
        &mut self,
        child: &mut Child,
        event: AppServerReaderEvent,
        expected_response_id: Option<u64>,
    ) -> Result<Option<Value>, String> {
        match event {
            AppServerReaderEvent::Message {
                message,
                received_at,
            } => self.route_message(message, received_at, expected_response_id),
            AppServerReaderEvent::InvalidJson {
                line,
                error,
                received_at,
            } => {
                self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                    level: "warning".to_string(),
                    message: format!(
                        "忽略一行无效 app-server JSON：{error} · {}",
                        truncate_transport_text(&line, 240)
                    ),
                    method: None,
                    created_at: received_at,
                });
                Ok(None)
            }
            AppServerReaderEvent::Closed => {
                Err(self.transport_error_context(child, "Codex app-server stdout 已关闭。"))
            }
            AppServerReaderEvent::Failed(error) => Err(self.transport_error_context(
                child,
                &format!("读取 Codex app-server stdout 失败：{error}"),
            )),
        }
    }

    fn route_message(
        &mut self,
        message: Value,
        received_at: String,
        expected_response_id: Option<u64>,
    ) -> Result<Option<Value>, String> {
        match classify_json_rpc_message(&message) {
            JsonRpcInboundKind::Response { request_id } => {
                if expected_response_id == Some(request_id) {
                    return Ok(Some(message));
                }
                self.pending_responses.insert(request_id, message);
            }
            JsonRpcInboundKind::Notification { method } => {
                let events = normalize_codex_notification_at(&message, &received_at);
                if events.is_empty() {
                    self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                        level: "debug".to_string(),
                        message: "收到尚未映射的 app-server notification".to_string(),
                        method: Some(method),
                        created_at: received_at,
                    });
                } else {
                    self.heartbeat.record_progress(&received_at);
                    self.last_heartbeat_notice = Instant::now();
                    self.pending_agent_events.extend(events);
                }
            }
            JsonRpcInboundKind::ServerRequest { request_id, method } => {
                self.pending_server_requests
                    .push_back(PendingServerRequest {
                        request_id,
                        method: method.clone(),
                        deadline: Instant::now() + APP_SERVER_SERVER_REQUEST_TIMEOUT,
                    });
                self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                    level: "warning".to_string(),
                    message: format!(
                        "收到尚未接入 UI 的 app-server 主动请求；若未处理将在 {} 秒后安全拒绝",
                        APP_SERVER_SERVER_REQUEST_TIMEOUT.as_secs()
                    ),
                    method: Some(method),
                    created_at: received_at,
                });
            }
            JsonRpcInboundKind::Unknown { reason } => {
                self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                    level: "warning".to_string(),
                    message: format!("收到无法分类的 app-server 消息：{reason}"),
                    method: message
                        .get("method")
                        .and_then(Value::as_str)
                        .map(ToString::to_string),
                    created_at: received_at,
                });
            }
        }

        Ok(None)
    }

    fn resolve_expired_server_requests(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let mut pending = VecDeque::new();

        while let Some(request) = self.pending_server_requests.pop_front() {
            if request.deadline > now {
                pending.push_back(request);
                continue;
            }

            let response = build_json_rpc_error_response(
                request.request_id,
                -32002,
                "VoiceCoder 尚未实现该 app-server 主动请求，已在超时后安全拒绝。",
            );
            self.write_json_line(&response, "server_request_timeout_response")?;
            self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                level: "warning".to_string(),
                message: "app-server 主动请求等待超时，已安全拒绝".to_string(),
                method: Some(request.method),
                created_at: current_agent_event_timestamp(),
            });
        }

        self.pending_server_requests = pending;
        Ok(())
    }

    fn heartbeat_diagnostic_event(&self) -> AgentEvent {
        let heartbeat = self.heartbeat.snapshot();
        let last_message_at = heartbeat
            .last_message_at
            .as_deref()
            .unwrap_or("尚未收到消息");
        let last_progress_at = heartbeat
            .last_progress_at
            .as_deref()
            .unwrap_or("尚未收到有效进展");

        AgentEvent::Diagnostic {
            level: "info".to_string(),
            message: format!(
                "仍在等待 Codex；最后消息：{last_message_at}；最后有效进展：{last_progress_at}"
            ),
            method: heartbeat.last_method,
            created_at: current_agent_event_timestamp(),
        }
    }

    fn ensure_child_is_running(&self, child: &mut Child) -> Result<(), String> {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("检查 Codex app-server 子进程状态失败：{error}"))?
        {
            return Err(self.transport_error_context(
                child,
                &format!(
                    "Codex app-server 已退出，退出码：{}。",
                    format_exit_status(status)
                ),
            ));
        }
        Ok(())
    }

    fn transport_error_context(&self, child: &mut Child, message: &str) -> String {
        let heartbeat = self.heartbeat.snapshot();
        let exit_status = child
            .try_wait()
            .ok()
            .flatten()
            .map(format_exit_status)
            .unwrap_or_else(|| "仍在运行或状态未知".to_string());
        let stderr = self
            .stderr_tail
            .lock()
            .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join(" | "))
            .unwrap_or_default();
        let last_message = heartbeat
            .last_message
            .unwrap_or_else(|| "没有已记录的协议消息".to_string());

        format!(
            "{message} 进程状态：{exit_status}；最后协议消息：{last_message}；stderr：{}；原始日志：{}",
            if stderr.is_empty() { "无" } else { &stderr },
            self.transport_log.path
        )
    }

    fn write_json_line(&mut self, value: &Value, kind: &str) -> Result<(), String> {
        let line = serde_json::to_string(value)
            .map_err(|error| format!("序列化 Codex app-server 消息失败：{error}"))?;
        self.transport_log.record("outbound", kind, value.clone())?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("写入 Codex app-server stdin 失败：{error}"))
    }

    fn join_readers(&mut self) {
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

fn spawn_app_server_stdout_reader(
    stdout: ChildStdout,
    sender: Sender<AppServerReaderEvent>,
    transport_log: AppServerTransportLog,
    heartbeat: SharedAppServerHeartbeat,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let received_at = current_agent_event_timestamp();
            match line {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<Value>(trimmed) {
                        Ok(message) => {
                            heartbeat.record_message(&message, &received_at);
                            let _ = transport_log.record("inbound", "message", message.clone());
                            if sender
                                .send(AppServerReaderEvent::Message {
                                    message,
                                    received_at,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = transport_log.record(
                                "inbound",
                                "invalid_json",
                                json!({ "line": trimmed, "error": error.to_string() }),
                            );
                            if sender
                                .send(AppServerReaderEvent::InvalidJson {
                                    line,
                                    error: error.to_string(),
                                    received_at,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = sender.send(AppServerReaderEvent::Failed(error.to_string()));
                    return;
                }
            }
        }
        let _ = sender.send(AppServerReaderEvent::Closed);
    })
}

fn spawn_app_server_stderr_reader(
    stderr: ChildStderr,
    transport_log: AppServerTransportLog,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            let _ = transport_log.record("inbound", "stderr", Value::String(line.clone()));
            if let Ok(mut tail) = stderr_tail.lock() {
                tail.push_back(line);
                while tail.len() > APP_SERVER_STDERR_TAIL_LINES {
                    tail.pop_front();
                }
            }
        }
    })
}

fn classify_json_rpc_message(message: &Value) -> JsonRpcInboundKind {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let request_id = message.get("id").filter(|id| !id.is_null());

    if let (Some(method), Some(request_id)) = (method.clone(), request_id) {
        return JsonRpcInboundKind::ServerRequest {
            request_id: request_id.clone(),
            method,
        };
    }

    if let Some(request_id) = request_id.and_then(Value::as_u64) {
        if message.get("result").is_some() || message.get("error").is_some() {
            return JsonRpcInboundKind::Response { request_id };
        }
    }

    if let Some(method) = method {
        return JsonRpcInboundKind::Notification { method };
    }

    JsonRpcInboundKind::Unknown {
        reason: "缺少可识别的 method 或 response id/result/error".to_string(),
    }
}

fn build_json_rpc_error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn sanitize_transport_log_stem(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate_transport_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn build_json_rpc_request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "method": method,
        "id": id,
        "params": params
    })
}

fn build_json_rpc_notification(method: &str, params: Value) -> Value {
    json!({
        "method": method,
        "params": params
    })
}

fn validate_json_rpc_response(message: Value) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        return Err(format!(
            "Codex app-server request 失败：{}",
            json_rpc_error_message(error)
        ));
    }

    if message.get("result").is_none() {
        return Err("Codex app-server 响应缺少 result。".to_string());
    }

    Ok(message)
}

fn extract_json_pointer_string(
    message: &Value,
    pointer: &str,
    error_message: &str,
) -> Result<String, String> {
    message
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| error_message.to_string())
}

fn format_exit_status(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

#[cfg(test)]
fn build_initialize_request() -> Value {
    build_json_rpc_request(
        FIRST_APP_SERVER_REQUEST_ID,
        "initialize",
        initialize_params(),
    )
}

#[cfg(test)]
fn build_initialized_notification() -> Value {
    build_json_rpc_notification("initialized", json!({}))
}

#[cfg(test)]
fn validate_initialize_response(message: Value) -> Result<Value, String> {
    validate_json_rpc_response(message).map_err(|error| {
        error.replacen("Codex app-server request", "Codex app-server initialize", 1)
    })
}

fn json_rpc_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_START_REQUEST_FIXTURE: &str =
        include_str!("../tests/fixtures/codex-app-server-v2/thread-start-request.json");
    const TURN_START_REQUEST_FIXTURE: &str =
        include_str!("../tests/fixtures/codex-app-server-v2/turn-start-request.json");
    const FILE_CHANGE_STARTED_FIXTURE: &str =
        include_str!("../tests/fixtures/codex-app-server-v2/file-change-started-notification.json");
    const FILE_CHANGE_APPROVAL_REQUEST_FIXTURE: &str =
        include_str!("../tests/fixtures/codex-app-server-v2/file-change-approval-request.json");
    const AUTO_APPROVAL_COMPLETED_FIXTURE: &str = include_str!(
        "../tests/fixtures/codex-app-server-v2/auto-approval-completed-notification.json"
    );

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
            build_json_rpc_error_response(
                Value::String("approval-1".to_string()),
                -32002,
                "Timed out"
            ),
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
            let mut session = start_codex_app_server_session(context, "smoke")?;
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

        assert_eq!(
            events,
            vec![AgentEvent::FileChange {
                path: "/tmp/voicecoder-demo/src/App.tsx".to_string(),
                change_type: Some("update".to_string()),
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
            extract_json_pointer_string(&turn_response, "/result/thread/id", "missing")
                .unwrap_err(),
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

    #[test]
    fn normalizes_thread_and_turn_started_notifications() {
        let thread_events = normalize_codex_notification_at(
            &json!({
                "method": "thread/started",
                "params": {
                    "thread": {
                        "id": "thread-1"
                    }
                }
            }),
            "2026-06-24T00:00:00Z",
        );
        let turn_events = normalize_codex_notification_at(
            &json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1"
                    }
                }
            }),
            "2026-06-24T00:00:01Z",
        );

        assert_eq!(
            thread_events,
            vec![AgentEvent::ThreadStarted {
                thread_id: "thread-1".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            turn_events,
            vec![AgentEvent::TurnStarted {
                turn_id: Some("turn-1".to_string()),
                created_at: "2026-06-24T00:00:01Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_agent_message_and_plan_deltas() {
        let message_events = normalize_codex_notification_at(
            &json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "delta": "正在修改首页",
                    "itemId": "item-1",
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
            "2026-06-24T00:00:00Z",
        );
        let plan_events = normalize_codex_notification_at(
            &json!({
                "method": "item/plan/delta",
                "params": {
                    "delta": "实现主要布局",
                    "itemId": "item-2",
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
            "2026-06-24T00:00:01Z",
        );

        assert_eq!(
            message_events,
            vec![AgentEvent::AgentMessage {
                text: "正在修改首页".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            plan_events,
            vec![AgentEvent::PlanUpdate {
                text: "实现主要布局".to_string(),
                created_at: "2026-06-24T00:00:01Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_turn_plan_updated_notification() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "turn/plan/updated",
                "params": {
                    "explanation": "计划已更新",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "plan": [
                        { "step": "读取项目结构", "status": "completed" },
                        { "step": "实现 demo", "status": "inProgress" }
                    ]
                }
            }),
            "2026-06-24T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::PlanUpdate {
                text: "计划已更新\n[completed] 读取项目结构\n[inProgress] 实现 demo".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_command_execution_items() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "item": {
                        "id": "item-1",
                        "type": "commandExecution",
                        "command": "npm run check",
                        "commandActions": [],
                        "cwd": "/tmp/demo",
                        "status": "completed"
                    },
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
            "2026-06-24T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::Command {
                command: "npm run check".to_string(),
                status: "completed".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_command_execution_items_without_status_as_unknown() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "item/started",
                "params": {
                    "item": {
                        "id": "item-1",
                        "type": "commandExecution",
                        "command": "npm run dev"
                    }
                }
            }),
            "2026-06-24T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::Command {
                command: "npm run dev".to_string(),
                status: "unknown".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_file_change_notifications() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "item/fileChange/patchUpdated",
                "params": {
                    "changes": [
                        {
                            "path": "/tmp/demo/src/App.tsx",
                            "kind": { "type": "update" },
                            "diff": "@@"
                        },
                        {
                            "path": "/tmp/demo/src/styles.css",
                            "kind": { "type": "add" },
                            "diff": "@@"
                        }
                    ],
                    "itemId": "item-1",
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
            "2026-06-24T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![
                AgentEvent::FileChange {
                    path: "/tmp/demo/src/App.tsx".to_string(),
                    change_type: Some("update".to_string()),
                    created_at: "2026-06-24T00:00:00Z".to_string(),
                },
                AgentEvent::FileChange {
                    path: "/tmp/demo/src/styles.css".to_string(),
                    change_type: Some("add".to_string()),
                    created_at: "2026-06-24T00:00:00Z".to_string(),
                },
            ]
        );
    }

    #[test]
    fn normalizes_turn_completed_with_final_message_and_error() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1",
                        "status": "failed",
                        "items": [
                            {
                                "id": "item-1",
                                "type": "agentMessage",
                                "text": "已完成第一版。"
                            }
                        ],
                        "error": {
                            "message": "Network blocked"
                        }
                    }
                }
            }),
            "2026-06-24T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![
                AgentEvent::TurnCompleted {
                    final_message: Some("已完成第一版。".to_string()),
                    created_at: "2026-06-24T00:00:00Z".to_string(),
                },
                AgentEvent::Error {
                    message: "Network blocked".to_string(),
                    created_at: "2026-06-24T00:00:00Z".to_string(),
                },
            ]
        );
    }

    #[test]
    fn normalizes_codex_exec_json_lifecycle_events() {
        let thread_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "thread.started",
                "thread_id": "exec-thread-1"
            }),
            "2026-06-24T00:00:00Z",
        );
        let turn_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "turn.started"
            }),
            "2026-06-24T00:00:01Z",
        );
        let completed_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "turn.completed",
                "final_message": "完成"
            }),
            "2026-06-24T00:00:02Z",
        );

        assert_eq!(
            thread_events,
            vec![AgentEvent::ThreadStarted {
                thread_id: "exec-thread-1".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            turn_events,
            vec![AgentEvent::TurnStarted {
                turn_id: None,
                created_at: "2026-06-24T00:00:01Z".to_string(),
            }]
        );
        assert_eq!(
            completed_events,
            vec![AgentEvent::TurnCompleted {
                final_message: Some("完成".to_string()),
                created_at: "2026-06-24T00:00:02Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_codex_exec_json_item_events() {
        let command_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "item.started",
                "item": {
                    "id": "item-1",
                    "type": "command_execution",
                    "command": "npm run check",
                    "status": "in_progress"
                }
            }),
            "2026-06-24T00:00:00Z",
        );
        let message_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "item.completed",
                "item": {
                    "id": "item-2",
                    "type": "agent_message",
                    "text": "Repo contains docs, sdk, and examples directories."
                }
            }),
            "2026-06-24T00:00:01Z",
        );
        let file_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "item.completed",
                "item": {
                    "id": "item-3",
                    "type": "file_change",
                    "changes": [
                        {
                            "path": "src/App.tsx",
                            "kind": { "type": "update" }
                        }
                    ]
                }
            }),
            "2026-06-24T00:00:02Z",
        );

        assert_eq!(
            command_events,
            vec![AgentEvent::Command {
                command: "npm run check".to_string(),
                status: "in_progress".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            message_events,
            vec![AgentEvent::AgentMessage {
                text: "Repo contains docs, sdk, and examples directories.".to_string(),
                created_at: "2026-06-24T00:00:01Z".to_string(),
            }]
        );
        assert_eq!(
            file_events,
            vec![AgentEvent::FileChange {
                path: "src/App.tsx".to_string(),
                change_type: Some("update".to_string()),
                created_at: "2026-06-24T00:00:02Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_codex_exec_json_single_file_change_with_camel_case_change_type() {
        let events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "item.completed",
                "item": {
                    "id": "item-3",
                    "type": "file_change",
                    "path": "src/App.tsx",
                    "changeType": "update"
                }
            }),
            "2026-06-24T00:00:02Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::FileChange {
                path: "src/App.tsx".to_string(),
                change_type: Some("update".to_string()),
                created_at: "2026-06-24T00:00:02Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_codex_exec_json_failed_turn_and_errors() {
        let failed_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "turn.failed",
                "error": {
                    "message": "Sandbox denied command"
                }
            }),
            "2026-06-24T00:00:00Z",
        );
        let error_events = normalize_codex_exec_json_event_at(
            &json!({
                "type": "error",
                "message": "Authentication required"
            }),
            "2026-06-24T00:00:01Z",
        );

        assert_eq!(
            failed_events,
            vec![AgentEvent::Error {
                message: "Sandbox denied command".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            error_events,
            vec![AgentEvent::Error {
                message: "Authentication required".to_string(),
                created_at: "2026-06-24T00:00:01Z".to_string(),
            }]
        );
    }

    #[test]
    fn serializes_agent_event_for_frontend_contract() {
        let value = serde_json::to_value(AgentEvent::FileChange {
            path: "/tmp/demo/src/App.tsx".to_string(),
            change_type: Some("update".to_string()),
            created_at: "2026-06-24T00:00:00Z".to_string(),
        })
        .unwrap();

        assert_eq!(
            value,
            json!({
                "type": "file_change",
                "path": "/tmp/demo/src/App.tsx",
                "changeType": "update",
                "createdAt": "2026-06-24T00:00:00Z"
            })
        );
    }

    #[test]
    fn serializes_agent_event_envelope_for_tauri_event_contract() {
        let value = serde_json::to_value(AgentEventEnvelope {
            demo_session_id: "demo-1".to_string(),
            run_id: "run-1".to_string(),
            event: AgentEvent::AgentMessage {
                text: "正在生成 demo".to_string(),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            },
        })
        .unwrap();

        assert_eq!(
            value,
            json!({
                "demoSessionId": "demo-1",
                "runId": "run-1",
                "event": {
                    "type": "agent_message",
                    "text": "正在生成 demo",
                    "createdAt": "2026-06-24T00:00:00Z"
                }
            })
        );
    }

    #[test]
    fn serializes_agent_run_started_runtime_for_session_logs() {
        let value = serde_json::to_value(AgentRunStartedEvent {
            demo_session_id: "demo-1".to_string(),
            run_id: "run-1".to_string(),
            project_path: "/tmp/demo".to_string(),
            provider: CodingAgentProviderKind::CodexAppServer,
            codex_thread_id: "thread-1".to_string(),
            codex_turn_id: "turn-1".to_string(),
            runtime: CodingAgentRuntimeMetadata {
                provider: CodingAgentProviderKind::CodexAppServer,
                version: "codex-cli 0.144.1".to_string(),
                transport: "stdio".to_string(),
                sandbox: "workspace-write".to_string(),
                approval_policy: Some("on-request".to_string()),
                approvals_reviewer: Some("auto_review".to_string()),
                transport_log_path: Some(
                    "/tmp/demo/.voicecoder/agent_run_run-1_app_server.jsonl".to_string(),
                ),
            },
            started_at: "2026-07-13T00:00:00Z".to_string(),
        })
        .unwrap();

        assert_eq!(
            value.pointer("/runtime/version").and_then(Value::as_str),
            Some("codex-cli 0.144.1")
        );
        assert_eq!(
            value
                .pointer("/runtime/approvalPolicy")
                .and_then(Value::as_str),
            Some("on-request")
        );
        assert_eq!(
            value
                .pointer("/runtime/approvalsReviewer")
                .and_then(Value::as_str),
            Some("auto_review")
        );
        assert_eq!(
            value
                .pointer("/runtime/transportLogPath")
                .and_then(Value::as_str),
            Some("/tmp/demo/.voicecoder/agent_run_run-1_app_server.jsonl")
        );
    }

    #[test]
    fn serializes_agent_run_completed_event_for_tauri_event_contract() {
        let value = serde_json::to_value(AgentRunCompletedEvent {
            demo_session_id: "demo-1".to_string(),
            run_id: "run-1".to_string(),
            final_message: Some("完成".to_string()),
            changed_files: vec!["/tmp/demo/src/App.tsx".to_string()],
            completed_at: "2026-06-24T00:00:00Z".to_string(),
        })
        .unwrap();

        assert_eq!(
            value,
            json!({
                "demoSessionId": "demo-1",
                "runId": "run-1",
                "finalMessage": "完成",
                "changedFiles": ["/tmp/demo/src/App.tsx"],
                "completedAt": "2026-06-24T00:00:00Z"
            })
        );
    }

    #[test]
    fn run_summary_collects_changed_files_final_message_and_errors() {
        let mut summary = AgentRunEventSummary::default();
        update_agent_run_summary(
            &mut summary,
            &AgentEvent::FileChange {
                path: "/tmp/demo/src/App.tsx".to_string(),
                change_type: Some("update".to_string()),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            },
        );
        update_agent_run_summary(
            &mut summary,
            &AgentEvent::FileChange {
                path: "/tmp/demo/src/App.tsx".to_string(),
                change_type: Some("update".to_string()),
                created_at: "2026-06-24T00:00:01Z".to_string(),
            },
        );
        update_agent_run_summary(
            &mut summary,
            &AgentEvent::TurnCompleted {
                final_message: Some("完成".to_string()),
                created_at: "2026-06-24T00:00:02Z".to_string(),
            },
        );

        assert_eq!(summary.changed_files, vec!["/tmp/demo/src/App.tsx"]);
        assert_eq!(summary.final_message, Some("完成".to_string()));
        assert!(summary.terminal);

        update_agent_run_summary(
            &mut summary,
            &AgentEvent::Error {
                message: "失败".to_string(),
                created_at: "2026-06-24T00:00:03Z".to_string(),
            },
        );
        assert_eq!(summary.error_message, Some("失败".to_string()));
    }

    #[test]
    fn ignores_unknown_or_incomplete_notifications() {
        assert!(normalize_codex_notification_at(
            &json!({
                "method": "unknown",
                "params": {}
            }),
            "2026-06-24T00:00:00Z",
        )
        .is_empty());
        assert!(normalize_codex_notification_at(
            &json!({
                "method": "thread/started",
                "params": {}
            }),
            "2026-06-24T00:00:00Z",
        )
        .is_empty());
    }
}
