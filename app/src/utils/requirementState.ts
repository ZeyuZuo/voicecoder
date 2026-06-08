import { useEffect, useReducer } from "react";
import type {
  RequirementState,
  RequirementUtterance,
  VoiceRequirementSession,
  VoiceTranscriptSegment
} from "../types/app";
import { createId } from "./project";

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
    };

export function createRequirementState(now: string): RequirementState {
  return {
    id: createId("requirement"),
    status: "collecting",
    utterances: [],
    summary: "",
    confirmedFacts: [],
    constraints: [],
    openQuestions: [],
    answeredQuestions: [],
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

    return {
      ...session,
      voiceSessionIds: appendUnique(session.voiceSessionIds, action.voiceSessionId),
      requirementState: {
        ...session.requirementState,
        status: "collecting",
        updatedAt: action.now
      },
      endedAt: undefined
    };
  }

  if (!action.segment.isFinal) {
    return session;
  }

  const text = action.segment.text.trim();
  if (!text) {
    return session;
  }

  const nextSession = session ?? createVoiceRequirementSession(action.segment.sessionId, action.now);
  const utterance = transcriptSegmentToUtterance(action.segment, text);
  const utterances = upsertUtteranceByTranscriptId(nextSession.requirementState.utterances, utterance);

  return {
    ...nextSession,
    voiceSessionIds: appendUnique(nextSession.voiceSessionIds, action.segment.sessionId),
    requirementState: {
      ...nextSession.requirementState,
      status: "collecting",
      utterances,
      updatedAt: action.now
    }
  };
}

export function useVoiceRequirementSession(voice: {
  sessionId?: string;
  segments: VoiceTranscriptSegment[];
}) {
  const [session, dispatch] = useReducer(requirementSessionReducer, undefined);

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
      dispatch({
        type: "append_voice_transcript",
        segment,
        now: nowString()
      });
    }
  }, [voice.segments]);

  return session;
}

function transcriptSegmentToUtterance(segment: VoiceTranscriptSegment, text: string): RequirementUtterance {
  return {
    id: `utterance_${segment.id}`,
    source: "voice",
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

function nowString() {
  return Date.now().toString();
}
