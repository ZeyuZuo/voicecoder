//! Shared coding-agent configuration, IPC contracts, events, and request state.

use super::{CODEX_APPROVALS_REVIEWER_ENV, CODEX_APPROVAL_POLICY_ENV};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
};

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
    pub(super) auto_provider: CodingAgentProviderKind,
    pub(super) provider_override: Option<CodingAgentProviderKind>,
    pub(super) active_provider_configured: bool,
    pub(super) active_provider_error: Option<String>,
    pub(super) diagnostics: Vec<CodingAgentProviderDiagnostic>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentProviderDiagnostic {
    pub(super) provider: CodingAgentProviderKind,
    pub(super) configured: bool,
    pub(super) missing_dependencies: Vec<String>,
    pub(super) executable: Option<String>,
    pub(super) version: Option<String>,
    pub(super) details: BTreeMap<String, String>,
    pub(super) error: Option<String>,
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
pub(super) enum CodingAgentApprovalPolicy {
    Untrusted,
    OnRequest,
    Never,
}

impl CodingAgentApprovalPolicy {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_lowercase().as_str() {
            "untrusted" => Ok(Self::Untrusted),
            "on-request" | "on_request" => Ok(Self::OnRequest),
            "never" => Ok(Self::Never),
            _ => Err(format!(
                "{CODEX_APPROVAL_POLICY_ENV} 只支持 untrusted、on-request 或 never。"
            )),
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CodingAgentApprovalsReviewer {
    User,
    AutoReview,
    GuardianSubagent,
}

impl CodingAgentApprovalsReviewer {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
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

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::AutoReview => "auto_review",
            Self::GuardianSubagent => "guardian_subagent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CodingAgentPermissionSettings {
    pub(super) approval_policy: CodingAgentApprovalPolicy,
    pub(super) approvals_reviewer: CodingAgentApprovalsReviewer,
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
    pub(super) fn app_server_thread_sandbox(self) -> &'static str {
        match self {
            CodingAgentSandboxMode::ReadOnly => "read-only",
            CodingAgentSandboxMode::WorkspaceWrite => "workspace-write",
            CodingAgentSandboxMode::DangerFullAccess => "danger-full-access",
        }
    }

    pub(super) fn app_server_turn_sandbox_policy(self, project_path: &str) -> Value {
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

    pub(super) fn codex_exec_sandbox_arg(self) -> &'static str {
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
    ServerRequest {
        request_id: Value,
        request_key: String,
        method: String,
        kind: String,
        status: String,
        requires_user_input: bool,
        auto_review: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        thread_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        item_id: Option<String>,
        details: Value,
        expires_at: String,
        created_at: String,
    },
    ServerRequestResolved {
        request_id: Value,
        request_key: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resolution: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
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
    pub(super) step: String,
    pub(super) status: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartInitialDemoRunRequest {
    pub(super) demo_session_id: String,
    pub(super) run_id: String,
    pub(super) project_path: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) sandbox: Option<CodingAgentSandboxMode>,
    #[serde(default)]
    pub(super) provider: Option<CodingAgentProviderKind>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveCodingAgentServerRequestRequest {
    pub(super) run_id: String,
    pub(super) request_id: Value,
    pub(super) action: String,
    #[serde(default)]
    pub(super) answers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub(super) content: Option<Value>,
    #[serde(default)]
    pub(super) scope: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct ServerRequestResolution {
    pub(super) request_id: Value,
    pub(super) action: String,
    pub(super) answers: BTreeMap<String, Vec<String>>,
    pub(super) content: Option<Value>,
    pub(super) scope: Option<String>,
}

#[derive(Clone, Default)]
pub struct CodingAgentRequestState {
    pub(super) responders: Arc<Mutex<BTreeMap<String, Sender<ServerRequestResolution>>>>,
}

impl CodingAgentRequestState {
    pub(super) fn register(
        &self,
        run_id: &str,
    ) -> Result<Receiver<ServerRequestResolution>, String> {
        let (sender, receiver) = mpsc::channel();
        let mut responders = self
            .responders
            .lock()
            .map_err(|_| "Coding Agent 请求响应注册表已损坏。".to_string())?;
        if responders.contains_key(run_id) {
            return Err(format!("AgentRun `{run_id}` 已经存在活动请求通道。"));
        }
        responders.insert(run_id.to_string(), sender);
        Ok(receiver)
    }

    pub(super) fn unregister(&self, run_id: &str) {
        if let Ok(mut responders) = self.responders.lock() {
            responders.remove(run_id);
        }
    }

    pub(super) fn resolve(
        &self,
        run_id: &str,
        resolution: ServerRequestResolution,
    ) -> Result<(), String> {
        let responders = self
            .responders
            .lock()
            .map_err(|_| "Coding Agent 请求响应注册表已损坏。".to_string())?;
        let sender = responders
            .get(run_id)
            .ok_or_else(|| format!("AgentRun `{run_id}` 当前没有可响应的 app-server 请求。"))?;
        sender
            .send(resolution)
            .map_err(|_| format!("AgentRun `{run_id}` 的 app-server 请求通道已经关闭。"))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunStartedEvent {
    pub(super) demo_session_id: String,
    pub(super) run_id: String,
    pub(super) project_path: String,
    pub(super) provider: CodingAgentProviderKind,
    pub(super) codex_thread_id: String,
    pub(super) codex_turn_id: String,
    pub(super) runtime: CodingAgentRuntimeMetadata,
    pub(super) started_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CodingAgentRuntimeMetadata {
    pub(super) provider: CodingAgentProviderKind,
    pub(super) version: String,
    pub(super) transport: String,
    pub(super) sandbox: String,
    pub(super) approval_policy: Option<String>,
    pub(super) approvals_reviewer: Option<String>,
    pub(super) transport_log_path: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventEnvelope {
    pub(super) demo_session_id: String,
    pub(super) run_id: String,
    pub(super) event: AgentEvent,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunCompletedEvent {
    pub(super) demo_session_id: String,
    pub(super) run_id: String,
    pub(super) final_message: Option<String>,
    pub(super) changed_files: Vec<String>,
    pub(super) status: String,
    pub(super) error: Option<String>,
    pub(super) completed_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorEvent {
    pub(super) demo_session_id: Option<String>,
    pub(super) run_id: Option<String>,
    pub(super) message: String,
    pub(super) occurred_at: String,
}
