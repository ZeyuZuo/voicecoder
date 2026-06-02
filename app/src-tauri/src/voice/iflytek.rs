use super::{
    emit_error, emit_stopped, now_millis, read_local_env, runtime, AsrProvider, AsrSession,
    AsrStartContext, VoiceProviderDiagnostic, VoiceProviderKind, VoiceStoppedReason,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::Sha1;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::AppHandle;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DEFAULT_IFLYTEK_LLM_ENDPOINT: &str =
    "wss://office-api-ast-dx.iflyaisol.com/ast/communicate/v1";
const DEFAULT_IFLYTEK_LLM_ROLE_TYPE: &str = "2";
const DEFAULT_IFLYTEK_LLM_LANG: &str = "autodialect";
const IFLYTEK_LLM_AUDIO_ENCODE: &str = "pcm_s16le";
const IFLYTEK_LLM_SAMPLE_RATE: &str = "16000";
const IFLYTEK_LLM_FRAME_BYTES: usize = 1280;
const IFLYTEK_LLM_FRAME_INTERVAL_MS: u64 = 40;
const IFLYTEK_LLM_FINAL_TIMEOUT_SECS: u64 = 8;
type HmacSha1 = Hmac<Sha1>;
const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub(crate) struct IflytekLlmAsrProvider;

pub(crate) struct IflytekLlmConfig {
    app_id: String,
    access_key_id: String,
    access_key_secret: String,
    endpoint: String,
    lang: String,
    role_type: String,
    feature_ids: Option<String>,
}

struct IflytekLlmAsrSession {
    audio_sender: UnboundedSender<Vec<u8>>,
}

#[derive(Deserialize)]
struct IflytekRealtimeResponse {
    action: Option<String>,
    code: Option<String>,
    data: Option<Value>,
    desc: Option<String>,
    sid: Option<String>,
}

enum IflytekMessageAction {
    Continue,
    Stop(VoiceStoppedReason),
}

impl AsrSession for IflytekLlmAsrSession {
    fn send_audio_chunk(&mut self, chunk: Vec<u8>) -> Result<(), String> {
        self.audio_sender
            .send(chunk)
            .map_err(|_| "讯飞大模型 ASR 音频发送通道已关闭。".to_string())
    }

    fn stop(&mut self) {}
}

impl AsrProvider for IflytekLlmAsrProvider {
    fn kind(&self) -> VoiceProviderKind {
        VoiceProviderKind::IflytekLlm
    }

    fn validate_start(&self) -> Result<(), String> {
        IflytekLlmConfig::from_env().map(|_| ())
    }

    fn diagnostic(&self) -> VoiceProviderDiagnostic {
        let missing_env = IflytekLlmConfig::missing_required_env();
        if !missing_env.is_empty() {
            return VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: Some(DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string()),
                details: BTreeMap::new(),
                error: Some("讯飞大模型 ASR 凭证未配置完整。".to_string()),
            };
        }

        match IflytekLlmConfig::from_env() {
            Ok(config) => {
                let mut details = BTreeMap::new();
                details.insert("appId".to_string(), config.app_id);
                details.insert("accessKeyId".to_string(), config.access_key_id);
                details.insert("lang".to_string(), config.lang);
                details.insert(
                    "audioEncode".to_string(),
                    IFLYTEK_LLM_AUDIO_ENCODE.to_string(),
                );
                details.insert(
                    "sampleRate".to_string(),
                    IFLYTEK_LLM_SAMPLE_RATE.to_string(),
                );
                details.insert("roleType".to_string(), config.role_type);
                details.insert(
                    "featureIdsCount".to_string(),
                    config
                        .feature_ids
                        .as_deref()
                        .map(count_feature_ids)
                        .unwrap_or(0)
                        .to_string(),
                );

                VoiceProviderDiagnostic {
                    provider: self.kind(),
                    configured: true,
                    missing_env,
                    endpoint: Some(config.endpoint),
                    details,
                    error: None,
                }
            }
            Err(error) => VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: Some(DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string()),
                details: BTreeMap::new(),
                error: Some(error),
            },
        }
    }

    fn start_session(
        &self,
        context: AsrStartContext,
    ) -> Result<Box<dyn AsrSession + Send>, String> {
        let config = IflytekLlmConfig::from_env()?;
        let sender = spawn_iflytek_llm_provider(
            context.app,
            context.session_id,
            Arc::clone(&context.cancel_signal),
            Arc::clone(&context.finish_signal),
            config,
        );

        Ok(Box::new(IflytekLlmAsrSession {
            audio_sender: sender,
        }))
    }
}

