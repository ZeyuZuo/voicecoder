//! JSON-RPC transport ownership, message routing, logging, and process diagnostics.

#[cfg(test)]
use super::initialize_params;
use super::model::{AgentEvent, CodingAgentApprovalsReviewer, ServerRequestResolution};
use super::protocol::*;
use super::{
    AGENT_UI_LONG_TEXT_CHARS, AGENT_UI_STATUS_MESSAGE_CHARS, AGENT_UI_TEXT_PREVIEW_CHARS,
    APP_SERVER_AUTO_REVIEW_TIMEOUT, APP_SERVER_HEARTBEAT_INTERVAL, APP_SERVER_LAST_MESSAGE_LIMIT,
    APP_SERVER_RESPONSE_TIMEOUT, APP_SERVER_SERVER_REQUEST_TIMEOUT, APP_SERVER_STDERR_TAIL_LINES,
    APP_SERVER_TRANSPORT_POLL_INTERVAL, APP_SERVER_USER_DECISION_TIMEOUT,
    FIRST_APP_SERVER_REQUEST_ID,
};
use crate::log_sanitizer::sanitize_json_for_log;
use chrono::Utc;
use serde_json::{json, Map, Value};
mod server_requests;
pub(super) use server_requests::*;

use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, ExitStatus},
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum JsonRpcInboundKind {
    Response { request_id: u64 },
    Notification { method: String },
    ServerRequest { request_id: Value, method: String },
    Unknown { reason: String },
}

pub(super) enum AppServerReaderEvent {
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

pub(super) struct PendingServerRequest {
    pub(super) request_id: Value,
    pub(super) method: String,
    pub(super) params: Value,
    pub(super) handling: ServerRequestHandling,
    pub(super) deadline: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ServerRequestHandling {
    AutoReview,
    UserDecision,
    UserInput { auto_resolve: bool },
    McpElicitation,
    Unsupported,
}

impl ServerRequestHandling {
    pub(super) fn allows_client_resolution(self) -> bool {
        !matches!(self, Self::AutoReview | Self::Unsupported)
    }

    pub(super) fn requires_user_input(self) -> bool {
        matches!(
            self,
            Self::UserDecision | Self::UserInput { .. } | Self::McpElicitation
        )
    }
}

pub(super) struct BuiltServerRequestResponse {
    pub(super) response: Value,
    pub(super) log_payload: Value,
    pub(super) status: String,
    pub(super) resolution: String,
    pub(super) message: String,
}

#[derive(Clone, Default)]
pub(super) struct AppServerHeartbeatSnapshot {
    pub(super) last_message_at: Option<String>,
    pub(super) last_progress_at: Option<String>,
    pub(super) last_method: Option<String>,
    pub(super) last_message: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct SharedAppServerHeartbeat(Arc<Mutex<AppServerHeartbeatSnapshot>>);

impl SharedAppServerHeartbeat {
    pub(super) fn record_message(&self, message: &Value, received_at: &str) {
        if let Ok(mut heartbeat) = self.0.lock() {
            heartbeat.last_message_at = Some(received_at.to_string());
            heartbeat.last_method = message
                .get("method")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            heartbeat.last_message = Some(truncate_transport_text(
                &sanitize_json_for_log(message).to_string(),
                APP_SERVER_LAST_MESSAGE_LIMIT,
            ));
        }
    }

    pub(super) fn record_progress(&self, occurred_at: &str) {
        if let Ok(mut heartbeat) = self.0.lock() {
            heartbeat.last_progress_at = Some(occurred_at.to_string());
        }
    }

