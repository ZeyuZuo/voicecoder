import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
  RequirementQuestion,
  RequirementState,
  RequirementUtterance,
  VoiceRequirementSession,
  VoiceSessionStatus,
  VoiceTranscriptSegment
} from "../types/app";
import { createId } from "./project";

const LIVE_SUMMARY_UTTERANCE_BATCH = 3;
const LIVE_SUMMARY_DEBOUNCE_MS = 7000;

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
    }
  | {
      type: "mark_processing";
      now: string;
    }
  | {
      type: "process_user_turn";
      now: string;
    }
  | {
      type: "confirm_requirement";
      now: string;
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
        updatedAt: action.now
      }
    };
  }

  if (action.type === "apply_live_summary") {
    const nextState = createLocalRequirementUnderstanding(session.requirementState, action.now);

    return {
      ...session,
      requirementState: {
        ...nextState,
        pendingAction: undefined,
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
        updatedAt: action.now
      }
    };
  }

  if (action.type === "process_user_turn") {
    const nextState = createLocalRequirementUnderstanding(session.requirementState, action.now);
    const needsClarification = nextState.utterances.length > 0 && nextState.acceptanceCriteria.length === 0;
    const openQuestions: RequirementQuestion[] = needsClarification ? getNextClarificationQuestions(nextState) : [];

    return {
      ...session,
      endedAt: action.now,
      requirementState: {
        ...nextState,
        openQuestions,
        activeQuestionId: openQuestions.find((question) => question.blocksCoding)?.id,
        status: openQuestions.some((question) => question.blocksCoding) ? "clarifying" : "ready_to_confirm",
        pendingAction: undefined,
        updatedAt: action.now
      }
    };
  }

  if (action.type === "confirm_requirement") {
    const nextState = createLocalRequirementUnderstanding(session.requirementState, action.now);

    return {
      ...session,
      requirementState: {
        ...nextState,
        status: "confirmed",
        codingPrompt: createLocalCodingPrompt(nextState),
        pendingAction: undefined,
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
      pendingAction: undefined,
      updatedAt: action.now
    }
  };
}

export function useVoiceRequirementSession(voice: {
  sessionId?: string;
  status: VoiceSessionStatus;
  segments: VoiceTranscriptSegment[];
}): VoiceRequirementController {
  const [session, dispatch] = useReducer(requirementSessionReducer, undefined);
  const sessionRef = useRef<VoiceRequirementSession | undefined>(undefined);
  const processedSegmentTextsRef = useRef<Map<string, string>>(new Map());
  const lastSummarizedUtteranceCountRef = useRef(0);
  const summaryTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>();

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

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

    window.setTimeout(() => {
      const latestSession = sessionRef.current;
      if (!latestSession) {
        return;
      }

      lastSummarizedUtteranceCountRef.current = latestSession.requirementState.utterances.length;
      dispatch({
        type: "apply_live_summary",
        now: nowString()
      });
    }, 420);
  }, []);

  useEffect(() => {
    if (!session || session.requirementState.status === "confirmed") {
      return;
    }

    const utteranceCount = session.requirementState.utterances.length;
    const unsummarizedCount = utteranceCount - lastSummarizedUtteranceCountRef.current;
    if (unsummarizedCount <= 0 || session.requirementState.pendingAction) {
      return;
    }

    if (unsummarizedCount >= LIVE_SUMMARY_UTTERANCE_BATCH) {
      clearSummaryTimer(summaryTimerRef.current);
      summaryTimerRef.current = undefined;
      applyLiveSummary();
      return;
    }

    clearSummaryTimer(summaryTimerRef.current);
    summaryTimerRef.current = setTimeout(applyLiveSummary, LIVE_SUMMARY_DEBOUNCE_MS);

    return () => {
      clearSummaryTimer(summaryTimerRef.current);
    };
  }, [applyLiveSummary, session]);

  const finishUserTurn = useCallback(() => {
    clearSummaryTimer(summaryTimerRef.current);
    summaryTimerRef.current = undefined;
    lastSummarizedUtteranceCountRef.current = sessionRef.current?.requirementState.utterances.length ?? 0;
    dispatch({
      type: "mark_processing",
      now: nowString()
    });

    window.setTimeout(() => {
      dispatch({
        type: "process_user_turn",
        now: nowString()
      });
    }, 360);
  }, []);

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

function createLocalRequirementUnderstanding(state: RequirementState, now: string): RequirementState {
  const texts = state.utterances.map((utterance) => utterance.text.trim()).filter(Boolean);
  const clarificationAnswers = state.utterances
    .filter((utterance) => utterance.source === "clarification_answer")
    .map((utterance) => utterance.text.trim())
    .filter(Boolean);
  const summary = createLocalSummary(texts);
  const acceptanceCriteria = state.acceptanceCriteria.length
    ? state.acceptanceCriteria
    : deriveAcceptanceCriteria(texts, clarificationAnswers);

  return {
    ...state,
    summary,
    requirementDocument: createLocalRequirementDocument(summary, texts, acceptanceCriteria),
    confirmedFacts: texts.slice(0, 4),
    acceptanceCriteria,
    updatedAt: now
  };
}

function createLocalSummary(texts: string[]) {
  if (!texts.length) {
    return "等待语音输入。";
  }

  const joined = texts.join(" ");
  return joined.length > 180 ? `${joined.slice(0, 180)}...` : joined;
}

function createLocalRequirementDocument(summary: string, texts: string[], acceptanceCriteria: string[]) {
  if (!texts.length) {
    return "";
  }

  const sections = [
    `目标：${summary}`,
    `原始语音要点：\n${texts.slice(-6).map((text) => `- ${text}`).join("\n")}`
  ];

  if (acceptanceCriteria.length) {
    sections.push(`验收标准：\n${acceptanceCriteria.map((item) => `- ${item}`).join("\n")}`);
  }

  return sections.join("\n\n");
}

function deriveAcceptanceCriteria(texts: string[], clarificationAnswers: string[]) {
  const latestClarificationAnswer = clarificationAnswers[clarificationAnswers.length - 1];
  if (latestClarificationAnswer) {
    return [`用户补充的关键验收标准：${latestClarificationAnswer}`];
  }

  const joined = texts.join(" ");
  if (!joined) {
    return [];
  }

  if (/验收|测试|通过|完成|效果|标准/.test(joined)) {
    return ["按用户语音描述的关键行为完成实现，并保留可验证的验收结果。"];
  }

  return [];
}

function getNextClarificationQuestions(state: RequirementState) {
  const existingBlockingQuestions = state.openQuestions.filter((question) => question.blocksCoding);
  if (existingBlockingQuestions.length) {
    return existingBlockingQuestions.slice(0, 3);
  }

  return [
    {
      id: createId("question"),
      question: "这次需求完成后，最重要的验收标准是什么？",
      reason: "缺少验收标准会影响后续实现范围和测试方式。",
      blocksCoding: true
    }
  ];
}

function createLocalCodingPrompt(state: RequirementState) {
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