impl IflytekLlmConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        Ok(Self {
            app_id: required_env("IFLYTEK_LLM_APP_ID")?,
            access_key_id: required_env("IFLYTEK_LLM_ACCESS_KEY_ID")?,
            access_key_secret: required_env("IFLYTEK_LLM_ACCESS_KEY_SECRET")?,
            endpoint: read_local_env("IFLYTEK_LLM_ENDPOINT")
                .unwrap_or_else(|| DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string()),
            lang: read_local_env("IFLYTEK_LLM_LANG")
                .unwrap_or_else(|| DEFAULT_IFLYTEK_LLM_LANG.to_string()),
            role_type: read_local_env("IFLYTEK_LLM_ROLE_TYPE")
                .unwrap_or_else(|| DEFAULT_IFLYTEK_LLM_ROLE_TYPE.to_string()),
            feature_ids: read_local_env("IFLYTEK_LLM_FEATURE_IDS"),
        })
    }

    pub(crate) fn missing_required_env() -> Vec<String> {
        [
            "IFLYTEK_LLM_APP_ID",
            "IFLYTEK_LLM_ACCESS_KEY_ID",
            "IFLYTEK_LLM_ACCESS_KEY_SECRET",
        ]
        .iter()
        .filter(|key| required_env(key).is_err())
        .map(|key| (*key).to_string())
        .collect()
    }

    fn signed_websocket_url(&self, request_id: &str) -> Result<String, String> {
        self.signed_websocket_url_with_utc(request_id, &current_iflytek_utc())
    }

    fn signed_websocket_url_with_utc(&self, request_id: &str, utc: &str) -> Result<String, String> {
        let mut params = BTreeMap::new();
        params.insert("accessKeyId", self.access_key_id.clone());
        params.insert("appId", self.app_id.clone());
        params.insert("audio_encode", IFLYTEK_LLM_AUDIO_ENCODE.to_string());
        if let Some(feature_ids) = self
            .feature_ids
            .as_ref()
            .filter(|feature_ids| !feature_ids.trim().is_empty())
        {
            params.insert("feature_ids", feature_ids.clone());
        }
        params.insert("lang", self.lang.clone());
        params.insert("role_type", self.role_type.clone());
        params.insert("samplerate", IFLYTEK_LLM_SAMPLE_RATE.to_string());
        params.insert("utc", utc.to_string());
        params.insert("uuid", request_id.to_string());

        let base_string = encode_query_params(&params);
        let signature = sign_hmac_sha1_base64(&self.access_key_secret, &base_string)?;
        let query = params
            .iter()
            .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
            .chain([format!("signature={}", encode(&signature))])
            .collect::<Vec<_>>()
            .join("&");

        Ok(format!("{}?{}", self.endpoint, query))
    }

    fn redact_signed_url(&self, signed_url: &str) -> String {
        signed_url
            .split_once('?')
            .map(|(base, query)| {
                let preview_query = query
                    .split('&')
                    .map(|pair| {
                        if pair.starts_with("accessKeyId=") {
                            "accessKeyId=<redacted>".to_string()
                        } else if pair.starts_with("signature=") {
                            "signature=<redacted>".to_string()
                        } else {
                            pair.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("&");
                format!("{base}?{preview_query}")
            })
            .unwrap_or_else(|| signed_url.to_string())
    }
}

fn spawn_iflytek_llm_provider(
    app: AppHandle,
    session_id: String,
    cancel_signal: Arc<AtomicBool>,
    finish_signal: Arc<AtomicBool>,
    config: IflytekLlmConfig,
) -> UnboundedSender<Vec<u8>> {
    let (audio_sender, mut audio_receiver) = unbounded_channel::<Vec<u8>>();

    thread::spawn(move || {
        runtime().block_on(async move {
            let request_id = create_iflytek_request_id(&session_id);
            let connection_url = match config.signed_websocket_url(&request_id) {
                Ok(url) => url,
                Err(error) => {
                    emit_error(
                        &app,
                        Some(session_id.clone()),
                        error,
                        Some("iflytek_sign_failed"),
                    );
                    emit_stopped(&app, Some(session_id), VoiceStoppedReason::Error);
                    return;
                }
            };

            eprintln!(
                "[voice][iflytek_llm] connecting {}",
                config.redact_signed_url(&connection_url)
            );

            let websocket = connect_async(&connection_url).await;
            let Ok((socket, _response)) = websocket else {
                let message = websocket
                    .err()
                    .map(|error| format!("讯飞大模型 ASR WebSocket 连接失败：{error}"))
                    .unwrap_or_else(|| "讯飞大模型 ASR WebSocket 连接失败。".to_string());
                emit_error(
                    &app,
                    Some(session_id.clone()),
                    message,
                    Some("iflytek_connect_failed"),
                );
                emit_stopped(&app, Some(session_id), VoiceStoppedReason::Error);
                return;
            };

            let (mut writer, mut reader) = socket.split();
            let (completion_sender, mut completion_receiver) =
                unbounded_channel::<VoiceStoppedReason>();
            let reader_app = app.clone();
            let reader_session_id = session_id.clone();
            let reader_cancel_signal = Arc::clone(&cancel_signal);

            let reader_task = tokio::spawn(async move {
                while let Some(message) = reader.next().await {
                    if reader_cancel_signal.load(Ordering::Relaxed) {
                        return;
                    }

                    match message {
                        Ok(Message::Text(text)) => {
                            match handle_iflytek_message(&reader_app, &reader_session_id, &text) {
                                IflytekMessageAction::Continue => {}
                                IflytekMessageAction::Stop(reason) => {
                                    let _ = completion_sender.send(reason);
                                    return;
                                }
                            }
                        }
                        Ok(Message::Close(_)) => {
                            let _ = completion_sender.send(VoiceStoppedReason::Completed);
                            return;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            emit_error(
                                &reader_app,
                                Some(reader_session_id.clone()),
                                format!("讯飞大模型 ASR 接收失败：{error}"),
                                Some("iflytek_receive_failed"),
                            );
                            let _ = completion_sender.send(VoiceStoppedReason::Error);
                            return;
                        }
                    }
                }

                let _ = completion_sender.send(VoiceStoppedReason::Completed);
            });

            let stop_reason = run_iflytek_send_loop(
                &app,
                &session_id,
                &request_id,
                &cancel_signal,
                &finish_signal,
                &mut writer,
                &mut audio_receiver,
                &mut completion_receiver,
            )
            .await;

            reader_task.abort();
            emit_stopped(&app, Some(session_id), stop_reason);
        });
    });

    audio_sender
}

async fn run_iflytek_send_loop<S>(
    app: &AppHandle,
    session_id: &str,
    request_id: &str,
    cancel_signal: &Arc<AtomicBool>,
    finish_signal: &Arc<AtomicBool>,
    writer: &mut S,
    audio_receiver: &mut UnboundedReceiver<Vec<u8>>,
    completion_receiver: &mut UnboundedReceiver<VoiceStoppedReason>,
) -> VoiceStoppedReason
where
    S: SinkExt<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    let mut pending_audio = Vec::new();

    loop {
        if cancel_signal.load(Ordering::Relaxed) {
            let _ = writer.close().await;
            return VoiceStoppedReason::User;
        }

        if finish_signal.load(Ordering::Relaxed) {
            while let Ok(chunk) = audio_receiver.try_recv() {
                pending_audio.extend(chunk);
            }

            if let Err(error) =
                send_pending_iflytek_audio(writer, &mut pending_audio, true, cancel_signal).await
            {
                emit_error(
                    app,
                    Some(session_id.to_string()),
                    error,
                    Some("iflytek_send_failed"),
                );
                let _ = writer.close().await;
                return VoiceStoppedReason::Error;
            }

            return match wait_for_iflytek_final(writer, request_id, completion_receiver).await {
                Ok(reason) => reason,
                Err(error) => {
                    emit_error(
                        app,
                        Some(session_id.to_string()),
                        error,
                        Some("iflytek_finish_failed"),
                    );
                    VoiceStoppedReason::Error
                }
            };
        }

        tokio::select! {
            completion = completion_receiver.recv() => {
                return completion.unwrap_or(VoiceStoppedReason::Completed);
            }
            chunk = audio_receiver.recv() => {
                let Some(chunk) = chunk else {
                    return VoiceStoppedReason::User;
                };
                pending_audio.extend(chunk);
            }
            _ = tokio::time::sleep(Duration::from_millis(IFLYTEK_LLM_FRAME_INTERVAL_MS)), if pending_audio.len() >= IFLYTEK_LLM_FRAME_BYTES => {
                if let Err(error) = send_pending_iflytek_audio(writer, &mut pending_audio, false, cancel_signal).await {
                    emit_error(
                        app,
                        Some(session_id.to_string()),
                        error,
                        Some("iflytek_send_failed"),
                    );
                    return VoiceStoppedReason::Error;
                }
            }
        }
    }
}

async fn send_pending_iflytek_audio<S>(
    writer: &mut S,
    pending_audio: &mut Vec<u8>,
    flush_tail: bool,
    cancel_signal: &Arc<AtomicBool>,
) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    while pending_audio.len() >= IFLYTEK_LLM_FRAME_BYTES
        || (flush_tail && !pending_audio.is_empty())
    {
        if cancel_signal.load(Ordering::Relaxed) {
            return Ok(());
        }

        let frame_size = if pending_audio.len() >= IFLYTEK_LLM_FRAME_BYTES {
            IFLYTEK_LLM_FRAME_BYTES
        } else {
            pending_audio.len()
        };
        let frame = pending_audio.drain(..frame_size).collect::<Vec<_>>();
        writer
            .send(Message::Binary(frame))
            .await
            .map_err(|error| format!("讯飞大模型 ASR 音频发送失败：{error}"))?;

        tokio::time::sleep(Duration::from_millis(IFLYTEK_LLM_FRAME_INTERVAL_MS)).await;
    }

    Ok(())
}

