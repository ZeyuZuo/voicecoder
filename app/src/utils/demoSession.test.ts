import test from "node:test";
import assert from "node:assert/strict";
import type { DemoSession, DevServerLifecycleEventEnvelope, RequirementUtterance } from "../types/app";
import {
  createInitialDemoPrompt,
  createDemoSession,
  demoSessionReducer,
  demoSessionStoreReducer,
  detectDevServerOutputIssue,
  formatDevServerPreviewError
} from "./demoSession";

test("creates a demo session that is ready to start", () => {
  const session = createTestSession();

  assert.equal(session.status, "ready_to_start");
  assert.equal(session.projectPath, "/tmp/demo");
  assert.equal(session.requirementId, "requirement-1");
  assert.equal(session.initialCodingPrompt, "请实现第一版 demo。");
});

test("initial demo prompt requires npm dev server compatibility", () => {
  const prompt = createInitialDemoPrompt(createTestSession());

  assert.ok(/Node\.js 前端项目/.test(prompt));
  assert.ok(/package\.json/.test(prompt));
  assert.ok(/scripts\.dev/.test(prompt));
  assert.ok(/npm run dev/.test(prompt));
  assert.ok(/不要只生成裸 index\.html/.test(prompt));
  assert.ok(/不要启动 dev server/.test(prompt));
  assert.ok(/VoiceCoder 会在后台统一启动 npm run dev/.test(prompt));
});

test("store reducer creates a demo session from confirmed requirement input", () => {
  const session = demoSessionStoreReducer(undefined, {
    type: "create_demo_session",
    input: {
      projectPath: "/tmp/demo",
      requirementId: "requirement-1",
      initialRequirementDocument: "目标：生成 demo。",
      initialCodingPrompt: "请实现 demo。",
      now: "1"
    }
  });

  assert.equal(session?.status, "ready_to_start");
  assert.equal(session?.projectPath, "/tmp/demo");
  assert.equal(session?.requirementId, "requirement-1");
});

