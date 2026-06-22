import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
  RequirementGap,
  RequirementProcessingResult,
  RequirementState,
  RequirementSummaryResult,
  SavedRequirementDocument,
  RequirementUtterance,
  VoiceRequirementSession,
  VoiceSessionStatus,
  VoiceTranscriptSegment
} from "../types/app";
import { createId } from "./project";

const LIVE_SUMMARY_QUIET_DELAY_MS = 6000;
const LIVE_SUMMARY_MAX_INTERVAL_MS = 30000;
const LIVE_SUMMARY_BATCH_UTTERANCES = 3;

export type RequirementSessionAction =
  | {
      type: "start_voice_session";
      voiceSessionId: string;
      now: string;
    }
  | {
      type: "append_voice_transcript";
      segment: VoiceTranscriptSegment;
      now: string;
      source?: RequirementUtterance["source"];
    }
  | {
      type: "mark_summarizing";
      now: string;
    }
  | {
      type: "apply_live_summary";
      now: string;
      requirementId: string;
      result: RequirementSummaryResult;
    }
  | {
      type: "apply_live_summary_error";
      now: string;
      requirementId: string;
      error: string;
    }
  | {
      type: "mark_finalizing";
      now: string;
    }
  | {
      type: "apply_process_result";
      now: string;
      requirementId: string;
      result: RequirementProcessingResult;
    }
  | {
      type: "apply_process_error";
      now: string;
      requirementId: string;
      error: string;
    }
  | {
      type: "mark_saving";
      now: string;
    }
  | {
      type: "apply_document_save_result";
      now: string;
      requirementId: string;
      savedRequirementDocumentPath?: string;
      error?: string;
    }
  | {
      type: "confirm_requirement";
      now: string;
      savedRequirementDocumentPath?: string;
      error?: string;
    };

export type VoiceRequirementController = {
  session?: VoiceRequirementSession;
  active: boolean;
  finishUserTurn: () => void;
  confirmRequirement: () => void;
};

export type VoiceInputPermission = {
  canUseMic: boolean;
  canFinishTurn: boolean;
  transcriptSource?: RequirementUtterance["source"];
};

export function createRequirementState(now: string): RequirementState {
  return {
    id: createId("requirement"),
    status: "listening",
    utterances: [],
    summary: "",
    requirementDocument: "",
    confirmedFacts: [],
    constraints: [],
    openGaps: [],
    openQuestions: [],
    answeredQuestions: [],
    activeQuestionId: undefined,
    acceptanceCriteria: [],
    outOfScope: [],
    risks: [],
    error: undefined,
    updatedAt: now
  };
}

export function createVoiceRequirementSession(voiceSessionId: string, now: string): VoiceRequirementSession {
  return {
    id: createId("voice_requirement"),
    voiceSessionIds: [voiceSessionId],
    requirementState: createRequirementState(now),
    startedAt: now
  };
}