async fn wait_for_iflytek_final<S>(
    writer: &mut S,
    request_id: &str,
    completion_receiver: &mut UnboundedReceiver<VoiceStoppedReason>,
) -> Result<VoiceStoppedReason, String>
where
    S: SinkExt<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    writer
        .send(Message::Text(
            json!({ "end": true, "sessionId": request_id }).to_string(),
        ))
        .await
        .map_err(|error| format!("讯飞大模型 ASR 结束包发送失败：{error}"))?;

    let stop_reason = match tokio::time::timeout(
        Duration::from_secs(IFLYTEK_LLM_FINAL_TIMEOUT_SECS),
        completion_receiver.recv(),
    )
    .await
    {
        Ok(Some(reason)) => Ok(reason),
        Ok(None) => Ok(VoiceStoppedReason::Completed),
        Err(_) => Ok(VoiceStoppedReason::Completed),
    };

    let _ = writer.close().await;
    stop_reason
}

fn handle_iflytek_message(
    app: &AppHandle,
    session_id: &str,
    raw_message: &str,
) -> IflytekMessageAction {
    let parsed = serde_json::from_str::<IflytekRealtimeResponse>(raw_message);
    let Ok(response) = parsed else {
        emit_error(
            app,
            Some(session_id.to_string()),
            "讯飞大模型 ASR 返回了无法解析的消息。".to_string(),
            Some("iflytek_parse_failed"),
        );
        return IflytekMessageAction::Stop(VoiceStoppedReason::Error);
    };

    if response.action.as_deref() == Some("error") {
        emit_error(
            app,
            Some(session_id.to_string()),
            format_iflytek_error(&response),
            Some("iflytek_api_error"),
        );
        return IflytekMessageAction::Stop(VoiceStoppedReason::Error);
    }

    if response.code.as_deref().is_some_and(|code| code != "0") {
        emit_error(
            app,
            Some(session_id.to_string()),
            format_iflytek_error(&response),
            Some("iflytek_api_error"),
        );
        return IflytekMessageAction::Stop(VoiceStoppedReason::Error);
    }

    if iflytek_data_is_final(response.data.as_ref()) {
        return IflytekMessageAction::Stop(VoiceStoppedReason::Completed);
    }

    IflytekMessageAction::Continue
}

