use chrono::Utc;
use serde::Serialize;
use std::{collections::BTreeMap, process::Command, sync::Mutex};
use tauri::{AppHandle, Emitter, State};

const DEFAULT_DEV_SERVER_EXECUTABLE: &str = "npm";
const DEV_SERVER_EVENT: &str = "dev-server://event";

#[derive(Default)]
pub(crate) struct DevServerState {
    active_session: Mutex<Option<DevServerSessionSnapshot>>,
}

impl DevServerState {
    fn snapshot(&self) -> Result<Option<DevServerSessionSnapshot>, String> {
        self.active_session
            .lock()
            .map(|session| session.clone())
            .map_err(|_| "Dev server state lock is poisoned.".to_string())
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

#[allow(dead_code)]
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
}