export function requirementSessionReducer(
  session: VoiceRequirementSession | undefined,
  action: RequirementSessionAction
): VoiceRequirementSession | undefined {
  if (action.type === "start_voice_session") {
    if (!session) {
      return createVoiceRequirementSession(action.voiceSessionId, action.now);
    }

    const permission = getVoiceInputPermission(session.requirementState);
    if (!permission.canUseMic) {
      return session;
    }

    return {
      ...session,
      voiceSessionIds: appendUnique(session.voiceSessionIds, action.voiceSessionId),
      requirementState: {
        ...session.requirementState,
        pendingAction: undefined,
        codingPrompt: undefined,
        savedRequirementDocumentPath: undefined,
        updatedAt: action.now
      },
      endedAt: undefined
    };
  }

  if (!session) {
    return undefined;
  }

  if (action.type === "mark_summarizing") {
    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        pendingAction: "summarize",
        error: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_live_summary") {
    if (
      action.requirementId !== session.requirementState.id ||
      session.requirementState.status !== "listening"
    ) {
      return session;
    }

    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        summary: action.result.summary,
        confirmedFacts: action.result.confirmedFacts,
        constraints: action.result.constraints,
        acceptanceCriteria: action.result.acceptanceCriteria,
        outOfScope: action.result.outOfScope,
        risks: action.result.risks,
        openGaps: normalizeSummaryGaps(action.result.openGaps),
        pendingAction: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_live_summary_error") {
    if (
      action.requirementId !== session.requirementState.id ||
      session.requirementState.status !== "listening"
    ) {
      return session;
    }

    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        pendingAction: undefined,
        error: action.error,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "mark_finalizing") {
    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        status: "finalizing",
        pendingAction: "finalize",
        error: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "mark_saving") {
    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        pendingAction: "save",
        error: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_process_result") {
    if (action.requirementId !== session.requirementState.id) {
      return session;
    }

    const openQuestions = action.result.questions.map((question) => ({
      id: question.id || createId("question"),
      question: question.question,
      reason: question.reason,
      blocksCoding: question.blocksCoding,
      answer: question.answer
    }));

    return {
      ...session,
      endedAt: action.now,
      requirementState: {
        ...session.requirementState,
        summary: action.result.summary,
        requirementDocument: action.result.requirementDocumentDraft,
        confirmedFacts: action.result.confirmedFacts,
        constraints: action.result.constraints,
        acceptanceCriteria: action.result.acceptanceCriteria,
        outOfScope: action.result.outOfScope,
        risks: action.result.risks,
        openQuestions,
        openGaps: [],
        activeQuestionId: undefined,
        codingPrompt: createCodingPromptFromProcessingResult(action.result),
        status: "document_ready",
        pendingAction: undefined,
        error: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_document_save_result") {
    if (action.requirementId !== session.requirementState.id) {
      return session;
    }

    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        savedRequirementDocumentPath: action.savedRequirementDocumentPath ?? session.requirementState.savedRequirementDocumentPath,
        pendingAction: undefined,
        error: action.error,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_process_error") {
    if (action.requirementId !== session.requirementState.id) {
      return session;
    }

    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        status: "listening",
        pendingAction: undefined,
        error: action.error,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "confirm_requirement") {
    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        status: "confirmed",
        codingPrompt: session.requirementState.codingPrompt ?? createCodingPromptFromConfirmedRequirement(session.requirementState),
        savedRequirementDocumentPath: action.savedRequirementDocumentPath ?? session.requirementState.savedRequirementDocumentPath,
        pendingAction: undefined,
        error: action.error,
        updatedAt: action.now
      }
    };
  }

  if (!action.segment.isFinal) {
    return session;
  }

  const permission = getVoiceInputPermission(session.requirementState);
  const transcriptSource = session.requirementState.status === "finalizing" ? "voice" : permission.transcriptSource;
  if (!transcriptSource) {
    return session;
  }

  if (!session.voiceSessionIds.includes(action.segment.sessionId)) {
    return session;
  }

  const text = normalizeTranscriptText(action.segment.text);
  if (!text) {
    return session;
  }

  const utterance = transcriptSegmentToUtterance(action.segment, text, action.source ?? transcriptSource);
  const utterances = upsertUtteranceByTranscriptId(session.requirementState.utterances, utterance);

  return {
    ...session,
    requirementState: {
      ...session.requirementState,
      utterances,
      answeredQuestions: session.requirementState.answeredQuestions,
      codingPrompt: undefined,
      savedRequirementDocumentPath: undefined,
      pendingAction: session.requirementState.status === "finalizing" ? session.requirementState.pendingAction : undefined,
      error: undefined,
      updatedAt: action.now
    }
  };
}

