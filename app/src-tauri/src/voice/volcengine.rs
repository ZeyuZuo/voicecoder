use super::{
    emit_error, emit_stopped, now_millis, now_millis_string, read_local_env, runtime, AsrProvider,
    AsrSession, AsrStartContext, VoiceProviderDiagnostic, VoiceProviderKind, VoiceStoppedReason,
    VoiceTranscriptEvent, TRANSCRIPT_EVENT,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
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
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, handshake::client::Request, Message},
};

const DEFAULT_VOLCENGINE_ASR_ENDPOINT: &str =
    "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async";
const DEFAULT_VOLCENGINE_ASR_RESOURCE_ID: &str = "volc.bigasr.sauc.duration";
const DEFAULT_VOLCENGINE_ASR_LANGUAGE: &str = "zh-CN";
const DEFAULT_VOLCENGINE_ASR_SAMPLE_RATE: u32 = 16_000;
const DEFAULT_VOLCENGINE_ASR_BITS: u8 = 16;
const DEFAULT_VOLCENGINE_ASR_CHANNELS: u8 = 1;
const DEFAULT_VOLCENGINE_ASR_END_WINDOW_SIZE: u32 = 400;
const DEFAULT_VOLCENGINE_ASR_FINAL_TIMEOUT_SECS: u64 = 12;
const VOLCENGINE_ASR_FRAME_INTERVAL_MS: u64 = 200;

const PROTOCOL_VERSION: u8 = 0x1;
const HEADER_SIZE_WORDS: u8 = 0x1;
const MESSAGE_TYPE_CLIENT_FULL_REQUEST: u8 = 0x1;
const MESSAGE_TYPE_CLIENT_AUDIO_ONLY_REQUEST: u8 = 0x2;
const MESSAGE_TYPE_SERVER_FULL_RESPONSE: u8 = 0x9;
const MESSAGE_TYPE_SERVER_ACK: u8 = 0xB;
const MESSAGE_TYPE_SERVER_ERROR: u8 = 0xF;
const MESSAGE_FLAG_SEQUENCE: u8 = 0x1;
const MESSAGE_FLAG_LAST_SEQUENCE: u8 = 0x2;
const SERIALIZATION_JSON: u8 = 0x1;
const COMPRESSION_NONE: u8 = 0x0;

pub(crate) struct VolcengineAsrProvider;

pub(crate) struct VolcengineAsrConfig {
    app_key: String,
    access_key: String,
    endpoint: String,
    resource_id: String,
    language: String,
    enable_nonstream: bool,
    enable_speaker_info: bool,
    enable_accelerate_text: bool,
    ssd_version: String,
    end_window_size: u32,
}

struct VolcengineAsrSession {
    audio_sender: UnboundedSender<Vec<u8>>,
}

#[derive(Debug, PartialEq)]
struct VolcengineParsedTranscript {
    id_key: String,
    speaker_id: Option<String>,
    text: String,
    is_final: bool,
    started_at_ms: Option<u32>,
    ended_at_ms: Option<u32>,
}

#[derive(Debug)]
struct VolcengineProtocolMessage {
    message_type: u8,
    sequence: Option<i32>,
    payload: Option<Value>,
    error: Option<String>,
}

enum VolcengineMessageAction {
    Continue,
    Stop(VoiceStoppedReason),
}

impl AsrSession for VolcengineAsrSession {
    fn send_audio_chunk(&mut self, chunk: Vec<u8>) -> Result<(), String> {
        self.audio_sender
            .send(chunk)
            .map_err(|_| "火山引擎 ASR 音频发送通道已关闭。".to_string())
    }

    fn stop(&mut self) {}
}

impl AsrProvider for VolcengineAsrProvider {
    fn kind(&self) -> VoiceProviderKind {
        VoiceProviderKind::Volcengine
    }

    fn validate_start(&self) -> Result<(), String> {
        VolcengineAsrConfig::from_env().map(|_| ())
    }

