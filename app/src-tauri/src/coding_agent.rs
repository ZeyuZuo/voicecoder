use crate::env_config::read_local_env;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
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
    ItemStarted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        item_type: String,
        lifecycle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        started_at: String,
        item: Value,
        created_at: String,
    },
    ItemDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        item_type: String,
        lifecycle: String,
        method: String,
        delta: Value,
        created_at: String,
    },
    ItemCompleted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        item_type: String,
        lifecycle: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        completed_at: String,
        item: Value,
        created_at: String,
    },
    PlanUpdated {
        thread_id: String,
        turn_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        explanation: Option<String>,
        plan: Vec<AgentPlanStep>,
        created_at: String,
    },
    TurnDiffUpdated {
        thread_id: String,
        turn_id: String,
        diff: String,
        created_at: String,
    },
    HookRunUpdated {
        thread_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        hook_id: String,
        lifecycle: String,
        run: Value,
        created_at: String,
    },
    ContextCompacted {
        thread_id: String,
        turn_id: String,
        created_at: String,
    },
    TokenUsageUpdated {
        thread_id: String,
        turn_id: String,
        token_usage: Value,
        created_at: String,
    },
    ModelRerouted {
        thread_id: String,
        turn_id: String,
        from_model: String,
        to_model: String,
        reason: String,
        created_at: String,
    },
    ModelSafetyBufferingUpdated {
        thread_id: String,
        turn_id: String,
        model: String,
        use_cases: Vec<String>,
        reasons: Vec<String>,
        show_buffering_ui: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        faster_model: Option<String>,
        created_at: String,
    },
    ModelVerificationUpdated {
        thread_id: String,
        turn_id: String,
        verifications: Vec<String>,
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
        thread_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        final_message: Option<String>,
        created_at: String,
    },
    Warning {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        created_at: String,
    },
    ConfigWarning {
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        range: Option<Value>,
        created_at: String,
    },
    GuardianWarning {
        thread_id: String,
        message: String,
        created_at: String,
    },
    Error {
        message: String,
        retryable: bool,
        terminal: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        created_at: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlanStep {
    step: String,
    status: String,
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
    status: String,
    error: Option<String>,
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
                        thread_id: None,
                        turn_id: None,
                        status: "completed".to_string(),
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
        "hook/started" => normalize_codex_hook_run(params, created_at, false),
        "hook/completed" => normalize_codex_hook_run(params, created_at, true),
        "item/agentMessage/delta"
        | "item/plan/delta"
        | "item/commandExecution/outputDelta"
        | "item/commandExecution/terminalInteraction"
        | "item/fileChange/outputDelta"
        | "item/fileChange/patchUpdated"
        | "item/mcpToolCall/progress"
        | "item/reasoning/summaryTextDelta"
        | "item/reasoning/summaryPartAdded"
        | "item/reasoning/textDelta" => normalize_codex_item_delta(method, params, created_at),
        "turn/plan/updated" => normalize_codex_plan_updated(params, created_at),
        "turn/diff/updated" => {
            let thread_id = extract_string(params, "/threadId");
            let turn_id = extract_string(params, "/turnId");
            let diff = extract_string(params, "/diff");
            match (thread_id, turn_id, diff) {
                (Some(thread_id), Some(turn_id), Some(diff)) => {
                    vec![AgentEvent::TurnDiffUpdated {
                        thread_id,
                        turn_id,
                        diff,
                        created_at: created_at.to_string(),
                    }]
                }
                _ => Vec::new(),
            }
        }
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            normalize_auto_approval_review(params, created_at)
        }
        "item/started" => normalize_codex_item_lifecycle(params, created_at, false),
        "item/completed" => normalize_codex_item_lifecycle(params, created_at, true),
        "thread/compacted" => normalize_codex_context_compacted(params, created_at),
        "thread/tokenUsage/updated" => normalize_codex_token_usage(params, created_at),
        "model/rerouted" => normalize_codex_model_rerouted(params, created_at),
        "model/safetyBuffering/updated" => {
            normalize_codex_model_safety_buffering(params, created_at)
        }
        "model/verification" => normalize_codex_model_verification(params, created_at),
        "turn/completed" => normalize_codex_turn_completed(params, created_at),
        "warning" => extract_string(params, "/message")
            .map(|message| {
                vec![AgentEvent::Warning {
                    message: project_ui_sensitive_text(&message, AGENT_UI_STATUS_MESSAGE_CHARS),
                    thread_id: extract_string(params, "/threadId"),
                    turn_id: extract_string(params, "/turnId"),
                    created_at: created_at.to_string(),
                }]
            })
            .unwrap_or_default(),
        "configWarning" => normalize_codex_config_warning(params, created_at),
        "guardianWarning" => normalize_codex_guardian_warning(params, created_at),
        "error" | "thread/realtime/error" => vec![AgentEvent::Error {
            message: project_ui_sensitive_text(
                &format_codex_error(params.get("error").unwrap_or(params)),
                AGENT_UI_STATUS_MESSAGE_CHARS,
            ),
            retryable: params
                .get("willRetry")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            terminal: !params
                .get("willRetry")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            thread_id: extract_string(params, "/threadId"),
            turn_id: extract_string(params, "/turnId"),
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
            thread_id: extract_string(event, "/thread_id")
                .or_else(|| extract_string(event, "/threadId")),
            turn_id: extract_string(event, "/turn_id").or_else(|| extract_string(event, "/turnId")),
            status: "completed".to_string(),
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
            retryable: false,
            terminal: true,
            thread_id: extract_string(event, "/thread_id")
                .or_else(|| extract_string(event, "/threadId")),
            turn_id: extract_string(event, "/turn_id").or_else(|| extract_string(event, "/turnId")),
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
            retryable: false,
            terminal: true,
            thread_id: extract_string(event, "/thread_id")
                .or_else(|| extract_string(event, "/threadId")),
            turn_id: extract_string(event, "/turn_id").or_else(|| extract_string(event, "/turnId")),
            created_at: created_at.to_string(),
        }],
        _ => Vec::new(),
    }
}

fn normalize_codex_item_lifecycle(
    params: &Value,
    created_at: &str,
    completed: bool,
) -> Vec<AgentEvent> {
    let Some(item) = params.get("item") else {
        return Vec::new();
    };
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(item_id) = extract_string(item, "/id") else {
        return Vec::new();
    };
    let item_type = extract_string(item, "/type").unwrap_or_else(|| "unknown".to_string());
    let status = extract_string(item, "/status");
    let projected_item = project_codex_item_for_ui(item, &item_type);

    if completed {
        vec![AgentEvent::ItemCompleted {
            thread_id,
            turn_id,
            item_id,
            item_type,
            lifecycle: "completed".to_string(),
            status,
            completed_at: extract_protocol_timestamp(params, "/completedAtMs", created_at),
            item: projected_item,
            created_at: created_at.to_string(),
        }]
    } else {
        vec![AgentEvent::ItemStarted {
            thread_id,
            turn_id,
            item_id,
            item_type,
            lifecycle: "in_progress".to_string(),
            status,
            started_at: extract_protocol_timestamp(params, "/startedAtMs", created_at),
            item: projected_item,
            created_at: created_at.to_string(),
        }]
    }
}

