//! Codex app-server and exec-json process/session lifecycle ownership.

use super::model::{
    AgentEvent, CodingAgentPermissionSettings, CodingAgentSandboxMode, CodingAgentSession,
    CodingAgentStartContext, ServerRequestResolution,
};
use super::protocol::{current_agent_event_timestamp, normalize_codex_exec_json_event};
use super::transport::{
    extract_json_pointer_string, format_exit_status, spawn_app_server_stderr_reader,
    AgentRunTransportLog, CodexAppServerClient,
};
use super::{
    codex_executable, codex_protocol_version_warning, resolve_coding_agent_permission_settings,
    validate_codex_executable, CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION,
    CODEX_APP_SERVER_TRANSPORT, DEFAULT_CODEX_SANDBOX,
};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    process::{Child, ChildStdout, Command, Stdio},
    sync::{mpsc::Receiver, Arc, Mutex},
    thread,
};

pub(super) struct CodexExecJsonSession {
    child: Child,
    stdout: BufReader<ChildStdout>,
    stderr_reader: Option<thread::JoinHandle<()>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    pub(super) transport_log: AgentRunTransportLog,
    exit_recorded: bool,
    pub(super) sandbox: CodingAgentSandboxMode,
    pub(super) permission_settings: CodingAgentPermissionSettings,
    pub(super) codex_version: String,
}

impl CodingAgentSession for CodexExecJsonSession {
    fn cancel(&mut self) -> Result<(), String> {
        let existing_status = self
            .child
            .try_wait()
            .map_err(|error| format!("检查 Codex exec --json 子进程状态失败：{error}"))?;
        let was_running = existing_status.is_none();
        let stop_result = if was_running {
            self.child
                .kill()
                .map_err(|error| format!("停止 Codex exec --json 失败：{error}"))
        } else {
            Ok(())
        };
        let final_status = existing_status.or_else(|| self.child.wait().ok());
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        self.record_exit(
            final_status.as_ref(),
            if was_running {
                "client_cancelled"
            } else {
                "process_already_exited"
            },
            stop_result.as_ref().err().map(String::as_str),
        );
        stop_result
    }
}

impl CodexExecJsonSession {
    pub(super) fn read_next_agent_events(&mut self) -> Result<Vec<AgentEvent>, String> {
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
                if let Some(reader) = self.stderr_reader.take() {
                    let _ = reader.join();
                }
                self.record_exit(Some(&status), "process_completed", None);
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

            let message = match serde_json::from_str::<Value>(trimmed) {
                Ok(message) => message,
                Err(error) => {
                    self.transport_log.record(
                        "inbound",
                        "invalid_json",
                        json!({ "line": trimmed, "error": error.to_string() }),
                    )?;
                    return Err(format!("Codex exec --json 输出不是合法 JSON：{error}"));
                }
            };
            self.transport_log
                .record("inbound", "message", message.clone())?;
            let events = normalize_codex_exec_json_event(&message);
            if !events.is_empty() {
                return Ok(events);
            }
        }
    }

    fn record_exit(
        &mut self,
        status: Option<&std::process::ExitStatus>,
        reason: &str,
        error: Option<&str>,
    ) {
        if self.exit_recorded {
            return;
        }
        self.exit_recorded = true;
        let stderr_tail = self
            .stderr_tail
            .lock()
            .map(|tail| tail.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.transport_log
            .record_process_exit(status, reason, error, &stderr_tail);
    }
}

