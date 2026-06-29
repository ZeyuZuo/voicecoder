import test from "node:test";
import assert from "node:assert/strict";
import type {
  RequirementProcessingResult,
  RequirementState,
  RequirementSummaryResult,
  VoiceRequirementSession,
  VoiceTranscriptSegment
} from "../types/app";
import {
  createVoiceRequirementSession,
  getVoiceInputPermission,
  requirementSessionReducer,
  shouldApplyLiveSummaryResult
} from "./requirementState";

test("final transcript does not create a requirement session before voice start", () => {
  const next = requirementSessionReducer(undefined, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "实现语音需求状态机"),
    now: "2"
  });

  assert.equal(next, undefined);
});

test("starting voice from idle creates a listening requirement session", () => {
  const next = requirementSessionReducer(undefined, {
    type: "start_voice_session",
    voiceSessionId: "voice-1",
    now: "1"
  });

  assert.equal(next?.requirementState.status, "listening");
  assert.deepEqual(next?.voiceSessionIds, ["voice-1"]);
});

test("document ready does not allow voice start to return to listening", () => {
  const session = sessionWithState("document_ready");
  const next = requirementSessionReducer(session, {
    type: "start_voice_session",
    voiceSessionId: "voice-2",
    now: "2"
  });

  assert.equal(next?.requirementState.status, "document_ready");
  assert.deepEqual(next?.voiceSessionIds, ["voice-1"]);
});

test("document ready ignores stray transcripts", () => {
  const session = sessionWithState("document_ready");
  const next = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "不要追加"),
    now: "2"
  });

  assert.equal(next?.requirementState.utterances.length, 0);
  assert.equal(next?.requirementState.status, "document_ready");
});

test("listening voice turn records transcript as voice utterance", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const next = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "帮我做一个网页端贪吃蛇"),
    now: "2"
  });

  assert.equal(next?.requirementState.status, "listening");
  assert.equal(next?.requirementState.utterances[0].source, "voice");
  assert.equal(next?.requirementState.utterances[0].text, "帮我做一个网页端贪吃蛇");
});

test("voice transcript trims leading punctuation and repeated whitespace", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const next = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", " ，  我想   做一个  网页端贪吃蛇  "),
    now: "2"
  });

  assert.equal(next?.requirementState.utterances[0].text, "我想 做一个 网页端贪吃蛇");
});

test("voice transcript ignores punctuation-only final segment", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const next = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "！？ ;: ，。"),
    now: "2"
  });

  assert.equal(next?.requirementState.utterances.length, 0);
});

test("live understanding updates summary and open gaps without leaving listening", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const next = requirementSessionReducer(session, {
    type: "apply_live_summary",
    requirementId: session.requirementState.id,
    result: summaryResult({
      summary: "用户想做网页端贪吃蛇游戏。",
      openGaps: [
        {
          question: "还缺少胜负判定和验收标准。",
          reason: "影响实现范围。",
          severity: "blocking"
        }
      ]
    }),
    now: "2"
  });

  assert.equal(next?.requirementState.status, "listening");
  assert.equal(next?.requirementState.summary, "用户想做网页端贪吃蛇游戏。");
  assert.equal(next?.requirementState.openGaps.length, 1);
  assert.equal(next?.requirementState.openGaps[0].status, "open");
});

test("live summary result ordering accepts newer coverage and rejects stale rewrites", () => {
  assert.equal(
    shouldApplyLiveSummaryResult(
      {
        requestId: 2,
        utteranceCount: 2
      },
      {
        requestId: 1,
        utteranceCount: 1
      }
    ),
    true
  );
  assert.equal(
    shouldApplyLiveSummaryResult(
      {
        requestId: 1,
        utteranceCount: 1
      },
      {
        requestId: 2,
        utteranceCount: 1
      }
    ),
    false
  );
  assert.equal(
    shouldApplyLiveSummaryResult(
      {
        requestId: 3,
        utteranceCount: 1
      },
      {
        requestId: 2,
        utteranceCount: 2
      }
    ),
    false
  );
});

