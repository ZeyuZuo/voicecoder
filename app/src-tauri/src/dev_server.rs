use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, State};

const DEFAULT_DEV_SERVER_EXECUTABLE: &str = "npm";
const DEV_SERVER_EVENT: &str = "dev-server://event";

pub(crate) struct DevServerState {
    inner: Arc<Mutex<DevServerStateInner>>,
}

impl Default for DevServerState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(DevServerStateInner::default())),
        }
    }
}

#[derive(Default)]
struct DevServerStateInner {
    snapshot: Option<DevServerSessionSnapshot>,
    process: Option<ActiveDevServerProcess>,
}

#[derive(Clone)]
struct ActiveDevServerProcess {
    session_id: String,
    child: Arc<Mutex<Child>>,
}

impl DevServerState {
    fn snapshot(&self) -> Result<Option<DevServerSessionSnapshot>, String> {
        self.inner
            .lock()
            .map(|inner| inner.snapshot.clone())
            .map_err(|_| "Dev server state lock is poisoned.".to_string())
    }

    fn replace_active_session(
        &self,
        session: DevServerSessionSnapshot,
        child: Arc<Mutex<Child>>,
    ) -> Result<Option<DevServerSessionSnapshot>, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Dev server state lock is poisoned.".to_string())?;

        let replaced_session = inner.process.take().and_then(|process| {
            kill_dev_server_process(&process);
            mark_snapshot_stopped(&mut inner.snapshot, &process.session_id);
            inner.snapshot.clone()
        });

        inner.process = Some(ActiveDevServerProcess {
            session_id: session.id.clone(),
            child,
        });
        inner.snapshot = Some(session);