fn project_codex_item_for_ui(item: &Value, item_type: &str) -> Value {
    let mut projected = ui_item_base(item, item_type);

    match item_type {
        "userMessage" | "contextCompaction" => {}
        "hookPrompt" => {
            if let Some(fragments) = item.get("fragments") {
                projected.insert("fragments".to_string(), project_ui_safe_value(fragments));
            }
        }
        "agentMessage" => {
            copy_ui_text(&mut projected, item, "text", AGENT_UI_LONG_TEXT_CHARS);
            copy_ui_value(&mut projected, item, "phase");
        }
        "plan" => copy_ui_text(&mut projected, item, "text", AGENT_UI_LONG_TEXT_CHARS),
        "reasoning" => {
            if let Some(summary) = item.get("summary") {
                let (projected_summary, summary_truncated) = project_reasoning_summary(summary);
                projected.insert("summary".to_string(), projected_summary);
                if summary_truncated {
                    projected.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
                }
            }
            let content_count = item
                .get("content")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let raw_text_available = content_count > 0
                || item
                    .get("rawTextAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            projected.insert(
                "rawTextAvailable".to_string(),
                Value::Bool(raw_text_available),
            );
            projected.insert("contentCount".to_string(), json!(content_count));
        }
        "commandExecution" => {
            copy_ui_text(&mut projected, item, "command", AGENT_UI_LONG_TEXT_CHARS);
            copy_ui_text(&mut projected, item, "cwd", AGENT_UI_TEXT_PREVIEW_CHARS);
            copy_ui_text(
                &mut projected,
                item,
                "processId",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
            copy_ui_value(&mut projected, item, "source");
            copy_ui_value(&mut projected, item, "status");
            if let Some(actions) = item.get("commandActions") {
                projected.insert("commandActions".to_string(), project_ui_safe_value(actions));
            }
            if let Some(output) = item.get("aggregatedOutput").and_then(Value::as_str) {
                let (tail, truncated) =
                    truncate_ui_text_tail(output, AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS);
                let restricted = is_ui_credential_text(&tail);
                projected.insert(
                    "aggregatedOutput".to_string(),
                    Value::String(project_ui_credential_text(
                        &tail,
                        AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS,
                    )),
                );
                projected.insert(
                    "aggregatedOutputTruncated".to_string(),
                    Value::Bool(truncated || restricted),
                );
            }
            copy_ui_value(&mut projected, item, "exitCode");
            copy_ui_value(&mut projected, item, "durationMs");
        }
        "fileChange" => {
            copy_ui_value(&mut projected, item, "status");
            if let Some(changes) = item.get("changes") {
                projected.insert("changes".to_string(), project_file_changes_for_ui(changes));
            }
        }
        "mcpToolCall" => project_mcp_tool_call_for_ui(item, &mut projected),
        "dynamicToolCall" => project_dynamic_tool_call_for_ui(item, &mut projected),
        "collabAgentToolCall" => {
            for key in ["tool", "status", "reasoningEffort"] {
                copy_ui_value(&mut projected, item, key);
            }
            for key in ["senderThreadId", "model"] {
                copy_ui_text(&mut projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
            }
            if let Some(prompt) = item.get("prompt").and_then(Value::as_str) {
                projected.insert(
                    "prompt".to_string(),
                    Value::String(project_ui_sensitive_text(
                        prompt,
                        AGENT_UI_TEXT_PREVIEW_CHARS,
                    )),
                );
            }
            if let Some(receiver_thread_ids) = item.get("receiverThreadIds") {
                projected.insert(
                    "receiverThreadIds".to_string(),
                    project_ui_safe_value(receiver_thread_ids),
                );
            }
            if let Some(agent_states) = item.get("agentsStates") {
                projected.insert(
                    "agentsStates".to_string(),
                    project_ui_safe_value(agent_states),
                );
            }
        }
        "subAgentActivity" => {
            copy_ui_value(&mut projected, item, "kind");
            for key in ["agentThreadId", "agentPath"] {
                copy_ui_text(&mut projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
            }
        }
        "webSearch" => {
            if let Some(query) = item.get("query").and_then(Value::as_str) {
                projected.insert(
                    "query".to_string(),
                    Value::String(project_ui_sensitive_text(
                        query,
                        AGENT_UI_TEXT_PREVIEW_CHARS,
                    )),
                );
            }
            if let Some(action) = item.get("action") {
                projected.insert("action".to_string(), project_web_search_action(action));
            }
        }
        "imageView" => copy_ui_text(&mut projected, item, "path", AGENT_UI_TEXT_PREVIEW_CHARS),
        "sleep" => copy_ui_value(&mut projected, item, "durationMs"),
        "imageGeneration" => {
            copy_ui_value(&mut projected, item, "status");
            copy_ui_sensitive_text(
                &mut projected,
                item,
                "revisedPrompt",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
            copy_ui_text(
                &mut projected,
                item,
                "savedPath",
                AGENT_UI_TEXT_PREVIEW_CHARS,
            );
            let result_length = item
                .get("result")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0);
            projected.insert(
                "resultAvailable".to_string(),
                Value::Bool(result_length > 0),
            );
            projected.insert("resultLength".to_string(), json!(result_length));
        }
        "enteredReviewMode" | "exitedReviewMode" => {
            copy_ui_sensitive_text(&mut projected, item, "review", AGENT_UI_TEXT_PREVIEW_CHARS)
        }
        _ => merge_unknown_item_projection(item, &mut projected),
    }

    Value::Object(projected)
}

fn ui_item_base(item: &Value, item_type: &str) -> Map<String, Value> {
    let mut projected = Map::new();
    if let Some(id) = item.get("id").and_then(Value::as_str) {
        projected.insert("id".to_string(), Value::String(id.to_string()));
    }
    projected.insert("type".to_string(), Value::String(item_type.to_string()));
    projected
}

fn copy_ui_value(projected: &mut Map<String, Value>, item: &Value, key: &str) {
    if let Some(value) = item.get(key) {
        projected.insert(key.to_string(), value.clone());
    }
}

fn copy_ui_text(projected: &mut Map<String, Value>, item: &Value, key: &str, max_chars: usize) {
    if let Some(value) = item.get(key).and_then(Value::as_str) {
        projected.insert(
            key.to_string(),
            Value::String(truncate_ui_text(value, max_chars)),
        );
    }
}

fn copy_ui_sensitive_text(
    projected: &mut Map<String, Value>,
    item: &Value,
    key: &str,
    max_chars: usize,
) {
    if let Some(value) = item.get(key).and_then(Value::as_str) {
        projected.insert(
            key.to_string(),
            Value::String(project_ui_sensitive_text(value, max_chars)),
        );
    }
}

fn project_reasoning_summary(summary: &Value) -> (Value, bool) {
    let Some(parts) = summary.as_array() else {
        return (Value::Array(Vec::new()), false);
    };
    let projected = Value::Array(
        parts
            .iter()
            .take(AGENT_UI_REASONING_PARTS)
            .filter_map(Value::as_str)
            .map(|part| Value::String(project_ui_sensitive_text(part, AGENT_UI_TEXT_PREVIEW_CHARS)))
            .collect(),
    );
    let truncated = parts.len() > AGENT_UI_REASONING_PARTS
        || parts.iter().any(|part| {
            part.as_str().is_some_and(|text| {
                ui_text_exceeds(text, AGENT_UI_TEXT_PREVIEW_CHARS)
                    || is_ui_binary_or_credential_text(text)
            })
        });
    (projected, truncated)
}

fn project_file_changes_for_ui(changes: &Value) -> Value {
    let Some(changes) = changes.as_array() else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        changes
            .iter()
            .filter_map(|change| {
                let mut projected = Map::new();
                copy_ui_text(&mut projected, change, "path", AGENT_UI_LONG_TEXT_CHARS);
                if let Some(kind) = change.get("kind") {
                    projected.insert("kind".to_string(), project_file_change_kind(kind));
                }
                copy_ui_value(&mut projected, change, "diff");
                (!projected.is_empty()).then_some(Value::Object(projected))
            })
            .collect(),
    )
}

fn project_file_change_kind(kind: &Value) -> Value {
    let Some(kind) = kind.as_object() else {
        return kind.clone();
    };
    let mut projected = Map::new();
    for key in ["type", "move_path", "movePath"] {
        if let Some(value) = kind.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(projected)
}

fn project_mcp_tool_call_for_ui(item: &Value, projected: &mut Map<String, Value>) {
    for key in ["status", "durationMs"] {
        copy_ui_value(projected, item, key);
    }
    for key in ["server", "tool", "pluginId"] {
        copy_ui_text(projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
    }
    for key in ["arguments", "result"] {
        if let Some(value) = item.get(key).filter(|value| !value.is_null()) {
            projected.insert(key.to_string(), project_ui_safe_value(value));
        }
    }
    if let Some(error) = item.get("error").and_then(Value::as_object) {
        let mut projected_error = Map::new();
        if let Some(message) = error.get("message").and_then(Value::as_str) {
            let redacted_or_truncated = is_ui_binary_or_credential_text(message)
                || ui_text_exceeds(message, AGENT_UI_TEXT_PREVIEW_CHARS);
            projected_error.insert(
                "message".to_string(),
                Value::String(project_ui_sensitive_text(
                    message,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                )),
            );
            if redacted_or_truncated {
                projected_error.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
            }
        }
        if !projected_error.is_empty() {
            projected.insert("error".to_string(), Value::Object(projected_error));
        }
    }
    if let Some(app_context) = item.get("appContext").and_then(Value::as_object) {
        let mut projected_context = Map::new();
        for key in ["appName", "actionName"] {
            if let Some(value) = app_context.get(key).and_then(Value::as_str) {
                projected_context.insert(
                    key.to_string(),
                    Value::String(truncate_ui_text(value, AGENT_UI_TEXT_PREVIEW_CHARS)),
                );
            }
        }
        if !projected_context.is_empty() {
            projected.insert("appContext".to_string(), Value::Object(projected_context));
        }
    }
}

fn project_dynamic_tool_call_for_ui(item: &Value, projected: &mut Map<String, Value>) {
    for key in ["status", "success", "durationMs"] {
        copy_ui_value(projected, item, key);
    }
    for key in ["namespace", "tool"] {
        copy_ui_text(projected, item, key, AGENT_UI_TEXT_PREVIEW_CHARS);
    }
    for key in ["arguments", "contentItems"] {
        if let Some(value) = item.get(key).filter(|value| !value.is_null()) {
            projected.insert(key.to_string(), project_ui_safe_value(value));
        }
    }
}

fn project_web_search_action(action: &Value) -> Value {
    let Some(action) = action.as_object() else {
        return Value::Null;
    };
    let mut projected = Map::new();
    for key in ["type", "query", "url", "pattern"] {
        if let Some(value) = action.get(key).and_then(Value::as_str) {
            projected.insert(
                key.to_string(),
                Value::String(project_ui_sensitive_text(
                    value,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                )),
            );
        }
    }
    if let Some(queries) = action.get("queries").and_then(Value::as_array) {
        projected.insert(
            "queries".to_string(),
            Value::Array(
                queries
                    .iter()
                    .take(AGENT_UI_STRUCTURED_MAX_ITEMS)
                    .filter_map(Value::as_str)
                    .map(|query| {
                        Value::String(project_ui_sensitive_text(
                            query,
                            AGENT_UI_TEXT_PREVIEW_CHARS,
                        ))
                    })
                    .collect(),
            ),
        );
    }
    Value::Object(projected)
}

fn merge_unknown_item_projection(item: &Value, projected: &mut Map<String, Value>) {
    let Value::Object(safe) = project_ui_safe_value(item) else {
        return;
    };
    for (key, value) in safe {
        if !matches!(key.as_str(), "id" | "type") {
            projected.insert(key, value);
        }
    }
}

fn project_codex_hook_run_for_ui(run: &Value) -> Value {
    let mut projected = Map::new();
    let mut projection_truncated = run.as_object().is_some_and(|fields| {
        fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "id" | "eventName"
                    | "handlerType"
                    | "executionMode"
                    | "scope"
                    | "sourcePath"
                    | "source"
                    | "displayOrder"
                    | "status"
                    | "statusMessage"
                    | "startedAt"
                    | "completedAt"
                    | "durationMs"
                    | "entries"
            )
        })
    });
    for key in [
        "id",
        "eventName",
        "handlerType",
        "executionMode",
        "scope",
        "sourcePath",
        "source",
        "status",
        "statusMessage",
    ] {
        if let Some(value) = run.get(key).and_then(Value::as_str) {
            let max_chars = if key == "statusMessage" {
                AGENT_UI_HOOK_ENTRY_CHARS
            } else {
                AGENT_UI_TEXT_PREVIEW_CHARS
            };
            let projected_value = if key == "statusMessage" {
                if ui_text_exceeds(value, max_chars) || is_ui_binary_or_credential_text(value) {
                    projection_truncated = true;
                }
                project_ui_sensitive_text(value, max_chars)
            } else {
                if ui_text_exceeds(value, max_chars) {
                    projection_truncated = true;
                }
                truncate_ui_text(value, max_chars)
            };
            projected.insert(key.to_string(), Value::String(projected_value));
        }
    }
    for key in ["displayOrder", "startedAt", "completedAt", "durationMs"] {
        if let Some(value) = run.get(key) {
            projected.insert(key.to_string(), value.clone());
        }
    }
    if let Some(entries) = run.get("entries").and_then(Value::as_array) {
        if entries.len() > AGENT_UI_HOOK_ENTRIES {
            projection_truncated = true;
        }
        let mut remaining_entry_chars = AGENT_UI_HOOK_TOTAL_CHARS;
        let mut projected_entries = Vec::new();
        for entry in entries.iter().take(AGENT_UI_HOOK_ENTRIES) {
            if remaining_entry_chars == 0 {
                projection_truncated = true;
                break;
            }
            let mut projected_entry = Map::new();
            if entry.as_object().is_some_and(|fields| {
                fields
                    .keys()
                    .any(|key| !matches!(key.as_str(), "kind" | "text"))
            }) {
                projection_truncated = true;
            }
            if let Some(kind) = entry.get("kind").and_then(Value::as_str) {
                projected_entry.insert(
                    "kind".to_string(),
                    Value::String(truncate_ui_text(kind, AGENT_UI_TEXT_PREVIEW_CHARS)),
                );
            }
            if let Some(text) = entry.get("text").and_then(Value::as_str) {
                let max_chars = remaining_entry_chars.min(AGENT_UI_HOOK_ENTRY_CHARS);
                if ui_text_exceeds(text, max_chars) || is_ui_binary_or_credential_text(text) {
                    projection_truncated = true;
                }
                let projected_text = project_ui_sensitive_text(text, max_chars);
                remaining_entry_chars =
                    remaining_entry_chars.saturating_sub(projected_text.chars().count());
                projected_entry.insert("text".to_string(), Value::String(projected_text));
            }
            if !projected_entry.is_empty() {
                projected_entries.push(Value::Object(projected_entry));
            }
        }
        projected.insert("entries".to_string(), Value::Array(projected_entries));
    }
    if projection_truncated {
        projected.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
    }
    Value::Object(projected)
}

struct UiProjectionBudget {
    remaining_chars: usize,
    remaining_nodes: usize,
    truncated: bool,
}

fn project_ui_safe_value(value: &Value) -> Value {
    let mut budget = UiProjectionBudget {
        remaining_chars: AGENT_UI_STRUCTURED_TOTAL_CHARS,
        remaining_nodes: AGENT_UI_STRUCTURED_MAX_NODES,
        truncated: false,
    };
    let projected = project_ui_safe_value_at(value, 0, &mut budget);
    if !budget.truncated {
        return projected;
    }
    match projected {
        Value::Object(mut fields) => {
            fields.insert("_uiProjectionTruncated".to_string(), Value::Bool(true));
            Value::Object(fields)
        }
        Value::Array(mut items) => {
            items.push(json!({ "_uiProjectionTruncated": true }));
            Value::Array(items)
        }
        value => json!({
            "preview": value,
            "_uiProjectionTruncated": true
        }),
    }
}

fn project_ui_safe_value_at(value: &Value, depth: usize, budget: &mut UiProjectionBudget) -> Value {
    if budget.remaining_nodes == 0 || depth > AGENT_UI_STRUCTURED_MAX_DEPTH {
        budget.truncated = true;
        return Value::String("[truncated]".to_string());
    }
    budget.remaining_nodes -= 1;

    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => {
            if is_ui_binary_or_credential_text(text) {
                budget.truncated = true;
                return Value::String("[redacted binary or credential data]".to_string());
            }
            let max_chars = budget.remaining_chars.min(AGENT_UI_TEXT_PREVIEW_CHARS);
            if max_chars == 0 {
                budget.truncated = true;
                return Value::String("[truncated]".to_string());
            }
            if ui_text_exceeds(text, max_chars) {
                budget.truncated = true;
            }
            let projected = truncate_ui_text(text, max_chars);
            budget.remaining_chars = budget
                .remaining_chars
                .saturating_sub(projected.chars().count());
            Value::String(projected)
        }
        Value::Array(items) => {
            let limit = items.len().min(AGENT_UI_STRUCTURED_MAX_ITEMS);
            let projected = items
                .iter()
                .take(limit)
                .map(|item| project_ui_safe_value_at(item, depth + 1, budget))
                .collect::<Vec<_>>();
            if items.len() > limit {
                budget.truncated = true;
            }
            Value::Array(projected)
        }
        Value::Object(fields) => {
            let mut projected = Map::new();
            let mut redacted_fields = 0usize;
            let mut omitted_fields = 0usize;
            for (index, (key, child)) in fields.iter().enumerate() {
                if index >= AGENT_UI_STRUCTURED_MAX_FIELDS || budget.remaining_nodes == 0 {
                    omitted_fields = fields.len().saturating_sub(index);
                    budget.truncated = true;
                    break;
                }
                if is_ui_sensitive_field(key) {
                    redacted_fields += 1;
                    budget.truncated = true;
                    continue;
                }
                if ui_text_exceeds(key, 128) {
                    budget.truncated = true;
                }
                projected.insert(
                    truncate_ui_text(key, 128),
                    project_ui_safe_value_at(child, depth + 1, budget),
                );
            }
            if redacted_fields > 0 {
                projected.insert("_redactedFieldCount".to_string(), json!(redacted_fields));
            }
            if omitted_fields > 0 {
                projected.insert("_omittedFieldCount".to_string(), json!(omitted_fields));
            }
            Value::Object(projected)
        }
    }
}