pub(super) fn start_codex_exec_json_session(
    context: CodingAgentStartContext,
    run_id: &str,
) -> Result<CodexExecJsonSession, String> {
    validate_coding_agent_start_context(&context)?;
    let codex_version = validate_codex_executable()?;
    let permission_settings = resolve_coding_agent_permission_settings()?;
    let sandbox = context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX);
    let transport_log = AgentRunTransportLog::create(&context.project_path, run_id, "exec_json")?;

    let executable = codex_executable();
    let args = build_codex_exec_json_args(&context, permission_settings);
    let logged_args = args
        .iter()
        .enumerate()
        .map(|(index, value)| {
            if index + 1 == args.len() {
                "[REDACTED_PROMPT_ARGUMENT]".to_string()
            } else {
                value.clone()
            }
        })
        .collect::<Vec<_>>();
    transport_log.record(
        "meta",
        "process_starting",
        json!({
            "codexVersion": codex_version,
            "protocolBaselineVersion": CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION,
            "executable": executable,
            "args": logged_args,
            "cwd": context.project_path,
            "transport": "process-jsonl",
            "sandbox": sandbox.codex_exec_sandbox_arg(),
            "approvalPolicy": permission_settings.approval_policy.as_str(),
            "approvalsReviewer": permission_settings.approvals_reviewer.as_str()
        }),
    )?;
    transport_log.record(
        "outbound",
        "process_request",
        json!({
            "prompt": "[REDACTED_USER_INPUT]",
            "promptChars": context.prompt.chars().count()
        }),
    )?;
    let mut child = match Command::new(&executable)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let message = format!("启动 Codex exec --json 失败：{error}");
            transport_log.record_process_exit(None, "spawn_failed", Some(&message), &[]);
            return Err(message);
        }
    };
    if let Err(error) =
        transport_log.record("meta", "process_started", json!({ "pid": child.id() }))
    {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            error,
            "process_log_failed",
        ));
    }

    let Some(stdout) = child.stdout.take() else {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            "Codex exec --json stdout 不可用。".to_string(),
            "missing_stdout",
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            "Codex exec --json stderr 不可用。".to_string(),
            "missing_stderr",
        ));
    };
    let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
    let stderr_reader =
        spawn_app_server_stderr_reader(stderr, transport_log.clone(), stderr_tail.clone());

    Ok(CodexExecJsonSession {
        child,
        stdout: BufReader::new(stdout),
        stderr_reader: Some(stderr_reader),
        stderr_tail,
        transport_log,
        exit_recorded: false,
        sandbox,
        permission_settings,
        codex_version,
    })
}

