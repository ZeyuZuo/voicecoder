use super::{
    clear_active_session, emit_stopped, now_millis_string, AsrProvider, AsrSession,
    AsrStartContext, VoiceProviderDiagnostic, VoiceProviderKind, VoiceStoppedReason,
    VoiceTranscriptEvent, TRANSCRIPT_EVENT,
};
use std::{collections::BTreeMap, sync::atomic::Ordering, thread, time::Duration};
use tauri::Emitter;

pub(crate) struct MockAsrProvider;

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
        spawn_mock_provider(context);
        Ok(Box::new(MockAsrSession))
    }
}

fn spawn_mock_provider(context: AsrStartContext) {
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
            if context.cancel_signal.load(Ordering::Relaxed) {
                emit_stopped(
                    &context.app,
                    Some(context.session_id.clone()),
                    VoiceStoppedReason::User,
                );
                return;
            }

            thread::sleep(Duration::from_millis(if *is_final { 850 } else { 600 }));

            if context.cancel_signal.load(Ordering::Relaxed) {
                emit_stopped(
                    &context.app,
                    Some(context.session_id.clone()),
                    VoiceStoppedReason::User,
                );
                return;
            }

            let _ = context.app.emit(
                TRANSCRIPT_EVENT,
                VoiceTranscriptEvent {
                    id: format!("{}-{index}", context.session_id),
                    session_id: context.session_id.clone(),
                    speaker_id: Some((*speaker_id).to_string()),
                    text: (*text).to_string(),
                    is_final: *is_final,
                    started_at_ms: Some((index as u32) * 1200),
                    ended_at_ms: Some((index as u32) * 1200 + 900),
                    created_at: now_millis_string(),
                },
            );
        }

        clear_active_session(&context.app, &context.session_id);
        emit_stopped(
            &context.app,
            Some(context.session_id),
            VoiceStoppedReason::Completed,
        );
    });
}
