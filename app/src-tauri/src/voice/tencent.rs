use super::{
    emit_error, emit_stopped, now_millis, now_millis_string, now_seconds, read_local_env, runtime,
    AsrProvider, AsrSession, AsrStartContext, VoiceProviderDiagnostic, VoiceProviderKind,
    VoiceStoppedReason, VoiceTranscriptEvent, TRANSCRIPT_EVENT,
};
use base64::{engine::general_purpose, Engine as _};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde::{de, Deserialize, Deserializer};
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

type HmacSha1 = Hmac<Sha1>;
const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

pub(crate) struct TencentAsrProvider;

pub(crate) struct TencentAsrConfig {
    pub(crate) app_id: String,
    secret_id: String,
    secret_key: String,
    pub(crate) engine_model_type: String,
    pub(crate) sentence_strategy: u8,
    pub(crate) voice_format: u8,
    pub(crate) need_vad: u8,
    pub(crate) host: String,
}

struct TencentAsrSession {
    audio_sender: UnboundedSender<Vec<u8>>,
}

#[derive(Deserialize)]
struct TencentRealtimeResponse {
    code: Option<i32>,
    message: Option<String>,
    message_id: Option<String>,
    r#final: Option<u8>,
    sentences: Option<TencentSpeakerSentences>,
    result: Option<TencentRealtimeResult>,
    speaker_context_id: Option<String>,
}

#[derive(Deserialize)]
struct TencentSpeakerSentences {
    sentence: Option<String>,
    sentence_type: Option<u8>,
    sentence_id: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    speaker_id: Option<i32>,
    start_time: Option<u32>,
    end_time: Option<u32>,
    sentence_list: Option<Vec<TencentSpeakerSentence>>,
}

#[derive(Clone, Deserialize)]
struct TencentSpeakerSentence {
    sentence: Option<String>,
    sentence_type: Option<u8>,
    sentence_id: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_optional_i32")]
    speaker_id: Option<i32>,
    start_time: Option<u32>,
    end_time: Option<u32>,
}

#[derive(Deserialize)]
struct TencentRealtimeResult {
    voice_text_str: Option<String>,
    slice_type: Option<u8>,
    index: Option<i32>,
    start_time: Option<u32>,
    end_time: Option<u32>,
}

enum TencentMessageAction {
    Continue,
    Stop(VoiceStoppedReason),
}

enum TencentParsedMessage {
    Continue(Vec<TencentSpeakerSentence>),
    Stop(VoiceStoppedReason, Vec<TencentSpeakerSentence>),
    Error { message: String, code: Option<i32> },
}

impl AsrSession for TencentAsrSession {
    fn send_audio_chunk(&mut self, chunk: Vec<u8>) -> Result<(), String> {
        self.audio_sender
            .send(chunk)
            .map_err(|_| "腾讯云 ASR 音频发送通道已关闭。".to_string())
    }

    fn stop(&mut self) {}
}

impl AsrProvider for TencentAsrProvider {
    fn kind(&self) -> VoiceProviderKind {
        VoiceProviderKind::Tencent
    }

    fn validate_start(&self) -> Result<(), String> {
        TencentAsrConfig::from_env().map(|_| ())
    }

    fn diagnostic(&self) -> VoiceProviderDiagnostic {
        let missing_env = TencentAsrConfig::missing_required_env();
        if !missing_env.is_empty() {
            return VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: None,
                details: BTreeMap::new(),
                error: Some("腾讯云 ASR 凭证未配置完整。".to_string()),
            };
        }

        match TencentAsrConfig::from_env() {
            Ok(config) => {
                let mut details = BTreeMap::new();
                details.insert("appId".to_string(), config.app_id.clone());
                details.insert(
                    "engineModelType".to_string(),
                    config.engine_model_type.clone(),
                );
                details.insert(
                    "sentenceStrategy".to_string(),
                    config.sentence_strategy.to_string(),
                );
                details.insert("voiceFormat".to_string(), config.voice_format.to_string());
                details.insert("needVad".to_string(), config.need_vad.to_string());

                VoiceProviderDiagnostic {
                    provider: self.kind(),
                    configured: true,
                    missing_env,
                    endpoint: Some(config.host),
                    details,
                    error: None,
                }
            }
            Err(error) => VoiceProviderDiagnostic {
                provider: self.kind(),
                configured: false,
                missing_env,
                endpoint: None,
                details: BTreeMap::new(),
                error: Some(error),
            },
        }
    }

    fn start_session(
        &self,
        context: AsrStartContext,
    ) -> Result<Box<dyn AsrSession + Send>, String> {
        let config = TencentAsrConfig::from_env()?;
        let sender = spawn_tencent_provider(
            context.app,
            context.session_id,
            Arc::clone(&context.cancel_signal),
            Arc::clone(&context.finish_signal),
            config,
        );

        Ok(Box::new(TencentAsrSession {
            audio_sender: sender,
        }))
    }
}

