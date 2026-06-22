use crate::env_config::read_local_env;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

const DEFAULT_CODEX_BIN: &str = "codex";
const FIRST_APP_SERVER_REQUEST_ID: u64 = 0;

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

fn start_codex_app_server_session(
    context: CodingAgentStartContext,
) -> Result<CodexAppServerSession, String> {
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

    Ok(CodexAppServerSession {
        child,
        client,
        project_path: context.project_path,
        initial_prompt: context.prompt,
    })
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
}
