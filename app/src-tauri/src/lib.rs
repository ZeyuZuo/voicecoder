use serde::Serialize;
use std::{fs, path::Path};

mod voice;

#[derive(Serialize)]
struct FileTreeEntry {
    name: String,
    path: String,
    is_directory: bool,
    children: Option<Vec<FileTreeEntry>>,
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

pub fn run() {
    tauri::Builder::default()
        .manage(voice::VoiceState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            read_project_tree,
            read_git_branch,
            voice::start_voice_session,
            voice::send_voice_audio_chunk,
            voice::get_voice_provider_status,
            voice::check_tencent_asr_config,
            voice::get_voice_session_snapshot,
            voice::stop_voice_session,
            voice::cancel_voice_session
        ])
        .run(tauri::generate_context!())
        .expect("failed to run VoiceCoder");
}
