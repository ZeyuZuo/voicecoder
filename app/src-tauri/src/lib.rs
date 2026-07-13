use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

mod coding_agent;
mod dev_server;
mod env_config;
mod llm;
mod voice;

#[derive(Serialize)]
struct FileTreeEntry {
    name: String,
    path: String,
    is_directory: bool,
    children: Option<Vec<FileTreeEntry>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveRequirementDocumentRequest {
    project_path: String,
    requirement_document: String,
    summary: Option<String>,
    coding_prompt: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveDemoSessionLogRequest {
    project_path: String,
    demo_session: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedRequirementDocument {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedDemoSessionLog {
    path: String,
}

#[tauri::command]
fn read_project_tree(path: String) -> Result<Vec<FileTreeEntry>, String> {
    let root = Path::new(&path);

    if !root.is_dir() {
        return Err("Selected path is not a directory.".to_string());
    }

    read_directory(root, 0)
}

#[tauri::command]
fn read_git_branch(path: String) -> Result<Option<String>, String> {
    let root = Path::new(&path);

    if !root.is_dir() {
        return Ok(None);
    }

    let git_dir = find_git_dir(root);
    let Some(git_dir) = git_dir else {
        return Ok(None);
    };

    let head_path = git_dir.join("HEAD");
    let head = fs::read_to_string(&head_path)
        .map_err(|error| format!("Failed to read Git HEAD: {error}"))?;
    let trimmed_head = head.trim();

    if let Some(branch) = trimmed_head.strip_prefix("ref: refs/heads/") {
        return Ok(Some(branch.to_string()));
    }

    if trimmed_head.is_empty() {
        return Ok(None);
    }

    Ok(Some(trimmed_head.chars().take(7).collect()))
}

#[tauri::command]
fn save_requirement_document(
    request: SaveRequirementDocumentRequest,
) -> Result<SavedRequirementDocument, String> {
    let project_root = Path::new(&request.project_path);
    if !project_root.is_dir() {
        return Err("当前项目路径不是有效文件夹，无法写入需求文档。".to_string());
    }

    let voicecoder_dir = project_root.join(".voicecoder");
    fs::create_dir_all(&voicecoder_dir)
        .map_err(|error| format!("创建 .voicecoder 目录失败：{error}"))?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S_%3f").to_string();
    let document_path = voicecoder_dir.join(format!("voice_requirements_{timestamp}.md"));
    fs::write(&document_path, build_requirement_markdown(&request))
        .map_err(|error| format!("写入需求文档失败：{error}"))?;

    Ok(SavedRequirementDocument {
        path: document_path.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn save_demo_session_log(
    request: SaveDemoSessionLogRequest,
) -> Result<SavedDemoSessionLog, String> {
    let project_root = Path::new(&request.project_path);
    if !project_root.is_dir() {
        return Err("当前项目路径不是有效文件夹，无法写入 DemoSession 日志。".to_string());
    }

    let voicecoder_dir = project_root.join(".voicecoder");
    fs::create_dir_all(&voicecoder_dir)
        .map_err(|error| format!("创建 .voicecoder 目录失败：{error}"))?;

    let session_id = request
        .demo_session
        .pointer("/id")
        .and_then(serde_json::Value::as_str)
        .map(sanitize_log_file_stem)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| Local::now().format("%Y%m%d_%H%M%S_%3f").to_string());
    let log_path = voicecoder_dir.join(format!("demo_session_{session_id}.json"));
    let log_payload = serde_json::json!({
        "savedAt": Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "demoSession": request.demo_session
    });
    let log_json = serde_json::to_string_pretty(&log_payload)
        .map_err(|error| format!("序列化 DemoSession 日志失败：{error}"))?;

    fs::write(&log_path, format!("{log_json}\n"))
        .map_err(|error| format!("写入 DemoSession 日志失败：{error}"))?;

    Ok(SavedDemoSessionLog {
        path: log_path.to_string_lossy().to_string(),
    })
}

fn read_directory(path: &Path, depth: usize) -> Result<Vec<FileTreeEntry>, String> {
    const MAX_DEPTH: usize = 3;
    const MAX_ENTRIES_PER_DIR: usize = 160;

    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("Failed to read directory: {error}"))?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            !matches!(
                name.as_str(),
                "node_modules" | ".git" | "target" | "dist" | ".next"
            )
        })
        .take(MAX_ENTRIES_PER_DIR)
        .map(|entry| {
            let entry_path = entry.path();
            let is_directory = entry_path.is_dir();
            let children = if is_directory && depth < MAX_DEPTH {
                read_directory(&entry_path, depth + 1).ok()
            } else {
                None
            };

            FileTreeEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                path: entry_path.to_string_lossy().to_string(),
                is_directory,
                children,
            }
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    Ok(entries)
}

fn find_git_dir(path: &Path) -> Option<std::path::PathBuf> {
    let dot_git = path.join(".git");

    if dot_git.is_dir() {
        return Some(dot_git);
    }

    if dot_git.is_file() {
        let git_file = fs::read_to_string(dot_git).ok()?;
        let git_dir_path = git_file.trim().strip_prefix("gitdir:")?.trim();
        let resolved = path.join(git_dir_path);
        if resolved.is_dir() {
            return Some(resolved);
        }
    }

    None
}

fn build_requirement_markdown(request: &SaveRequirementDocumentRequest) -> String {
    let mut sections = vec![
        "# 语音需求文档".to_string(),
        format!("生成时间：{}", Local::now().format("%Y-%m-%d %H:%M:%S")),
    ];

    if let Some(summary) = request
        .summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("## 当前理解\n\n{}", summary.trim()));
    }

    sections.push(format!(
        "## 需求文档\n\n{}",
        request.requirement_document.trim()
    ));

    if let Some(coding_prompt) = request
        .coding_prompt
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("## Coding Prompt\n\n{}", coding_prompt.trim()));
    }

    format!("{}\n", sections.join("\n\n"))
}

fn sanitize_log_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || matches!(char, '-' | '_') {
                char
            } else {
                '_'
            }
        })
        .collect()
}

pub fn run() {
    install_rustls_crypto_provider();

    tauri::Builder::default()
        .manage(dev_server::DevServerState::default())
        .manage(voice::VoiceState::default())
        .manage(coding_agent::CodingAgentRequestState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            read_project_tree,
            read_git_branch,
            save_requirement_document,
            save_demo_session_log,
            dev_server::get_dev_server_snapshot,
            dev_server::get_dev_server_diagnostic,
            dev_server::start_demo_dev_server,
            dev_server::stop_demo_dev_server,
            voice::start_voice_session,
            voice::send_voice_audio_chunk,
            voice::get_voice_provider_status,
            coding_agent::get_coding_agent_provider_status,
            coding_agent::start_initial_demo_run,
            coding_agent::resolve_coding_agent_server_request,
            llm::get_llm_provider_status,
            llm::test_llm_provider_connection,
            llm::summarize_requirement_state,
            llm::process_requirement_turn,
            voice::check_tencent_asr_config,
            voice::get_voice_session_snapshot,
            voice::stop_voice_session,
            voice::cancel_voice_session
        ])
        .run(tauri::generate_context!())
        .expect("failed to run VoiceCoder");
}

pub(crate) fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