        Ok(replaced_session)
    }

    fn mark_session_running(
        &self,
        session_id: &str,
    ) -> Result<Option<DevServerSessionSnapshot>, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Dev server state lock is poisoned.".to_string())?;

        if let Some(snapshot) = inner
            .snapshot
            .as_mut()
            .filter(|snapshot| snapshot.id == session_id)
        {
            snapshot.status = DevServerSessionStatus::Running;
            snapshot.updated_at = current_dev_server_timestamp();
            return Ok(Some(snapshot.clone()));
        }

        Ok(None)
    }

    fn set_error_snapshot(&self, snapshot: DevServerSessionSnapshot) -> Result<(), String> {
        self.inner
            .lock()
            .map(|mut inner| {
                inner.snapshot = Some(snapshot);
            })
            .map_err(|_| "Dev server state lock is poisoned.".to_string())
    }

    fn stop_active_session(
        &self,
        requested_session_id: Option<&str>,
    ) -> Result<Option<DevServerSessionSnapshot>, String> {
        stop_active_session(&self.inner, requested_session_id)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServerSessionSnapshot {
    id: String,
    project_path: String,
    command: Vec<String>,
    status: DevServerSessionStatus,
    preview_url: Option<String>,
    started_at: Option<String>,
    updated_at: String,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum DevServerSessionStatus {
    Idle,
    Starting,
    Running,
    Ready,
    Stopped,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServerDiagnostic {
    configured: bool,
    command: Vec<String>,
    executable: Option<String>,
    version: Option<String>,
    missing_dependencies: Vec<String>,
    details: BTreeMap<String, String>,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevServerLifecycleEventEnvelope {
    session_id: String,
    project_path: String,
    event: DevServerLifecycleEvent,
    occurred_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[allow(dead_code)]
pub enum DevServerLifecycleEvent {
    Starting {
        command: Vec<String>,
    },
    Output {
        stream: DevServerOutputStream,
        text: String,
    },
    Ready {
        url: String,
    },
    Stopped {
        reason: DevServerStoppedReason,
        exit_code: Option<i32>,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum DevServerOutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum DevServerStoppedReason {
    Exited,
    User,
    Replaced,
    Error,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartDevServerRequest {
    project_path: String,
    session_id: Option<String>,
    command: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopDevServerRequest {
    session_id: Option<String>,
}

#[tauri::command]
pub fn get_dev_server_snapshot(
    state: State<'_, DevServerState>,
) -> Result<Option<DevServerSessionSnapshot>, String> {
    state.snapshot()
}

#[tauri::command]
pub fn get_dev_server_diagnostic() -> DevServerDiagnostic {
    build_dev_server_diagnostic(default_dev_server_command(), npm_version_result())
}

#[tauri::command]
pub fn start_demo_dev_server(
    app: AppHandle,
    state: State<'_, DevServerState>,
    request: StartDevServerRequest,
) -> Result<DevServerSessionSnapshot, String> {
    let project_path = normalize_project_path(&request.project_path)?;
    let command = normalize_dev_server_command(request.command)?;
    let session_id = request
        .session_id
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(create_dev_server_session_id);
    let now = current_dev_server_timestamp();
    let session =
        create_dev_server_session_snapshot(session_id, project_path.clone(), command.clone(), now);

    let spawn_result = spawn_dev_server_process(&project_path, &command);
    let mut child = match spawn_result {
        Ok(child) => child,
        Err(error) => {
            let mut error_snapshot = session;
            error_snapshot.status = DevServerSessionStatus::Error;
            error_snapshot.error = Some(error.clone());
            error_snapshot.updated_at = current_dev_server_timestamp();
            state.set_error_snapshot(error_snapshot.clone())?;
            let _ = emit_dev_server_lifecycle_event(
                &app,
                create_dev_server_lifecycle_event(
                    &error_snapshot,
                    DevServerLifecycleEvent::Error {
                        message: error.clone(),
                    },
                ),
            );
            return Err(error);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    let replaced_session = state.replace_active_session(session.clone(), child.clone())?;

    if let Some(replaced_session) = replaced_session {
        emit_dev_server_lifecycle_event(
            &app,
            create_dev_server_lifecycle_event(
                &replaced_session,
                DevServerLifecycleEvent::Stopped {
                    reason: DevServerStoppedReason::Replaced,
                    exit_code: None,
                },
            ),
        )?;
    }

    emit_dev_server_lifecycle_event(
        &app,
        create_dev_server_lifecycle_event(
            &session,
            DevServerLifecycleEvent::Starting {
                command: command.clone(),
            },
        ),
    )?;

    if let Some(stdout) = stdout {
        spawn_dev_server_output_reader(
            app.clone(),
            state.inner.clone(),
            session.id.clone(),
            project_path.clone(),
            DevServerOutputStream::Stdout,
            stdout,
        );
    }

    if let Some(stderr) = stderr {
        spawn_dev_server_output_reader(
            app.clone(),
            state.inner.clone(),
            session.id.clone(),
            project_path.clone(),
            DevServerOutputStream::Stderr,
            stderr,
        );
    }

    spawn_dev_server_exit_monitor(
        app,
        state.inner.clone(),
        session.id.clone(),
        project_path,
        child,
    );

    Ok(state.mark_session_running(&session.id)?.unwrap_or(session))
}

#[tauri::command]
pub fn stop_demo_dev_server(
    app: AppHandle,
    state: State<'_, DevServerState>,
    request: StopDevServerRequest,
) -> Result<Option<DevServerSessionSnapshot>, String> {
    let stopped_session = state.stop_active_session(request.session_id.as_deref())?;

    if let Some(stopped_session) = stopped_session.as_ref() {
        emit_dev_server_lifecycle_event(
            &app,
            create_dev_server_lifecycle_event(
                stopped_session,
                DevServerLifecycleEvent::Stopped {
                    reason: DevServerStoppedReason::User,
                    exit_code: None,
                },
            ),
        )?;
    }

    Ok(stopped_session)
}

pub(crate) fn emit_dev_server_lifecycle_event(
    app: &AppHandle,
    payload: DevServerLifecycleEventEnvelope,
) -> Result<(), String> {
    app.emit(DEV_SERVER_EVENT, payload)
        .map_err(|error| format!("Failed to emit dev server lifecycle event: {error}"))
}

#[allow(dead_code)]
pub(crate) fn create_dev_server_session_snapshot(
    id: String,
    project_path: String,
    command: Vec<String>,
    now: String,
) -> DevServerSessionSnapshot {
    DevServerSessionSnapshot {
        id,
        project_path,
        command,
        status: DevServerSessionStatus::Starting,
        preview_url: None,
        started_at: Some(now.clone()),
        updated_at: now,
        error: None,
    }
}

#[allow(dead_code)]
pub(crate) fn create_dev_server_lifecycle_event(
    session: &DevServerSessionSnapshot,
    event: DevServerLifecycleEvent,
) -> DevServerLifecycleEventEnvelope {
    DevServerLifecycleEventEnvelope {
        session_id: session.id.clone(),
        project_path: session.project_path.clone(),
        event,
        occurred_at: current_dev_server_timestamp(),
    }
}

fn normalize_project_path(project_path: &str) -> Result<String, String> {
    let project_path = project_path.trim();
    if project_path.is_empty() {
        return Err("项目路径不能为空，无法启动 dev server。".to_string());
    }

    if !Path::new(project_path).is_dir() {
        return Err("项目路径不是有效文件夹，无法启动 dev server。".to_string());
    }

    Ok(project_path.to_string())
}

fn normalize_dev_server_command(command: Option<Vec<String>>) -> Result<Vec<String>, String> {
    let command = command.unwrap_or_else(default_dev_server_command);
    let command = command
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if command.is_empty() {
        return Err("dev server 启动命令不能为空。".to_string());
    }

    Ok(command)
}

fn spawn_dev_server_process(project_path: &str, command: &[String]) -> Result<Child, String> {
    let executable = command
        .first()
        .ok_or_else(|| "dev server 启动命令不能为空。".to_string())?;
    let args = command
        .iter()
        .skip(1)
        .map(String::as_str)
        .collect::<Vec<_>>();

    let mut command = Command::new(executable);
    command
        .args(args)
        .current_dir(project_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command
        .spawn()
        .map_err(|error| format!("启动 dev server 失败：{error}"))
}

fn spawn_dev_server_output_reader<R>(
    app: AppHandle,
    state: Arc<Mutex<DevServerStateInner>>,
    session_id: String,
    project_path: String,
    stream: DevServerOutputStream,
    reader: R,
) where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(reader);

        for line in reader.lines() {
            if !is_active_session(&state, &session_id) {
                break;
            }

            match line {
                Ok(text) => {
                    let _ = emit_dev_server_lifecycle_event(
                        &app,
                        DevServerLifecycleEventEnvelope {
                            session_id: session_id.clone(),
                            project_path: project_path.clone(),
                            event: DevServerLifecycleEvent::Output { stream, text },
                            occurred_at: current_dev_server_timestamp(),
                        },
                    );
                }
                Err(error) => {
                    let message = format!("读取 dev server 输出失败：{error}");
                    let _ = fail_active_session(&state, &session_id, message.clone());
                    let _ = emit_dev_server_lifecycle_event(
                        &app,
                        DevServerLifecycleEventEnvelope {
                            session_id: session_id.clone(),
                            project_path: project_path.clone(),
                            event: DevServerLifecycleEvent::Error { message },
                            occurred_at: current_dev_server_timestamp(),
                        },
                    );
                    break;
                }
            }
        }
    });
}

fn spawn_dev_server_exit_monitor(
    app: AppHandle,
    state: Arc<Mutex<DevServerStateInner>>,
    session_id: String,
    project_path: String,
    child: Arc<Mutex<Child>>,
) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(250));

        let exit_status = match child.lock() {
            Ok(mut child) => match child.try_wait() {
                Ok(Some(status)) => Some(Ok(status.code())),
                Ok(None) => None,
                Err(error) => Some(Err(format!("检查 dev server 退出状态失败：{error}"))),
            },
            Err(_) => Some(Err("Dev server process lock is poisoned.".to_string())),
        };

        match exit_status {
            Some(Ok(exit_code)) => {
                if finish_active_session(&state, &session_id).is_some() {
                    let _ = emit_dev_server_lifecycle_event(
                        &app,
                        DevServerLifecycleEventEnvelope {
                            session_id,
                            project_path,
                            event: DevServerLifecycleEvent::Stopped {
                                reason: DevServerStoppedReason::Exited,
                                exit_code,
                            },
                            occurred_at: current_dev_server_timestamp(),
                        },
                    );
                }
                break;
            }
            Some(Err(message)) => {
                if fail_active_session(&state, &session_id, message.clone()).is_some() {
                    let _ = emit_dev_server_lifecycle_event(
                        &app,
                        DevServerLifecycleEventEnvelope {
                            session_id,
                            project_path,
                            event: DevServerLifecycleEvent::Error { message },
                            occurred_at: current_dev_server_timestamp(),
                        },
                    );
                }
                break;
            }
            None => {}
        }
    });
}

fn is_active_session(state: &Arc<Mutex<DevServerStateInner>>, session_id: &str) -> bool {
    state
        .lock()
        .map(|inner| {
            inner
                .process
                .as_ref()
                .is_some_and(|process| process.session_id == session_id)
        })
        .unwrap_or(false)
}

fn stop_active_session(
    state: &Arc<Mutex<DevServerStateInner>>,
    requested_session_id: Option<&str>,
) -> Result<Option<DevServerSessionSnapshot>, String> {
    let mut inner = state
        .lock()
        .map_err(|_| "Dev server state lock is poisoned.".to_string())?;
    let Some(process) = inner.process.take() else {
        return Ok(None);
    };

    if let Some(requested_session_id) = requested_session_id {
        if requested_session_id != process.session_id {
            inner.process = Some(process);
            return Err("没有找到匹配的 dev server session。".to_string());
        }
    }

    kill_dev_server_process(&process);
    mark_snapshot_stopped(&mut inner.snapshot, &process.session_id);

    Ok(inner.snapshot.clone())
}

fn finish_active_session(
    state: &Arc<Mutex<DevServerStateInner>>,
    session_id: &str,
) -> Option<DevServerSessionSnapshot> {
    let mut inner = state.lock().ok()?;
    if !inner
        .process
        .as_ref()
        .is_some_and(|process| process.session_id == session_id)
    {
        return None;
    }

    inner.process = None;
    mark_snapshot_stopped(&mut inner.snapshot, session_id);
    inner.snapshot.clone()
}

fn fail_active_session(
    state: &Arc<Mutex<DevServerStateInner>>,
    session_id: &str,
    message: String,
) -> Option<DevServerSessionSnapshot> {
    let mut inner = state.lock().ok()?;
    if !inner
        .process
        .as_ref()
        .is_some_and(|process| process.session_id == session_id)
    {
        return None;
    }

    inner.process = None;
    if let Some(snapshot) = inner
        .snapshot
        .as_mut()
        .filter(|snapshot| snapshot.id == session_id)
    {
        snapshot.status = DevServerSessionStatus::Error;
        snapshot.error = Some(message);
        snapshot.updated_at = current_dev_server_timestamp();
    }

    inner.snapshot.clone()
}

fn mark_snapshot_stopped(snapshot: &mut Option<DevServerSessionSnapshot>, session_id: &str) {
    if let Some(snapshot) = snapshot
        .as_mut()
        .filter(|snapshot| snapshot.id == session_id)
    {
        snapshot.status = DevServerSessionStatus::Stopped;
        snapshot.updated_at = current_dev_server_timestamp();
    }
}

fn kill_dev_server_process(process: &ActiveDevServerProcess) {
    if let Ok(mut child) = process.child.lock() {
        if terminate_dev_server_process_group(child.id()) {
            return;
        }

        let _ = child.kill();
    }
}

#[cfg(unix)]
fn terminate_dev_server_process_group(process_id: u32) -> bool {
    Command::new("kill")
        .args(["-TERM", &format!("-{process_id}")])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn terminate_dev_server_process_group(_process_id: u32) -> bool {
    false
}

fn build_dev_server_diagnostic(
    command: Vec<String>,
    version_result: Result<String, String>,
) -> DevServerDiagnostic {
    let mut details = BTreeMap::new();
    details.insert("defaultCommand".to_string(), command.join(" "));

    match version_result {
        Ok(version) => DevServerDiagnostic {
            configured: true,
            command,
            executable: Some(DEFAULT_DEV_SERVER_EXECUTABLE.to_string()),
            version: Some(version),
            missing_dependencies: vec![],
            details,
            error: None,
        },
        Err(error) => DevServerDiagnostic {
            configured: false,
            command,
            executable: Some(DEFAULT_DEV_SERVER_EXECUTABLE.to_string()),
            version: None,
            missing_dependencies: vec![DEFAULT_DEV_SERVER_EXECUTABLE.to_string()],
            details,
            error: Some(error),
        },
    }
}

fn default_dev_server_command() -> Vec<String> {
    vec![
        DEFAULT_DEV_SERVER_EXECUTABLE.to_string(),
        "run".to_string(),
        "dev".to_string(),
    ]
}

fn npm_version_result() -> Result<String, String> {
    Command::new(DEFAULT_DEV_SERVER_EXECUTABLE)
        .arg("--version")
        .output()
        .map_err(|error| format!("Failed to run npm --version: {error}"))
        .and_then(|output| {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
            }
        })
}

fn current_dev_server_timestamp() -> String {
    Utc::now().to_rfc3339()
}

fn create_dev_server_session_id() -> String {
    format!("dev_server_{}", Utc::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn builds_configured_dev_server_diagnostic() {
        let diagnostic = build_dev_server_diagnostic(
            vec!["npm".to_string(), "run".to_string(), "dev".to_string()],
            Ok("10.0.0".to_string()),
        );

        assert!(diagnostic.configured);
        assert_eq!(diagnostic.command, vec!["npm", "run", "dev"]);
        assert_eq!(diagnostic.executable.as_deref(), Some("npm"));
        assert_eq!(diagnostic.version.as_deref(), Some("10.0.0"));
        assert!(diagnostic.missing_dependencies.is_empty());
    }

    #[test]
    fn builds_missing_npm_diagnostic() {
        let diagnostic = build_dev_server_diagnostic(
            vec!["npm".to_string(), "run".to_string(), "dev".to_string()],
            Err("npm not found".to_string()),
        );

        assert!(!diagnostic.configured);
        assert_eq!(diagnostic.missing_dependencies, vec!["npm"]);
        assert_eq!(diagnostic.error.as_deref(), Some("npm not found"));
    }

    #[test]
    fn creates_starting_session_snapshot() {
        let session = create_dev_server_session_snapshot(
            "dev_server_1".to_string(),
            "/tmp/demo".to_string(),
            default_dev_server_command(),
            "2026-06-24T12:00:00Z".to_string(),
        );

        assert_eq!(session.id, "dev_server_1");
        assert_eq!(session.project_path, "/tmp/demo");
        assert_eq!(session.status, DevServerSessionStatus::Starting);
        assert_eq!(session.started_at.as_deref(), Some("2026-06-24T12:00:00Z"));
        assert!(session.preview_url.is_none());
    }

    #[test]
    fn serializes_ready_lifecycle_event_for_frontend_contract() {
        let session = create_dev_server_session_snapshot(
            "dev_server_1".to_string(),
            "/tmp/demo".to_string(),
            default_dev_server_command(),
            "2026-06-24T12:00:00Z".to_string(),
        );
        let envelope = DevServerLifecycleEventEnvelope {
            session_id: session.id,
            project_path: session.project_path,
            event: DevServerLifecycleEvent::Ready {
                url: "http://localhost:5173".to_string(),
            },
            occurred_at: "2026-06-24T12:00:01Z".to_string(),
        };

        assert_eq!(
            to_value(envelope).unwrap(),
            json!({
                "sessionId": "dev_server_1",
                "projectPath": "/tmp/demo",
                "event": {
                    "type": "ready",
                    "url": "http://localhost:5173"
                },
                "occurredAt": "2026-06-24T12:00:01Z"
            })
        );
    }

    #[test]
    fn dev_server_event_name_is_stable() {
        assert_eq!(DEV_SERVER_EVENT, "dev-server://event");
    }

    #[test]
    fn normalizes_missing_command_to_npm_run_dev() {
        assert_eq!(
            normalize_dev_server_command(None).unwrap(),
            vec!["npm", "run", "dev"]
        );
    }

    #[test]
    fn normalizes_custom_command_and_rejects_empty_command() {
        assert_eq!(
            normalize_dev_server_command(Some(vec![
                " npm ".to_string(),
                "".to_string(),
                " run ".to_string(),
                " dev ".to_string()
            ]))
            .unwrap(),
            vec!["npm", "run", "dev"]
        );
        assert!(normalize_dev_server_command(Some(vec![" ".to_string()])).is_err());
    }

    #[test]
    fn serializes_output_lifecycle_event_for_frontend_contract() {
        let envelope = DevServerLifecycleEventEnvelope {
            session_id: "dev_server_1".to_string(),
            project_path: "/tmp/demo".to_string(),
            event: DevServerLifecycleEvent::Output {
                stream: DevServerOutputStream::Stdout,
                text: "VITE ready".to_string(),
            },
            occurred_at: "2026-06-24T12:00:01Z".to_string(),
        };

        assert_eq!(
            to_value(envelope).unwrap(),
            json!({
                "sessionId": "dev_server_1",
                "projectPath": "/tmp/demo",
                "event": {
                    "type": "output",
                    "stream": "stdout",
                    "text": "VITE ready"
                },
                "occurredAt": "2026-06-24T12:00:01Z"
            })
        );
    }

    #[test]
    fn stop_without_active_process_returns_none() {
        let state = DevServerState::default();

        assert!(state.stop_active_session(None).unwrap().is_none());
    }
}