test("finish action enters finalizing and final LLM result enters document ready", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const finalizing = requirementSessionReducer(session, {
    type: "mark_finalizing",
    now: "2"
  });
  const processed = requirementSessionReducer(finalizing, {
    type: "apply_process_result",
    requirementId: finalizing?.requirementState.id ?? "",
    result: processingResult({
      requirementDocumentDraft: "目标：实现网页端贪吃蛇游戏。"
    }),
    now: "3"
  });

  assert.equal(finalizing?.requirementState.status, "finalizing");
  assert.equal(processed?.requirementState.status, "document_ready");
  assert.equal(processed?.requirementState.requirementDocument, "目标：实现网页端贪吃蛇游戏。");
  assert.ok(/网页端贪吃蛇/.test(processed?.requirementState.codingPrompt ?? ""));
});

test("processing errors restore listening so user can retry finalization", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const finalizing = requirementSessionReducer(session, {
    type: "mark_finalizing",
    now: "2"
  });
  const failed = requirementSessionReducer(finalizing, {
    type: "apply_process_error",
    requirementId: finalizing?.requirementState.id ?? "",
    error: "LLM 请求失败",
    now: "3"
  });

  assert.equal(failed?.requirementState.status, "listening");
  assert.equal(failed?.requirementState.error, "LLM 请求失败");
});

test("document save result keeps document ready and stores path", () => {
  const session = sessionWithState("document_ready");
  const next = requirementSessionReducer(session, {
    type: "apply_document_save_result",
    requirementId: session.requirementState.id,
    savedRequirementDocumentPath: "/tmp/.voicecoder/voice_requirements.md",
    now: "2"
  });

  assert.equal(next?.requirementState.status, "document_ready");
  assert.equal(next?.requirementState.savedRequirementDocumentPath, "/tmp/.voicecoder/voice_requirements.md");
});

test("confirming document ready marks requirement confirmed", () => {
  const session = sessionWithState("document_ready", {
    requirementDocument: "目标：实现网页端贪吃蛇游戏。",
    codingPrompt: "请实现网页端贪吃蛇游戏。"
  });
  const next = requirementSessionReducer(session, {
    type: "confirm_requirement",
    now: "2"
  });

  assert.equal(next?.requirementState.status, "confirmed");
  assert.equal(next?.requirementState.codingPrompt, "请实现网页端贪吃蛇游戏。");
});

test("voice input permission is centralized by requirement state", () => {
  assert.deepEqual(getVoiceInputPermission(undefined), {
    canUseMic: true,
    canFinishTurn: false,
    transcriptSource: "voice"
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("listening").requirementState), {
    canUseMic: true,
    canFinishTurn: false,
    transcriptSource: "voice"
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("finalizing").requirementState), {
    canUseMic: false,
    canFinishTurn: false
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("document_ready").requirementState), {
    canUseMic: false,
    canFinishTurn: false
  });
  assert.deepEqual(getVoiceInputPermission(sessionWithState("confirmed").requirementState), {
    canUseMic: false,
    canFinishTurn: false
  });
});

test("listening turn can finish only after voice transcript arrives", () => {
  const session = createVoiceRequirementSession("voice-1", "1");
  const withTranscript = requirementSessionReducer(session, {
    type: "append_voice_transcript",
    segment: finalSegment("voice-1", "seg-1", "实现语音需求确认"),
    now: "2"
  });

  assert.equal(getVoiceInputPermission(session.requirementState).canFinishTurn, false);
  assert.equal(getVoiceInputPermission(withTranscript?.requirementState).canFinishTurn, true);
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

function processingResult(overrides: Partial<RequirementProcessingResult> = {}): RequirementProcessingResult {
  return {
    summary: "实现网页端贪吃蛇游戏",
    requirementDocumentDraft: "目标：实现网页端贪吃蛇游戏。",
    confirmedFacts: [],
    constraints: [],
    acceptanceCriteria: [],
    outOfScope: [],
    risks: [],
    questions: [],
    readyToConfirm: true,
    ...overrides
  };
}

function summaryResult(overrides: Partial<RequirementSummaryResult> = {}): RequirementSummaryResult {
  return {
    summary: "用户正在描述需求。",
    confirmedFacts: [],
    constraints: [],
    acceptanceCriteria: [],
    outOfScope: [],
    risks: [],
    openGaps: [],
    ...overrides
  };
}
