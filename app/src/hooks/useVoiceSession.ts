import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  VoiceErrorEvent,
  VoiceProviderKind,
  VoiceProviderStatus,
  VoiceSessionStartedEvent,
  VoiceSessionSnapshot,
  VoiceSessionStatus,
  VoiceStoppedEvent,
  VoiceTranscriptEvent,
  VoiceTranscriptSegment
} from "../types/app";
import { createVoiceCapture, requestVoiceInputStream, type VoiceCaptureController } from "../utils/audioCapture";

type VoiceSessionState = {
  status: VoiceSessionStatus;
  sessionId?: string;
  provider: VoiceProviderKind;
  providerStatus?: VoiceProviderStatus;
  sessionSnapshot?: VoiceSessionSnapshot;
  partialText: string;
  segments: VoiceTranscriptSegment[];
  error?: string;
};

export type VoiceSessionController = VoiceSessionState & {
  start: () => Promise<void>;
  stop: () => Promise<void>;
  toggle: () => void;
  recording: boolean;
  busy: boolean;
};

const DEFAULT_AUDIO_CHUNK_SIZE_BYTES = 6400;
const IFLYTEK_LLM_AUDIO_CHUNK_SIZE_BYTES = 1280;

const initialVoiceState: VoiceSessionState = {
  status: "idle",
  provider: "mock",
  partialText: "",
  segments: []
};