fn is_ui_sensitive_field(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
    normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("authorization")
        || normalized.contains("cookie")
        || normalized.contains("apikey")
        || normalized.contains("credential")
        || normalized.contains("privatekey")
        || normalized.contains("signature")
        || normalized == "stdin"
        || normalized == "raw"
        || normalized.starts_with("rawtext")
        || normalized.starts_with("rawcontent")
}

fn is_ui_binary_or_credential_text(value: &str) -> bool {
    let trimmed = value.trim();
    if is_ui_credential_text(trimmed) {
        return true;
    }
    let marker_window = trimmed
        .chars()
        .take(AGENT_UI_STATUS_MESSAGE_CHARS)
        .collect::<String>()
        .to_ascii_lowercase();
    if marker_window.contains("data:") && marker_window.contains(";base64,") {
        return true;
    }
    if trimmed.len() < 512 {
        return false;
    }
    let mut base64_chars = 0usize;
    let mut non_whitespace_chars = 0usize;
    let mut whitespace_chars = 0usize;
    for character in trimmed.chars().take(AGENT_UI_STATUS_MESSAGE_CHARS) {
        if character.is_whitespace() {
            whitespace_chars += 1;
            continue;
        }
        non_whitespace_chars += 1;
        if character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=') {
            base64_chars += 1;
        }
    }
    let total_chars = non_whitespace_chars + whitespace_chars;
    non_whitespace_chars >= 512
        && base64_chars.saturating_mul(100) / non_whitespace_chars >= 98
        && whitespace_chars.saturating_mul(100) / total_chars.max(1) <= 2
}

fn is_ui_credential_text(value: &str) -> bool {
    let marker_window = value
        .trim()
        .chars()
        .take(AGENT_UI_STATUS_MESSAGE_CHARS)
        .collect::<String>()
        .to_ascii_lowercase();
    marker_window.contains("bearer ")
        || marker_window.contains("-----begin ") && marker_window.contains("private key-----")
        || contains_ui_secret_key_marker(&marker_window)
        || [
            "apikey=",
            "apikey:",
            "api_key=",
            "api_key:",
            "api-key=",
            "api-key:",
            "token=",
            "token:",
            "password=",
            "password:",
            "authorization=",
            "authorization:",
            "signature=",
            "x-amz-credential=",
            "x-amz-signature=",
            "x-goog-signature=",
        ]
        .iter()
        .any(|marker| marker_window.contains(marker))
}

fn contains_ui_secret_key_marker(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(3).enumerate().any(|(index, marker)| {
        if marker != b"sk-" {
            return false;
        }
        let has_boundary =
            index == 0 || (!bytes[index - 1].is_ascii_alphanumeric() && bytes[index - 1] != b'_');
        let suffix_length = bytes[index + 3..]
            .iter()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || matches!(**character, b'_' | b'-')
            })
            .count();
        has_boundary && suffix_length >= 12
    })
}

fn project_ui_sensitive_text(value: &str, max_chars: usize) -> String {
    if is_ui_binary_or_credential_text(value) {
        truncate_ui_text("[redacted credential or binary data]", max_chars)
    } else {
        truncate_ui_text(value, max_chars)
    }
}

fn project_ui_credential_text(value: &str, max_chars: usize) -> String {
    if is_ui_credential_text(value) {
        truncate_ui_text("[redacted credential data]", max_chars)
    } else {
        truncate_ui_text(value, max_chars)
    }
}

fn ui_text_exceeds(value: &str, max_chars: usize) -> bool {
    value.chars().nth(max_chars).is_some()
}

fn truncate_ui_text(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let mut projected = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_none() {
        return projected;
    }
    if max_chars > 0 {
        projected.pop();
        projected.push('…');
    }
    projected
}

fn truncate_ui_text_tail(value: &str, max_chars: usize) -> (String, bool) {
    if max_chars == 0 {
        return (String::new(), !value.is_empty());
    }
    let Some((start, _)) = value.char_indices().rev().nth(max_chars.saturating_sub(1)) else {
        return (value.to_string(), false);
    };
    (value[start..].to_string(), start > 0)
}

