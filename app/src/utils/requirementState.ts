import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
  RequirementProcessingResult,
  RequirementQuestion,
  RequirementState,
  RequirementSummaryResult,
  SavedRequirementDocument,
  RequirementUtterance,
  VoiceRequirementSession,
  VoiceSessionStatus,
  VoiceTranscriptSegment
} from "../types/app";
import { createId } from "./project";

const LIVE_SUMMARY_INTERVAL_MS = 30000;

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
      type: "mark_processing";
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
      type: "mark_finalizing";
      now: string;
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
    status: "collecting",
    utterances: [],
    summary: "",
    requirementDocument: "",
    confirmedFacts: [],
    constraints: [],
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
      !["collecting", "clarifying"].includes(session.requirementState.status)
    ) {
      return session;
    }

    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        summary: action.result.summary,
        risks: mergeUnique(session.requirementState.risks, action.result.uncertainties),
        pendingAction: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_live_summary_error") {
    if (
      action.requirementId !== session.requirementState.id ||
      !["collecting", "clarifying"].includes(session.requirementState.status)
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

  if (action.type === "mark_processing") {
    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        status: "processing",
        pendingAction: "process",
        error: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "mark_finalizing") {
    return {
      ...session,
      requirementState: {
        ...session.requirementState,
        pendingAction: "finalize",
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
    const hasBlockingQuestion = openQuestions.some((question) => question.blocksCoding);

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
        activeQuestionId: openQuestions.find((question) => question.blocksCoding)?.id,
        status: action.result.readyToConfirm && !hasBlockingQuestion ? "ready_to_confirm" : "clarifying",
        pendingAction: undefined,
        error: undefined,
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
        status: session.requirementState.openQuestions.some((question) => question.blocksCoding) ? "clarifying" : "collecting",
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
        codingPrompt: createCodingPromptFromConfirmedRequirement(session.requirementState),
        savedRequirementDocumentPath: action.savedRequirementDocumentPath,
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
  if (!permission.transcriptSource) {
    return session;
  }

  if (!session.voiceSessionIds.includes(action.segment.sessionId)) {
    return session;
  }

  const text = action.segment.text.trim();
  if (!text) {
    return session;
  }

  const utterance = transcriptSegmentToUtterance(action.segment, text, action.source ?? permission.transcriptSource);
  const utterances = upsertUtteranceByTranscriptId(session.requirementState.utterances, utterance);
  const nextAnsweredQuestions = upsertAnsweredQuestionForUtterance(session.requirementState, utterance);

  return {
    ...session,
    requirementState: {
      ...session.requirementState,
      utterances,
      answeredQuestions: nextAnsweredQuestions,
      codingPrompt: undefined,
      savedRequirementDocumentPath: undefined,
      pendingAction: undefined,
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
  const summaryTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>();
  const liveSummaryRequestRef = useRef(0);
  const processRequestRef = useRef(0);
  const projectPathRef = useRef<string | undefined>(projectPath);

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

        lastSummarizedUtteranceCountRef.current = latestSession.requirementState.utterances.length;
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
    if (!session || session.requirementState.status === "processing" || session.requirementState.status === "ready_to_confirm" || session.requirementState.status === "confirmed") {
      return;
    }

    const utteranceCount = session.requirementState.utterances.length;
    const unsummarizedCount = utteranceCount - lastSummarizedUtteranceCountRef.current;
    if (unsummarizedCount <= 0 || session.requirementState.pendingAction) {
      return;
    }

    clearSummaryTimer(summaryTimerRef.current);
    summaryTimerRef.current = setTimeout(applyLiveSummary, LIVE_SUMMARY_INTERVAL_MS);

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
    const requirementId = currentSession.requirementState.id;
    lastSummarizedUtteranceCountRef.current = currentSession.requirementState.utterances.length;
    dispatch({
      type: "mark_processing",
      now: nowString()
    });

    const requestId = processRequestRef.current + 1;
    processRequestRef.current = requestId;
    const processingState: RequirementState = {
      ...currentSession.requirementState,
      status: "processing",
      pendingAction: "process"
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

        dispatch({
          type: "apply_process_error",
          requirementId,
          error: toErrorMessage(error),
          now: nowString()
        });
      });
  }, []);

  const confirmRequirement = useCallback(() => {
    const currentSession = sessionRef.current;
    if (!currentSession) {
      return;
    }

    const state = currentSession.requirementState;
    const codingPrompt = createCodingPromptFromConfirmedRequirement(state);
    dispatch({
      type: "mark_finalizing",
      now: nowString()
    });

    const selectedProjectPath = projectPathRef.current;
    if (!selectedProjectPath) {
      dispatch({
        type: "confirm_requirement",
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
          type: "confirm_requirement",
          savedRequirementDocumentPath: savedDocument.path,
          now: nowString()
        });
      })
      .catch((error) => {
        dispatch({
          type: "confirm_requirement",
          error: toErrorMessage(error),
          now: nowString()
        });
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

  if (state.status === "collecting") {
    return {
      canUseMic: true,
      canFinishTurn: state.utterances.some((utterance) => utterance.source === "voice"),
      transcriptSource: "voice"
    };
  }

  if (state.status === "clarifying") {
    return {
      canUseMic: true,
      canFinishTurn: Boolean(state.activeQuestionId && state.answeredQuestions.some((question) => question.id === state.activeQuestionId)),
      transcriptSource: "clarification_answer"
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

function upsertAnsweredQuestionForUtterance(state: RequirementState, utterance: RequirementUtterance) {
  if (utterance.source !== "clarification_answer" || !state.activeQuestionId) {
    return state.answeredQuestions;
  }

  const question = state.openQuestions.find((item) => item.id === state.activeQuestionId);
  if (!question) {
    return state.answeredQuestions;
  }

  const answeredQuestion: RequirementQuestion = {
    ...question,
    answer: utterance.text
  };
  const existingIndex = state.answeredQuestions.findIndex((item) => item.id === answeredQuestion.id);
  if (existingIndex < 0) {
    return [...state.answeredQuestions, answeredQuestion];
  }

  const nextQuestions = [...state.answeredQuestions];
  nextQuestions[existingIndex] = answeredQuestion;
  return nextQuestions;
}

function appendUnique(values: string[], value: string) {
  return values.includes(value) ? values : [...values, value];
}

function mergeUnique(current: string[], incoming: string[]) {
  return incoming.reduce((values, value) => {
    const trimmed = value.trim();
    return trimmed && !values.includes(trimmed) ? [...values, trimmed] : values;
  }, current);
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