impl TencentAsrConfig {
    pub(crate) fn from_env() -> Result<Self, String> {
        let app_id = required_env("TENCENTCLOUD_APP_ID")?;
        let secret_id = required_env("TENCENTCLOUD_SECRET_ID")?;
        let secret_key = required_env("TENCENTCLOUD_SECRET_KEY")?;

        Ok(Self {
            app_id,
            secret_id,
            secret_key,
            engine_model_type: read_local_env("TENCENT_ASR_ENGINE_MODEL_TYPE")
                .unwrap_or_else(|| "16k_zh_en_speaker".to_string()),
            sentence_strategy: read_local_env("TENCENT_ASR_SENTENCE_STRATEGY")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0),
            voice_format: read_local_env("TENCENT_ASR_VOICE_FORMAT")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(1),
            need_vad: read_local_env("TENCENT_ASR_NEED_VAD")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(1),
            host: read_local_env("TENCENT_ASR_HOST")
                .unwrap_or_else(|| "asr.cloud.tencent.com".to_string()),
        })
    }

    pub(crate) fn is_available() -> bool {
        Self::missing_required_env().is_empty()
    }

    pub(crate) fn missing_required_env() -> Vec<String> {
        [
            "TENCENTCLOUD_APP_ID",
            "TENCENTCLOUD_SECRET_ID",
            "TENCENTCLOUD_SECRET_KEY",
        ]
        .iter()
        .filter(|key| required_env(key).is_err())
        .map(|key| (*key).to_string())
        .collect()
    }

    pub(crate) fn signed_websocket_url(&self, session_id: &str) -> Result<String, String> {
        let mut params = BTreeMap::new();
        params.insert("engine_model_type", self.engine_model_type.clone());
        params.insert("enable_speaker_context", "0".to_string());
        params.insert("expired", (now_seconds() + 3600).to_string());
        params.insert("filter_dirty", "0".to_string());
        params.insert("filter_modal", "0".to_string());
        params.insert("filter_punc", "0".to_string());
        params.insert("needvad", self.need_vad.to_string());
        params.insert("nonce", (now_millis() % 1_000_000_000).to_string());
        params.insert("result_mod", "1".to_string());
        params.insert("secretid", self.secret_id.clone());
        params.insert("sentence_strategy", self.sentence_strategy.to_string());
        params.insert("speaker_context_id", "".to_string());
        params.insert("speaker_diarization", "1".to_string());
        params.insert("timestamp", now_seconds().to_string());
        params.insert("voice_format", self.voice_format.to_string());
        params.insert("voice_id", session_id.to_string());

        let query_to_sign = params
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        let sign_source = format!("{}/asr/v2/{}?{}", self.host, self.app_id, query_to_sign);
        let signature = sign_hmac_sha1_base64(&self.secret_key, &sign_source)?;
        let encoded_query = params
            .iter()
            .map(|(key, value)| format!("{key}={}", encode(value)))
            .chain([format!("signature={}", encode(&signature))])
            .collect::<Vec<_>>()
            .join("&");

        Ok(format!(
            "wss://{}/asr/v2/{}?{}",
            self.host, self.app_id, encoded_query
        ))
    }

    pub(crate) fn redact_signed_url(&self, signed_url: &str) -> String {
        signed_url
            .split_once('?')
            .map(|(base, query)| {
                let preview_query = query
                    .split('&')
                    .map(|pair| {
                        if pair.starts_with("secretid=") {
                            "secretid=<redacted>".to_string()
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

fn spawn_tencent_provider(
    app: AppHandle,
    session_id: String,
    cancel_signal: Arc<AtomicBool>,
    finish_signal: Arc<AtomicBool>,
    config: TencentAsrConfig,
) -> UnboundedSender<Vec<u8>> {
    let (audio_sender, mut audio_receiver) = unbounded_channel::<Vec<u8>>();

    thread::spawn(move || {
        runtime().block_on(async move {
            let connection_url = match config.signed_websocket_url(&session_id) {
                Ok(url) => url,
                Err(error) => {
                    emit_error(
                        &app,
                        Some(session_id.clone()),
                        error,
                        Some("tencent_sign_failed"),
                    );
                    emit_stopped(&app, Some(session_id), VoiceStoppedReason::Error);
                    return;
                }
            };

            let websocket = connect_async(&connection_url).await;
            let Ok((socket, _response)) = websocket else {
                let message = websocket
                    .err()
                    .map(|error| format!("腾讯云 ASR WebSocket 连接失败：{error}"))
                    .unwrap_or_else(|| "腾讯云 ASR WebSocket 连接失败。".to_string());
                emit_error(
                    &app,
                    Some(session_id.clone()),
                    message,
                    Some("tencent_connect_failed"),
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
            let reader_finish_signal = Arc::clone(&finish_signal);

            let reader_task = tokio::spawn(async move {
                while let Some(message) = reader.next().await {
                    if reader_cancel_signal.load(Ordering::Relaxed) {
                        return;
                    }

                    match message {
                        Ok(Message::Text(text)) => {
                            match handle_tencent_message(
                                &reader_app,
                                &reader_session_id,
                                &reader_cancel_signal,
                                &reader_finish_signal,
                                &text,
                            ) {
                                TencentMessageAction::Continue => {}
                                TencentMessageAction::Stop(reason) => {
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
                                format!("腾讯云 ASR 接收失败：{error}"),
                                Some("tencent_receive_failed"),
                            );
                            let _ = completion_sender.send(VoiceStoppedReason::Error);
                            return;
                        }
                    }
                }

                let _ = completion_sender.send(VoiceStoppedReason::Completed);
            });

            let stop_reason = run_tencent_send_loop(
                &app,
                &session_id,
                &cancel_signal,
                &finish_signal,
                &mut writer,
                &mut audio_receiver,
                &mut completion_receiver,
            )
            .await;

            if matches!(stop_reason, VoiceStoppedReason::User) {
                let stop_reason =
                    match wait_for_tencent_final(&mut writer, &mut completion_receiver).await {
                        Ok(reason) => reason,
                        Err(error) => {
                            emit_error(
                                &app,
                                Some(session_id.clone()),
                                error,
                                Some("tencent_finish_failed"),
                            );
                            VoiceStoppedReason::Error
                        }
                    };
                reader_task.abort();
                emit_stopped(&app, Some(session_id), stop_reason);
                return;
            }

            reader_task.abort();
            emit_stopped(&app, Some(session_id), stop_reason);
        });
    });

    audio_sender
}

async fn run_tencent_send_loop<S>(
    app: &AppHandle,
    session_id: &str,
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
    loop {
        if should_finish_tencent_stream(cancel_signal, finish_signal) {
            return VoiceStoppedReason::User;
        }

        tokio::select! {
            completion = completion_receiver.recv() => {
                if let Some(reason) = completion {
                    return reason;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(20)), if finish_signal.load(Ordering::Relaxed) => {
                return VoiceStoppedReason::User;
            }
            chunk = audio_receiver.recv() => {
                let Some(chunk) = chunk else {
                    return VoiceStoppedReason::User;
                };

                if should_finish_tencent_stream(cancel_signal, finish_signal) {
                    return VoiceStoppedReason::User;
                }

                if let Err(error) = writer.send(Message::Binary(chunk)).await {
                    emit_error(
                        app,
                        Some(session_id.to_string()),
                        format!("腾讯云 ASR 音频发送失败：{error}"),
                        Some("tencent_send_failed"),
                    );
                    return VoiceStoppedReason::Error;
                }
            }
        }
    }
}

fn should_finish_tencent_stream(
    cancel_signal: &Arc<AtomicBool>,
    finish_signal: &Arc<AtomicBool>,
) -> bool {
    cancel_signal.load(Ordering::Relaxed) || finish_signal.load(Ordering::Relaxed)
}

async fn wait_for_tencent_final<S>(
    writer: &mut S,
    completion_receiver: &mut UnboundedReceiver<VoiceStoppedReason>,
) -> Result<VoiceStoppedReason, String>
where
    S: SinkExt<Message> + Unpin,
    <S as futures_util::Sink<Message>>::Error: std::fmt::Display,
{
    writer
        .send(Message::Text("{\"type\":\"end\"}".to_string()))
        .await
        .map_err(|error| format!("腾讯云 ASR 结束包发送失败：{error}"))?;

    let stop_reason =
        match tokio::time::timeout(Duration::from_secs(8), completion_receiver.recv()).await {
            Ok(Some(reason)) => Ok(reason),
            Ok(None) => Ok(VoiceStoppedReason::Completed),
            Err(_) => Ok(VoiceStoppedReason::Completed),
        };

    let _ = writer.close().await;
    stop_reason
}

fn handle_tencent_message(
    app: &AppHandle,
    session_id: &str,
    stop_signal: &Arc<AtomicBool>,
    finish_signal: &Arc<AtomicBool>,
    raw_message: &str,
) -> TencentMessageAction {
    match parse_tencent_message(raw_message) {
        TencentParsedMessage::Continue(sentences) => {
            for sentence in sentences {
                emit_tencent_sentence(app, session_id, sentence);
            }
            TencentMessageAction::Continue
        }
        TencentParsedMessage::Stop(reason, sentences) => {
            for sentence in sentences {
                emit_tencent_sentence(app, session_id, sentence);
            }
            stop_signal.store(true, Ordering::Relaxed);
            TencentMessageAction::Stop(reason)
        }
        TencentParsedMessage::Error { message, code } => {
            if finish_signal.load(Ordering::Relaxed)
                && is_tencent_finish_timeout_error(code, &message)
            {
                stop_signal.store(true, Ordering::Relaxed);
                return TencentMessageAction::Stop(VoiceStoppedReason::Completed);
            }

            emit_error(
                app,
                Some(session_id.to_string()),
                message,
                Some("tencent_api_error"),
            );
            stop_signal.store(true, Ordering::Relaxed);
            TencentMessageAction::Stop(VoiceStoppedReason::Error)
        }
    }
}

fn parse_tencent_message(raw_message: &str) -> TencentParsedMessage {
    let parsed = serde_json::from_str::<TencentRealtimeResponse>(raw_message);
    let Ok(response) = parsed else {
        return TencentParsedMessage::Error {
            message: "腾讯云 ASR 返回了无法解析的消息。".to_string(),
            code: None,
        };
    };

    if let Some(code) = response.code {
        if code != 0 {
            return TencentParsedMessage::Error {
                message: response
                    .message
                    .unwrap_or_else(|| format!("腾讯云 ASR 返回错误码 {code}。")),
                code: Some(code),
            };
        }
    }

    log_tencent_speaker_diagnostic(&response);

    let sentences = response
        .sentences
        .map(TencentSpeakerSentences::into_sentence_events)
        .or_else(|| {
            response
                .result
                .map(TencentRealtimeResult::into_sentence_events)
        })
        .unwrap_or_default();

    if response.r#final == Some(1) {
        return TencentParsedMessage::Stop(VoiceStoppedReason::Completed, sentences);
    }

    TencentParsedMessage::Continue(sentences)
}

fn is_tencent_finish_timeout_error(code: Option<i32>, message: &str) -> bool {
    matches!(code, Some(4008 | 4009))
        || (message.contains("15秒") && message.contains("音频"))
        || message.contains("音频分片等待超时")
        || message.contains("客户端连接断开")
}

fn log_tencent_speaker_diagnostic(response: &TencentRealtimeResponse) {
    let Some(sentences) = response.sentences.as_ref() else {
        return;
    };

    let speaker_ids = tencent_speaker_ids(sentences);
    if speaker_ids.is_empty() {
        return;
    }

    eprintln!(
        "[voice][tencent] message_id={} final={:?} speaker_context_id={} speaker_ids={:?}",
        response.message_id.as_deref().unwrap_or("-"),
        response.r#final,
        response.speaker_context_id.as_deref().unwrap_or("-"),
        speaker_ids
    );
}

fn tencent_speaker_ids(sentences: &TencentSpeakerSentences) -> Vec<i32> {
    let mut speaker_ids = Vec::new();

    if let Some(sentence_list) = sentences.sentence_list.as_ref() {
        for sentence in sentence_list {
            if let Some(speaker_id) = sentence.speaker_id {
                speaker_ids.push(speaker_id);
            }
        }
    } else if let Some(speaker_id) = sentences.speaker_id {
        speaker_ids.push(speaker_id);
    }

    speaker_ids.sort_unstable();
    speaker_ids.dedup();
    speaker_ids
}

impl TencentSpeakerSentences {
    fn into_sentence_events(self) -> Vec<TencentSpeakerSentence> {
        if let Some(sentence_list) = self.sentence_list {
            return sentence_list;
        }

        vec![TencentSpeakerSentence {
            sentence: self.sentence,
            sentence_type: self.sentence_type,
            sentence_id: self.sentence_id,
            speaker_id: self.speaker_id,
            start_time: self.start_time,
            end_time: self.end_time,
        }]
    }
}

impl TencentRealtimeResult {
    fn into_sentence_events(self) -> Vec<TencentSpeakerSentence> {
        let sentence_type = self
            .slice_type
            .map(|slice_type| if slice_type == 2 { 1 } else { 0 });

        vec![TencentSpeakerSentence {
            sentence: self.voice_text_str,
            sentence_type,
            sentence_id: self.index,
            speaker_id: None,
            start_time: self.start_time,
            end_time: self.end_time,
        }]
    }
}

fn emit_tencent_sentence(app: &AppHandle, session_id: &str, sentence: TencentSpeakerSentence) {
    let Some(text) = sentence.sentence.filter(|text| !text.trim().is_empty()) else {
        return;
    };

    let is_final = sentence.sentence_type == Some(1);
    let speaker_id = normalized_tencent_speaker_id(sentence.speaker_id);
    let sentence_key = sentence
        .sentence_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| now_millis().to_string());
    let _ = app.emit(
        TRANSCRIPT_EVENT,
        VoiceTranscriptEvent {
            id: format!("{session_id}-{sentence_key}"),
            session_id: session_id.to_string(),
            speaker_id,
            text,
            is_final,
            started_at_ms: sentence.start_time,
            ended_at_ms: sentence.end_time,
            created_at: now_millis_string(),
        },
    );
}

fn normalized_tencent_speaker_id(speaker_id: Option<i32>) -> Option<String> {
    speaker_id.and_then(|speaker| {
        if speaker < 0 {
            None
        } else {
            Some(format!("speaker-{}", speaker + 1))
        }
    })
}

fn required_env(key: &str) -> Result<String, String> {
    read_local_env(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("缺少本地环境变量 {key}，请先配置腾讯云 ASR 凭证。"))
}

fn sign_hmac_sha1_base64(secret_key: &str, source: &str) -> Result<String, String> {
    let mut mac = HmacSha1::new_from_slice(secret_key.as_bytes())
        .map_err(|_| "腾讯云 ASR 签名初始化失败。".to_string())?;
    mac.update(source.as_bytes());
    Ok(general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_ENCODE_SET).to_string()
}

fn deserialize_optional_i32<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;

    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => number
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| de::Error::custom("speaker_id must fit in i32")),
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed.parse::<i32>().map(Some).map_err(de::Error::custom)
            }
        }
        _ => Err(de::Error::custom("speaker_id must be number or string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_single_tencent_sentence() {
        let raw_message = r#"{
            "code": 0,
            "message": "success",
            "final": 0,
            "sentences": {
                "sentence": "开始实现语音输入。",
                "sentence_type": 1,
                "speaker_id": 2,
                "start_time": 120,
                "end_time": 880
            }
        }"#;

        let parsed = serde_json::from_str::<TencentRealtimeResponse>(raw_message)
            .expect("response should parse");
        let sentences = parsed
            .sentences
            .expect("sentences should exist")
            .into_sentence_events();

        assert_eq!(parsed.r#final, Some(0));
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0].sentence.as_deref(), Some("开始实现语音输入。"));
        assert_eq!(sentences[0].sentence_type, Some(1));
        assert_eq!(sentences[0].speaker_id, Some(2));
        assert_eq!(sentences[0].start_time, Some(120));
        assert_eq!(sentences[0].end_time, Some(880));
    }

    #[test]
    fn parses_tencent_sentence_list() {
        let raw_message = r#"{
            "code": 0,
            "final": 1,
            "sentences": {
                "sentence_list": [
                    {
                        "sentence": "第一句。",
                        "sentence_type": 1,
                        "sentence_id": 1,
                        "speaker_id": 0
                    },
                    {
                        "sentence": "第二句。",
                        "sentence_type": 0,
                        "sentence_id": 2,
                        "speaker_id": 1
                    }
                ]
            }
        }"#;

        let parsed = serde_json::from_str::<TencentRealtimeResponse>(raw_message)
            .expect("response should parse");
        let sentences = parsed
            .sentences
            .expect("sentences should exist")
            .into_sentence_events();

        assert_eq!(parsed.r#final, Some(1));
        assert_eq!(sentences.len(), 2);
        assert_eq!(sentences[0].sentence.as_deref(), Some("第一句。"));
        assert_eq!(sentences[0].speaker_id, Some(0));
        assert_eq!(sentences[1].sentence.as_deref(), Some("第二句。"));
        assert_eq!(sentences[1].sentence_type, Some(0));
    }

    #[test]
    fn parses_string_tencent_speaker_ids() {
        let raw_message = r#"{
            "code": 0,
            "final": 0,
            "sentences": {
                "sentence_list": [
                    {
                        "sentence": "字符串 speaker。",
                        "sentence_type": 1,
                        "sentence_id": 3,
                        "speaker_id": "1"
                    }
                ]
            }
        }"#;

        match parse_tencent_message(raw_message) {
            TencentParsedMessage::Continue(sentences) => {
                assert_eq!(sentences.len(), 1);
                assert_eq!(sentences[0].speaker_id, Some(1));
                assert_eq!(
                    tencent_speaker_ids(&TencentSpeakerSentences {
                        sentence: None,
                        sentence_type: None,
                        sentence_id: None,
                        speaker_id: None,
                        start_time: None,
                        end_time: None,
                        sentence_list: Some(sentences),
                    }),
                    vec![1]
                );
            }
            _ => panic!("string speaker_id should parse"),
        }
    }

    #[test]
    fn parses_tencent_realtime_result_payload() {
        let raw_message = r#"{
            "code": 0,
            "final": 0,
            "result": {
                "voice_text_str": "普通实时识别返回。",
                "slice_type": 2,
                "index": 7,
                "start_time": 240,
                "end_time": 1320
            }
        }"#;

        match parse_tencent_message(raw_message) {
            TencentParsedMessage::Continue(sentences) => {
                assert_eq!(sentences.len(), 1);
                assert_eq!(sentences[0].sentence.as_deref(), Some("普通实时识别返回。"));
                assert_eq!(sentences[0].sentence_type, Some(1));
                assert_eq!(sentences[0].sentence_id, Some(7));
                assert_eq!(sentences[0].speaker_id, None);
                assert_eq!(sentences[0].start_time, Some(240));
                assert_eq!(sentences[0].end_time, Some(1320));
            }
            _ => panic!("result payload should become a transcript event"),
        }
    }

    #[test]
    fn tencent_invalid_json_stops_with_error() {
        match parse_tencent_message("{not json") {
            TencentParsedMessage::Error { message, code } => {
                assert!(message.contains("无法解析"));
                assert_eq!(code, None);
            }
            _ => panic!("invalid JSON should become an error"),
        }
    }

    #[test]
    fn tencent_api_error_stops_with_error() {
        let raw_message = r#"{
            "code": 4001,
            "message": "signature invalid"
        }"#;

        match parse_tencent_message(raw_message) {
            TencentParsedMessage::Error { message, code } => {
                assert_eq!(message, "signature invalid");
                assert_eq!(code, Some(4001));
            }
            _ => panic!("Tencent API error should become an error"),
        }
    }

    #[test]
    fn tencent_final_message_requests_completed_stop() {
        let raw_message = r#"{
            "code": 0,
            "final": 1
        }"#;

        match parse_tencent_message(raw_message) {
            TencentParsedMessage::Stop(VoiceStoppedReason::Completed, sentences) => {
                assert!(sentences.is_empty());
            }
            _ => panic!("final message should request completed stop"),
        }
    }

    #[test]
    fn ignores_unstable_tencent_speaker_id() {
        assert_eq!(normalized_tencent_speaker_id(Some(-1)), None);
        assert_eq!(
            normalized_tencent_speaker_id(Some(0)).as_deref(),
            Some("speaker-1")
        );
        assert_eq!(
            normalized_tencent_speaker_id(Some(3)).as_deref(),
            Some("speaker-4")
        );
    }

    #[test]
    fn maps_tencent_speaker_indexes_to_human_labels() {
        let sentence = TencentSpeakerSentence {
            sentence: Some("第二个人说话。".to_string()),
            sentence_type: Some(1),
            sentence_id: Some(12),
            speaker_id: Some(1),
            start_time: Some(100),
            end_time: Some(600),
        };

        assert_eq!(
            normalized_tencent_speaker_id(sentence.speaker_id).as_deref(),
            Some("speaker-2")
        );
    }

    #[test]
    fn recognizes_tencent_finish_timeout_errors() {
        assert!(is_tencent_finish_timeout_error(
            None,
            "客户端超过15秒未发送音频数据"
        ));
        assert!(is_tencent_finish_timeout_error(Some(4008), "timeout"));
        assert!(!is_tencent_finish_timeout_error(
            Some(4001),
            "signature invalid"
        ));
    }

    #[test]
    fn signs_tencent_websocket_url_without_exposing_secret() {
        let config = TencentAsrConfig {
            app_id: "test-app".to_string(),
            secret_id: "test-secret-id".to_string(),
            secret_key: "test-secret-key".to_string(),
            engine_model_type: "16k_zh_en_speaker".to_string(),
            sentence_strategy: 0,
            voice_format: 1,
            need_vad: 1,
            host: "asr.cloud.tencent.com".to_string(),
        };

        let url = config
            .signed_websocket_url("voice-test")
            .expect("signed URL should be generated");

        assert!(url.starts_with("wss://asr.cloud.tencent.com/asr/v2/test-app?"));
        assert!(url.contains("engine_model_type=16k_zh_en_speaker"));
        assert!(url.contains("enable_speaker_context=0"));
        assert!(url.contains("result_mod=1"));
        assert!(url.contains("sentence_strategy=0"));
        assert!(url.contains("speaker_context_id="));
        assert!(url.contains("speaker_diarization=1"));
        assert!(url.contains("voice_id=voice-test"));
        assert!(url.contains("signature="));
        assert!(!url.contains("test-secret-key"));
    }

    #[test]
    fn redacts_tencent_diagnostic_url() {
        let config = TencentAsrConfig {
            app_id: "test-app".to_string(),
            secret_id: "test-secret-id".to_string(),
            secret_key: "test-secret-key".to_string(),
            engine_model_type: "16k_zh_en_speaker".to_string(),
            sentence_strategy: 0,
            voice_format: 1,
            need_vad: 1,
            host: "asr.cloud.tencent.com".to_string(),
        };
        let url = config
            .signed_websocket_url("voice-test")
            .expect("signed URL should be generated");
        let preview = config.redact_signed_url(&url);

        assert!(preview.contains("secretid=<redacted>"));
        assert!(preview.contains("signature=<redacted>"));
        assert!(!preview.contains("test-secret-id"));
        assert!(!preview.contains("test-secret-key"));
        assert!(!preview.contains("signature_original"));
    }

    #[test]
    fn encodes_signature_reserved_characters() {
        assert_eq!(encode("+/="), "%2B%2F%3D");
    }

    #[test]
    fn tencent_stream_finishes_when_user_stops_or_cancels() {
        let cancel_signal = Arc::new(AtomicBool::new(false));
        let finish_signal = Arc::new(AtomicBool::new(false));

        assert!(!should_finish_tencent_stream(
            &cancel_signal,
            &finish_signal
        ));

        finish_signal.store(true, Ordering::Relaxed);
        assert!(should_finish_tencent_stream(&cancel_signal, &finish_signal));

        finish_signal.store(false, Ordering::Relaxed);
        cancel_signal.store(true, Ordering::Relaxed);
        assert!(should_finish_tencent_stream(&cancel_signal, &finish_signal));
    }
}
