use super::{
    emit_error, emit_stopped, now_millis, now_millis_string, read_local_env, runtime, AsrProvider,
    AsrSession, AsrStartContext, VoiceProviderDiagnostic, VoiceProviderKind, VoiceStoppedReason,
    VoiceTranscriptEvent, TRANSCRIPT_EVENT,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::Local;
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
use tauri::{AppHandle, Emitter};
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
    api_key: String,
    api_secret: String,
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
    res_type: Option<String>,
    sid: Option<String>,
}

#[derive(Default)]
struct IflytekSpeakerState {
    current_speaker_id: Option<String>,
}

#[derive(Debug, PartialEq)]
struct IflytekParsedTranscript {
    id_key: String,
    speaker_id: Option<String>,
    text: String,
    is_final: bool,
    started_at_ms: Option<u32>,
    ended_at_ms: Option<u32>,
}

struct IflytekTranscriptBuilder {
    group_index: usize,
    segment_key: String,
    text: String,
    speaker_id: Option<String>,
    is_final: bool,
    started_at_ms: Option<u32>,
    ended_at_ms: Option<u32>,
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
                details.insert("apiKey".to_string(), config.api_key);
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
            api_key: required_env_with_legacy("IFLYTEK_LLM_API_KEY", "IFLYTEK_LLM_ACCESS_KEY_ID")?,
            api_secret: required_env_with_legacy(
                "IFLYTEK_LLM_API_SECRET",
                "IFLYTEK_LLM_ACCESS_KEY_SECRET",
            )?,
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
        let mut missing_env = Vec::new();
        if required_env("IFLYTEK_LLM_APP_ID").is_err() {
            missing_env.push("IFLYTEK_LLM_APP_ID".to_string());
        }
        if required_env_with_legacy("IFLYTEK_LLM_API_KEY", "IFLYTEK_LLM_ACCESS_KEY_ID").is_err() {
            missing_env.push("IFLYTEK_LLM_API_KEY".to_string());
        }
        if required_env_with_legacy("IFLYTEK_LLM_API_SECRET", "IFLYTEK_LLM_ACCESS_KEY_SECRET")
            .is_err()
        {
            missing_env.push("IFLYTEK_LLM_API_SECRET".to_string());
        }
        missing_env
    }

    fn signed_websocket_url(&self, request_id: &str) -> Result<String, String> {
        self.signed_websocket_url_with_utc(request_id, &current_iflytek_utc())
    }

    fn signed_websocket_url_with_utc(&self, request_id: &str, utc: &str) -> Result<String, String> {
        let mut params = BTreeMap::new();
        params.insert("accessKeyId", self.api_key.clone());
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
        let signature = sign_hmac_sha1_base64(&self.api_secret, &base_string)?;
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
                let redacted_url = config.redact_signed_url(&connection_url);
                let message = websocket
                    .err()
                    .map(|error| {
                        eprintln!(
                            "[voice][iflytek_llm] connect failed url={} error={}",
                            redacted_url, error
                        );
                        format_iflytek_connect_error(&error.to_string())
                    })
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
                let mut speaker_state = IflytekSpeakerState::default();

                while let Some(message) = reader.next().await {
                    if reader_cancel_signal.load(Ordering::Relaxed) {
                        return;
                    }

                    match message {
                        Ok(Message::Text(text)) => {
                            match handle_iflytek_message(
                                &reader_app,
                                &reader_session_id,
                                &mut speaker_state,
                                &text,
                            ) {
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

fn format_iflytek_connect_error(error: &str) -> String {
    if error.contains("invalid response status") {
        return format!(
            "讯飞大模型 ASR WebSocket 连接失败：{error}。握手响应不是标准 HTTP 状态行，常见原因是代理/TUN/DNS/网关把请求转到了错误服务，或当前网络无法直连讯飞 endpoint。"
        );
    }

    format!("讯飞大模型 ASR WebSocket 连接失败：{error}")
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
    speaker_state: &mut IflytekSpeakerState,
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

    if let Some(error_message) = iflytek_response_error_message(&response) {
        emit_error(
            app,
            Some(session_id.to_string()),
            error_message,
            Some("iflytek_api_error"),
        );
        return IflytekMessageAction::Stop(VoiceStoppedReason::Error);
    }

    for transcript in parse_iflytek_transcript_events(response.data.as_ref(), speaker_state) {
        emit_iflytek_transcript(app, session_id, transcript);
    }

    if iflytek_data_is_final(response.data.as_ref()) {
        return IflytekMessageAction::Stop(VoiceStoppedReason::Completed);
    }

    IflytekMessageAction::Continue
}

fn format_iflytek_error(response: &IflytekRealtimeResponse) -> String {
    let data = normalized_iflytek_data(response.data.as_ref());
    let code = response
        .code
        .as_deref()
        .map(str::to_string)
        .or_else(|| {
            data.as_ref()
                .and_then(|data| value_to_string(data.get("code")))
        })
        .unwrap_or_else(|| "未知错误码".to_string());
    let desc = response
        .desc
        .clone()
        .or_else(|| {
            data.as_ref()
                .and_then(|data| value_to_string(data.get("desc")))
        })
        .or_else(|| {
            data.as_ref()
                .and_then(|data| value_to_string(data.get("message")))
        })
        .unwrap_or_else(|| "讯飞大模型 ASR 返回错误。".to_string());
    let sid = response.sid.as_deref().unwrap_or("-");
    format!("讯飞大模型 ASR 返回错误码 {code}：{desc}（sid: {sid}）")
}

fn iflytek_response_error_message(response: &IflytekRealtimeResponse) -> Option<String> {
    if response.action.as_deref() == Some("error") {
        return Some(format_iflytek_error(response));
    }

    if response.code.as_deref().is_some_and(|code| code != "0") {
        return Some(format_iflytek_error(response));
    }

    let data = normalized_iflytek_data(response.data.as_ref());
    let data_is_abnormal = data
        .as_ref()
        .and_then(|data| data.get("normal"))
        .and_then(Value::as_bool)
        == Some(false);
    let is_failure_result = response.res_type.as_deref() == Some("frc");

    if data_is_abnormal || is_failure_result {
        return Some(format_iflytek_error(response));
    }

    None
}

fn parse_iflytek_transcript_events(
    data: Option<&Value>,
    speaker_state: &mut IflytekSpeakerState,
) -> Vec<IflytekParsedTranscript> {
    let Some(data) = normalized_iflytek_data(data) else {
        return Vec::new();
    };

    let Some(st) = data.pointer("/cn/st") else {
        return Vec::new();
    };

    let segment_key = value_to_string(data.get("seg_id"))
        .or_else(|| value_to_string(st.get("seg_id")))
        .unwrap_or_else(|| now_millis().to_string());
    let segment_started_at_ms = value_to_u32(st.get("bg"));
    let segment_ended_at_ms = value_to_u32(st.get("ed"));
    let is_final = value_to_string(st.get("type")).as_deref() == Some("0");
    log_iflytek_speaker_diagnostics(&data, st);
    let mut transcripts = Vec::new();
    let mut builder = IflytekTranscriptBuilder::new(
        segment_key,
        speaker_state.current_speaker_id.clone(),
        is_final,
        segment_started_at_ms,
        segment_ended_at_ms,
    );

    let Some(rt_items) = st.get("rt").and_then(Value::as_array) else {
        return Vec::new();
    };

    for rt_item in rt_items {
        let Some(ws_items) = rt_item.get("ws").and_then(Value::as_array) else {
            continue;
        };

        for ws_item in ws_items {
            let Some(cw_items) = ws_item.get("cw").and_then(Value::as_array) else {
                continue;
            };

            for cw_item in cw_items {
                let next_speaker_id = cw_item
                    .get("rl")
                    .and_then(normalized_iflytek_speaker_id)
                    .map(|speaker_id| {
                        speaker_state.current_speaker_id = Some(speaker_id.clone());
                        speaker_id
                    })
                    .or_else(|| speaker_state.current_speaker_id.clone());

                if builder.should_split_for_speaker(next_speaker_id.as_ref()) {
                    let segment_key = builder.segment_key.clone();
                    if let Some(transcript) = builder.finish() {
                        transcripts.push(transcript);
                    }
                    let group_index = transcripts.len();
                    builder = IflytekTranscriptBuilder::new(
                        segment_key,
                        next_speaker_id.clone(),
                        is_final,
                        segment_started_at_ms,
                        segment_ended_at_ms,
                    );
                    builder.group_index = group_index;
                } else {
                    builder.speaker_id = next_speaker_id.clone();
                }

                let word_kind = value_to_string(cw_item.get("wp"));
                if word_kind.as_deref() == Some("g") {
                    continue;
                }

                let Some(word) = value_to_string(cw_item.get("w")) else {
                    continue;
                };
                if word.trim().is_empty() {
                    continue;
                }

                builder.push_word(
                    &word,
                    value_to_u32(cw_item.get("wb")),
                    value_to_u32(cw_item.get("we")),
                );
            }
        }
    }

    if let Some(transcript) = builder.finish() {
        transcripts.push(transcript);
    }

    transcripts
}

fn log_iflytek_speaker_diagnostics(data: &Value, st: &Value) {
    let rt_count = st
        .get("rt")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    eprintln!(
        "[voice][iflytek_llm] st seg_id={} type={} bg={} ed={} keys={:?} rt_count={}",
        value_to_string(data.get("seg_id"))
            .or_else(|| value_to_string(st.get("seg_id")))
            .unwrap_or_else(|| "-".to_string()),
        value_to_string(st.get("type")).unwrap_or_else(|| "-".to_string()),
        value_to_string(st.get("bg")).unwrap_or_else(|| "-".to_string()),
        value_to_string(st.get("ed")).unwrap_or_else(|| "-".to_string()),
        value_object_keys(st),
        rt_count
    );

    let Some(rt_items) = st.get("rt").and_then(Value::as_array) else {
        return;
    };

    for (rt_index, rt_item) in rt_items.iter().take(4).enumerate() {
        let Some(ws_items) = rt_item.get("ws").and_then(Value::as_array) else {
            eprintln!(
                "[voice][iflytek_llm] rt index={} keys={:?} ws_count=0",
                rt_index,
                value_object_keys(rt_item)
            );
            continue;
        };
        eprintln!(
            "[voice][iflytek_llm] rt index={} keys={:?} ws_count={}",
            rt_index,
            value_object_keys(rt_item),
            ws_items.len()
        );

        for (ws_index, ws_item) in ws_items.iter().take(8).enumerate() {
            let Some(cw_items) = ws_item.get("cw").and_then(Value::as_array) else {
                eprintln!(
                    "[voice][iflytek_llm] ws index={}.{} keys={:?} cw_count=0",
                    rt_index,
                    ws_index,
                    value_object_keys(ws_item)
                );
                continue;
            };

            for (cw_index, cw_item) in cw_items.iter().take(3).enumerate() {
                let normalized = cw_item
                    .get("rl")
                    .and_then(normalized_iflytek_speaker_id)
                    .unwrap_or_else(|| "<inherit-or-none>".to_string());
                eprintln!(
                    "[voice][iflytek_llm] cw index={}.{}.{} keys={:?} wp={} wb={} we={} rl={} normalized={}",
                    rt_index,
                    ws_index,
                    cw_index,
                    value_object_keys(cw_item),
                    summarized_value(cw_item.get("wp")),
                    summarized_value(cw_item.get("wb")),
                    summarized_value(cw_item.get("we")),
                    summarized_value(cw_item.get("rl")),
                    normalized
                );
            }
        }
    }
}

impl IflytekTranscriptBuilder {
    fn new(
        segment_key: String,
        speaker_id: Option<String>,
        is_final: bool,
        started_at_ms: Option<u32>,
        ended_at_ms: Option<u32>,
    ) -> Self {
        Self {
            group_index: 0,
            segment_key,
            text: String::new(),
            speaker_id,
            is_final,
            started_at_ms,
            ended_at_ms,
        }
    }

    fn should_split_for_speaker(&self, next_speaker_id: Option<&String>) -> bool {
        !self.text.is_empty()
            && next_speaker_id.is_some()
            && self.speaker_id.as_ref() != next_speaker_id
    }

    fn push_word(&mut self, word: &str, started_at_ms: Option<u32>, ended_at_ms: Option<u32>) {
        if self.text.is_empty() {
            self.started_at_ms = started_at_ms.or(self.started_at_ms);
        }
        self.ended_at_ms = ended_at_ms.or(self.ended_at_ms);
        self.text.push_str(word);
    }

    fn finish(self) -> Option<IflytekParsedTranscript> {
        let text = self.text.trim().to_string();
        if text.is_empty() {
            return None;
        }

        Some(IflytekParsedTranscript {
            id_key: format!("{}-{}", self.segment_key, self.group_index),
            speaker_id: self.speaker_id,
            text,
            is_final: self.is_final,
            started_at_ms: self.started_at_ms,
            ended_at_ms: self.ended_at_ms,
        })
    }
}

fn emit_iflytek_transcript(app: &AppHandle, session_id: &str, transcript: IflytekParsedTranscript) {
    let _ = app.emit(
        TRANSCRIPT_EVENT,
        VoiceTranscriptEvent {
            id: format!("{session_id}-{}", transcript.id_key),
            session_id: session_id.to_string(),
            speaker_id: transcript.speaker_id,
            text: transcript.text,
            is_final: transcript.is_final,
            started_at_ms: transcript.started_at_ms,
            ended_at_ms: transcript.ended_at_ms,
            created_at: now_millis_string(),
        },
    );
}

fn normalized_iflytek_data(data: Option<&Value>) -> Option<Value> {
    match data? {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        value => Some(value.clone()),
    }
}

fn normalized_iflytek_speaker_id(value: &Value) -> Option<String> {
    let role = value_to_i32(Some(value))?;
    if role <= 0 {
        return None;
    }

    Some(format!("speaker-{role}"))
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn value_object_keys(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    object.keys().cloned().collect()
}

fn summarized_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => {
            let text = text.replace('\n', "\\n");
            if text.chars().count() > 80 {
                let preview = text.chars().take(80).collect::<String>();
                format!("string({}):{}...", text.chars().count(), preview)
            } else {
                format!("string({}):{}", text.chars().count(), text)
            }
        }
        Some(Value::Number(number)) => format!("number:{number}"),
        Some(Value::Bool(value)) => format!("bool:{value}"),
        Some(Value::Array(values)) => format!("array(len={})", values.len()),
        Some(Value::Object(object)) => {
            let keys = object.keys().cloned().collect::<Vec<_>>();
            format!("object(keys={keys:?})")
        }
        Some(Value::Null) => "null".to_string(),
        None => "none".to_string(),
    }
}

fn value_to_i32(value: Option<&Value>) -> Option<i32> {
    match value? {
        Value::Number(number) => number.as_i64().and_then(|value| i32::try_from(value).ok()),
        Value::String(text) => text.trim().parse::<i32>().ok(),
        _ => None,
    }
}

fn value_to_u32(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Number(number) => number.as_u64().and_then(|value| u32::try_from(value).ok()),
        Value::String(text) => text.trim().parse::<u32>().ok(),
        _ => None,
    }
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

fn required_env_with_legacy(key: &str, legacy_key: &str) -> Result<String, String> {
    required_env(key).or_else(|_| required_env(legacy_key))
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
    Local::now().format("%Y-%m-%dT%H:%M:%S%z").to_string()
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
            api_key: "test-api-key".to_string(),
            api_secret: "test-api-secret".to_string(),
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
        assert!(url.contains("accessKeyId=test-api-key"));
        assert!(url.contains("audio_encode=pcm_s16le"));
        assert!(url.contains("feature_ids=feature-a%2Cfeature-b"));
        assert!(url.contains("lang=autodialect"));
        assert!(url.contains("role_type=2"));
        assert!(url.contains("samplerate=16000"));
        assert!(url.contains("utc=2026-06-02T12%3A34%3A56%2B0000"));
        assert!(url.contains("uuid=voice-test"));
        assert!(url.contains("signature="));
        assert!(!url.contains("test-api-secret"));
    }

    #[test]
    fn redacts_iflytek_diagnostic_url() {
        let config = IflytekLlmConfig {
            app_id: "test-app".to_string(),
            api_key: "test-api-key".to_string(),
            api_secret: "test-api-secret".to_string(),
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
        assert!(!preview.contains("test-api-key"));
        assert!(!preview.contains("test-api-secret"));
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
    fn detects_iflytek_llm_failure_result_frame() {
        let response = IflytekRealtimeResponse {
            action: None,
            code: None,
            data: Some(json!({
                "normal": false,
                "code": "35013",
                "desc": "utc time invalid"
            })),
            desc: None,
            res_type: Some("frc".to_string()),
            sid: Some("sid-test".to_string()),
        };

        let message =
            iflytek_response_error_message(&response).expect("failure frame should be an error");

        assert!(message.contains("35013"));
        assert!(message.contains("utc time invalid"));
        assert!(message.contains("sid-test"));
    }

    #[test]
    fn parses_iflytek_transcript_event() {
        let mut speaker_state = IflytekSpeakerState::default();
        let data = json!({
            "seg_id": 7,
            "cn": {
                "st": {
                    "bg": "120",
                    "ed": "880",
                    "type": "0",
                    "rt": [
                        {
                            "ws": [
                                {
                                    "cw": [
                                        { "w": "你好", "wp": "n", "rl": "1", "wb": "120", "we": "420" },
                                        { "w": "。", "wp": "p", "rl": "0", "wb": "420", "we": "480" }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            }
        });

        let transcripts = parse_iflytek_transcript_events(Some(&data), &mut speaker_state);

        assert_eq!(
            transcripts,
            vec![IflytekParsedTranscript {
                id_key: "7-0".to_string(),
                speaker_id: Some("speaker-1".to_string()),
                text: "你好。".to_string(),
                is_final: true,
                started_at_ms: Some(120),
                ended_at_ms: Some(480),
            }]
        );
        assert_eq!(
            speaker_state.current_speaker_id.as_deref(),
            Some("speaker-1")
        );
    }

    #[test]
    fn parses_iflytek_transcript_from_string_data() {
        let mut speaker_state = IflytekSpeakerState::default();
        let data = json!({
            "seg_id": "8",
            "cn": {
                "st": {
                    "type": "1",
                    "rt": [
                        {
                            "ws": [
                                {
                                    "cw": [
                                        { "w": "中间", "wp": "n", "rl": 2 },
                                        { "w": "结果", "wp": "n", "rl": 0 }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            }
        });

        let transcripts = parse_iflytek_transcript_events(
            Some(&Value::String(data.to_string())),
            &mut speaker_state,
        );

        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].id_key, "8-0");
        assert_eq!(transcripts[0].speaker_id.as_deref(), Some("speaker-2"));
        assert_eq!(transcripts[0].text, "中间结果");
        assert!(!transcripts[0].is_final);
    }

    #[test]
    fn carries_iflytek_speaker_for_role_zero() {
        let mut speaker_state = IflytekSpeakerState {
            current_speaker_id: Some("speaker-3".to_string()),
        };
        let data = json!({
            "seg_id": 9,
            "cn": {
                "st": {
                    "type": "0",
                    "rt": [
                        {
                            "ws": [
                                {
                                    "cw": [
                                        { "w": "继续说话", "wp": "n", "rl": "0" }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            }
        });

        let transcripts = parse_iflytek_transcript_events(Some(&data), &mut speaker_state);

        assert_eq!(transcripts.len(), 1);
        assert_eq!(transcripts[0].speaker_id.as_deref(), Some("speaker-3"));
        assert_eq!(transcripts[0].text, "继续说话");
    }

    #[test]
    fn splits_iflytek_transcript_when_speaker_changes() {
        let mut speaker_state = IflytekSpeakerState::default();
        let data = json!({
            "seg_id": 10,
            "cn": {
                "st": {
                    "type": "0",
                    "rt": [
                        {
                            "ws": [
                                {
                                    "cw": [
                                        { "w": "甲说", "wp": "n", "rl": "1" },
                                        { "w": "乙答", "wp": "n", "rl": "2" }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            }
        });

        let transcripts = parse_iflytek_transcript_events(Some(&data), &mut speaker_state);

        assert_eq!(transcripts.len(), 2);
        assert_eq!(transcripts[0].id_key, "10-0");
        assert_eq!(transcripts[0].speaker_id.as_deref(), Some("speaker-1"));
        assert_eq!(transcripts[0].text, "甲说");
        assert_eq!(transcripts[1].id_key, "10-1");
        assert_eq!(transcripts[1].speaker_id.as_deref(), Some("speaker-2"));
        assert_eq!(transcripts[1].text, "乙答");
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