export function useVoiceSession(): VoiceSessionController {
  const [state, setState] = useState<VoiceSessionState>(initialVoiceState);
  const captureRef = useRef<VoiceCaptureController | null>(null);
  const sessionRequestRef = useRef(0);
  const releaseErrorRef = useRef<string | undefined>(undefined);
  const activeSessionIdRef = useRef<string | undefined>(undefined);
  const suppressUnboundSessionEventsRef = useRef(false);
  const ignoredSessionIdsRef = useRef<Set<string>>(new Set());

  const releaseCapture = useCallback(() => {
    captureRef.current?.stop();
    captureRef.current = null;
  }, []);

  const refreshSessionSnapshot = useCallback(() => {
    if (!isTauri()) {
      return;
    }

    void invoke<VoiceSessionSnapshot>("get_voice_session_snapshot")
      .then((sessionSnapshot) => {
        setState((current) => ({
          ...current,
          sessionSnapshot,
          provider: sessionSnapshot.provider ?? current.provider
        }));
      })
      .catch(() => undefined);
  }, []);

  const releaseVoiceSession = useCallback((message?: string) => {
    releaseErrorRef.current = message;
    sessionRequestRef.current += 1;
    releaseCapture();

    if (isTauri()) {
      void invoke("cancel_voice_session").catch(() => undefined);
    }

    if (message) {
      setState((current) => ({
        ...current,
        status: "error",
        error: message,
        partialText: ""
      }));
    }
  }, [releaseCapture]);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }

    void invoke<VoiceProviderStatus>("get_voice_provider_status")
      .then((providerStatus) => {
        setState((current) => ({
          ...current,
          providerStatus,
          provider: providerStatus.autoProvider
        }));
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!isTauri()) {
      return;
    }

    const unlistenPromises = [
      listen<VoiceSessionStartedEvent>("voice://session-started", (event) => {
        activeSessionIdRef.current = event.payload.sessionId;
        setState((current) => ({
          ...current,
          status: "recording",
          sessionId: event.payload.sessionId,
          provider: event.payload.provider,
          partialText: "",
          error: undefined
        }));
        refreshSessionSnapshot();
      }),
      listen<VoiceTranscriptEvent>("voice://transcript", (event) => {
        setState((current) => {
          if (event.payload.sessionId !== current.sessionId) {
            return current;
          }

          if (!event.payload.isFinal) {
            return {
              ...current,
              partialText: event.payload.text
            };
          }

          const existingSegmentIndex = current.segments.findIndex((segment) => segment.id === event.payload.id);
          if (existingSegmentIndex >= 0) {
            const nextSegments = [...current.segments];
            nextSegments[existingSegmentIndex] = event.payload;
            return {
              ...current,
              partialText: "",
              segments: nextSegments
            };
          }

          return {
            ...current,
            partialText: "",
            segments: [...current.segments, event.payload]
          };
        });
      }),
      listen<VoiceErrorEvent>("voice://error", (event) => {
        if (isStaleSessionEvent(event.payload.sessionId, activeSessionIdRef.current, suppressUnboundSessionEventsRef.current, ignoredSessionIdsRef.current)) {
          return;
        }

        releaseVoiceSession(event.payload.message);
        refreshSessionSnapshot();
      }),
      listen<VoiceStoppedEvent>("voice://stopped", (event) => {
        if (isStaleSessionEvent(event.payload.sessionId, activeSessionIdRef.current, suppressUnboundSessionEventsRef.current, ignoredSessionIdsRef.current)) {
          return;
        }

        const releaseError = releaseErrorRef.current;
        releaseErrorRef.current = undefined;
        activeSessionIdRef.current = undefined;
        releaseCapture();
        setState((current) => ({
          ...current,
          status: event.payload.reason === "error" || releaseError ? "error" : "idle",
          sessionId: undefined,
          partialText: "",
          error: releaseError ?? current.error
        }));
        refreshSessionSnapshot();
      })
    ];

    return () => {
      releaseCapture();
      for (const unlistenPromise of unlistenPromises) {
        void unlistenPromise.then((unlisten) => unlisten());
      }
    };
  }, [releaseCapture, releaseVoiceSession]);

  const start = useCallback(async () => {
    const requestId = sessionRequestRef.current + 1;
    sessionRequestRef.current = requestId;
    releaseErrorRef.current = undefined;
    activeSessionIdRef.current = undefined;
    setState((current) => ({
      ...current,
      status: "starting",
      partialText: "",
      error: undefined
    }));

    if (!isTauri()) {
      setState((current) => ({
        ...current,
        status: "error",
        error: "语音输入需要在 Tauri 客户端中使用。"
      }));
      return;
    }

    try {
      setState((current) => ({
        ...current,
        status: "requesting-permission"
      }));
      const stream = await requestVoiceInputStream();
      if (sessionRequestRef.current !== requestId) {
        stopMediaStreamTracks(stream);
        return;
      }

      suppressUnboundSessionEventsRef.current = true;
      await invoke("cancel_voice_session").catch(() => undefined);
      suppressUnboundSessionEventsRef.current = false;
      const sessionId = await invoke<string>("start_voice_session", { provider: "auto" satisfies VoiceProviderKind });
      const sessionSnapshot = await loadSessionSnapshot();
      const audioChunkSizeBytes = getAudioChunkSizeBytes(sessionSnapshot?.provider);
      if (sessionSnapshot) {
        setState((current) => ({
          ...current,
          sessionSnapshot,
          provider: sessionSnapshot.provider ?? current.provider
        }));
      } else {
        refreshSessionSnapshot();
      }
      if (sessionRequestRef.current !== requestId) {
        stopMediaStreamTracks(stream);
        await stopActiveBackendSession();
        return;
      }
      activeSessionIdRef.current = sessionId;
      setState((current) => ({
        ...current,
        status: "requesting-permission",
        sessionId
      }));
      captureRef.current = await createVoiceCapture(stream, {
        sessionId,
        chunkSizeBytes: audioChunkSizeBytes,
        onError: (message) => {
          releaseVoiceSession(message);
        }
      });
      if (sessionRequestRef.current !== requestId) {
        releaseCapture();
        stopMediaStreamTracks(stream);
        await stopActiveBackendSession();
        return;
      }
      setState((current) => ({
        ...current,
        status: "recording"
      }));
    } catch (error) {
      suppressUnboundSessionEventsRef.current = false;
      activeSessionIdRef.current = undefined;
      releaseCapture();
      if (isTauri()) {
        void invoke("cancel_voice_session").catch(() => undefined);
      }
      setState((current) => ({
        ...current,
        status: "error",
        error: error instanceof Error ? error.message : String(error)
      }));
    }
  }, [refreshSessionSnapshot, releaseCapture, releaseVoiceSession]);

  const stop = useCallback(async () => {
    releaseErrorRef.current = undefined;
    releaseCapture();
    setState((current) => ({
      ...current,
      status: current.status === "idle" ? "idle" : "transcribing"
    }));

    if (!isTauri()) {
      activeSessionIdRef.current = undefined;
      setState((current) => ({
        ...current,
        status: "idle",
        sessionId: undefined,
        partialText: ""
      }));
      return;
    }

    try {
      await invoke("stop_voice_session");
      refreshSessionSnapshot();
    } catch (error) {
      activeSessionIdRef.current = undefined;
      setState((current) => ({
        ...current,
        status: "error",
        error: error instanceof Error ? error.message : String(error)
      }));
    }
  }, [refreshSessionSnapshot, releaseCapture]);

  const toggle = useCallback(() => {
    if (state.status === "recording" || state.status === "starting" || state.status === "requesting-permission") {
      void stop();
      return;
    }

    void start();
  }, [start, state.status, stop]);

  return useMemo(
    () => ({
      ...state,
      start,
      stop,
      toggle,
      recording: state.status === "recording",
      busy: state.status === "starting" || state.status === "requesting-permission" || state.status === "transcribing"
    }),
    [start, state, stop, toggle]
  );
}

async function stopActiveBackendSession() {
  if (isTauri()) {
    await invoke("cancel_voice_session").catch(() => undefined);
  }
}

async function loadSessionSnapshot() {
  if (!isTauri()) {
    return undefined;
  }

  return invoke<VoiceSessionSnapshot>("get_voice_session_snapshot").catch(() => undefined);
}

function getAudioChunkSizeBytes(provider: VoiceSessionSnapshot["provider"]) {
  if (provider === "iflytek_llm") {
    return IFLYTEK_LLM_AUDIO_CHUNK_SIZE_BYTES;
  }

  return DEFAULT_AUDIO_CHUNK_SIZE_BYTES;
}

function stopMediaStreamTracks(stream: MediaStream) {
  for (const track of stream.getTracks()) {
    track.stop();
  }
}

function isStaleSessionEvent(
  eventSessionId: string | undefined,
  activeSessionId: string | undefined,
  suppressUnboundSessionEvents: boolean,
  ignoredSessionIds: Set<string>
) {
  if (!eventSessionId) {
    return false;
  }

  if (ignoredSessionIds.has(eventSessionId)) {
    return true;
  }

  if (!activeSessionId && suppressUnboundSessionEvents) {
    ignoredSessionIds.add(eventSessionId);
    return true;
  }

  return activeSessionId !== eventSessionId;
}
