import test from "node:test";
import assert from "node:assert/strict";
import type { DemoSession, RequirementUtterance } from "../types/app";
import { createDemoSession, demoSessionReducer } from "./demoSession";

test("creates a demo session that is ready to start", () => {
  const session = createTestSession();

  assert.equal(session.status, "ready_to_start");
  assert.equal(session.projectPath, "/tmp/demo");
  assert.equal(session.requirementId, "requirement-1");
  assert.equal(session.initialCodingPrompt, "请实现第一版 demo。");
});

test("starts and completes the initial build run", () => {
  const session = createTestSession();
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    kind: "initial_build",
    prompt: session.initialCodingPrompt,
    now: "2"
  });
  const run = running.runs[0];
  const completed = demoSessionReducer(running, {
    type: "complete_agent_run",
    runId: run.id,
    finalMessage: "第一版 demo 已生成。",
    changedFiles: ["/tmp/demo/src/App.tsx"],
    currentPreviewUrl: "http://localhost:5173",
    now: "3"
  });

  assert.equal(running.status, "agent_running");
  assert.equal(run.status, "running");
  assert.equal(completed.status, "preview_ready");
  assert.equal(completed.runs[0].status, "succeeded");
  assert.equal(completed.runs[0].finalMessage, "第一版 demo 已生成。");
  assert.deepEqual(completed.runs[0].changedFiles, ["/tmp/demo/src/App.tsx"]);
  assert.equal(completed.currentPreviewUrl, "http://localhost:5173");
});

test("agent events update thread metadata and changed files", () => {
  const running = demoSessionReducer(createTestSession(), {
    type: "start_agent_run",
    kind: "initial_build",
    prompt: "生成 demo",
    now: "2"
  });
  const run = running.runs[0];
  const withThread = demoSessionReducer(running, {
    type: "append_agent_event",
    runId: run.id,
    event: {
      type: "thread_started",
      threadId: "thread-1",
      createdAt: "3"
    },
    now: "3"
  });
  const withFile = demoSessionReducer(withThread, {
    type: "append_agent_event",
    runId: run.id,
    event: {
      type: "file_change",
      path: "/tmp/demo/src/App.tsx",
      createdAt: "4"
    },
    now: "4"
  });

  assert.equal(withThread.codexThreadId, "thread-1");
  assert.equal(withThread.runs[0].codexThreadId, "thread-1");
  assert.equal(withFile.runs[0].events.length, 2);
  assert.deepEqual(withFile.runs[0].changedFiles, ["/tmp/demo/src/App.tsx"]);
});

test("feedback result can start a follow-up change run", () => {
  const previewReady = completeInitialBuild(createTestSession());
  const listening = demoSessionReducer(previewReady, {
    type: "start_feedback_listening",
    now: "4"
  });
  const processed = demoSessionReducer(listening, {
    type: "apply_feedback_result",
    utterances: [utterance("按钮太小，颜色换成蓝色。")],
    summary: "用户希望调大按钮并改成蓝色。",
    modificationPrompt: "将主按钮调大，并把按钮颜色改成蓝色。",
    now: "5"
  });
  const feedbackTurn = processed.feedbackTurns[0];
  const modifying = demoSessionReducer(processed, {
    type: "start_agent_run",
    kind: "feedback_change",
    prompt: feedbackTurn.modificationPrompt,
    feedbackTurnId: feedbackTurn.id,
    now: "6"
  });

  assert.equal(listening.status, "feedback_listening");
  assert.equal(processed.status, "feedback_processing");
  assert.equal(feedbackTurn.summary, "用户希望调大按钮并改成蓝色。");
  assert.equal(modifying.status, "agent_modifying");
  assert.equal(modifying.runs[1].kind, "feedback_change");
  assert.equal(modifying.feedbackTurns[0].linkedAgentRunId, modifying.runs[1].id);
});

test("failed active run moves the session to error", () => {
  const running = demoSessionReducer(createTestSession(), {
    type: "start_agent_run",
    kind: "initial_build",
    prompt: "生成 demo",
    now: "2"
  });
  const failed = demoSessionReducer(running, {
    type: "fail_agent_run",
    runId: running.runs[0].id,
    error: "Codex app-server exited.",
    now: "3"
  });

  assert.equal(failed.status, "error");
  assert.equal(failed.error, "Codex app-server exited.");
  assert.equal(failed.runs[0].status, "failed");
});

function createTestSession(): DemoSession {
  return createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "目标：生成一个交互式 demo。",
    initialCodingPrompt: "请实现第一版 demo。",
    now: "1"
  });
}

function completeInitialBuild(session: DemoSession): DemoSession {
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    kind: "initial_build",
    prompt: session.initialCodingPrompt,
    now: "2"
  });

  return demoSessionReducer(running, {
    type: "complete_agent_run",
    runId: running.runs[0].id,
    finalMessage: "done",
    now: "3"
  });
}

function utterance(text: string): RequirementUtterance {
  return {
    id: `utterance-${text}`,
    source: "voice",
    text,
    createdAt: "1"
  };
}
