import test from "node:test";
import assert from "node:assert/strict";
import type { RequirementState, VoiceRequirementSession, VoiceTranscriptSegment } from "../types/app";
import {
  createVoiceRequirementSession,
  getVoiceInputPermission,
  requirementSessionReducer
} from "./requirementState";

test("final transcript does not create a requirement session before voice start", () => {
  const next = requirementSessionReducer(undefined, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "实现语音需求状态机"),
    now: "2"
  });

  assert.equal(next, undefined);
});

test("starting voice from idle creates a collecting requirement session", () => {
  const next = requirementSessionReducer(undefined, {
    type: "start_voice_session",
    voiceSessionId: "voice-1",
    now: "1"
  });

  assert.equal(next?.requirementState.status, "collecting");
  assert.deepEqual(next?.voiceSessionIds, ["voice-1"]);
});

test("ready to confirm does not allow voice start to return to collecting", () => {
  const session = sessionWithState("ready_to_confirm");
  const next = requirementSessionReducer(session, {
    type: "start_voice_session",
    voiceSessionId: "voice-2",
    now: "2"
  });

  assert.equal(next?.requirementState.status, "ready_to_confirm");
  assert.deepEqual(next?.voiceSessionIds, ["voice-1"]);
});

test("ready to confirm ignores stray transcripts", () => {
  const session = sessionWithState("ready_to_confirm");
  const next = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "不要追加"),
    now: "2"
  });

  assert.equal(next?.requirementState.utterances.length, 0);
  assert.equal(next?.requirementState.status, "ready_to_confirm");
});

test("clarifying voice turn records transcript as clarification answer", () => {
  const session = sessionWithState("clarifying", {
    openQuestions: [
      {
        id: "q1",
        question: "验收标准是什么？",
        reason: "影响测试范围",
        blocksCoding: true
      }
    ],
    activeQuestionId: "q1"
  });
  const started = requirementSessionReducer(session, {
    type: "start_voice_session",
    voiceSessionId: "voice-2",
    now: "2"
  });
  const next = requirementSessionReducer(started, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-2", "seg-1", "必须生成需求文档"),
    now: "3"
  });

  assert.equal(next?.requirementState.status, "clarifying");
  assert.equal(next?.requirementState.utterances[0].source, "clarification_answer");
  assert.equal(next?.requirementState.answeredQuestions[0].answer, "必须生成需求文档");
});

test("unclear processing result enters clarification loop", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const withTranscript = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "做一个语音需求功能"),
    now: "2"
  });
  const processing = requirementSessionReducer(withTranscript, {
    type: "process_user_turn",
    now: "3"
  });

  assert.equal(processing?.requirementState.status, "clarifying");
  assert.ok(processing?.requirementState.activeQuestionId);
  assert.equal(processing?.requirementState.openQuestions.length, 1);
});

test("clarification answer can resolve loop into ready to confirm", () => {
  const clarifying = sessionWithState("clarifying", {
    openQuestions: [
      {
        id: "q1",
        question: "验收标准是什么？",
        reason: "影响测试范围",
        blocksCoding: true
      }
    ],
    activeQuestionId: "q1"
  });
  const withAnswer = requirementSessionReducer(clarifying, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "必须展示需求文档并且确认前不能编码"),
    now: "2"
  });
  const processed = requirementSessionReducer(withAnswer, {
    type: "process_user_turn",
    now: "3"
  });

  assert.equal(processed?.requirementState.status, "ready_to_confirm");
  assert.equal(processed?.requirementState.openQuestions.length, 0);
});

test("voice input permission is centralized by requirement state", () => {
  assert.deepEqual(getVoiceInputPermission(undefined), {
    canUseMic: true,
    canFinishTurn: false,
    transcriptSource: "voice"
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("collecting").requirementState), {
    canUseMic: true,
    canFinishTurn: false,
    transcriptSource: "voice"
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("clarifying").requirementState), {
    canUseMic: true,
    canFinishTurn: false,
    transcriptSource: "clarification_answer"
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("processing").requirementState), {
    canUseMic: false,
    canFinishTurn: false
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("ready_to_confirm").requirementState), {
    canUseMic: false,
    canFinishTurn: false
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("confirmed").requirementState), {
    canUseMic: false,
    canFinishTurn: false
  });
});

test("collecting turn can finish only after voice transcript arrives", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const withTranscript = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "实现语音需求确认"),
    now: "2"
  });

  assert.equal(getVoiceInputPermission(session.requirementState).canFinishTurn, false);
  assert.equal(getVoiceInputPermission(withTranscript?.requirementState).canFinishTurn, true);
});

test("clarifying turn can finish only after current question is answered", () => {
  const clarifying = sessionWithState("clarifying", {
    openQuestions: [
      {
        id: "q1",
        question: "验收标准是什么？",
        reason: "影响测试范围",
        blocksCoding: true
      }
    ],
    activeQuestionId: "q1"
  });
  const withAnswer = requirementSessionReducer(clarifying, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "必须展示需求文档"),
    now: "2"
  });

  assert.equal(getVoiceInputPermission(clarifying.requirementState).canFinishTurn, false);
  assert.equal(getVoiceInputPermission(withAnswer?.requirementState).canFinishTurn, true);
});

function sessionWithState(
  status: RequirementState["status"],
  overrides: Partial<RequirementState> = {}
): VoiceRequirementSession {
  const session = createVoiceRequirementSession("voice-1", "1");

  return {
    ...session,
    requirementState: {
      ...session.requirementState,
      status,
      ...overrides
    }
  };
}

function finalSegment(sessionId: string, id: string, text: string): VoiceTranscriptSegment {
  return {
    id,
    sessionId,
    text,
    isFinal: true,
    createdAt: "1"
  };
}