pub(super) fn build_codex_exec_json_args(
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
pub(super) struct CodexAppServerSession {
    child: Child,
    pub(super) client: CodexAppServerClient,
    project_path: String,
    pub(super) sandbox: CodingAgentSandboxMode,
    pub(super) permission_settings: CodingAgentPermissionSettings,
    pub(super) codex_version: String,
    pub(super) codex_thread_id: String,
    pub(super) initial_turn_id: String,
    initial_prompt: String,
}

impl CodingAgentSession for CodexAppServerSession {
    fn cancel(&mut self) -> Result<(), String> {
        let _ = self.client.cancel_pending_server_requests();
        let existing_status = self
            .child
            .try_wait()
            .map_err(|error| format!("检查 Codex app-server 子进程状态失败：{error}"))?;
        let was_running = existing_status.is_none();
        let stop_result = if was_running {
            self.child
                .kill()
                .map_err(|error| format!("停止 Codex app-server 失败：{error}"))
        } else {
            Ok(())
        };
        let final_status = existing_status.or_else(|| self.child.wait().ok());
        self.client.join_readers();
        let stderr_tail = self
            .client
            .stderr_tail
            .lock()
            .map(|tail| tail.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.client.transport_log.record_process_exit(
            final_status.as_ref(),
            if was_running {
                "client_cancelled"
            } else {
                "process_already_exited"
            },
            stop_result.as_ref().err().map(String::as_str),
            &stderr_tail,
        );
        stop_result
    }
}

impl CodexAppServerSession {
    pub(super) fn take_pending_agent_events(&mut self) -> Vec<AgentEvent> {
        self.client.take_pending_agent_events()
    }

    pub(super) fn read_next_agent_events(&mut self) -> Result<Vec<AgentEvent>, String> {
        self.client.read_next_agent_events(&mut self.child)
    }
}

pub(super) fn start_codex_app_server_session(
    context: CodingAgentStartContext,
    run_id: &str,
    request_receiver: Option<Receiver<ServerRequestResolution>>,
) -> Result<CodexAppServerSession, String> {
    validate_coding_agent_start_context(&context)?;
    let codex_version = validate_codex_executable()?;
    let permission_settings = resolve_coding_agent_permission_settings()?;
    let transport_log = AgentRunTransportLog::create(&context.project_path, run_id, "app_server")?;

    let executable = codex_executable();
    let process_args = ["app-server", "--stdio"];
    transport_log.record(
        "meta",
        "process_starting",
        json!({
            "codexVersion": codex_version,
            "protocolBaselineVersion": CODEX_APP_SERVER_PROTOCOL_BASELINE_VERSION,
            "executable": executable,
            "args": process_args,
            "cwd": context.project_path,
            "transport": CODEX_APP_SERVER_TRANSPORT,
            "sandbox": context.sandbox.unwrap_or(DEFAULT_CODEX_SANDBOX).app_server_thread_sandbox(),
            "approvalPolicy": permission_settings.approval_policy.as_str(),
            "approvalsReviewer": permission_settings.approvals_reviewer.as_str()
        }),
    )?;
    let mut child = match Command::new(&executable)
        .args(process_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let message = format!("启动 Codex app-server 失败：{error}");
            transport_log.record_process_exit(None, "spawn_failed", Some(&message), &[]);
            return Err(message);
        }
    };
    if let Err(error) =
        transport_log.record("meta", "process_started", json!({ "pid": child.id() }))
    {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            error,
            "process_log_failed",
        ));
    }

    let Some(stdin) = child.stdin.take() else {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            "Codex app-server stdin 不可用。".to_string(),
            "missing_stdin",
        ));
    };
    let Some(stdout) = child.stdout.take() else {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            "Codex app-server stdout 不可用。".to_string(),
            "missing_stdout",
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        return Err(cleanup_child_with_error(
            &mut child,
            &transport_log,
            "Codex app-server stderr 不可用。".to_string(),
            "missing_stderr",
        ));
    };
    let mut client = CodexAppServerClient::new(
        stdin,
        stdout,
        stderr,
        transport_log,
        request_receiver,
        permission_settings.approvals_reviewer,
    );
    if let Some(message) = codex_protocol_version_warning(&codex_version) {
        client
            .pending_agent_events
            .push_back(AgentEvent::Diagnostic {
                level: "warning".to_string(),
                message,
                method: None,
                created_at: current_agent_event_timestamp(),
            });
    }
    if let Err(error) = initialize_codex_app_server(&mut child, &mut client) {
        let error = cleanup_child_with_error(
            &mut child,
            &client.transport_log,
            error,
            "initialize_failed",
        );
        client.join_readers();
        return Err(error);
    }
    let run_handles =
        match start_initial_codex_turn(&mut child, &mut client, &context, permission_settings) {
            Ok(run_handles) => run_handles,
            Err(error) => {
                let error = cleanup_child_with_error(
                    &mut child,
                    &client.transport_log,
                    error,
                    "turn_start_failed",
                );
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

pub(super) fn validate_coding_agent_start_context(
    context: &CodingAgentStartContext,
) -> Result<(), String> {
    if context.project_path.trim().is_empty() {
        return Err("Coding Agent 启动失败：项目路径不能为空。".to_string());
    }
    if context.prompt.trim().is_empty() {
        return Err("Coding Agent 启动失败：prompt 不能为空。".to_string());
    }
    Ok(())
}

fn cleanup_child_with_error(
    child: &mut Child,
    transport_log: &AgentRunTransportLog,
    error: String,
    reason: &str,
) -> String {
    let _ = child.kill();
    let status = child.wait().ok();
    transport_log.record_process_exit(status.as_ref(), reason, Some(&error), &[]);
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

pub(super) fn initialize_params() -> Value {
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

pub(super) fn build_thread_start_params(
    project_path: &str,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
) -> Value {
    json!({
        "cwd": project_path,
        "approvalPolicy": permission_settings.approval_policy.as_str(),
        "approvalsReviewer": permission_settings.approvals_reviewer.as_str(),
        "sandbox": sandbox.app_server_thread_sandbox(),
        "threadSource": "user"
    })
}

#[allow(dead_code)]
pub(super) fn build_thread_resume_params(
    thread_id: &str,
    project_path: &str,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
) -> Value {
    json!({
        "threadId": thread_id,
        "cwd": project_path,
        "approvalPolicy": permission_settings.approval_policy.as_str(),
        "approvalsReviewer": permission_settings.approvals_reviewer.as_str(),
        "sandbox": sandbox.app_server_thread_sandbox()
    })
}

pub(super) fn build_turn_start_params(
    thread_id: &str,
    project_path: &str,
    sandbox: CodingAgentSandboxMode,
    permission_settings: CodingAgentPermissionSettings,
    prompt: &str,
) -> Value {
    json!({
        "threadId": thread_id,
        "cwd": project_path,
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
