//! Atomic, redacted DemoSession snapshots for local audit logs.

use crate::log_sanitizer::sanitize_json_for_log;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(test)]
use std::time::SystemTime;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const DEMO_SESSION_LOG_SCHEMA_VERSION: u64 = 2;
const DEMO_SESSION_LOG_PREFIX: &str = "demo_session_";
const DEMO_SESSION_LOG_SUFFIX: &str = ".json";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveDemoSessionLogRequest {
    project_path: String,
    demo_session: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedDemoSessionLog {
    path: String,
}

#[cfg(test)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestDemoSessionLog {
    schema_version: u64,
    saved_at: String,
    path: String,
    demo_session: Value,
}

#[tauri::command]
pub(crate) fn save_demo_session_log(
    request: SaveDemoSessionLogRequest,
) -> Result<SavedDemoSessionLog, String> {
    let project_root = validated_project_root(&request.project_path)?;
    let voicecoder_dir = project_root.join(".voicecoder");
    fs::create_dir_all(&voicecoder_dir)
        .map_err(|error| format!("创建 .voicecoder 目录失败：{error}"))?;

    let session_id = request
        .demo_session
        .pointer("/id")
        .and_then(Value::as_str)
        .map(sanitize_log_file_stem)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| Utc::now().format("%Y%m%d_%H%M%S_%3f").to_string());
    let log_path = voicecoder_dir.join(format!(
        "{DEMO_SESSION_LOG_PREFIX}{session_id}{DEMO_SESSION_LOG_SUFFIX}"
    ));

    if existing_snapshot_is_newer(&log_path, &request.demo_session) {
        restrict_log_permissions(&log_path)?;
        return Ok(SavedDemoSessionLog {
            path: display_path(&log_path),
        });
    }

    let saved_at = Utc::now().to_rfc3339();
    let log_payload = json!({
        "schemaVersion": DEMO_SESSION_LOG_SCHEMA_VERSION,
        "snapshotKind": "structured-agent-domain",
        "savedAt": saved_at,
        "demoSession": sanitize_json_for_log(&request.demo_session)
    });
    let log_json = serde_json::to_string_pretty(&log_payload)
        .map_err(|error| format!("序列化 DemoSession 日志失败：{error}"))?;
    atomic_write(&log_path, format!("{log_json}\n").as_bytes())?;
    restrict_log_permissions(&log_path)?;

    Ok(SavedDemoSessionLog {
        path: display_path(&log_path),
    })
}

