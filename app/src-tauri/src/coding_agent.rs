use crate::env_config::read_local_env;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    thread,
};
use tauri::{AppHandle, Emitter};

const DEFAULT_CODEX_BIN: &str = "codex";
const FIRST_APP_SERVER_REQUEST_ID: u64 = 0;
const DEFAULT_CODEX_SANDBOX: CodingAgentSandboxMode = CodingAgentSandboxMode::WorkspaceWrite;
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
    started_at: String,
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
        validate_codex_executable().map(|_| ())
    }

    fn diagnostic(&self) -> CodingAgentProviderDiagnostic {
        codex_diagnostic(
            self.kind(),
            [
                ("transport", "stdio"),
                ("command", "codex app-server --stdio"),
                ("threadMode", "persistent"),
            ],
        )
    }

    fn start_session(
        &self,
        context: CodingAgentStartContext,
    ) -> Result<Box<dyn CodingAgentSession + Send>, String> {
        self.validate_start()?;
        Ok(Box::new(start_codex_app_server_session(context)?))
    }
}

pub(crate) struct CodexExecJsonProvider;

impl CodingAgentProvider for CodexExecJsonProvider {
    fn kind(&self) -> CodingAgentProviderKind {
        CodingAgentProviderKind::CodexExecJson
    }

    fn validate_start(&self) -> Result<(), String> {
        validate_codex_executable().map(|_| ())
    }

