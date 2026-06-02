mod tencent;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use tencent::{TencentAsrConfig, TencentAsrProvider};
use tokio::runtime::Runtime;

const SESSION_STARTED_EVENT: &str = "voice://session-started";
pub(crate) const TRANSCRIPT_EVENT: &str = "voice://transcript";
pub(crate) const ERROR_EVENT: &str = "voice://error";
const STOPPED_EVENT: &str = "voice://stopped";

#[derive(Default)]
pub struct VoiceState {
    active_session: Mutex<Option<ActiveVoiceSession>>,
}

struct ActiveVoiceSession {
    session_id: String,
    provider: VoiceProviderKind,
    cancel_signal: Arc<AtomicBool>,
    finish_signal: Arc<AtomicBool>,
    received_audio_chunks: usize,
    provider_session: Option<Box<dyn AsrSession + Send>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceProviderKind {
    Auto,
    Mock,
    Tencent,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceSessionStartedEvent {
    session_id: String,
    provider: VoiceProviderKind,
    started_at: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VoiceTranscriptEvent {
    pub id: String,
    pub session_id: String,
    pub speaker_id: Option<String>,
    pub text: String,
    pub is_final: bool,
    pub started_at_ms: Option<u32>,
    pub ended_at_ms: Option<u32>,
    pub created_at: String,
}

pub(crate) trait AsrSession {
    fn send_audio_chunk(&mut self, chunk: Vec<u8>) -> Result<(), String>;
    fn stop(&mut self);
}

pub(crate) trait AsrProvider {
    fn kind(&self) -> VoiceProviderKind;
    fn validate_start(&self) -> Result<(), String> {
        Ok(())
    }
    fn diagnostic(&self) -> VoiceProviderDiagnostic;
    fn start_session(&self, context: AsrStartContext)
        -> Result<Box<dyn AsrSession + Send>, String>;
}

pub(crate) struct AsrStartContext {
    pub app: AppHandle,
    pub session_id: String,
    pub cancel_signal: Arc<AtomicBool>,
    pub finish_signal: Arc<AtomicBool>,
}

struct MockAsrProvider;

struct MockAsrSession;

impl AsrSession for MockAsrSession {
    fn send_audio_chunk(&mut self, _chunk: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    fn stop(&mut self) {}
}

impl AsrProvider for MockAsrProvider {
    fn kind(&self) -> VoiceProviderKind {
        VoiceProviderKind::Mock
    }

    fn diagnostic(&self) -> VoiceProviderDiagnostic {
        VoiceProviderDiagnostic {
            provider: self.kind(),
            configured: true,
            missing_env: Vec::new(),
            endpoint: None,
            details: BTreeMap::new(),
            error: None,
        }
    }

    fn start_session(
        &self,
        context: AsrStartContext,
    ) -> Result<Box<dyn AsrSession + Send>, String> {
        spawn_mock_provider(
            context.app,
            context.session_id,
            Arc::clone(&context.cancel_signal),
        );
        Ok(Box::new(MockAsrSession))
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceErrorEvent {
    session_id: Option<String>,
    message: String,
    code: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProviderStatus {
    auto_provider: VoiceProviderKind,
    provider_override: Option<VoiceProviderKind>,
    tencent_configured: bool,
    missing_tencent_env: Vec<String>,
    diagnostics: Vec<VoiceProviderDiagnostic>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TencentAsrConfigCheck {
    ok: bool,
    missing_env: Vec<String>,
    host: Option<String>,
    app_id: Option<String>,
    engine_model_type: Option<String>,
    sentence_strategy: Option<u8>,
    voice_format: Option<u8>,
    need_vad: Option<u8>,
    signed_url_preview: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProviderDiagnostic {
    provider: VoiceProviderKind,
    configured: bool,
    missing_env: Vec<String>,
    endpoint: Option<String>,
    details: BTreeMap<String, String>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSessionSnapshot {
    active: bool,
    session_id: Option<String>,
    provider: Option<VoiceProviderKind>,
    received_audio_chunks: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceStoppedEvent {
    session_id: Option<String>,
    reason: VoiceStoppedReason,
    stopped_at: String,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VoiceStoppedReason {
    User,
    Completed,
    Error,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAudioChunk {
    session_id: String,
    sample_rate: u32,
    channels: u8,
    format: VoiceAudioFormat,
    sequence: u32,
    data: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum VoiceAudioFormat {
    PcmS16le,
}

#[tauri::command]
pub fn start_voice_session(
    app: AppHandle,
    state: tauri::State<'_, VoiceState>,
    provider: VoiceProviderKind,
) -> Result<String, String> {
    let mut active_session = state
        .active_session
        .lock()
        .map_err(|_| "Voice session state is unavailable.".to_string())?;

    if active_session.is_some() {
        return Err("已有语音会话正在运行。".to_string());
    }

    let session_id = create_session_id();
    let cancel_signal = Arc::new(AtomicBool::new(false));
    let finish_signal = Arc::new(AtomicBool::new(false));
    let resolved_provider = resolve_provider(provider);
    let provider_adapter = provider_for_kind(resolved_provider)?;
    provider_adapter.validate_start()?;

    *active_session = Some(ActiveVoiceSession {
        session_id: session_id.clone(),
        provider: resolved_provider,
        cancel_signal: Arc::clone(&cancel_signal),
        finish_signal: Arc::clone(&finish_signal),
        received_audio_chunks: 0,
        provider_session: None,
    });

    if let Err(error) = app.emit(
        SESSION_STARTED_EVENT,
        VoiceSessionStartedEvent {
            session_id: session_id.clone(),
            provider: resolved_provider,
            started_at: now_millis_string(),
        },
    ) {
        *active_session = None;
        return Err(format!("Failed to emit voice start event: {error}"));
    }

    let provider_session = match provider_adapter.start_session(AsrStartContext {
        app: app.clone(),
        session_id: session_id.clone(),
        cancel_signal: Arc::clone(&cancel_signal),
        finish_signal: Arc::clone(&finish_signal),
    }) {
        Ok(provider_session) => provider_session,
        Err(error) => {
            *active_session = None;
            return Err(error);
        }
    };
    let Some(session) = active_session.as_mut() else {
        return Err("Voice session was stopped before ASR provider started.".to_string());
    };
    session.provider_session = Some(provider_session);

    Ok(session_id)
}

#[tauri::command]
pub fn send_voice_audio_chunk(
    app: AppHandle,
    state: tauri::State<'_, VoiceState>,
    chunk: VoiceAudioChunk,
) -> Result<(), String> {
    if chunk.sample_rate != 16_000 || chunk.channels != 1 {
        return Err("语音分片必须是 16kHz 单声道音频。".to_string());
    }

    if !matches!(chunk.format, VoiceAudioFormat::PcmS16le) {
        return Err("语音分片必须是 pcm_s16le 格式。".to_string());
    }

    let mut active_session = state
        .active_session
        .lock()
        .map_err(|_| "Voice session state is unavailable.".to_string())?;

    let Some(session) = active_session.as_mut() else {
        return Err("没有正在运行的语音会话。".to_string());
    };

    if session.session_id != chunk.session_id {
        return Err("语音分片不属于当前会话。".to_string());
    }

    if chunk.data.is_empty() {
        return Err("语音分片不能为空。".to_string());
    }

    session.received_audio_chunks += 1;

    let send_result = match session.provider {
        VoiceProviderKind::Auto => Ok(()),
        VoiceProviderKind::Mock | VoiceProviderKind::Tencent => {
            let Some(provider_session) = session.provider_session.as_mut() else {
                return Err("ASR Provider 尚未连接。".to_string());
            };

            let _sequence = chunk.sequence;
            provider_session.send_audio_chunk(chunk.data)
        }
    };

    if let Err(error) = send_result {
        let failed_session_id = session.session_id.clone();
        session.cancel_signal.store(true, Ordering::Relaxed);
        *active_session = None;
        drop(active_session);
        emit_error(
            &app,
            Some(failed_session_id.clone()),
            error.clone(),
            Some("asr_audio_channel_closed"),
        );
        emit_stopped(&app, Some(failed_session_id), VoiceStoppedReason::Error);
        return Err(error);
    }

    Ok(())
}

#[tauri::command]
pub fn get_voice_provider_status() -> VoiceProviderStatus {
    let missing_tencent_env = TencentAsrConfig::missing_required_env();
    let tencent_configured = missing_tencent_env.is_empty();
    let provider_override = provider_override_from_env();

    VoiceProviderStatus {
        auto_provider: resolve_provider(VoiceProviderKind::Auto),
        provider_override,
        tencent_configured,
        missing_tencent_env,
        diagnostics: provider_diagnostics(),
    }
}

#[tauri::command]
pub fn check_tencent_asr_config() -> TencentAsrConfigCheck {
    let missing_env = TencentAsrConfig::missing_required_env();

    if !missing_env.is_empty() {
        return TencentAsrConfigCheck {
            ok: false,
            missing_env,
            host: None,
            app_id: None,
            engine_model_type: None,
            sentence_strategy: None,
            voice_format: None,
            need_vad: None,
            signed_url_preview: None,
            error: Some("腾讯云 ASR 凭证未配置完整。".to_string()),
        };
    }

    match TencentAsrConfig::from_env() {
        Ok(config) => {
            let signed_url_preview = config
                .signed_websocket_url("voice-diagnostic")
                .map(|url| config.redact_signed_url(&url));

            TencentAsrConfigCheck {
                ok: signed_url_preview.is_ok(),
                missing_env,
                host: Some(config.host.clone()),
                app_id: Some(config.app_id.clone()),
                engine_model_type: Some(config.engine_model_type.clone()),
                sentence_strategy: Some(config.sentence_strategy),
                voice_format: Some(config.voice_format),
                need_vad: Some(config.need_vad),
                signed_url_preview: signed_url_preview.ok(),
                error: None,
            }
        }
        Err(error) => TencentAsrConfigCheck {
            ok: false,
            missing_env,
            host: None,
            app_id: None,
            engine_model_type: None,
            sentence_strategy: None,
            voice_format: None,
            need_vad: None,
            signed_url_preview: None,
            error: Some(error),
        },
    }
}

#[tauri::command]
pub fn get_voice_session_snapshot(
    state: tauri::State<'_, VoiceState>,
) -> Result<VoiceSessionSnapshot, String> {
    let active_session = state
        .active_session
        .lock()
        .map_err(|_| "Voice session state is unavailable.".to_string())?;

    let Some(session) = active_session.as_ref() else {
        return Ok(VoiceSessionSnapshot {
            active: false,
            session_id: None,
            provider: None,
            received_audio_chunks: 0,
        });
    };

    Ok(VoiceSessionSnapshot {
        active: true,
        session_id: Some(session.session_id.clone()),
        provider: Some(session.provider),
        received_audio_chunks: session.received_audio_chunks,
    })
}

#[tauri::command]
pub fn stop_voice_session(
    app: AppHandle,
    state: tauri::State<'_, VoiceState>,
) -> Result<(), String> {
    let mut active_session = state
        .active_session
        .lock()
        .map_err(|_| "Voice session state is unavailable.".to_string())?;

    if let Some(session) = active_session.as_mut() {
        match session.provider {
            VoiceProviderKind::Tencent => {
                session.finish_signal.store(true, Ordering::Relaxed);
                if let Some(provider_session) = session.provider_session.as_mut() {
                    provider_session.stop();
                }
            }
            VoiceProviderKind::Auto | VoiceProviderKind::Mock => {
                let Some(session) = active_session.take() else {
                    return Ok(());
                };
                let session_id = session.session_id.clone();
                if let Some(mut provider_session) = session.provider_session {
                    provider_session.stop();
                }
                session.cancel_signal.store(true, Ordering::Relaxed);
                drop(active_session);
                emit_stopped(&app, Some(session_id), VoiceStoppedReason::User);
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub fn cancel_voice_session(
    app: AppHandle,
    state: tauri::State<'_, VoiceState>,
) -> Result<(), String> {
    let mut active_session = state
        .active_session
        .lock()
        .map_err(|_| "Voice session state is unavailable.".to_string())?;

    let Some(session) = active_session.take() else {
        return Ok(());
    };

    let session_id = session.session_id.clone();
    if let Some(mut provider_session) = session.provider_session {
        provider_session.stop();
    }
    session.cancel_signal.store(true, Ordering::Relaxed);
    drop(active_session);
    emit_stopped(&app, Some(session_id), VoiceStoppedReason::User);

    Ok(())
}

fn spawn_mock_provider(app: AppHandle, session_id: String, stop_signal: Arc<AtomicBool>) {
    thread::spawn(move || {
        let script = [
            ("speaker-1", "我想先用语音描述这个需求。", true),
            ("speaker-1", "前端点击麦克风后进入录音模式，", false),
            (
                "speaker-1",
                "前端点击麦克风后进入录音模式，并实时显示转写。",
                true,
            ),
            ("speaker-2", "后端要负责语音服务和资源释放。", true),
        ];

        for (index, (speaker_id, text, is_final)) in script.iter().enumerate() {
            if stop_signal.load(Ordering::Relaxed) {
                emit_stopped(&app, Some(session_id.clone()), VoiceStoppedReason::User);
                return;
            }

            thread::sleep(Duration::from_millis(if *is_final { 850 } else { 600 }));

            if stop_signal.load(Ordering::Relaxed) {
                emit_stopped(&app, Some(session_id.clone()), VoiceStoppedReason::User);
                return;
            }

            let _ = app.emit(
                TRANSCRIPT_EVENT,
                VoiceTranscriptEvent {
                    id: format!("{session_id}-{index}"),
                    session_id: session_id.clone(),
                    speaker_id: Some((*speaker_id).to_string()),
                    text: (*text).to_string(),
                    is_final: *is_final,
                    started_at_ms: Some((index as u32) * 1200),
                    ended_at_ms: Some((index as u32) * 1200 + 900),
                    created_at: now_millis_string(),
                },
            );
        }

        clear_active_session(&app, &session_id);
        emit_stopped(&app, Some(session_id), VoiceStoppedReason::Completed);
    });
}

fn provider_for_kind(provider: VoiceProviderKind) -> Result<Box<dyn AsrProvider + Send>, String> {
    match provider {
        VoiceProviderKind::Auto => {
            Err("auto provider must be resolved before session start".to_string())
        }
        VoiceProviderKind::Mock => Ok(Box::new(MockAsrProvider)),
        VoiceProviderKind::Tencent => Ok(Box::new(TencentAsrProvider)),
    }
}

fn provider_diagnostics() -> Vec<VoiceProviderDiagnostic> {
    vec![
        MockAsrProvider.diagnostic(),
        TencentAsrProvider.diagnostic(),
    ]
}

fn resolve_provider(provider: VoiceProviderKind) -> VoiceProviderKind {
    match provider {
        VoiceProviderKind::Auto => provider_override_from_env().unwrap_or_else(|| {
            if TencentAsrConfig::is_available() {
                VoiceProviderKind::Tencent
            } else {
                VoiceProviderKind::Mock
            }
        }),
        explicit_provider => explicit_provider,
    }
}

fn provider_override_from_env() -> Option<VoiceProviderKind> {
    read_local_env("VOICECODER_ASR_PROVIDER").and_then(|value| parse_provider_override(&value))
}

fn parse_provider_override(value: &str) -> Option<VoiceProviderKind> {
    match value.trim().to_lowercase().as_str() {
        "mock" => Some(VoiceProviderKind::Mock),
        "tencent" => Some(VoiceProviderKind::Tencent),
        "auto" => None,
        _ => None,
    }
}

fn read_local_env(key: &str) -> Option<String> {
    if let Ok(value) = env::var(key) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }

    for env_path in candidate_env_files() {
        let Ok(content) = fs::read_to_string(env_path) else {
            continue;
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let Some((line_key, line_value)) = trimmed.split_once('=') else {
                continue;
            };

            if line_key.trim() == key {
                return Some(clean_env_value(line_value));
            }
        }
    }

    None
}

fn candidate_env_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(current_dir) = env::current_dir() {
        paths.push(current_dir.join(".env"));
        if let Some(parent) = current_dir.parent() {
            paths.push(parent.join(".env"));
        }
    }

    paths
}

fn clean_env_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn clear_active_session(app: &AppHandle, session_id: &str) {
    let state = app.state::<VoiceState>();
    let Ok(mut active_session) = state.active_session.lock() else {
        return;
    };

    if active_session
        .as_ref()
        .map(|session| session.session_id.as_str())
        == Some(session_id)
    {
        *active_session = None;
    }
}

fn emit_stopped(app: &AppHandle, session_id: Option<String>, reason: VoiceStoppedReason) {
    if let Some(id) = session_id.as_deref() {
        clear_active_session(app, id);
    }

    let _ = app.emit(
        STOPPED_EVENT,
        VoiceStoppedEvent {
            session_id,
            reason,
            stopped_at: now_millis_string(),
        },
    );
}

fn emit_error(app: &AppHandle, session_id: Option<String>, message: String, code: Option<&str>) {
    let _ = app.emit(
        ERROR_EVENT,
        VoiceErrorEvent {
            session_id,
            message,
            code: code.map(str::to_string),
        },
    );
}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to start VoiceCoder async runtime"))
}

fn create_session_id() -> String {
    format!("voice-{}", now_millis())
}

fn now_millis_string() -> String {
    now_millis().to_string()
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_status_reports_missing_tencent_credentials() {
        let status = get_voice_provider_status();

        if status.provider_override == Some(VoiceProviderKind::Mock) {
            assert_eq!(status.auto_provider, VoiceProviderKind::Mock);
        } else if status.provider_override == Some(VoiceProviderKind::Tencent) {
            assert_eq!(status.auto_provider, VoiceProviderKind::Tencent);
        } else if status.tencent_configured {
            assert_eq!(status.auto_provider, VoiceProviderKind::Tencent);
            assert!(status.missing_tencent_env.is_empty());
        } else {
            assert_eq!(status.auto_provider, VoiceProviderKind::Mock);
            assert!(!status.missing_tencent_env.is_empty());
            assert!(status
                .missing_tencent_env
                .iter()
                .all(|key| !key.contains("SECRET_KEY_VALUE")));
        }
    }

    #[test]
    fn provider_override_parser_accepts_known_values() {
        assert_eq!(
            parse_provider_override("mock"),
            Some(VoiceProviderKind::Mock)
        );
        assert_eq!(
            parse_provider_override(" Tencent "),
            Some(VoiceProviderKind::Tencent)
        );
        assert_eq!(parse_provider_override("auto"), None);
        assert_eq!(parse_provider_override("unknown"), None);
    }

    #[test]
    fn voice_state_defaults_to_inactive_session() {
        let state = VoiceState::default();
        let active_session = state
            .active_session
            .lock()
            .expect("voice state lock should be available");

        assert!(active_session.is_none());
    }
}