test("store reducer keeps the same demo session for duplicate requirement input", () => {
  const session = createTestSession();
  const next = demoSessionStoreReducer(session, {
    type: "create_demo_session",
    input: {
      projectPath: session.projectPath,
      requirementId: session.requirementId,
      initialRequirementDocument: "目标：变化不应重建。",
      initialCodingPrompt: "请实现变化。",
      now: "2"
    }
  });

  assert.equal(next, session);
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

test("agent events update turn metadata and de-duplicate changed files", () => {
  const running = demoSessionReducer(createTestSession(), {
    type: "start_agent_run",
    kind: "initial_build",
    prompt: "生成 demo",
    now: "2"
  });
  const run = running.runs[0];
  const withTurn = demoSessionReducer(running, {
    type: "append_agent_event",
    runId: run.id,
    event: {
      type: "turn_started",
      turnId: "turn-1",
      createdAt: "3"
    },
    now: "3"
  });
  const withFile = demoSessionReducer(withTurn, {
    type: "append_agent_event",
    runId: run.id,
    event: {
      type: "file_change",
      path: "/tmp/demo/src/App.tsx",
      createdAt: "4"
    },
    now: "4"
  });
  const withDuplicateFile = demoSessionReducer(withFile, {
    type: "append_agent_event",
    runId: run.id,
    event: {
      type: "file_change",
      path: "/tmp/demo/src/App.tsx",
      createdAt: "5"
    },
    now: "5"
  });

  assert.equal(withTurn.runs[0].codexTurnId, "turn-1");
  assert.deepEqual(withDuplicateFile.runs[0].changedFiles, ["/tmp/demo/src/App.tsx"]);
});

test("ready dev server event can attach the preview URL after initial build", () => {
  const previewReady = completeInitialBuild(createTestSession());
  const withPreview = demoSessionReducer(previewReady, {
    type: "set_preview_url",
    currentPreviewUrl: "http://localhost:5173",
    now: "4"
  });

  assert.equal(withPreview.status, "preview_ready");
  assert.equal(withPreview.currentPreviewUrl, "http://localhost:5173");
  assert.equal(withPreview.updatedAt, "4");
});

test("stopping preview clears the current URL while keeping the generated demo ready", () => {
  const previewReady = completeInitialBuild(createTestSession());
  const withPreview = demoSessionReducer(previewReady, {
    type: "set_preview_url",
    currentPreviewUrl: "http://localhost:5173",
    now: "4"
  });
  const stopped = demoSessionReducer(withPreview, {
    type: "stop_preview",
    now: "5"
  });

  assert.equal(stopped.status, "preview_ready");
  assert.equal(stopped.currentPreviewUrl, undefined);
  assert.equal(stopped.error, undefined);
  assert.equal(stopped.updatedAt, "5");
});

test("preview URL updates are ignored before the initial build has completed", () => {
  const session = createTestSession();
  const withPreview = demoSessionReducer(session, {
    type: "set_preview_url",
    currentPreviewUrl: "http://localhost:5173",
    now: "2"
  });

  assert.equal(withPreview.status, "ready_to_start");
  assert.equal(withPreview.currentPreviewUrl, undefined);
});

test("preview URL can update while collecting feedback", () => {
  const previewReady = completeInitialBuild(createTestSession());
  const listening = demoSessionReducer(previewReady, {
    type: "start_feedback_listening",
    now: "4"
  });
  const withPreview = demoSessionReducer(listening, {
    type: "set_preview_url",
    currentPreviewUrl: "http://localhost:5173",
    now: "5"
  });

  assert.equal(withPreview.status, "preview_ready");
  assert.equal(withPreview.currentPreviewUrl, "http://localhost:5173");
});

test("preview failure moves the session to error until a preview URL exists", () => {
  const previewReady = completeInitialBuild(createTestSession());
  const failed = demoSessionReducer(previewReady, {
    type: "fail_preview",
    error: "dev server 启动失败：端口已被占用。",
    now: "4"
  });
  const withPreview = demoSessionReducer(previewReady, {
    type: "set_preview_url",
    currentPreviewUrl: "http://localhost:5173",
    now: "4"
  });
  const ignoredFailure = demoSessionReducer(withPreview, {
    type: "fail_preview",
    error: "dev server stopped",
    now: "5"
  });

  assert.equal(failed.status, "error");
  assert.equal(failed.error, "dev server 启动失败：端口已被占用。");
  assert.equal(ignoredFailure.status, "preview_ready");
  assert.equal(ignoredFailure.currentPreviewUrl, "http://localhost:5173");
});

test("dev server output issue detection recognizes occupied ports", () => {
  assert.equal(
    detectDevServerOutputIssue("Error: listen EADDRINUSE: address already in use :::5173"),
    "dev server 启动失败：端口已被占用。"
  );
  assert.equal(detectDevServerOutputIssue("Local: http://localhost:5173/"), undefined);
});

test("dev server preview errors are normalized for UI display", () => {
  assert.equal(
    formatDevServerPreviewError(devServerEnvelope({
      type: "error",
      message: "npm not found"
    })),
    "dev server 出错：npm not found"
  );
  assert.equal(
    formatDevServerPreviewError(devServerEnvelope({
      type: "stopped",
      reason: "exited",
      exitCode: 1
    })),
    "dev server 在预览 URL 就绪前退出，退出码 1。"
  );
  assert.equal(formatDevServerPreviewError(devServerEnvelope({
    type: "ready",
    url: "http://localhost:5173"
  })), undefined);
  assert.equal(formatDevServerPreviewError(devServerEnvelope({
    type: "stopped",
    reason: "user"
  })), undefined);
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

function devServerEnvelope(event: DevServerLifecycleEventEnvelope["event"]): DevServerLifecycleEventEnvelope {
  return {
    sessionId: "dev_server_1",
    projectPath: "/tmp/demo",
    event,
    occurredAt: "1"
  };
}