    pub(super) fn snapshot(&self) -> AppServerHeartbeatSnapshot {
        self.0.lock().map(|value| value.clone()).unwrap_or_default()
    }
}

#[derive(Clone)]
pub(super) struct AgentRunTransportLog {
    pub(super) path: String,
    pub(super) file: Arc<Mutex<File>>,
}

impl AgentRunTransportLog {
    pub(super) fn create(
        project_path: &str,
        run_id: &str,
        transport: &str,
    ) -> Result<Self, String> {
        let voicecoder_dir = Path::new(project_path).join(".voicecoder");
        fs::create_dir_all(&voicecoder_dir)
            .map_err(|error| format!("创建 app-server 诊断目录失败：{error}"))?;
        let file_name = format!(
            "agent_run_{}_{}.jsonl",
            sanitize_transport_log_stem(run_id),
            sanitize_transport_log_stem(transport)
        );
        let path = voicecoder_dir.join(file_name);
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|error| format!("创建 app-server 原始日志失败：{error}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("限制 app-server 原始日志权限失败：{error}"))?;
        }

        Ok(Self {
            path: path.to_string_lossy().to_string(),
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub(super) fn record(&self, direction: &str, kind: &str, payload: Value) -> Result<(), String> {
        let record = json!({
            "recordedAt": current_agent_event_timestamp(),
            "direction": direction,
            "kind": kind,
            "payload": sanitize_transport_log_payload(&payload)
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

    pub(super) fn record_process_exit(
        &self,
        status: Option<&ExitStatus>,
        reason: &str,
        error: Option<&str>,
        stderr_tail: &[String],
    ) {
        let _ = self.record(
            "meta",
            "process_exit",
            json!({
                "reason": reason,
                "status": status.map(|status| format_exit_status(*status)),
                "success": status.map(ExitStatus::success),
                "error": error,
                "stderrTail": stderr_tail
            }),
        );
    }
}

fn sanitize_transport_log_payload(payload: &Value) -> Value {
    let mut sanitized = sanitize_json_for_log(payload);
    let method = payload.get("method").and_then(Value::as_str);
    if matches!(
        method,
        Some("turn/start" | "turn/steer" | "thread/inject_items")
    ) {
        if let Some(input) = sanitized.pointer_mut("/params/input") {
            *input = Value::String("[REDACTED_USER_INPUT]".to_string());
        }
    }
    sanitized
}

#[allow(dead_code)]
pub(super) struct CodexAppServerClient {
    pub(super) stdin: ChildStdin,
    pub(super) receiver: Receiver<AppServerReaderEvent>,
    pub(super) stdout_reader: Option<thread::JoinHandle<()>>,
    pub(super) stderr_reader: Option<thread::JoinHandle<()>>,
    pub(super) stderr_tail: Arc<Mutex<VecDeque<String>>>,
    pub(super) heartbeat: SharedAppServerHeartbeat,
    pub(super) transport_log: AgentRunTransportLog,
    pub(super) next_request_id: u64,
    pub(super) pending_responses: BTreeMap<u64, Value>,
    pub(super) pending_agent_events: VecDeque<AgentEvent>,
    pub(super) pending_server_requests: VecDeque<PendingServerRequest>,
    pub(super) server_request_resolutions: Option<Receiver<ServerRequestResolution>>,
    pub(super) approvals_reviewer: CodingAgentApprovalsReviewer,
    pub(super) last_heartbeat_notice: Instant,
}

impl CodexAppServerClient {
    pub(super) fn new(
        stdin: ChildStdin,
        stdout: ChildStdout,
        stderr: ChildStderr,
        transport_log: AgentRunTransportLog,
        server_request_resolutions: Option<Receiver<ServerRequestResolution>>,
        approvals_reviewer: CodingAgentApprovalsReviewer,
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
            server_request_resolutions,
            approvals_reviewer,
            last_heartbeat_notice: Instant::now(),
        }
    }

    pub(super) fn log_path(&self) -> &str {
        &self.transport_log.path
    }

    pub(super) fn next_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    pub(super) fn send_request(
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

    pub(super) fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_json_line(&build_json_rpc_notification(method, params), "notification")
    }

    pub(super) fn take_pending_agent_events(&mut self) -> Vec<AgentEvent> {
        self.pending_agent_events.drain(..).collect()
    }

    pub(super) fn read_response(
        &mut self,
        child: &mut Child,
        request_id: u64,
    ) -> Result<Value, String> {
        if let Some(response) = self.pending_responses.remove(&request_id) {
            return validate_json_rpc_response(response);
        }

        let deadline = Instant::now() + APP_SERVER_RESPONSE_TIMEOUT;
        loop {
            self.process_server_request_resolutions()?;
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

    pub(super) fn read_next_agent_events(
        &mut self,
        child: &mut Child,
    ) -> Result<Vec<AgentEvent>, String> {
        loop {
            self.process_server_request_resolutions()?;
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

    pub(super) fn process_reader_event(
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
                        truncate_transport_text(&sanitize_transport_text(&line), 240)
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

    pub(super) fn route_message(
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
                if method == "serverRequest/resolved" {
                    self.clear_resolved_server_request(&message);
                }
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
                let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
                let handling =
                    classify_server_request_handling(&method, &params, self.approvals_reviewer);
                let timeout = server_request_timeout(handling, &params);
                let expires_at = server_request_expiry_timestamp(timeout);
                self.pending_server_requests
                    .push_back(PendingServerRequest {
                        request_id: request_id.clone(),
                        method: method.clone(),
                        params: params.clone(),
                        handling,
                        deadline: Instant::now() + timeout,
                    });
                self.pending_agent_events
                    .push_back(build_server_request_event(
                        request_id,
                        method.clone(),
                        params,
                        handling,
                        expires_at,
                        received_at.clone(),
                    ));
                if handling == ServerRequestHandling::Unsupported {
                    self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                        level: "warning".to_string(),
                        message: format!(
                            "VoiceCoder 不支持 app-server 主动请求 `{method}`；将在 {} 秒后安全取消",
                            APP_SERVER_SERVER_REQUEST_TIMEOUT.as_secs()
                        ),
                        method: Some(method),
                        created_at: received_at,
                    });
                }
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

    pub(super) fn process_server_request_resolutions(&mut self) -> Result<(), String> {
        loop {
            let resolution = match self.server_request_resolutions.as_ref() {
                Some(receiver) => match receiver.try_recv() {
                    Ok(resolution) => resolution,
                    Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return Ok(()),
                },
                None => return Ok(()),
            };
            self.resolve_server_request_from_client(resolution)?;
        }
    }

    pub(super) fn resolve_server_request_from_client(
        &mut self,
        resolution: ServerRequestResolution,
    ) -> Result<(), String> {
        let Some(index) = self
            .pending_server_requests
            .iter()
            .position(|request| request.request_id == resolution.request_id)
        else {
            self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                level: "warning".to_string(),
                message: "收到已失效或不存在的 app-server 请求响应，已忽略".to_string(),
                method: None,
                created_at: current_agent_event_timestamp(),
            });
            return Ok(());
        };
        let request = self
            .pending_server_requests
            .remove(index)
            .expect("pending server request index must exist");
        if !request.handling.allows_client_resolution() {
            self.pending_server_requests.insert(index, request);
            self.pending_agent_events.push_back(AgentEvent::Diagnostic {
                level: "warning".to_string(),
                message: "该审批已由 Codex 自动审查接管，忽略页面的重复响应".to_string(),
                method: None,
                created_at: current_agent_event_timestamp(),
            });
            return Ok(());
        }

        let built = build_server_request_response(&request, &resolution)?;
        self.write_json_line_with_log_payload(
            &built.response,
            "server_request_response",
            &built.log_payload,
        )?;
        self.pending_agent_events
            .push_back(server_request_resolved_event(
                request.request_id,
                built.status,
                Some(built.resolution),
                Some(built.message),
                current_agent_event_timestamp(),
            ));
        Ok(())
    }

    pub(super) fn clear_resolved_server_request(&mut self, notification: &Value) {
        let Some(request_id) = notification.pointer("/params/requestId") else {
            return;
        };
        self.pending_server_requests
            .retain(|request| request.request_id != *request_id);
    }

    pub(super) fn resolve_expired_server_requests(&mut self) -> Result<(), String> {
        let now = Instant::now();
        let mut pending = VecDeque::new();

        while let Some(request) = self.pending_server_requests.pop_front() {
            if request.deadline > now {
                pending.push_back(request);
                continue;
            }

            let built = build_server_request_timeout_response(&request)?;
            self.write_json_line_with_log_payload(
                &built.response,
                "server_request_timeout_response",
                &built.log_payload,
            )?;
            self.pending_agent_events
                .push_back(server_request_resolved_event(
                    request.request_id,
                    built.status,
                    Some(built.resolution),
                    Some(built.message),
                    current_agent_event_timestamp(),
                ));
        }

        self.pending_server_requests = pending;
        Ok(())
    }

    pub(super) fn cancel_pending_server_requests(&mut self) -> Result<(), String> {
        while let Some(request) = self.pending_server_requests.pop_front() {
            let built = if request.handling == ServerRequestHandling::Unsupported {
                unsupported_server_request_response(request.request_id.clone(), &request.method)
            } else {
                build_server_request_response(
                    &request,
                    &ServerRequestResolution {
                        request_id: request.request_id.clone(),
                        action: "cancel".to_string(),
                        answers: BTreeMap::new(),
                        content: None,
                        scope: None,
                    },
                )?
            };
            self.write_json_line_with_log_payload(
                &built.response,
                "server_request_run_cancel_response",
                &built.log_payload,
            )?;
        }
        Ok(())
    }

    pub(super) fn heartbeat_diagnostic_event(&self) -> AgentEvent {
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

    pub(super) fn ensure_child_is_running(&self, child: &mut Child) -> Result<(), String> {
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

    pub(super) fn transport_error_context(&self, child: &mut Child, message: &str) -> String {
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

    pub(super) fn write_json_line(&mut self, value: &Value, kind: &str) -> Result<(), String> {
        self.write_json_line_with_log_payload(value, kind, value)
    }

    pub(super) fn write_json_line_with_log_payload(
        &mut self,
        value: &Value,
        kind: &str,
        log_payload: &Value,
    ) -> Result<(), String> {
        let line = serde_json::to_string(value)
            .map_err(|error| format!("序列化 Codex app-server 消息失败：{error}"))?;
        self.transport_log
            .record("outbound", kind, log_payload.clone())?;
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| format!("写入 Codex app-server stdin 失败：{error}"))
    }

    pub(super) fn join_readers(&mut self) {
        if let Some(reader) = self.stdout_reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn spawn_app_server_stdout_reader(
    stdout: ChildStdout,
    sender: Sender<AppServerReaderEvent>,
    transport_log: AgentRunTransportLog,
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

pub(super) fn spawn_app_server_stderr_reader(
    stderr: ChildStderr,
    transport_log: AgentRunTransportLog,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            let _ = transport_log.record("inbound", "stderr", Value::String(line.clone()));
            let sanitized_line = sanitize_transport_text(&line);
            if let Ok(mut tail) = stderr_tail.lock() {
                tail.push_back(sanitized_line);
                while tail.len() > APP_SERVER_STDERR_TAIL_LINES {
                    tail.pop_front();
                }
            }
        }
    })
}

pub(super) fn classify_json_rpc_message(message: &Value) -> JsonRpcInboundKind {
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

pub(super) fn build_json_rpc_result_response(id: Value, result: Value) -> Value {
    json!({ "id": id, "result": result })
}

pub(super) fn build_json_rpc_error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

pub(super) fn sanitize_transport_log_stem(value: &str) -> String {
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

pub(super) fn truncate_transport_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn sanitize_transport_text(value: &str) -> String {
    sanitize_json_for_log(&Value::String(value.to_string()))
        .as_str()
        .unwrap_or("[REDACTED_CREDENTIAL_TEXT]")
        .to_string()
}

pub(super) fn build_json_rpc_request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "method": method,
        "id": id,
        "params": params
    })
}

pub(super) fn build_json_rpc_notification(method: &str, params: Value) -> Value {
    json!({
        "method": method,
        "params": params
    })
}

pub(super) fn validate_json_rpc_response(message: Value) -> Result<Value, String> {
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

pub(super) fn extract_json_pointer_string(
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

pub(super) fn format_exit_status(status: std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string())
}

#[cfg(test)]
pub(super) fn build_initialize_request() -> Value {
    build_json_rpc_request(
        FIRST_APP_SERVER_REQUEST_ID,
        "initialize",
        initialize_params(),
    )
}

#[cfg(test)]
pub(super) fn build_initialized_notification() -> Value {
    build_json_rpc_notification("initialized", json!({}))
}

#[cfg(test)]
pub(super) fn validate_initialize_response(message: Value) -> Result<Value, String> {
    validate_json_rpc_response(message).map_err(|error| {
        error.replacen("Codex app-server request", "Codex app-server initialize", 1)
    })
}

pub(super) fn json_rpc_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}