    fn diagnostic(&self) -> VoiceProviderDiagnostic {
        let missing_env = VolcengineAsrConfig::missing_required_env();
        if !missing_env.is_empty() {
            return VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: Some(DEFAULT_VOLCENGINE_ASR_ENDPOINT.to_string()),
                details: BTreeMap::new(),
                error: Some("火山引擎 ASR 凭证未配置完整。".to_string()),
            };
        }

        match VolcengineAsrConfig::from_env() {
            Ok(config) => {
                let mut details = BTreeMap::new();
                details.insert("appKey".to_string(), config.app_key.clone());
                details.insert("resourceId".to_string(), config.resource_id.clone());
                details.insert("language".to_string(), config.language.clone());
                details.insert(
                    "enableNonstream".to_string(),
                    config.enable_nonstream.to_string(),
                );
                details.insert(
                    "enableSpeakerInfo".to_string(),
                    config.enable_speaker_info.to_string(),
                );
                details.insert(
                    "enableAccelerateText".to_string(),
                    config.enable_accelerate_text.to_string(),
                );
                details.insert("ssdVersion".to_string(), config.ssd_version.clone());
                details.insert(
                    "endWindowSize".to_string(),
                    config.end_window_size.to_string(),
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
                endpoint: Some(DEFAULT_VOLCENGINE_ASR_ENDPOINT.to_string()),
                details: BTreeMap::new(),
                error: Some(error),
            },
        }
    }

    fn start_session(
        &self,
        context: AsrStartContext,
    ) -> Result<Box<dyn AsrSession + Send>, String> {
        let config = VolcengineAsrConfig::from_env()?;
        let sender = spawn_volcengine_provider(
            context.app,
            context.session_id,
            Arc::clone(&context.cancel_signal),
            Arc::clone(&context.finish_signal),
            config,
        );

        Ok(Box::new(VolcengineAsrSession {
            audio_sender: sender,
        }))
    }
}

impl VolcengineAsrConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        Ok(Self {
            app_key: required_env("VOLCENGINE_ASR_APP_KEY")?,
            access_key: required_env("VOLCENGINE_ASR_ACCESS_KEY")?,
            endpoint: read_local_env("VOLCENGINE_ASR_ENDPOINT")
                .unwrap_or_else(|| DEFAULT_VOLCENGINE_ASR_ENDPOINT.to_string()),
            resource_id: read_local_env("VOLCENGINE_ASR_RESOURCE_ID")
                .unwrap_or_else(|| DEFAULT_VOLCENGINE_ASR_RESOURCE_ID.to_string()),
            language: read_local_env("VOLCENGINE_ASR_LANGUAGE")
                .unwrap_or_else(|| DEFAULT_VOLCENGINE_ASR_LANGUAGE.to_string()),
            enable_nonstream: optional_bool_env("VOLCENGINE_ASR_ENABLE_NONSTREAM", true),
            enable_speaker_info: optional_bool_env("VOLCENGINE_ASR_ENABLE_SPEAKER_INFO", true),
            enable_accelerate_text: optional_bool_env(
                "VOLCENGINE_ASR_ENABLE_ACCELERATE_TEXT",
                true,
            ),
            ssd_version: read_local_env("VOLCENGINE_ASR_SSD_VERSION")
                .unwrap_or_else(|| "200".to_string()),
            end_window_size: read_local_env("VOLCENGINE_ASR_END_WINDOW_SIZE")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(DEFAULT_VOLCENGINE_ASR_END_WINDOW_SIZE),
        })
    }

    pub(crate) fn missing_required_env() -> Vec<String> {
        ["VOLCENGINE_ASR_APP_KEY", "VOLCENGINE_ASR_ACCESS_KEY"]
            .iter()
            .filter(|key| required_env(key).is_err())
            .map(|key| (*key).to_string())
            .collect()
    }

    fn connect_request(&self, connect_id: &str) -> Result<Request, String> {
        let mut request = self
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|error| format!("火山引擎 ASR WebSocket 请求构造失败：{error}"))?;
        let headers = request.headers_mut();
        headers.insert(
            "X-Api-App-Key",
            self.app_key
                .parse()
                .map_err(|_| "火山引擎 ASR App Key 不是合法 Header 值。".to_string())?,
        );
        headers.insert(
            "X-Api-Access-Key",
            self.access_key
                .parse()
                .map_err(|_| "火山引擎 ASR Access Key 不是合法 Header 值。".to_string())?,
        );
        headers.insert(
            "X-Api-Resource-Id",
            self.resource_id
                .parse()
                .map_err(|_| "火山引擎 ASR Resource ID 不是合法 Header 值。".to_string())?,
        );
        headers.insert(
            "X-Api-Connect-Id",
            connect_id
                .parse()
                .map_err(|_| "火山引擎 ASR Connect ID 不是合法 Header 值。".to_string())?,
        );
        Ok(request)
    }

    fn initial_payload(&self, session_id: &str) -> Value {
        json!({
            "user": {
                "uid": session_id
            },
            "audio": {
                "format": "pcm",
                "codec": "raw",
                "rate": DEFAULT_VOLCENGINE_ASR_SAMPLE_RATE,
                "bits": DEFAULT_VOLCENGINE_ASR_BITS,
                "channel": DEFAULT_VOLCENGINE_ASR_CHANNELS
            },
            "request": {
                "model_name": "bigmodel",
                "language": self.language,
                "result_type": "single",
                "show_utterances": true,
                "enable_itn": true,
                "enable_punc": true,
                "enable_ddc": true,
                "enable_nonstream": self.enable_nonstream,
                "enable_speaker_info": self.enable_speaker_info,
                "enable_accelerate_text": self.enable_accelerate_text,
                "ssd_version": self.ssd_version,
                "end_window_size": self.end_window_size
            }
        })
    }
}