export function useVoiceRequirementSession(voice: {
  sessionId?: string;
  status: VoiceSessionStatus;
  segments: VoiceTranscriptSegment[];
}, projectPath?: string): VoiceRequirementController {
  const [session, dispatch] = useReducer(requirementSessionReducer, undefined);
  const sessionRef = useRef<VoiceRequirementSession | undefined>(undefined);
  const processedSegmentTextsRef = useRef<Map<string, string>>(new Map());
  const lastSummarizedUtteranceCountRef = useRef(0);
  const lastSummaryAtRef = useRef(0);
  const summaryTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>();
  const liveSummaryRequestRef = useRef(0);
  const processRequestRef = useRef(0);
  const finalizingRequestRequirementIdRef = useRef<string | undefined>();
  const projectPathRef = useRef<string | undefined>(projectPath);
  const savedRequirementIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  useEffect(() => {
    projectPathRef.current = projectPath;
  }, [projectPath]);

  useEffect(() => {
    if (!voice.sessionId) {
      return;
    }

    dispatch({
      type: "start_voice_session",
      voiceSessionId: voice.sessionId,
      now: nowString()
    });
  }, [voice.sessionId]);

  useEffect(() => {
    for (const segment of voice.segments) {
      if (!segment.isFinal) {
        continue;
      }

      const processedText = processedSegmentTextsRef.current.get(segment.id);
      if (processedText === segment.text) {
        continue;
      }

      processedSegmentTextsRef.current.set(segment.id, segment.text);
      dispatch({
        type: "append_voice_transcript",
        segment,
        now: nowString()
      });
    }
  }, [voice.segments]);

  const applyLiveSummary = useCallback(() => {
    const currentSession = sessionRef.current;
    if (!currentSession?.requirementState.utterances.length) {
      return;
    }

    dispatch({
      type: "mark_summarizing",
      now: nowString()
    });

    const requestId = liveSummaryRequestRef.current + 1;
    liveSummaryRequestRef.current = requestId;
    const requirementId = currentSession.requirementState.id;
    const summarizedUtteranceCount = currentSession.requirementState.utterances.length;

    void invokeTauri<RequirementSummaryResult>("summarize_requirement_state", {
      request: {
        state: currentSession.requirementState
      }
    })
      .then((result) => {
        if (liveSummaryRequestRef.current !== requestId) {
          return;
        }

        const latestSession = sessionRef.current;
        if (!latestSession || latestSession.requirementState.id !== requirementId) {
          return;
        }

        lastSummarizedUtteranceCountRef.current = summarizedUtteranceCount;
        lastSummaryAtRef.current = Date.now();
        dispatch({
          type: "apply_live_summary",
          requirementId,
          result,
          now: nowString()
        });
      })
      .catch((error) => {
        if (liveSummaryRequestRef.current !== requestId) {
          return;
        }

        dispatch({
          type: "apply_live_summary_error",
          requirementId,
          error: toErrorMessage(error),
          now: nowString()
        });
      });
  }, []);

  useEffect(() => {
    if (!session || session.requirementState.status !== "listening") {
      return;
    }

    const utteranceCount = session.requirementState.utterances.length;
    const unsummarizedCount = utteranceCount - lastSummarizedUtteranceCountRef.current;
    if (unsummarizedCount <= 0 || session.requirementState.pendingAction) {
      return;
    }

    const lastSummarizedAt = Number(lastSummaryAtRef.current || 0);
    const elapsedSinceSummary = Date.now() - lastSummarizedAt;
    if (unsummarizedCount >= LIVE_SUMMARY_BATCH_UTTERANCES || (lastSummarizedAt > 0 && elapsedSinceSummary >= LIVE_SUMMARY_MAX_INTERVAL_MS)) {
      applyLiveSummary();
      return;
    }

    clearSummaryTimer(summaryTimerRef.current);
    summaryTimerRef.current = setTimeout(applyLiveSummary, LIVE_SUMMARY_QUIET_DELAY_MS);

    return () => {
      clearSummaryTimer(summaryTimerRef.current);
    };
  }, [applyLiveSummary, session]);

  const finishUserTurn = useCallback(() => {
    clearSummaryTimer(summaryTimerRef.current);
    summaryTimerRef.current = undefined;
    const currentSession = sessionRef.current;
    if (!currentSession) {
      return;
    }

    liveSummaryRequestRef.current += 1;
    lastSummarizedUtteranceCountRef.current = currentSession.requirementState.utterances.length;
    dispatch({
      type: "mark_finalizing",
      now: nowString()
    });

    finalizingRequestRequirementIdRef.current = undefined;
  }, []);

  useEffect(() => {
    const currentSession = sessionRef.current;
    if (
      !currentSession ||
      currentSession.requirementState.status !== "finalizing" ||
      currentSession.requirementState.pendingAction !== "finalize" ||
      finalizingRequestRequirementIdRef.current === currentSession.requirementState.id ||
      voice.status === "recording" ||
      voice.status === "starting" ||
      voice.status === "requesting-permission" ||
      voice.status === "transcribing"
    ) {
      return;
    }

    const requestId = processRequestRef.current + 1;
    processRequestRef.current = requestId;
    const requirementId = currentSession.requirementState.id;
    finalizingRequestRequirementIdRef.current = requirementId;
    const processingState: RequirementState = {
      ...currentSession.requirementState,
      status: "finalizing",
      pendingAction: "finalize"
    };

    void invokeTauri<RequirementProcessingResult>("process_requirement_turn", {
      request: {
        state: processingState
      }
    })
      .then((result) => {
        if (processRequestRef.current !== requestId) {
          return;
        }

        dispatch({
          type: "apply_process_result",
          requirementId,
          result,
          now: nowString()
        });
      })
      .catch((error) => {
        if (processRequestRef.current !== requestId) {
          return;
        }

        finalizingRequestRequirementIdRef.current = undefined;
        dispatch({
          type: "apply_process_error",
          requirementId,
          error: toErrorMessage(error),
          now: nowString()
        });
      });
  }, [session, voice.status]);

  useEffect(() => {
    const currentSession = sessionRef.current;
    const state = currentSession?.requirementState;
    if (
      !currentSession ||
      !state ||
      state.status !== "document_ready" ||
      state.pendingAction ||
      state.savedRequirementDocumentPath ||
      !state.requirementDocument ||
      savedRequirementIdsRef.current.has(state.id)
    ) {
      return;
    }

    savedRequirementIdsRef.current.add(state.id);
    dispatch({
      type: "mark_saving",
      now: nowString()
    });

    const codingPrompt = state.codingPrompt ?? createCodingPromptFromConfirmedRequirement(state);
    const selectedProjectPath = projectPathRef.current;
    if (!selectedProjectPath) {
      dispatch({
        type: "apply_document_save_result",
        requirementId: state.id,
        error: "未选择项目，需求文档未写入文件。",
        now: nowString()
      });
      return;
    }

    void invokeTauri<SavedRequirementDocument>("save_requirement_document", {
      request: {
        projectPath: selectedProjectPath,
        requirementDocument: state.requirementDocument,
        summary: state.summary,
        codingPrompt
      }
    })
      .then((savedDocument) => {
        window.dispatchEvent(new CustomEvent("voicecoder:project-files-changed", {
          detail: {
            projectPath: selectedProjectPath,
            changedPath: savedDocument.path
          }
        }));

        dispatch({
          type: "apply_document_save_result",
          requirementId: state.id,
          savedRequirementDocumentPath: savedDocument.path,
          now: nowString()
        });
      })
      .catch((error) => {
        dispatch({
          type: "apply_document_save_result",
          requirementId: state.id,
          error: toErrorMessage(error),
          now: nowString()
        });
      });
  }, [session]);

  const confirmRequirement = useCallback(() => {
    dispatch({
      type: "confirm_requirement",
      now: nowString()
    });
  }, []);

  return useMemo(
    () => ({
      session,
      active: Boolean(session),
      finishUserTurn,
      confirmRequirement
    }),
    [confirmRequirement, finishUserTurn, session]
  );
}