    fn diagnostic(&self) -> CodingAgentProviderDiagnostic {
        codex_diagnostic(
            self.kind(),
            [
                ("transport", "process-jsonl"),
                (
                    "command",
                    "codex exec --json --sandbox workspace-write --cd <project> <prompt>",
                ),
                ("threadMode", "single-run"),
            ],
        )
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
    let provider = CodingAgentProviderRegistry::resolve_provider(
        request.provider.unwrap_or(CodingAgentProviderKind::Auto),
    );
    if provider != CodingAgentProviderKind::CodexAppServer {
        return Err("当前后台事件流只支持 codex_app_server provider。".to_string());
    }
    CodexAppServerProvider.validate_start()?;

    let context = CodingAgentStartContext {
        project_path: request.project_path.clone(),
        prompt: request.prompt.clone(),
        sandbox: request.sandbox,
    };
    let mut session = start_codex_app_server_session(context)?;

    emit_agent_run_started(
        &app,
        AgentRunStartedEvent {
            demo_session_id: request.demo_session_id.clone(),
            run_id: request.run_id.clone(),
            project_path: request.project_path.clone(),
            provider,
            codex_thread_id: session.codex_thread_id.clone(),
            codex_turn_id: session.initial_turn_id.clone(),
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

#[allow(dead_code)]
struct CodexAppServerSession {
    child: Child,
    client: CodexAppServerClient,
    project_path: String,
    sandbox: CodingAgentSandboxMode,
    codex_thread_id: String,
    initial_turn_id: String,
    initial_prompt: String,
}

impl CodingAgentSession for CodexAppServerSession {
    fn cancel(&mut self) -> Result<(), String> {
        self.child
            .kill()
            .map_err(|error| format!("停止 Codex app-server 失败：{error}"))?;
        let _ = self.child.wait();
        Ok(())
    }
}

impl CodexAppServerSession {
    fn take_pending_agent_events(&mut self) -> Vec<AgentEvent> {
        self.client
            .take_pending_notifications()
            .iter()
            .flat_map(normalize_codex_notification)
            .collect()
    }

    fn read_next_agent_events(&mut self) -> Result<Vec<AgentEvent>, String> {
        loop {
            let message = self.client.read_message(&mut self.child)?;
            let events = normalize_codex_notification(&message);
            if !events.is_empty() {
                return Ok(events);
            }
        }
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
) -> Result<CodexAppServerSession, String> {
    validate_coding_agent_start_context(&context)?;

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
    let mut client = CodexAppServerClient::new(stdin, stdout);
    if let Err(error) = initialize_codex_app_server(&mut child, &mut client) {
        return Err(cleanup_child_with_error(&mut child, error));
    }
    let run_handles = match start_initial_codex_turn(&mut child, &mut client, &context) {
        Ok(run_handles) => run_handles,
        Err(error) => return Err(cleanup_child_with_error(&mut child, error)),
    };
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);

    Ok(CodexAppServerSession {
        child,
        client,
        project_path: context.project_path,
        sandbox,
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
) -> Result<CodexAppServerRunHandles, String> {
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);
    let thread_response = client.send_request(
        child,
        "thread/start",
        build_thread_start_params(&context.project_path, sandbox),
    )?;
    let thread_id = extract_json_pointer_string(
        &thread_response,
        "/result/thread/id",
        "Codex app-server thread/start 响应缺少 thread.id。",
    )?;

    let turn_response = client.send_request(
        child,
        "turn/start",
        build_turn_start_params(&thread_id, &context.project_path, sandbox, &context.prompt),
    )?;
    let turn_id = extract_json_pointer_string(
        &turn_response,
        "/result/turn/id",
        "Codex app-server turn/start 响应缺少 turn.id。",
    )?;

    Ok(CodexAppServerRunHandles { thread_id, turn_id })
}

fn build_thread_start_params(project_path: &str, sandbox: CodingAgentSandboxMode) -> Value {
    json!({
        "cwd": project_path,
        "runtimeWorkspaceRoots": [project_path],
        "sandbox": sandbox.app_server_thread_sandbox(),
        "threadSource": "user"
    })
}

fn build_turn_start_params(
    thread_id: &str,
    project_path: &str,
    sandbox: CodingAgentSandboxMode,
    prompt: &str,
) -> Value {
    json!({
        "threadId": thread_id,
        "cwd": project_path,
        "runtimeWorkspaceRoots": [project_path],
        "sandboxPolicy": sandbox.app_server_turn_sandbox_policy(project_path),
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

#[allow(dead_code)]
struct CodexAppServerClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_request_id: u64,
    pending_responses: BTreeMap<u64, Value>,
    pending_notifications: VecDeque<Value>,
}

impl CodexAppServerClient {
    fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout),
            next_request_id: FIRST_APP_SERVER_REQUEST_ID,
            pending_responses: BTreeMap::new(),
            pending_notifications: VecDeque::new(),
        }
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
        self.write_json_line(&request)?;
        self.read_response(child, request_id)
    }

    fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_json_line(&build_json_rpc_notification(method, params))
    }

    #[allow(dead_code)]
    fn take_pending_notifications(&mut self) -> Vec<Value> {
        self.pending_notifications.drain(..).collect()
    }

    fn read_response(&mut self, child: &mut Child, request_id: u64) -> Result<Value, String> {
        if let Some(response) = self.pending_responses.remove(&request_id) {
            return validate_json_rpc_response(response);
        }

        loop {
            let message = self.read_message(child)?;
            if let Some(message_id) = message.get("id").and_then(Value::as_u64) {
                if message_id == request_id {
                    return validate_json_rpc_response(message);
                }

                self.pending_responses.insert(message_id, message);
                continue;
            }

            self.pending_notifications.push_back(message);
        }
    }

    fn read_message(&mut self, child: &mut Child) -> Result<Value, String> {
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|error| format!("检查 Codex app-server 子进程状态失败：{error}"))?
            {
                return Err(format!(
                    "Codex app-server 已退出，退出码：{}。",
                    format_exit_status(status)
                ));
            }

            let mut line = String::new();
            let read_bytes = self
                .stdout
                .read_line(&mut line)
                .map_err(|error| format!("读取 Codex app-server stdout 失败：{error}"))?;
            if read_bytes == 0 {
                let exit_status = child
                    .try_wait()
                    .map_err(|error| format!("检查 Codex app-server 子进程状态失败：{error}"))?;
                return Err(match exit_status {
                    Some(status) => format!(
                        "Codex app-server stdout 已关闭，退出码：{}。",
                        format_exit_status(status)
                    ),
                    None => "Codex app-server stdout 已关闭。".to_string(),
                });
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            return serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("Codex app-server 输出不是合法 JSON：{error}"));
        }
    }

    fn write_json_line(&mut self, value: &Value) -> Result<(), String> {
        let line = serde_json::to_string(value)
            .map_err(|error| format!("序列化 Codex app-server 消息失败：{error}"))?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("写入 Codex app-server stdin 失败：{error}"))
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
    fn builds_thread_start_params_with_cwd_and_workspace_sandbox() {
        let params = build_thread_start_params(
            "/tmp/voicecoder-demo",
            CodingAgentSandboxMode::WorkspaceWrite,
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
        );
        let turn_params = build_turn_start_params(
            "thread-1",
            "/tmp/voicecoder-demo",
            CodingAgentSandboxMode::DangerFullAccess,
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