fn spawn_volcengine_provider(
    app: AppHandle,
    session_id: String,
    cancel_signal: Arc<AtomicBool>,
    finish_signal: Arc<AtomicBool>,
    config: VolcengineAsrConfig,
) -> UnboundedSender<Vec<u8>> {
    let (audio_sender, mut audio_receiver) = unbounded_channel::<Vec<u8>>();

    thread::spawn(move || {
        runtime().block_on(async move {
            let connect_id = create_volcengine_connect_id(&session_id);
            let request = match config.connect_request(&connect_id) {
                Ok(request) => request,
                Err(error) => {
                    emit_error(
                        &app,
                        Some(session_id.clone()),
                        error,
                        Some("volcengine_request_failed"),
                    );
                    emit_stopped(&app, Some(session_id), VoiceStoppedReason::Error);
                    return;
                }
            };

            eprintln!(
                "[voice][volcengine] connecting {} resource_id={}",
                config.endpoint, config.resource_id
            );

            let websocket = connect_async(request).await;
            let Ok((socket, _response)) = websocket else {
                let message = websocket
                    .err()
                    .map(|error| format!("火山引擎 ASR WebSocket 连接失败：{error}"))
                    .unwrap_or_else(|| "火山引擎 ASR WebSocket 连接失败。".to_string());
                emit_error(
                    &app,
                    Some(session_id.clone()),
                    message,
                    Some("volcengine_connect_failed"),
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
                        Ok(Message::Binary(bytes)) => {
                            match handle_volcengine_message(&reader_app, &reader_session_id, &bytes)
                            {
                                VolcengineMessageAction::Continue => {}
                                VolcengineMessageAction::Stop(reason) => {
                                    let _ = completion_sender.send(reason);
                                    return;
                                }
                            }
                        }
                        Ok(Message::Text(text)) => {
                            match handle_volcengine_text_message(
                                &reader_app,
                                &reader_session_id,
                                &text,
                            ) {
                                VolcengineMessageAction::Continue => {}
                                VolcengineMessageAction::Stop(reason) => {
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
                                format!("火山引擎 ASR 接收失败：{error}"),
                                Some("volcengine_receive_failed"),
                            );
                            let _ = completion_sender.send(VoiceStoppedReason::Error);
                            return;
                        }
                    }
                }

                let _ = completion_sender.send(VoiceStoppedReason::Completed);
            });

            let stop_reason = run_volcengine_send_loop(
                &app,
                &session_id,
                &config,
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

async fn run_volcengine_send_loop<S>(
    app: &AppHandle,
    session_id: &str,
    config: &VolcengineAsrConfig,
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
    let mut sequence = 1;
    let initial_frame =
        encode_volcengine_full_request(sequence, &config.initial_payload(session_id));
    if let Err(error) = writer.send(Message::Binary(initial_frame)).await {
        emit_error(
            app,
            Some(session_id.to_string()),
            format!("火山引擎 ASR 初始化请求发送失败：{error}"),
            Some("volcengine_send_failed"),
        );
        return VoiceStoppedReason::Error;
    }
    sequence += 1;

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

            while !pending_audio.is_empty() {
                let frame_payload = pending_audio.split_off(0);
                let frame = encode_volcengine_audio_request(sequence, &frame_payload, false);
                sequence += 1;
                if let Err(error) = writer.send(Message::Binary(frame)).await {
                    emit_error(
                        app,
                        Some(session_id.to_string()),
                        format!("火山引擎 ASR 音频发送失败：{error}"),
                        Some("volcengine_send_failed"),
                    );
                    return VoiceStoppedReason::Error;
                }
            }

            let final_frame = encode_volcengine_audio_request(-sequence, &[], true);
            if let Err(error) = writer.send(Message::Binary(final_frame)).await {
                emit_error(
                    app,
                    Some(session_id.to_string()),
                    format!("火山引擎 ASR 结束帧发送失败：{error}"),
                    Some("volcengine_finish_failed"),
                );
                return VoiceStoppedReason::Error;
            }

            return match tokio::time::timeout(
                Duration::from_secs(DEFAULT_VOLCENGINE_ASR_FINAL_TIMEOUT_SECS),
                completion_receiver.recv(),
            )
            .await
            {
                Ok(Some(reason)) => reason,
                Ok(None) | Err(_) => VoiceStoppedReason::Completed,
            };
        }

        tokio::select! {
            completion = completion_receiver.recv() => {
                if let Some(reason) = completion {
                    return reason;
                }
            }
            chunk = audio_receiver.recv() => {
                let Some(chunk) = chunk else {
                    return VoiceStoppedReason::User;
                };
                pending_audio.extend(chunk);
            }
            _ = tokio::time::sleep(Duration::from_millis(VOLCENGINE_ASR_FRAME_INTERVAL_MS)), if !pending_audio.is_empty() => {
                let frame_payload = pending_audio.split_off(0);
                let frame = encode_volcengine_audio_request(sequence, &frame_payload, false);
                sequence += 1;
                if let Err(error) = writer.send(Message::Binary(frame)).await {
                    emit_error(
                        app,
                        Some(session_id.to_string()),
                        format!("火山引擎 ASR 音频发送失败：{error}"),
                        Some("volcengine_send_failed"),
                    );
                    return VoiceStoppedReason::Error;
                }
            }
        }
    }
}

fn handle_volcengine_text_message(
    app: &AppHandle,
    session_id: &str,
    raw_message: &str,
) -> VolcengineMessageAction {
    match serde_json::from_str::<Value>(raw_message) {
        Ok(value) => emit_volcengine_payload(app, session_id, None, &value),
        Err(_) => VolcengineMessageAction::Continue,
    }
}

fn handle_volcengine_message(
    app: &AppHandle,
    session_id: &str,
    bytes: &[u8],
) -> VolcengineMessageAction {
    let parsed = match decode_volcengine_message(bytes) {
        Ok(message) => message,
        Err(error) => {
            emit_error(
                app,
                Some(session_id.to_string()),
                error,
                Some("volcengine_decode_failed"),
            );
            return VolcengineMessageAction::Stop(VoiceStoppedReason::Error);
        }
    };

    if parsed.message_type == MESSAGE_TYPE_SERVER_ERROR {
        emit_error(
            app,
            Some(session_id.to_string()),
            parsed
                .error
                .unwrap_or_else(|| "火山引擎 ASR 返回错误。".to_string()),
            Some("volcengine_api_error"),
        );
        return VolcengineMessageAction::Stop(VoiceStoppedReason::Error);
    }

    if let Some(payload) = parsed.payload.as_ref() {
        return emit_volcengine_payload(app, session_id, parsed.sequence, payload);
    }

    if parsed.sequence.is_some_and(|sequence| sequence < 0) {
        return VolcengineMessageAction::Stop(VoiceStoppedReason::Completed);
    }

    VolcengineMessageAction::Continue
}

fn emit_volcengine_payload(
    app: &AppHandle,
    session_id: &str,
    sequence: Option<i32>,
    payload: &Value,
) -> VolcengineMessageAction {
    for transcript in parse_volcengine_transcript_events(payload, sequence) {
        emit_volcengine_transcript(app, session_id, transcript);
    }

    if sequence.is_some_and(|sequence| sequence < 0) || payload_is_final(payload) {
        return VolcengineMessageAction::Stop(VoiceStoppedReason::Completed);
    }

    VolcengineMessageAction::Continue
}

fn parse_volcengine_transcript_events(
    payload: &Value,
    sequence: Option<i32>,
) -> Vec<VolcengineParsedTranscript> {
    let result = normalized_volcengine_result(payload);
    let Some(result) = result.as_ref() else {
        return Vec::new();
    };

    let utterances = result
        .get("utterances")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if !utterances.is_empty() {
        log_volcengine_speaker_diagnostics(result, &utterances, sequence);
        return utterances
            .iter()
            .enumerate()
            .filter_map(|(index, utterance)| parse_volcengine_utterance(utterance, index, sequence))
            .collect();
    }

    let text = value_to_string(result.get("text")).unwrap_or_default();
    if text.trim().is_empty() {
        return Vec::new();
    }

    vec![VolcengineParsedTranscript {
        id_key: sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| now_millis().to_string()),
        speaker_id: normalized_volcengine_speaker_id(result),
        text,
        is_final: value_to_bool(result.get("definite")).unwrap_or(false)
            || sequence.is_some_and(|sequence| sequence < 0),
        started_at_ms: value_to_u32(result.get("start_time")),
        ended_at_ms: value_to_u32(result.get("end_time")),
    }]
}

fn parse_volcengine_utterance(
    utterance: &Value,
    index: usize,
    sequence: Option<i32>,
) -> Option<VolcengineParsedTranscript> {
    let text = value_to_string(utterance.get("text"))?;
    if text.trim().is_empty() {
        return None;
    }

    Some(VolcengineParsedTranscript {
        id_key: format!(
            "{}-{index}",
            sequence
                .map(|sequence| sequence.to_string())
                .unwrap_or_else(|| now_millis().to_string())
        ),
        speaker_id: normalized_volcengine_speaker_id(utterance),
        text,
        is_final: value_to_bool(utterance.get("definite")).unwrap_or(false)
            || sequence.is_some_and(|sequence| sequence < 0),
        started_at_ms: value_to_u32(utterance.get("start_time")),
        ended_at_ms: value_to_u32(utterance.get("end_time")),
    })
}

fn emit_volcengine_transcript(
    app: &AppHandle,
    session_id: &str,
    transcript: VolcengineParsedTranscript,
) {
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

fn encode_volcengine_full_request(sequence: i32, payload: &Value) -> Vec<u8> {
    encode_volcengine_frame(
        MESSAGE_TYPE_CLIENT_FULL_REQUEST,
        MESSAGE_FLAG_SEQUENCE,
        sequence,
        serde_json::to_vec(payload).unwrap_or_default(),
    )
}

fn encode_volcengine_audio_request(sequence: i32, payload: &[u8], is_final: bool) -> Vec<u8> {
    encode_volcengine_frame(
        MESSAGE_TYPE_CLIENT_AUDIO_ONLY_REQUEST,
        if is_final {
            MESSAGE_FLAG_LAST_SEQUENCE
        } else {
            MESSAGE_FLAG_SEQUENCE
        },
        sequence,
        payload.to_vec(),
    )
}

fn encode_volcengine_frame(
    message_type: u8,
    flags: u8,
    sequence: i32,
    payload: Vec<u8>,
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(12 + payload.len());
    frame.push((PROTOCOL_VERSION << 4) | HEADER_SIZE_WORDS);
    frame.push((message_type << 4) | flags);
    frame.push((SERIALIZATION_JSON << 4) | COMPRESSION_NONE);
    frame.push(0);
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    frame
}

fn decode_volcengine_message(bytes: &[u8]) -> Result<VolcengineProtocolMessage, String> {
    if bytes.len() < 4 {
        return Err("火山引擎 ASR 返回了过短的协议帧。".to_string());
    }

    let header_size = ((bytes[0] & 0x0f) as usize) * 4;
    if header_size < 4 || bytes.len() < header_size {
        return Err("火山引擎 ASR 返回了无效的协议头。".to_string());
    }

    let message_type = bytes[1] >> 4;
    let flags = bytes[1] & 0x0f;
    let serialization = bytes[2] >> 4;
    let mut offset = header_size;
    let sequence = if flags == MESSAGE_FLAG_SEQUENCE || flags == MESSAGE_FLAG_LAST_SEQUENCE {
        let sequence = read_i32_be(bytes, &mut offset)?;
        Some(sequence)
    } else {
        None
    };

    let has_payload = matches!(
        message_type,
        MESSAGE_TYPE_SERVER_FULL_RESPONSE | MESSAGE_TYPE_SERVER_ACK | MESSAGE_TYPE_SERVER_ERROR
    ) && bytes.len() >= offset + 4;
    let payload = if has_payload {
        let payload_size = read_u32_be(bytes, &mut offset)? as usize;
        if bytes.len() < offset + payload_size {
            return Err(format!(
                "火山引擎 ASR 协议帧长度不匹配：expected={} actual={}",
                payload_size,
                bytes.len().saturating_sub(offset)
            ));
        }
        let payload_bytes = &bytes[offset..offset + payload_size];
        if payload_bytes.is_empty() {
            None
        } else if serialization == SERIALIZATION_JSON {
            serde_json::from_slice::<Value>(payload_bytes).ok()
        } else {
            std::str::from_utf8(payload_bytes)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
        }
    } else {
        None
    };

    let error = if message_type == MESSAGE_TYPE_SERVER_ERROR {
        payload
            .as_ref()
            .and_then(|payload| {
                value_to_string(payload.get("message"))
                    .or_else(|| value_to_string(payload.get("error")))
                    .or_else(|| value_to_string(payload.get("desc")))
            })
            .or_else(|| Some("火山引擎 ASR 返回错误。".to_string()))
    } else {
        None
    };

    Ok(VolcengineProtocolMessage {
        message_type,
        sequence,
        payload,
        error,
    })
}

fn read_i32_be(bytes: &[u8], offset: &mut usize) -> Result<i32, String> {
    if bytes.len() < *offset + 4 {
        return Err("火山引擎 ASR 协议帧缺少 sequence。".to_string());
    }
    let value = i32::from_be_bytes(
        bytes[*offset..*offset + 4]
            .try_into()
            .map_err(|_| "火山引擎 ASR sequence 解析失败。".to_string())?,
    );
    *offset += 4;
    Ok(value)
}

fn read_u32_be(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    if bytes.len() < *offset + 4 {
        return Err("火山引擎 ASR 协议帧缺少 payload size。".to_string());
    }
    let value = u32::from_be_bytes(
        bytes[*offset..*offset + 4]
            .try_into()
            .map_err(|_| "火山引擎 ASR payload size 解析失败。".to_string())?,
    );
    *offset += 4;
    Ok(value)
}

fn normalized_volcengine_result(payload: &Value) -> Option<Value> {
    if let Some(result) = payload.get("result") {
        return normalize_json_value(result);
    }
    if let Some(data) = payload.get("data") {
        return normalize_json_value(data);
    }
    if payload.get("utterances").is_some() || payload.get("text").is_some() {
        return Some(payload.clone());
    }
    None
}

fn normalize_json_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(text) => serde_json::from_str::<Value>(text).ok(),
        value => Some(value.clone()),
    }
}

fn payload_is_final(payload: &Value) -> bool {
    value_to_bool(payload.get("definite")).unwrap_or(false)
        || value_to_bool(payload.pointer("/result/definite")).unwrap_or(false)
        || value_to_bool(payload.pointer("/data/definite")).unwrap_or(false)
}

fn log_volcengine_speaker_diagnostics(result: &Value, utterances: &[Value], sequence: Option<i32>) {
    eprintln!(
        "[voice][volcengine] result sequence={:?} keys={:?} definite={:?} utterances={}",
        sequence,
        value_object_keys(result),
        result.get("definite"),
        utterances.len()
    );

    for (index, utterance) in utterances.iter().take(4).enumerate() {
        let candidate = find_volcengine_speaker_candidate(utterance);
        let normalized = normalized_volcengine_speaker_id(utterance);
        eprintln!(
            "[voice][volcengine] utterance index={} keys={:?} additions={} speaker_candidate={} normalized={:?}",
            index,
            value_object_keys(utterance),
            summarized_value(utterance.get("additions")),
            summarized_owned_value(candidate.as_ref()),
            normalized
        );
    }
}

fn normalized_volcengine_speaker_id(value: &Value) -> Option<String> {
    let candidate = find_volcengine_speaker_candidate(value)?;
    match candidate {
        Value::Number(number) => number
            .as_i64()
            .filter(|index| *index >= 0)
            .map(|index| format!("speaker-{}", index + 1)),
        Value::String(text) => {
            let label = text.trim();
            if label.is_empty() {
                None
            } else if label.starts_with("speaker-") {
                Some(label.to_string())
            } else if let Ok(index) = label.parse::<i64>() {
                if index < 0 {
                    None
                } else {
                    Some(format!("speaker-{}", index + 1))
                }
            } else {
                Some(format!("speaker-{label}"))
            }
        }
        _ => None,
    }
}

fn find_volcengine_speaker_candidate(value: &Value) -> Option<Value> {
    for key in ["speaker", "speaker_id", "speakerId", "speakerID"] {
        if let Some(candidate) = value.get(key) {
            return Some(candidate.clone());
        }
    }

    for key in [
        "additions",
        "speaker_info",
        "speakerInfo",
        "speaker_result",
        "speakerResult",
    ] {
        let Some(container) = value.get(key) else {
            continue;
        };
        if let Some(candidate) = find_volcengine_speaker_candidate_in_container(container) {
            return Some(candidate);
        }
    }

    None
}

fn find_volcengine_speaker_candidate_in_container(value: &Value) -> Option<Value> {
    if let Some(candidate) = find_volcengine_speaker_candidate(value) {
        return Some(candidate);
    }

    if let Value::String(text) = value {
        if let Ok(parsed) = serde_json::from_str::<Value>(text) {
            return find_volcengine_speaker_candidate(&parsed);
        }
    }

    None
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

fn summarized_owned_value(value: Option<&Value>) -> String {
    summarized_value(value)
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn value_to_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(text) => match text.trim().to_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) => number.as_i64().map(|value| value != 0),
        _ => None,
    }
}