#[cfg(test)]
fn read_latest_demo_session_log_for_test(
    project_path: String,
) -> Result<Option<TestDemoSessionLog>, String> {
    let project_root = validated_project_root(&project_path)?;
    let voicecoder_dir = project_root.join(".voicecoder");
    if !voicecoder_dir.is_dir() {
        return Ok(None);
    }

    let mut candidates = fs::read_dir(&voicecoder_dir)
        .map_err(|error| format!("读取 .voicecoder 目录失败：{error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with(DEMO_SESSION_LOG_PREFIX)
                || !name.ends_with(DEMO_SESSION_LOG_SUFFIX)
            {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    for (_, path) in candidates {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(payload) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Some(demo_session) = payload.get("demoSession").filter(|value| value.is_object())
        else {
            continue;
        };
        restrict_log_permissions(&path)?;

        return Ok(Some(TestDemoSessionLog {
            schema_version: payload
                .get("schemaVersion")
                .and_then(Value::as_u64)
                .unwrap_or(1),
            saved_at: payload
                .get("savedAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            path: display_path(&path),
            demo_session: demo_session.clone(),
        }));
    }

    Ok(None)
}

fn validated_project_root(project_path: &str) -> Result<&Path, String> {
    let project_root = Path::new(project_path);
    if project_root.is_dir() {
        Ok(project_root)
    } else {
        Err("当前项目路径不是有效文件夹，无法访问 DemoSession 日志。".to_string())
    }
}

fn existing_snapshot_is_newer(path: &Path, incoming: &Value) -> bool {
    let Some(incoming_updated_at) = incoming.get("updatedAt").and_then(Value::as_str) else {
        return false;
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|payload| {
            payload
                .pointer("/demoSession/updatedAt")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .is_some_and(|existing_updated_at| existing_updated_at.as_str() > incoming_updated_at)
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let temp_path = temporary_snapshot_path(path);
    let write_result = (|| -> Result<(), std::io::Error> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        if cfg!(windows) && path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(&temp_path, path)
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("原子写入 DemoSession 日志失败：{error}"));
    }
    Ok(())
}

fn temporary_snapshot_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("demo_session.json");
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn sanitize_log_file_stem(value: &str) -> String {
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

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn restrict_log_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("限制 DemoSession 日志权限失败：{error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_a_structured_redacted_snapshot() {
        let root = test_root("structured");
        let session = json!({
            "id": "demo/unsafe",
            "projectPath": display_path(&root),
            "updatedAt": "2026-07-13T10:00:00Z",
            "initialCodingPrompt": "private product requirement",
            "runs": [{
                "id": "run-1",
                "codexThreadId": "thread-1",
                "itemsById": { "item-1": { "status": "completed" } },
                "itemOrder": ["item-1"]
            }],
            "apiKey": "sk-example-secret-value"
        });

        let saved = save_demo_session_log(SaveDemoSessionLogRequest {
            project_path: display_path(&root),
            demo_session: session,
        })
        .unwrap();
        let loaded = read_latest_demo_session_log_for_test(display_path(&root))
            .unwrap()
            .unwrap();
        let content = fs::read_to_string(&saved.path).unwrap();

        assert!(saved.path.ends_with("demo_session_demo_unsafe.json"));
        assert_eq!(loaded.schema_version, DEMO_SESSION_LOG_SCHEMA_VERSION);
        assert_eq!(
            loaded
                .demo_session
                .pointer("/runs/0/itemsById/item-1/status"),
            Some(&json!("completed"))
        );
        assert_eq!(
            loaded.demo_session.get("apiKey"),
            Some(&json!("[REDACTED_CREDENTIAL]"))
        );
        assert!(!content.contains("sk-example-secret-value"));
        assert_eq!(
            loaded.demo_session.get("initialCodingPrompt"),
            Some(&json!("private product requirement"))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&saved.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_corrupt_snapshots_and_uses_the_latest_valid_log() {
        let root = test_root("corrupt");
        let voicecoder_dir = root.join(".voicecoder");
        fs::create_dir_all(&voicecoder_dir).unwrap();
        fs::write(
            voicecoder_dir.join("demo_session_valid.json"),
            r#"{"savedAt":"1","demoSession":{"id":"valid"}}"#,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            voicecoder_dir.join("demo_session_corrupt.json"),
            "{not-json",
        )
        .unwrap();

        let loaded = read_latest_demo_session_log_for_test(display_path(&root))
            .unwrap()
            .unwrap();

        assert_eq!(loaded.demo_session.get("id"), Some(&json!("valid")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_older_async_save_cannot_overwrite_a_newer_snapshot() {
        let root = test_root("ordering");
        let save = |updated_at: &str, status: &str| {
            save_demo_session_log(SaveDemoSessionLogRequest {
                project_path: display_path(&root),
                demo_session: json!({
                    "id": "demo-1",
                    "updatedAt": updated_at,
                    "status": status
                }),
            })
            .unwrap();
        };

        save("2026-07-13T10:00:02Z", "succeeded");
        save("2026-07-13T10:00:01Z", "running");
        let loaded = read_latest_demo_session_log_for_test(display_path(&root))
            .unwrap()
            .unwrap();

        assert_eq!(loaded.demo_session.get("status"), Some(&json!("succeeded")));
        fs::remove_dir_all(root).unwrap();
    }

    fn test_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "voicecoder-demo-session-{label}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