fn format_iflytek_error(response: &IflytekRealtimeResponse) -> String {
    let code = response.code.as_deref().unwrap_or("未知错误码");
    let desc = response
        .desc
        .as_deref()
        .unwrap_or("讯飞大模型 ASR 返回错误。");
    let sid = response.sid.as_deref().unwrap_or("-");
    format!("讯飞大模型 ASR 返回错误码 {code}：{desc}（sid: {sid}）")
}

fn iflytek_data_is_final(data: Option<&Value>) -> bool {
    let Some(data) = data else {
        return false;
    };

    match data {
        Value::Object(map) => {
            if map.get("ls").and_then(Value::as_bool) == Some(true) {
                return true;
            }

            map.values().any(|value| iflytek_data_is_final(Some(value)))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| iflytek_data_is_final(Some(value))),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .as_ref()
            .is_some_and(|value| iflytek_data_is_final(Some(value))),
        _ => false,
    }
}

fn required_env(key: &str) -> Result<String, String> {
    read_local_env(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("缺少本地环境变量 {key}，请先配置讯飞大模型 ASR 凭证。"))
}

fn count_feature_ids(value: &str) -> usize {
    value
        .split(',')
        .filter(|feature_id| !feature_id.trim().is_empty())
        .count()
}

fn encode_query_params(params: &BTreeMap<&str, String>) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn sign_hmac_sha1_base64(secret_key: &str, source: &str) -> Result<String, String> {
    let mut mac = HmacSha1::new_from_slice(secret_key.as_bytes())
        .map_err(|_| "讯飞大模型 ASR 签名初始化失败。".to_string())?;
    mac.update(source.as_bytes());
    Ok(general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_ENCODE_SET).to_string()
}