fn value_to_u32(value: Option<&Value>) -> Option<u32> {
    match value? {
        Value::Number(number) => number.as_u64().and_then(|value| u32::try_from(value).ok()),
        Value::String(text) => text.parse::<u32>().ok(),
        _ => None,
    }
}

fn required_env(key: &str) -> Result<String, String> {
    read_local_env(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("缺少本地环境变量 {key}，请先配置火山引擎 ASR 凭证。"))
}

fn optional_bool_env(key: &str, default_value: bool) -> bool {
    read_local_env(key)
        .and_then(|value| match value.trim().to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default_value)
}

fn create_volcengine_connect_id(session_id: &str) -> String {
    format!("{session_id}-{}", now_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_volcengine_final_audio_frame_with_negative_sequence() {
        let frame = encode_volcengine_audio_request(-7, &[], true);

        assert_eq!(frame[0], 0x11);
        assert_eq!(frame[1], 0x22);
        assert_eq!(i32::from_be_bytes(frame[4..8].try_into().unwrap()), -7);
        assert_eq!(u32::from_be_bytes(frame[8..12].try_into().unwrap()), 0);
    }

    #[test]
    fn decodes_volcengine_json_protocol_message() {
        let payload = json!({
            "result": {
                "text": "你好",
                "definite": true
            }
        });
        let frame = encode_volcengine_frame(
            MESSAGE_TYPE_SERVER_FULL_RESPONSE,
            MESSAGE_FLAG_SEQUENCE,
            3,
            serde_json::to_vec(&payload).unwrap(),
        );

        let message = decode_volcengine_message(&frame).unwrap();

        assert_eq!(message.message_type, MESSAGE_TYPE_SERVER_FULL_RESPONSE);
        assert_eq!(message.sequence, Some(3));
        assert_eq!(
            message.payload.unwrap().pointer("/result/text").unwrap(),
            &Value::String("你好".to_string())
        );
    }

    #[test]
    fn parses_volcengine_utterances_with_speakers() {
        let payload = json!({
            "result": {
                "utterances": [
                    {
                        "text": "甲说话",
                        "start_time": 10,
                        "end_time": 200,
                        "definite": true,
                        "additions": {
                            "speaker": "0"
                        }
                    },
                    {
                        "text": "乙回答",
                        "start_time": 210,
                        "end_time": 420,
                        "speaker_id": 1
                    }
                ]
            }
        });

        let transcripts = parse_volcengine_transcript_events(&payload, Some(5));

        assert_eq!(
            transcripts[0],
            VolcengineParsedTranscript {
                id_key: "5-0".to_string(),
                speaker_id: Some("speaker-1".to_string()),
                text: "甲说话".to_string(),
                is_final: true,
                started_at_ms: Some(10),
                ended_at_ms: Some(200),
            }
        );
        assert_eq!(
            transcripts[1],
            VolcengineParsedTranscript {
                id_key: "5-1".to_string(),
                speaker_id: Some("speaker-2".to_string()),
                text: "乙回答".to_string(),
                is_final: false,
                started_at_ms: Some(210),
                ended_at_ms: Some(420),
            }
        );
    }

    #[test]
    fn parses_volcengine_speaker_from_stringified_additions() {
        let payload = json!({
            "result": {
                "utterances": [
                    {
                        "text": "字符串 speaker",
                        "additions": "{\"speaker\":\"0\"}"
                    }
                ]
            }
        });

        let transcripts = parse_volcengine_transcript_events(&payload, Some(1));
        let speaker_id = transcripts
            .first()
            .and_then(|transcript| transcript.speaker_id.as_deref());

        assert_eq!(speaker_id, Some("speaker-1"));
    }

    #[test]
    fn parses_volcengine_speaker_from_nested_info() {
        let payload = json!({
            "result": {
                "utterances": [
                    {
                        "text": "嵌套 speaker",
                        "speaker_info": {
                            "speaker_id": 2
                        }
                    }
                ]
            }
        });

        let transcripts = parse_volcengine_transcript_events(&payload, Some(1));
        let speaker_id = transcripts
            .first()
            .and_then(|transcript| transcript.speaker_id.as_deref());

        assert_eq!(speaker_id, Some("speaker-3"));
    }
}