export function getVoiceInputPermission(state: RequirementState | undefined): VoiceInputPermission {
  if (!state) {
    return {
      canUseMic: true,
      canFinishTurn: false,
      transcriptSource: "voice"
    };
  }

  if (state.status === "listening" || state.status === "collecting") {
    return {
      canUseMic: true,
      canFinishTurn: state.utterances.some((utterance) => utterance.source === "voice"),
      transcriptSource: "voice"
    };
  }

  return {
    canUseMic: false,
    canFinishTurn: false
  };
}

function transcriptSegmentToUtterance(
  segment: VoiceTranscriptSegment,
  text: string,
  source: RequirementUtterance["source"]
): RequirementUtterance {
  return {
    id: `utterance_${segment.id}`,
    source,
    speakerId: segment.speakerId,
    text,
    createdAt: segment.createdAt,
    transcriptId: segment.id
  };
}

function upsertUtteranceByTranscriptId(utterances: RequirementUtterance[], utterance: RequirementUtterance) {
  const existingIndex = utterances.findIndex((item) => item.transcriptId === utterance.transcriptId);
  if (existingIndex < 0) {
    return [...utterances, utterance];
  }

  const existing = utterances[existingIndex];
  if (existing.text === utterance.text && existing.speakerId === utterance.speakerId) {
    return utterances;
  }

  const nextUtterances = [...utterances];
  nextUtterances[existingIndex] = {
    ...existing,
    speakerId: utterance.speakerId,
    text: utterance.text
  };
  return nextUtterances;
}

function appendUnique(values: string[], value: string) {
  return values.includes(value) ? values : [...values, value];
}

function normalizeSummaryGaps(gaps: Array<Omit<RequirementGap, "id" | "status"> & { id?: string; status?: RequirementGap["status"] }>) {
  return gaps
    .filter((gap) => gap.question.trim())
    .slice(0, 3)
    .map((gap) => ({
      id: gap.id || createId("gap"),
      question: gap.question,
      reason: gap.reason,
      severity: gap.severity,
      status: gap.status ?? "open"
    }));
}

function createCodingPromptFromProcessingResult(result: RequirementProcessingResult) {
  return `请根据以下已整理需求进行实现：\n\n${result.requirementDocumentDraft}`;
}

function createCodingPromptFromConfirmedRequirement(state: RequirementState) {
  const document = state.requirementDocument || state.summary;
  return `请根据以下已确认需求进行实现：\n\n${document}`;
}

function clearSummaryTimer(timer: ReturnType<typeof setTimeout> | undefined) {
  if (timer) {
    clearTimeout(timer);
  }
}

function nowString() {
  return Date.now().toString();
}

function normalizeTranscriptText(text: string) {
  return text
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^[，。！？、；：,.!?;:\s]+/u, "")
    .trim();
}

async function invokeTauri<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const tauri = await import("@tauri-apps/api/core");
  if (!tauri.isTauri()) {
    throw new Error("真实 LLM 需要在 Tauri 客户端中使用。");
  }

  return tauri.invoke<T>(command, args);
}

function toErrorMessage(error: unknown) {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  return "LLM 处理失败，请检查 provider 配置和网络。";
}