fn current_iflytek_utc() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%S+0000").to_string()
}

fn create_iflytek_request_id(session_id: &str) -> String {
    format!("{session_id}-{}", now_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_configured_feature_ids() {
        assert_eq!(count_feature_ids(""), 0);
        assert_eq!(count_feature_ids("feature-a"), 1);
        assert_eq!(count_feature_ids("feature-a, feature-b,,feature-c"), 3);
    }

    #[test]
    fn signs_iflytek_websocket_url_without_exposing_secret() {
        let config = IflytekLlmConfig {
            app_id: "test-app".to_string(),
            access_key_id: "test-access-key-id".to_string(),
            access_key_secret: "test-access-key-secret".to_string(),
            endpoint: DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string(),
            lang: "autodialect".to_string(),
            role_type: "2".to_string(),
            feature_ids: Some("feature-a,feature-b".to_string()),
        };

        let url = config
            .signed_websocket_url_with_utc("voice-test", "2026-06-02T12:34:56+0000")
            .expect("signed URL should be generated");

        assert!(url.starts_with(DEFAULT_IFLYTEK_LLM_ENDPOINT));
        assert!(url.contains("appId=test-app"));
        assert!(url.contains("accessKeyId=test-access-key-id"));
        assert!(url.contains("audio_encode=pcm_s16le"));
        assert!(url.contains("feature_ids=feature-a%2Cfeature-b"));
        assert!(url.contains("lang=autodialect"));
        assert!(url.contains("role_type=2"));
        assert!(url.contains("samplerate=16000"));
        assert!(url.contains("utc=2026-06-02T12%3A34%3A56%2B0000"));
        assert!(url.contains("uuid=voice-test"));
        assert!(url.contains("signature="));
        assert!(!url.contains("test-access-key-secret"));
    }

    #[test]
    fn redacts_iflytek_diagnostic_url() {
        let config = IflytekLlmConfig {
            app_id: "test-app".to_string(),
            access_key_id: "test-access-key-id".to_string(),
            access_key_secret: "test-access-key-secret".to_string(),
            endpoint: DEFAULT_IFLYTEK_LLM_ENDPOINT.to_string(),
            lang: "autodialect".to_string(),
            role_type: "2".to_string(),
            feature_ids: None,
        };
        let url = config
            .signed_websocket_url_with_utc("voice-test", "2026-06-02T12:34:56+0000")
            .expect("signed URL should be generated");
        let preview = config.redact_signed_url(&url);

        assert!(preview.contains("accessKeyId=<redacted>"));
        assert!(preview.contains("signature=<redacted>"));
        assert!(!preview.contains("test-access-key-id"));
        assert!(!preview.contains("test-access-key-secret"));
    }

    #[test]
    fn detects_iflytek_final_frame() {
        let data = json!({
            "cn": {
                "st": {
                    "type": "0"
                }
            },
            "ls": true
        });

        assert!(iflytek_data_is_final(Some(&data)));
        assert!(iflytek_data_is_final(Some(&Value::String(
            data.to_string()
        ))));
        assert!(!iflytek_data_is_final(Some(&json!({ "ls": false }))));
    }

    #[test]
    fn encodes_iflytek_signature_reserved_characters() {
        assert_eq!(
            encode("2026-06-02T12:34:56+0000"),
            "2026-06-02T12%3A34%3A56%2B0000"
        );
        assert_eq!(encode("feature-a,feature-b"), "feature-a%2Cfeature-b");
    }
}