fn normalize_codex_hook_run(params: &Value, created_at: &str, completed: bool) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(run) = params.get("run") else {
        return Vec::new();
    };
    let Some(hook_id) = extract_string(run, "/id") else {
        return Vec::new();
    };

    vec![AgentEvent::HookRunUpdated {
        thread_id,
        turn_id: extract_string(params, "/turnId"),
        hook_id,
        lifecycle: if completed {
            "completed".to_string()
        } else {
            "in_progress".to_string()
        },
        run: project_codex_hook_run_for_ui(run),
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_context_compacted(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };

    vec![AgentEvent::ContextCompacted {
        thread_id,
        turn_id,
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_token_usage(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(token_usage) = params.get("tokenUsage") else {
        return Vec::new();
    };

    vec![AgentEvent::TokenUsageUpdated {
        thread_id,
        turn_id,
        token_usage: project_token_usage_for_ui(token_usage),
        created_at: created_at.to_string(),
    }]
}

fn project_token_usage_for_ui(token_usage: &Value) -> Value {
    let mut projected = Map::new();
    for key in ["total", "last"] {
        let Some(breakdown) = token_usage.get(key) else {
            continue;
        };
        let mut projected_breakdown = Map::new();
        for field in [
            "totalTokens",
            "inputTokens",
            "cachedInputTokens",
            "outputTokens",
            "reasoningOutputTokens",
        ] {
            if let Some(value) = breakdown.get(field).filter(|value| value.is_number()) {
                projected_breakdown.insert(field.to_string(), value.clone());
            }
        }
        projected.insert(key.to_string(), Value::Object(projected_breakdown));
    }
    if let Some(context_window) = token_usage.get("modelContextWindow") {
        if context_window.is_number() || context_window.is_null() {
            projected.insert("modelContextWindow".to_string(), context_window.clone());
        }
    }
    Value::Object(projected)
}

fn normalize_codex_model_rerouted(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(from_model) = extract_string(params, "/fromModel") else {
        return Vec::new();
    };
    let Some(to_model) = extract_string(params, "/toModel") else {
        return Vec::new();
    };
    let Some(reason) = extract_string(params, "/reason") else {
        return Vec::new();
    };

    vec![AgentEvent::ModelRerouted {
        thread_id,
        turn_id,
        from_model: truncate_ui_text(&from_model, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        to_model: truncate_ui_text(&to_model, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        reason: project_ui_sensitive_text(&reason, AGENT_UI_TEXT_PREVIEW_CHARS),
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_model_safety_buffering(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(model) = extract_string(params, "/model") else {
        return Vec::new();
    };
    let Some(use_cases) = extract_bounded_string_array(
        params,
        "/useCases",
        AGENT_UI_MODEL_STATUS_ITEMS,
        AGENT_UI_TEXT_PREVIEW_CHARS,
    ) else {
        return Vec::new();
    };
    let Some(reasons) = extract_bounded_string_array(
        params,
        "/reasons",
        AGENT_UI_MODEL_STATUS_ITEMS,
        AGENT_UI_TEXT_PREVIEW_CHARS,
    ) else {
        return Vec::new();
    };
    let Some(show_buffering_ui) = params.get("showBufferingUi").and_then(Value::as_bool) else {
        return Vec::new();
    };

    vec![AgentEvent::ModelSafetyBufferingUpdated {
        thread_id,
        turn_id,
        model: truncate_ui_text(&model, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        use_cases,
        reasons,
        show_buffering_ui,
        faster_model: extract_string(params, "/fasterModel")
            .map(|value| truncate_ui_text(&value, AGENT_UI_MODEL_IDENTIFIER_CHARS)),
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_model_verification(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(verifications) = extract_bounded_string_array(
        params,
        "/verifications",
        AGENT_UI_MODEL_STATUS_ITEMS,
        AGENT_UI_TEXT_PREVIEW_CHARS,
    ) else {
        return Vec::new();
    };

    vec![AgentEvent::ModelVerificationUpdated {
        thread_id,
        turn_id,
        verifications,
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_config_warning(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(summary) = extract_string(params, "/summary") else {
        return Vec::new();
    };

    vec![AgentEvent::ConfigWarning {
        summary: project_ui_sensitive_text(&summary, AGENT_UI_TEXT_PREVIEW_CHARS),
        details: extract_string(params, "/details")
            .map(|value| project_ui_sensitive_text(&value, AGENT_UI_STATUS_MESSAGE_CHARS)),
        path: extract_string(params, "/path")
            .map(|value| truncate_ui_text(&value, AGENT_UI_TEXT_PREVIEW_CHARS)),
        range: params.get("range").and_then(project_config_text_range),
        created_at: created_at.to_string(),
    }]
}

fn project_config_text_range(range: &Value) -> Option<Value> {
    let mut projected = Map::new();
    for key in ["start", "end"] {
        let position = range.get(key)?;
        let mut projected_position = Map::new();
        for field in ["line", "column"] {
            if let Some(value) = position.get(field).filter(|value| value.is_number()) {
                projected_position.insert(field.to_string(), value.clone());
            }
        }
        projected.insert(key.to_string(), Value::Object(projected_position));
    }
    Some(Value::Object(projected))
}

fn normalize_codex_guardian_warning(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(message) = extract_string(params, "/message") else {
        return Vec::new();
    };

    vec![AgentEvent::GuardianWarning {
        thread_id,
        message: project_ui_sensitive_text(&message, AGENT_UI_STATUS_MESSAGE_CHARS),
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_item_delta(method: &str, params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let Some(item_id) = extract_string(params, "/itemId") else {
        return Vec::new();
    };
    let item_type = match method {
        "item/agentMessage/delta" => "agentMessage",
        "item/plan/delta" => "plan",
        "item/commandExecution/outputDelta" | "item/commandExecution/terminalInteraction" => {
            "commandExecution"
        }
        "item/fileChange/outputDelta" | "item/fileChange/patchUpdated" => "fileChange",
        "item/mcpToolCall/progress" => "mcpToolCall",
        "item/reasoning/summaryTextDelta"
        | "item/reasoning/summaryPartAdded"
        | "item/reasoning/textDelta" => "reasoning",
        _ => "unknown",
    };
    let Some(delta) = normalize_codex_item_delta_payload(method, params) else {
        return Vec::new();
    };

    vec![AgentEvent::ItemDelta {
        thread_id,
        turn_id,
        item_id,
        item_type: item_type.to_string(),
        lifecycle: "in_progress".to_string(),
        method: method.to_string(),
        delta,
        created_at: created_at.to_string(),
    }]
}

fn normalize_codex_item_delta_payload(method: &str, params: &Value) -> Option<Value> {
    match method {
        "item/fileChange/patchUpdated" => params
            .get("changes")
            .cloned()
            .or_else(|| Some(Value::Array(Vec::new()))),
        "item/reasoning/summaryTextDelta" => {
            let summary_index = params.get("summaryIndex")?.as_u64()?;
            if summary_index >= AGENT_UI_REASONING_PARTS as u64 {
                return None;
            }
            Some(json!({
                "text": truncate_ui_text(
                    &extract_string(params, "/delta")?,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                ),
                "summaryIndex": summary_index,
                "visibility": "summary"
            }))
        }
        "item/reasoning/summaryPartAdded" => {
            let summary_index = params.get("summaryIndex")?.as_u64()?;
            if summary_index >= AGENT_UI_REASONING_PARTS as u64 {
                return None;
            }
            Some(json!({
                "summaryIndex": summary_index,
                "visibility": "summary"
            }))
        }
        "item/reasoning/textDelta" => {
            let text_length = extract_string(params, "/delta")?.chars().count();
            Some(json!({
                "contentIndex": params.get("contentIndex")?.as_u64()?,
                "textLength": text_length,
                "visibility": "restricted_debug"
            }))
        }
        "item/mcpToolCall/progress" => Some(json!({
            "message": project_ui_sensitive_text(
                &extract_string(params, "/message")?,
                AGENT_UI_TEXT_PREVIEW_CHARS,
            )
        })),
        "item/commandExecution/terminalInteraction" => {
            let interaction_length = extract_string(params, "/stdin")?.chars().count();
            Some(json!({
                "processId": truncate_ui_text(
                    &extract_string(params, "/processId")?,
                    AGENT_UI_TEXT_PREVIEW_CHARS,
                ),
                "interaction": {
                    "type": "stdin",
                    "characterCount": interaction_length
                }
            }))
        }
        "item/agentMessage/delta" | "item/plan/delta" => Some(Value::String(truncate_ui_text(
            &extract_string(params, "/delta")?,
            AGENT_UI_LONG_TEXT_CHARS,
        ))),
        "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
            let (tail, _) = truncate_ui_text_tail(
                &extract_string(params, "/delta")?,
                AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS,
            );
            Some(Value::String(tail))
        }
        _ => Some(
            params
                .get("delta")
                .or_else(|| params.get("part"))
                .cloned()
                .unwrap_or_else(|| params.clone()),
        ),
    }
}

fn normalize_codex_plan_updated(params: &Value, created_at: &str) -> Vec<AgentEvent> {
    let Some(thread_id) = extract_string(params, "/threadId") else {
        return Vec::new();
    };
    let Some(turn_id) = extract_string(params, "/turnId") else {
        return Vec::new();
    };
    let plan = params
        .get("plan")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .take(AGENT_UI_PLAN_STEPS)
                .filter_map(|step| {
                    Some(AgentPlanStep {
                        step: truncate_ui_text(
                            &extract_string(step, "/step")?,
                            AGENT_UI_TEXT_PREVIEW_CHARS,
                        ),
                        status: truncate_ui_text(
                            &extract_string(step, "/status")
                                .unwrap_or_else(|| "pending".to_string()),
                            AGENT_UI_MODEL_IDENTIFIER_CHARS,
                        ),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    vec![AgentEvent::PlanUpdated {
        thread_id,
        turn_id,
        explanation: extract_string(params, "/explanation")
            .filter(|value| !value.is_empty())
            .map(|value| truncate_ui_text(&value, AGENT_UI_STATUS_MESSAGE_CHARS)),
        plan,
        created_at: created_at.to_string(),
    }]
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
        status: truncate_ui_text(&status, AGENT_UI_MODEL_IDENTIFIER_CHARS),
        action: extract_string(params, "/action/type")
            .map(|value| truncate_ui_text(&value, AGENT_UI_MODEL_IDENTIFIER_CHARS)),
        rationale: extract_string(params, "/review/rationale")
            .filter(|value| !value.is_empty())
            .map(|value| project_ui_sensitive_text(&value, AGENT_UI_STATUS_MESSAGE_CHARS)),
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
    let status = truncate_ui_text(
        &extract_string(params, "/turn/status").unwrap_or_else(|| "completed".to_string()),
        AGENT_UI_MODEL_IDENTIFIER_CHARS,
    );
    let mut events = vec![AgentEvent::TurnCompleted {
        thread_id: extract_string(params, "/threadId"),
        turn_id: extract_string(params, "/turn/id"),
        status: status.clone(),
        final_message: extract_final_agent_message(params)
            .map(|message| truncate_ui_text(&message, AGENT_UI_LONG_TEXT_CHARS)),
        created_at: created_at.to_string(),
    }];

    if let Some(error) = params
        .pointer("/turn/error")
        .filter(|error| !error.is_null())
    {
        events.push(AgentEvent::Error {
            message: project_ui_sensitive_text(
                &format_codex_error(error),
                AGENT_UI_STATUS_MESSAGE_CHARS,
            ),
            retryable: false,
            terminal: status == "failed",
            thread_id: extract_string(params, "/threadId"),
            turn_id: extract_string(params, "/turn/id"),
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
            items
                .iter()
                .rev()
                .find(|item| {
                    item.get("type").and_then(Value::as_str) == Some("agentMessage")
                        && item.get("phase").and_then(Value::as_str) == Some("final_answer")
                })
                .or_else(|| {
                    items.iter().rev().find(|item| {
                        item.get("type").and_then(Value::as_str) == Some("agentMessage")
                    })
                })
                .and_then(|item| extract_string(item, "/text").filter(|text| !text.is_empty()))
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
        lines.push(truncate_ui_text(
            &explanation,
            AGENT_UI_STATUS_MESSAGE_CHARS,
        ));
    }

    if let Some(plan) = params.get("plan").and_then(Value::as_array) {
        lines.extend(plan.iter().take(AGENT_UI_PLAN_STEPS).filter_map(|step| {
            let step_text =
                truncate_ui_text(&extract_string(step, "/step")?, AGENT_UI_TEXT_PREVIEW_CHARS);
            let status = truncate_ui_text(
                &extract_string(step, "/status").unwrap_or_else(|| "pending".to_string()),
                AGENT_UI_MODEL_IDENTIFIER_CHARS,
            );
            Some(format!("[{status}] {step_text}"))
        }));
    }

    if lines.is_empty() {
        None
    } else {
        Some(truncate_ui_text(
            &lines.join("\n"),
            AGENT_UI_LONG_TEXT_CHARS,
        ))
    }
}

fn format_exec_plan_update(item: &Value) -> Option<String> {
    extract_string(item, "/text")
        .or_else(|| extract_string(item, "/message"))
        .filter(|text| !text.is_empty())
        .map(|text| truncate_ui_text(&text, AGENT_UI_LONG_TEXT_CHARS))
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

fn extract_bounded_string_array(
    value: &Value,
    pointer: &str,
    max_items: usize,
    max_chars: usize,
) -> Option<Vec<String>> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(max_items)
                .filter_map(Value::as_str)
                .map(|item| project_ui_sensitive_text(item, max_chars))
                .collect()
        })
}

fn extract_protocol_timestamp(value: &Value, pointer: &str, fallback: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_i64)
        .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        .map(|timestamp| timestamp.to_rfc3339())
        .unwrap_or_else(|| fallback.to_string())
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
            vec![AgentEvent::ItemDelta {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-1".to_string(),
                item_type: "agentMessage".to_string(),
                lifecycle: "in_progress".to_string(),
                method: "item/agentMessage/delta".to_string(),
                delta: json!("正在修改首页"),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            plan_events,
            vec![AgentEvent::ItemDelta {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-2".to_string(),
                item_type: "plan".to_string(),
                lifecycle: "in_progress".to_string(),
                method: "item/plan/delta".to_string(),
                delta: json!("实现主要布局"),
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
            vec![AgentEvent::PlanUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                explanation: Some("计划已更新".to_string()),
                plan: vec![
                    AgentPlanStep {
                        step: "读取项目结构".to_string(),
                        status: "completed".to_string(),
                    },
                    AgentPlanStep {
                        step: "实现 demo".to_string(),
                        status: "inProgress".to_string(),
                    },
                ],
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
                    "completedAtMs": 1782259200000_i64,
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
            vec![AgentEvent::ItemCompleted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-1".to_string(),
                item_type: "commandExecution".to_string(),
                lifecycle: "completed".to_string(),
                status: Some("completed".to_string()),
                completed_at: "2026-06-24T00:00:00+00:00".to_string(),
                item: json!({
                    "id": "item-1",
                    "type": "commandExecution",
                    "command": "npm run check",
                    "commandActions": [],
                    "cwd": "/tmp/demo",
                    "status": "completed"
                }),
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
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1782259200000_i64,
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
            vec![AgentEvent::ItemStarted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-1".to_string(),
                item_type: "commandExecution".to_string(),
                lifecycle: "in_progress".to_string(),
                status: None,
                started_at: "2026-06-24T00:00:00+00:00".to_string(),
                item: json!({
                    "id": "item-1",
                    "type": "commandExecution",
                    "command": "npm run dev"
                }),
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
            vec![AgentEvent::ItemDelta {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-1".to_string(),
                item_type: "fileChange".to_string(),
                lifecycle: "in_progress".to_string(),
                method: "item/fileChange/patchUpdated".to_string(),
                delta: json!([
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
                ]),
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_command_output_delta_for_the_same_item() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "item/commandExecution/outputDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "command-1",
                    "delta": "tests passed\n"
                }
            }),
            "2026-07-13T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::ItemDelta {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "command-1".to_string(),
                item_type: "commandExecution".to_string(),
                lifecycle: "in_progress".to_string(),
                method: "item/commandExecution/outputDelta".to_string(),
                delta: json!("tests passed\n"),
                created_at: "2026-07-13T00:00:00Z".to_string(),
            }]
        );
    }

    #[test]
    fn normalizes_turn_diff_as_latest_snapshot() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "turn/diff/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "diff": "diff --git a/src/App.tsx b/src/App.tsx\n-old\n+new\n"
                }
            }),
            "2026-07-13T00:00:00Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::TurnDiffUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                diff: "diff --git a/src/App.tsx b/src/App.tsx\n-old\n+new\n".to_string(),
                created_at: "2026-07-13T00:00:00Z".to_string(),
            }]
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
                    thread_id: Some("thread-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    status: "failed".to_string(),
                    final_message: Some("已完成第一版。".to_string()),
                    created_at: "2026-06-24T00:00:00Z".to_string(),
                },
                AgentEvent::Error {
                    message: "Network blocked".to_string(),
                    retryable: false,
                    terminal: true,
                    thread_id: Some("thread-1".to_string()),
                    turn_id: Some("turn-1".to_string()),
                    created_at: "2026-06-24T00:00:00Z".to_string(),
                },
            ]
        );
    }

    #[test]
    fn preserves_agent_message_phase_and_item_lifecycle_timestamps() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1782259200000_i64,
                    "item": {
                        "id": "message-1",
                        "type": "agentMessage",
                        "text": "最终答复",
                        "phase": "final_answer"
                    }
                }
            }),
            "2026-06-24T00:00:01Z",
        );

        assert_eq!(
            events,
            vec![AgentEvent::ItemCompleted {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "message-1".to_string(),
                item_type: "agentMessage".to_string(),
                lifecycle: "completed".to_string(),
                status: None,
                completed_at: "2026-06-24T00:00:00+00:00".to_string(),
                item: json!({
                    "id": "message-1",
                    "type": "agentMessage",
                    "text": "最终答复",
                    "phase": "final_answer"
                }),
                created_at: "2026-06-24T00:00:01Z".to_string(),
            }]
        );
    }

    #[test]
    fn distinguishes_warnings_retryable_errors_and_terminal_errors() {
        let warning = normalize_codex_notification_at(
            &json!({
                "method": "warning",
                "params": {
                    "message": "上下文接近限制",
                    "threadId": "thread-1"
                }
            }),
            "2026-06-24T00:00:00Z",
        );
        let retryable = normalize_codex_notification_at(
            &json!({
                "method": "error",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "willRetry": true,
                    "error": { "message": "连接中断" }
                }
            }),
            "2026-06-24T00:00:01Z",
        );
        let terminal = normalize_codex_notification_at(
            &json!({
                "method": "error",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "willRetry": false,
                    "error": { "message": "重试次数耗尽" }
                }
            }),
            "2026-06-24T00:00:02Z",
        );

        assert!(matches!(warning.as_slice(), [AgentEvent::Warning { .. }]));
        assert!(matches!(
            retryable.as_slice(),
            [AgentEvent::Error {
                retryable: true,
                terminal: false,
                ..
            }]
        ));
        assert!(matches!(
            terminal.as_slice(),
            [AgentEvent::Error {
                retryable: false,
                terminal: true,
                ..
            }]
        ));

        let mut summary = AgentRunEventSummary::default();
        update_agent_run_summary(&mut summary, &retryable[0]);
        assert!(!summary.terminal);
        assert!(summary.error_message.is_none());
        update_agent_run_summary(&mut summary, &terminal[0]);
        assert!(summary.terminal);
        assert_eq!(summary.completion_status(), "failed");
    }

    #[test]
    fn preserves_interrupted_turn_status() {
        let events = normalize_codex_notification_at(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1",
                        "status": "interrupted",
                        "items": []
                    }
                }
            }),
            "2026-06-24T00:00:00Z",
        );
        let mut summary = AgentRunEventSummary::default();
        update_agent_run_summary(&mut summary, &events[0]);

        assert!(matches!(
            events.as_slice(),
            [AgentEvent::TurnCompleted { status, .. }] if status == "interrupted"
        ));
        assert!(summary.terminal);
        assert_eq!(summary.completion_status(), "interrupted");
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
                thread_id: None,
                turn_id: None,
                status: "completed".to_string(),
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
                retryable: false,
                terminal: true,
                thread_id: None,
                turn_id: None,
                created_at: "2026-06-24T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            error_events,
            vec![AgentEvent::Error {
                message: "Authentication required".to_string(),
                retryable: false,
                terminal: true,
                thread_id: None,
                turn_id: None,
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
            status: "completed".to_string(),
            error: None,
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
                "status": "completed",
                "error": null,
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
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                status: "completed".to_string(),
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
                retryable: false,
                terminal: true,
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                created_at: "2026-06-24T00:00:03Z".to_string(),
            },
        );
        assert_eq!(summary.error_message, Some("失败".to_string()));
    }

    #[test]
    fn normalizes_reasoning_parts_and_redacts_raw_reasoning_text() {
        let summary_part = normalize_codex_notification_at(
            &json!({
                "method": "item/reasoning/summaryPartAdded",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "reasoning-1",
                    "summaryIndex": 1
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let summary_delta = normalize_codex_notification_at(
            &json!({
                "method": "item/reasoning/summaryTextDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "reasoning-1",
                    "summaryIndex": 1,
                    "delta": "检查测试"
                }
            }),
            "2026-07-13T00:00:01Z",
        );
        let second_summary_delta = normalize_codex_notification_at(
            &json!({
                "method": "item/reasoning/summaryTextDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "reasoning-1",
                    "summaryIndex": 2,
                    "delta": "准备修改"
                }
            }),
            "2026-07-13T00:00:02Z",
        );
        let raw_delta = normalize_codex_notification_at(
            &json!({
                "method": "item/reasoning/textDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "reasoning-1",
                    "contentIndex": 3,
                    "delta": "raw-secret"
                }
            }),
            "2026-07-13T00:00:03Z",
        );

        assert!(matches!(
            summary_part.as_slice(),
            [AgentEvent::ItemDelta { delta, .. }]
                if delta == &json!({ "summaryIndex": 1, "visibility": "summary" })
        ));
        assert!(matches!(
            summary_delta.as_slice(),
            [AgentEvent::ItemDelta { delta, .. }]
                if delta == &json!({
                    "text": "检查测试",
                    "summaryIndex": 1,
                    "visibility": "summary"
                })
        ));
        assert!(matches!(
            second_summary_delta.as_slice(),
            [AgentEvent::ItemDelta { delta, .. }]
                if delta == &json!({
                    "text": "准备修改",
                    "summaryIndex": 2,
                    "visibility": "summary"
                })
        ));
        assert!(matches!(
            raw_delta.as_slice(),
            [AgentEvent::ItemDelta { delta, .. }]
                if delta == &json!({
                    "contentIndex": 3,
                    "textLength": 10,
                    "visibility": "restricted_debug"
                })
        ));
        assert!(!serde_json::to_string(&raw_delta)
            .expect("raw reasoning event should serialize")
            .contains("raw-secret"));
    }

    #[test]
    fn normalizes_mcp_progress_and_redacts_terminal_stdin() {
        let progress = normalize_codex_notification_at(
            &json!({
                "method": "item/mcpToolCall/progress",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "mcp-1",
                    "message": "正在读取资源"
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let terminal = normalize_codex_notification_at(
            &json!({
                "method": "item/commandExecution/terminalInteraction",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "command-1",
                    "processId": "pty-42",
                    "stdin": "super-secret-input"
                }
            }),
            "2026-07-13T00:00:01Z",
        );

        assert!(matches!(
            progress.as_slice(),
            [AgentEvent::ItemDelta { delta, .. }]
                if delta == &json!({ "message": "正在读取资源" })
        ));
        assert!(matches!(
            terminal.as_slice(),
            [AgentEvent::ItemDelta { delta, .. }]
                if delta == &json!({
                    "processId": "pty-42",
                    "interaction": {
                        "type": "stdin",
                        "characterCount": 18
                    }
                })
        ));
        let serialized =
            serde_json::to_string(&terminal).expect("terminal interaction event should serialize");
        assert!(!serialized.contains("super-secret-input"));
        assert!(!serialized.contains("\"stdin\":"));
    }

    #[test]
    fn normalizes_hook_run_lifecycle_notifications() {
        let started_run = json!({
            "id": "hook-1",
            "eventName": "preToolUse",
            "handlerType": "command",
            "executionMode": "sync",
            "scope": "thread",
            "sourcePath": "/tmp/demo/.codex/hooks.json",
            "source": "project",
            "displayOrder": 0,
            "status": "running",
            "statusMessage": "检查命令",
            "startedAt": 1783900800000_i64,
            "completedAt": null,
            "durationMs": null,
            "entries": []
        });
        let mut completed_run = started_run.clone();
        completed_run["status"] = json!("completed");
        completed_run["completedAt"] = json!(1783900800100_i64);
        completed_run["durationMs"] = json!(100);
        completed_run["entries"] = json!([{ "kind": "feedback", "text": "允许执行" }]);

        let started = normalize_codex_notification_at(
            &json!({
                "method": "hook/started",
                "params": {
                    "threadId": "thread-1",
                    "turnId": null,
                    "run": started_run.clone()
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let completed = normalize_codex_notification_at(
            &json!({
                "method": "hook/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "run": completed_run.clone()
                }
            }),
            "2026-07-13T00:00:01Z",
        );

        assert_eq!(
            started,
            vec![AgentEvent::HookRunUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: None,
                hook_id: "hook-1".to_string(),
                lifecycle: "in_progress".to_string(),
                run: started_run,
                created_at: "2026-07-13T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            completed,
            vec![AgentEvent::HookRunUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                hook_id: "hook-1".to_string(),
                lifecycle: "completed".to_string(),
                run: completed_run,
                created_at: "2026-07-13T00:00:01Z".to_string(),
            }]
        );
    }

    #[test]
    fn hook_run_projection_caps_entries_and_preserves_timing_metadata() {
        let entries = (0..(AGENT_UI_HOOK_ENTRIES + 10))
            .map(|index| {
                json!({
                    "kind": "feedback",
                    "text": format!(
                        "{index}: {}",
                        "hook output ".repeat(AGENT_UI_HOOK_ENTRY_CHARS / 4)
                    ),
                    "secretToken": "HOOK_SECRET_MUST_NOT_CROSS_IPC"
                })
            })
            .collect::<Vec<_>>();
        let events = normalize_codex_notification_at(
            &json!({
                "method": "hook/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "run": {
                        "id": "hook-bounded",
                        "eventName": "postToolUse",
                        "handlerType": "command",
                        "executionMode": "sync",
                        "scope": "turn",
                        "sourcePath": "/tmp/demo/.codex/hooks.json",
                        "source": "project",
                        "displayOrder": 7,
                        "status": "completed",
                        "statusMessage": "完成",
                        "startedAt": 1000,
                        "completedAt": 1200,
                        "durationMs": 200,
                        "entries": entries,
                        "arbitraryPayload": "not projected"
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );

        let [AgentEvent::HookRunUpdated { run, .. }] = events.as_slice() else {
            panic!("hook lifecycle event");
        };
        let projected_entries = run
            .get("entries")
            .and_then(Value::as_array)
            .expect("projected hook entries");
        assert!(projected_entries.len() <= AGENT_UI_HOOK_ENTRIES);
        assert!(projected_entries.iter().all(|entry| {
            entry
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.chars().count() <= AGENT_UI_HOOK_ENTRY_CHARS)
        }));
        assert!(
            projected_entries
                .iter()
                .filter_map(|entry| entry.get("text").and_then(Value::as_str))
                .map(|text| text.chars().count())
                .sum::<usize>()
                <= AGENT_UI_HOOK_TOTAL_CHARS
        );
        assert_eq!(run.get("displayOrder").and_then(Value::as_u64), Some(7));
        assert_eq!(run.get("startedAt").and_then(Value::as_u64), Some(1000));
        assert_eq!(run.get("completedAt").and_then(Value::as_u64), Some(1200));
        assert_eq!(run.get("durationMs").and_then(Value::as_u64), Some(200));
        assert!(run.get("arbitraryPayload").is_none());
        assert_eq!(run.get("_uiProjectionTruncated"), Some(&Value::Bool(true)));
        assert!(!serde_json::to_string(&events)
            .expect("hook event should serialize")
            .contains("HOOK_SECRET_MUST_NOT_CROSS_IPC"));
    }

    #[test]
    fn reasoning_and_model_status_collections_are_bounded() {
        let summary = (0..(AGENT_UI_REASONING_PARTS + 10))
            .map(|index| format!("{index}:{}", "s".repeat(AGENT_UI_TEXT_PREVIEW_CHARS + 20)))
            .collect::<Vec<_>>();
        let reasoning = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "reasoning-bounded",
                        "type": "reasoning",
                        "summary": summary,
                        "content": []
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let out_of_range_delta = normalize_codex_notification_at(
            &json!({
                "method": "item/reasoning/summaryTextDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "reasoning-bounded",
                    "summaryIndex": AGENT_UI_REASONING_PARTS,
                    "delta": "must be ignored"
                }
            }),
            "2026-07-13T00:00:01Z",
        );
        let many_status_values = (0..(AGENT_UI_MODEL_STATUS_ITEMS + 10))
            .map(|index| {
                format!(
                    "{index}: {}",
                    "model status ".repeat(AGENT_UI_TEXT_PREVIEW_CHARS / 4)
                )
            })
            .collect::<Vec<_>>();
        let model = normalize_codex_notification_at(
            &json!({
                "method": "model/safetyBuffering/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "model": "model-b",
                    "useCases": many_status_values,
                    "reasons": ["policy-check"],
                    "showBufferingUi": true,
                    "fasterModel": null
                }
            }),
            "2026-07-13T00:00:02Z",
        );

        let [AgentEvent::ItemCompleted { item, .. }] = reasoning.as_slice() else {
            panic!("reasoning lifecycle event");
        };
        let projected_summary = item
            .get("summary")
            .and_then(Value::as_array)
            .expect("projected reasoning summary");
        assert_eq!(projected_summary.len(), AGENT_UI_REASONING_PARTS);
        assert!(projected_summary.iter().all(|part| {
            part.as_str()
                .is_some_and(|text| text.chars().count() <= AGENT_UI_TEXT_PREVIEW_CHARS)
        }));
        assert!(out_of_range_delta.is_empty());

        let [AgentEvent::ModelSafetyBufferingUpdated { use_cases, .. }] = model.as_slice() else {
            panic!("model buffering event");
        };
        assert_eq!(use_cases.len(), AGENT_UI_MODEL_STATUS_ITEMS);
        assert!(use_cases
            .iter()
            .all(|value| value.chars().count() <= AGENT_UI_TEXT_PREVIEW_CHARS));
    }

    #[test]
    fn normalizes_context_token_usage_and_schema_model_statuses() {
        let token_usage = json!({
            "total": {
                "totalTokens": 120,
                "inputTokens": 80,
                "cachedInputTokens": 20,
                "outputTokens": 40,
                "reasoningOutputTokens": 10
            },
            "last": {
                "totalTokens": 30,
                "inputTokens": 20,
                "cachedInputTokens": 5,
                "outputTokens": 10,
                "reasoningOutputTokens": 3
            },
            "modelContextWindow": null
        });
        let context = normalize_codex_notification_at(
            &json!({
                "method": "thread/compacted",
                "params": { "threadId": "thread-1", "turnId": "turn-1" }
            }),
            "2026-07-13T00:00:00Z",
        );
        let usage = normalize_codex_notification_at(
            &json!({
                "method": "thread/tokenUsage/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "tokenUsage": token_usage.clone()
                }
            }),
            "2026-07-13T00:00:01Z",
        );
        let rerouted = normalize_codex_notification_at(
            &json!({
                "method": "model/rerouted",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "fromModel": "model-a",
                    "toModel": "model-b",
                    "reason": "highRiskCyberActivity"
                }
            }),
            "2026-07-13T00:00:02Z",
        );
        let buffering = normalize_codex_notification_at(
            &json!({
                "method": "model/safetyBuffering/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "model": "model-b",
                    "useCases": ["cyber", "future-use-case"],
                    "reasons": ["policy-check"],
                    "showBufferingUi": true,
                    "fasterModel": "model-fast"
                }
            }),
            "2026-07-13T00:00:03Z",
        );
        let verification = normalize_codex_notification_at(
            &json!({
                "method": "model/verification",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "verifications": ["trustedAccessForCyber"]
                }
            }),
            "2026-07-13T00:00:04Z",
        );

        assert!(matches!(
            context.as_slice(),
            [AgentEvent::ContextCompacted {
                thread_id,
                turn_id,
                ..
            }] if thread_id == "thread-1" && turn_id == "turn-1"
        ));
        assert_eq!(
            usage,
            vec![AgentEvent::TokenUsageUpdated {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                token_usage,
                created_at: "2026-07-13T00:00:01Z".to_string(),
            }]
        );
        assert!(matches!(
            rerouted.as_slice(),
            [AgentEvent::ModelRerouted { reason, .. }] if reason == "highRiskCyberActivity"
        ));
        assert!(matches!(
            buffering.as_slice(),
            [AgentEvent::ModelSafetyBufferingUpdated {
                use_cases,
                reasons,
                show_buffering_ui: true,
                faster_model: Some(faster_model),
                ..
            }] if use_cases == &["cyber", "future-use-case"]
                && reasons == &["policy-check"]
                && faster_model == "model-fast"
        ));
        assert!(matches!(
            verification.as_slice(),
            [AgentEvent::ModelVerificationUpdated { verifications, .. }]
                if verifications == &["trustedAccessForCyber"]
        ));
    }

    #[test]
    fn model_string_enums_remain_forward_compatible() {
        let rerouted = normalize_codex_notification_at(
            &json!({
                "method": "model/rerouted",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "fromModel": "model-a",
                    "toModel": "model-b",
                    "reason": "futureRoutingPolicy"
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let verification = normalize_codex_notification_at(
            &json!({
                "method": "model/verification",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "verifications": ["futureVerification"]
                }
            }),
            "2026-07-13T00:00:01Z",
        );

        assert!(matches!(
            rerouted.as_slice(),
            [AgentEvent::ModelRerouted { reason, .. }] if reason == "futureRoutingPolicy"
        ));
        assert!(matches!(
            verification.as_slice(),
            [AgentEvent::ModelVerificationUpdated { verifications, .. }]
                if verifications == &["futureVerification"]
        ));
    }

    #[test]
    fn normalizes_config_and_guardian_warnings() {
        let config = normalize_codex_notification_at(
            &json!({
                "method": "configWarning",
                "params": {
                    "summary": "配置项已弃用",
                    "details": "请迁移到新配置项",
                    "path": "/tmp/demo/.codex/config.toml",
                    "range": {
                        "start": { "line": 3, "column": 1 },
                        "end": { "line": 3, "column": 12 }
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let guardian = normalize_codex_notification_at(
            &json!({
                "method": "guardianWarning",
                "params": {
                    "threadId": "thread-1",
                    "message": "该操作需要额外审查"
                }
            }),
            "2026-07-13T00:00:01Z",
        );

        assert_eq!(
            config,
            vec![AgentEvent::ConfigWarning {
                summary: "配置项已弃用".to_string(),
                details: Some("请迁移到新配置项".to_string()),
                path: Some("/tmp/demo/.codex/config.toml".to_string()),
                range: Some(json!({
                    "start": { "line": 3, "column": 1 },
                    "end": { "line": 3, "column": 12 }
                })),
                created_at: "2026-07-13T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(
            guardian,
            vec![AgentEvent::GuardianWarning {
                thread_id: "thread-1".to_string(),
                message: "该操作需要额外审查".to_string(),
                created_at: "2026-07-13T00:00:01Z".to_string(),
            }]
        );
    }

    #[test]
    fn all_thread_item_types_keep_lifecycle_events_after_ui_projection() {
        let items = vec![
            json!({
                "id": "user-1",
                "type": "userMessage",
                "clientId": null,
                "content": []
            }),
            json!({ "id": "hook-prompt-1", "type": "hookPrompt", "fragments": [] }),
            json!({
                "id": "message-1",
                "type": "agentMessage",
                "text": "完成",
                "phase": "final_answer",
                "memoryCitation": null
            }),
            json!({ "id": "plan-1", "type": "plan", "text": "实现功能" }),
            json!({
                "id": "reasoning-1",
                "type": "reasoning",
                "summary": ["检查实现"],
                "content": ["restricted"]
            }),
            json!({
                "id": "command-1",
                "type": "commandExecution",
                "command": "npm test",
                "cwd": "/tmp/demo",
                "processId": null,
                "source": "agent",
                "status": "completed",
                "commandActions": [],
                "aggregatedOutput": "ok",
                "exitCode": 0,
                "durationMs": 15
            }),
            json!({
                "id": "file-1",
                "type": "fileChange",
                "status": "completed",
                "changes": [{
                    "path": "/tmp/demo/src/App.tsx",
                    "kind": { "type": "update", "move_path": null },
                    "diff": "@@ -1 +1 @@\n-old\n+new\n"
                }]
            }),
            json!({
                "id": "mcp-1",
                "type": "mcpToolCall",
                "server": "docs",
                "tool": "search",
                "status": "completed",
                "arguments": { "query": "Codex" },
                "appContext": null,
                "pluginId": null,
                "result": { "content": [], "structuredContent": null, "_meta": null },
                "error": null,
                "durationMs": 42
            }),
            json!({
                "id": "dynamic-1",
                "type": "dynamicToolCall",
                "namespace": null,
                "tool": "render",
                "arguments": {},
                "status": "completed",
                "contentItems": [{ "type": "inputText", "text": "done" }],
                "success": true,
                "durationMs": 10
            }),
            json!({
                "id": "collab-1",
                "type": "collabAgentToolCall",
                "tool": "spawnAgent",
                "status": "completed",
                "senderThreadId": "thread-1",
                "receiverThreadIds": ["thread-2"],
                "prompt": "检查测试",
                "model": null,
                "reasoningEffort": null,
                "agentsStates": {
                    "thread-2": { "status": "completed", "message": "完成" }
                }
            }),
            json!({
                "id": "activity-1",
                "type": "subAgentActivity",
                "kind": "interacted",
                "agentThreadId": "thread-2",
                "agentPath": "reviewer"
            }),
            json!({
                "id": "search-1",
                "type": "webSearch",
                "query": "Codex app server",
                "action": { "type": "openPage", "url": "https://developers.openai.com" }
            }),
            json!({ "id": "view-1", "type": "imageView", "path": "/tmp/demo/a.png" }),
            json!({
                "id": "image-1",
                "type": "imageGeneration",
                "status": "completed",
                "revisedPrompt": null,
                "result": "opaque-result",
                "savedPath": "/tmp/demo/generated.png"
            }),
            json!({ "id": "sleep-1", "type": "sleep", "durationMs": 100 }),
            json!({ "id": "review-in-1", "type": "enteredReviewMode", "review": "review" }),
            json!({ "id": "review-out-1", "type": "exitedReviewMode", "review": "review" }),
            json!({ "id": "compact-1", "type": "contextCompaction" }),
        ];
        assert_eq!(items.len(), 18);

        for item in items {
            let item_id = extract_string(&item, "/id").expect("fixture item id");
            let item_type = extract_string(&item, "/type").expect("fixture item type");
            let started = normalize_codex_notification_at(
                &json!({
                    "method": "item/started",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "startedAtMs": 1783900800000_i64,
                        "item": item.clone()
                    }
                }),
                "2026-07-13T00:00:00Z",
            );
            let completed = normalize_codex_notification_at(
                &json!({
                    "method": "item/completed",
                    "params": {
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "completedAtMs": 1783900800100_i64,
                        "item": item.clone()
                    }
                }),
                "2026-07-13T00:00:01Z",
            );

            assert!(matches!(
                started.as_slice(),
                [AgentEvent::ItemStarted {
                    item_id: actual_id,
                    item_type: actual_type,
                    item: actual_item,
                    ..
                }] if actual_id == &item_id
                    && actual_type == &item_type
                    && actual_item.pointer("/id").and_then(Value::as_str) == Some(item_id.as_str())
                    && actual_item.pointer("/type").and_then(Value::as_str) == Some(item_type.as_str())
            ));
            assert!(matches!(
                completed.as_slice(),
                [AgentEvent::ItemCompleted {
                    item_id: actual_id,
                    item_type: actual_type,
                    item: actual_item,
                    ..
                }] if actual_id == &item_id
                    && actual_type == &item_type
                    && actual_item.pointer("/id").and_then(Value::as_str) == Some(item_id.as_str())
                    && actual_item.pointer("/type").and_then(Value::as_str) == Some(item_type.as_str())
            ));
        }
    }

    #[test]
    fn lifecycle_ui_projection_omits_raw_binary_stdin_and_sensitive_tool_data() {
        let reasoning = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "reasoning-1",
                        "type": "reasoning",
                        "summary": ["安全摘要"],
                        "content": ["RAW_REASONING_MUST_NOT_CROSS_IPC"]
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let image = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "image-1",
                        "type": "imageGeneration",
                        "status": "completed",
                        "revisedPrompt": null,
                        "result": "data:image/png;base64,BASE64_IMAGE_MUST_NOT_CROSS_IPC",
                        "savedPath": "/tmp/demo/image.png"
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let mcp = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "mcp-1",
                        "type": "mcpToolCall",
                        "server": "docs",
                        "tool": "read",
                        "status": "completed",
                        "arguments": {
                            "query": "safe query",
                            "apiKey": "MCP_SECRET_MUST_NOT_CROSS_IPC",
                            "url": "https://example.test/file?X-Amz-Signature=SIGNED_URL_MUST_NOT_CROSS_IPC"
                        },
                        "result": {
                            "content": [{
                                "type": "image",
                                "data": "data:image/png;base64,MCP_BASE64_MUST_NOT_CROSS_IPC"
                            }],
                            "structuredContent": null,
                            "_meta": { "cookie": "MCP_COOKIE_MUST_NOT_CROSS_IPC" }
                        },
                        "error": null,
                        "durationMs": 5
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let dynamic = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "dynamic-1",
                        "type": "dynamicToolCall",
                        "namespace": null,
                        "tool": "render",
                        "arguments": { "password": "DYNAMIC_SECRET_MUST_NOT_CROSS_IPC" },
                        "status": "completed",
                        "contentItems": [{
                            "type": "inputImage",
                            "imageUrl": "data:image/png;base64,DYNAMIC_BASE64_MUST_NOT_CROSS_IPC"
                        }],
                        "success": true,
                        "durationMs": 5
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let command = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "command-1",
                        "type": "commandExecution",
                        "command": "read -s password",
                        "cwd": "/tmp/demo",
                        "status": "completed",
                        "commandActions": [],
                        "stdin": "TERMINAL_SECRET_MUST_NOT_CROSS_IPC",
                        "aggregatedOutput": "x".repeat(AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS + 100),
                        "exitCode": 0,
                        "durationMs": 5
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let command_with_secret_output = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "command-secret-output",
                        "type": "commandExecution",
                        "command": "printenv",
                        "cwd": "/tmp/demo",
                        "status": "completed",
                        "commandActions": [],
                        "aggregatedOutput": "token=COMMAND_OUTPUT_SECRET_MUST_NOT_CROSS_IPC",
                        "exitCode": 0,
                        "durationMs": 5
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let unknown = normalize_codex_notification_at(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "completedAtMs": 1783900800000_i64,
                    "item": {
                        "id": "future-1",
                        "type": "futureTool",
                        "safe": "visible",
                        "rawText": "UNKNOWN_RAW_MUST_NOT_CROSS_IPC",
                        "nested": { "authorization": "Bearer UNKNOWN_SECRET" }
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );

        let [AgentEvent::ItemCompleted { item, .. }] = reasoning.as_slice() else {
            panic!("reasoning lifecycle event");
        };
        assert!(item.get("content").is_none());
        assert_eq!(item.get("rawTextAvailable"), Some(&Value::Bool(true)));
        assert_eq!(item.get("contentCount").and_then(Value::as_u64), Some(1));

        let [AgentEvent::ItemCompleted { item, .. }] = image.as_slice() else {
            panic!("image lifecycle event");
        };
        assert!(item.get("result").is_none());
        assert_eq!(item.get("resultAvailable"), Some(&Value::Bool(true)));
        assert!(item.get("resultLength").and_then(Value::as_u64).is_some());

        let [AgentEvent::ItemCompleted { item, .. }] = command.as_slice() else {
            panic!("command lifecycle event");
        };
        assert!(item.get("stdin").is_none());
        assert_eq!(
            item.get("aggregatedOutput")
                .and_then(Value::as_str)
                .map(|output| output.chars().count()),
            Some(AGENT_UI_COMMAND_OUTPUT_TAIL_CHARS)
        );
        assert_eq!(
            item.get("aggregatedOutputTruncated"),
            Some(&Value::Bool(true))
        );

        let [AgentEvent::ItemCompleted { item, .. }] = command_with_secret_output.as_slice() else {
            panic!("command secret output lifecycle event");
        };
        assert_eq!(
            item.get("aggregatedOutputTruncated"),
            Some(&Value::Bool(true))
        );

        let [AgentEvent::ItemCompleted { item, .. }] = mcp.as_slice() else {
            panic!("mcp lifecycle event");
        };
        assert_eq!(
            item.pointer("/arguments/_uiProjectionTruncated"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            item.pointer("/result/_uiProjectionTruncated"),
            Some(&Value::Bool(true))
        );

        let [AgentEvent::ItemCompleted { item, .. }] = dynamic.as_slice() else {
            panic!("dynamic lifecycle event");
        };
        assert!(item
            .get("contentItems")
            .and_then(Value::as_array)
            .and_then(|items| items.last())
            .and_then(|marker| marker.get("_uiProjectionTruncated"))
            .and_then(Value::as_bool)
            .unwrap_or(false));

        let [AgentEvent::ItemCompleted { item, .. }] = unknown.as_slice() else {
            panic!("unknown lifecycle event");
        };
        assert_eq!(item.get("_uiProjectionTruncated"), Some(&Value::Bool(true)));

        let serialized = serde_json::to_string(&[
            reasoning,
            image,
            mcp,
            dynamic,
            command,
            command_with_secret_output,
            unknown,
        ])
        .expect("projected lifecycle events should serialize");
        for forbidden in [
            "RAW_REASONING_MUST_NOT_CROSS_IPC",
            "BASE64_IMAGE_MUST_NOT_CROSS_IPC",
            "MCP_SECRET_MUST_NOT_CROSS_IPC",
            "SIGNED_URL_MUST_NOT_CROSS_IPC",
            "MCP_BASE64_MUST_NOT_CROSS_IPC",
            "MCP_COOKIE_MUST_NOT_CROSS_IPC",
            "DYNAMIC_SECRET_MUST_NOT_CROSS_IPC",
            "DYNAMIC_BASE64_MUST_NOT_CROSS_IPC",
            "TERMINAL_SECRET_MUST_NOT_CROSS_IPC",
            "COMMAND_OUTPUT_SECRET_MUST_NOT_CROSS_IPC",
            "UNKNOWN_RAW_MUST_NOT_CROSS_IPC",
            "UNKNOWN_SECRET",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert!(serialized.contains("safe query"));
        assert!(serialized.contains("visible"));
    }

    #[test]
    fn status_text_projection_redacts_inline_credential_markers() {
        let warning = normalize_codex_notification_at(
            &json!({
                "method": "warning",
                "params": {
                    "threadId": "thread-1",
                    "message": "request failed: token=INLINE_TOKEN_MUST_NOT_CROSS_IPC"
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let error = normalize_codex_notification_at(
            &json!({
                "method": "error",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "willRetry": true,
                    "error": { "message": "Bearer INLINE_BEARER_MUST_NOT_CROSS_IPC" }
                }
            }),
            "2026-07-13T00:00:01Z",
        );
        let progress = normalize_codex_notification_at(
            &json!({
                "method": "item/mcpToolCall/progress",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "mcp-1",
                    "message": "using sk-INLINE_KEY_MUST_NOT_CROSS_IPC"
                }
            }),
            "2026-07-13T00:00:02Z",
        );
        let hook = normalize_codex_notification_at(
            &json!({
                "method": "hook/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "run": {
                        "id": "hook-secret",
                        "eventName": "postToolUse",
                        "handlerType": "command",
                        "executionMode": "sync",
                        "scope": "turn",
                        "sourcePath": "/tmp/demo/.codex/hooks.json",
                        "source": "project",
                        "displayOrder": 0,
                        "status": "failed",
                        "statusMessage": "authorization: INLINE_AUTH_MUST_NOT_CROSS_IPC",
                        "startedAt": 1000,
                        "completedAt": 1200,
                        "durationMs": 200,
                        "entries": [{
                            "kind": "error",
                            "text": "-----BEGIN PRIVATE KEY-----\nINLINE_PRIVATE_KEY"
                        }]
                    }
                }
            }),
            "2026-07-13T00:00:03Z",
        );

        let serialized = serde_json::to_string(&[warning, error, progress, hook])
            .expect("status events should serialize");
        for forbidden in [
            "INLINE_TOKEN_MUST_NOT_CROSS_IPC",
            "INLINE_BEARER_MUST_NOT_CROSS_IPC",
            "INLINE_KEY_MUST_NOT_CROSS_IPC",
            "INLINE_AUTH_MUST_NOT_CROSS_IPC",
            "INLINE_PRIVATE_KEY",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert!(serialized.contains("redacted credential or binary data"));
        assert_eq!(
            project_ui_sensitive_text("task-123 completed", AGENT_UI_STATUS_MESSAGE_CHARS),
            "task-123 completed"
        );
    }

    #[test]
    fn structured_ui_projection_marks_field_array_node_and_character_limits() {
        let many_fields = Value::Object(
            (0..(AGENT_UI_STRUCTURED_MAX_FIELDS + 20))
                .map(|index| (format!("field-{index}"), json!(index)))
                .collect(),
        );
        let projected_fields = project_ui_safe_value(&many_fields);
        assert_eq!(
            projected_fields.get("_uiProjectionTruncated"),
            Some(&Value::Bool(true))
        );
        assert!(projected_fields.get("_omittedFieldCount").is_some());

        let many_items = json!((0..(AGENT_UI_STRUCTURED_MAX_ITEMS + 20)).collect::<Vec<_>>());
        let projected_items = project_ui_safe_value(&many_items);
        let projected_items = projected_items
            .as_array()
            .expect("array projection should stay an array");
        assert_eq!(projected_items.len(), AGENT_UI_STRUCTURED_MAX_ITEMS + 1);
        assert_eq!(
            projected_items
                .last()
                .and_then(|marker| marker.get("_uiProjectionTruncated")),
            Some(&Value::Bool(true))
        );

        let deep_value = json!({ "a": { "b": { "c": { "d": { "e": { "f": "deep" } } } } } });
        let projected_deep_value = project_ui_safe_value(&deep_value);
        assert_eq!(
            projected_deep_value.get("_uiProjectionTruncated"),
            Some(&Value::Bool(true))
        );

        let long_scalar = Value::String("long text ".repeat(AGENT_UI_TEXT_PREVIEW_CHARS));
        let projected_scalar = project_ui_safe_value(&long_scalar);
        assert_eq!(
            projected_scalar.get("_uiProjectionTruncated"),
            Some(&Value::Bool(true))
        );
        assert!(projected_scalar.get("preview").is_some());
    }

    #[test]
    fn turn_and_plan_status_payloads_are_bounded() {
        let long_text = "z".repeat(AGENT_UI_LONG_TEXT_CHARS + 500);
        let final_events = normalize_codex_notification_at(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1",
                        "status": "completed",
                        "items": [{
                            "id": "message-1",
                            "type": "agentMessage",
                            "phase": "final_answer",
                            "text": long_text
                        }]
                    }
                }
            }),
            "2026-07-13T00:00:00Z",
        );
        let plan_steps = (0..(AGENT_UI_PLAN_STEPS + 10))
            .map(|index| {
                json!({
                    "step": format!("{index}:{}", "step ".repeat(400)),
                    "status": "inProgress"
                })
            })
            .collect::<Vec<_>>();
        let plan_events = normalize_codex_notification_at(
            &json!({
                "method": "turn/plan/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "explanation": "explanation ".repeat(1000),
                    "plan": plan_steps
                }
            }),
            "2026-07-13T00:00:01Z",
        );

        let [AgentEvent::TurnCompleted {
            final_message: Some(final_message),
            ..
        }] = final_events.as_slice()
        else {
            panic!("turn completion event");
        };
        assert!(final_message.chars().count() <= AGENT_UI_LONG_TEXT_CHARS);

        let [AgentEvent::PlanUpdated {
            explanation: Some(explanation),
            plan,
            ..
        }] = plan_events.as_slice()
        else {
            panic!("plan update event");
        };
        assert!(explanation.chars().count() <= AGENT_UI_STATUS_MESSAGE_CHARS);
        assert_eq!(plan.len(), AGENT_UI_PLAN_STEPS);
        assert!(plan
            .iter()
            .all(|step| step.step.chars().count() <= AGENT_UI_TEXT_PREVIEW_CHARS));
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
