import test from "node:test";
import assert from "node:assert/strict";
import type { DemoSession, DevServerLifecycleEventEnvelope, RequirementUtterance } from "../types/app";
import { AGENT_COMMAND_OUTPUT_TAIL_LIMIT } from "./agentProgress";
import {
  AGENT_EVENT_BATCH_INTERVAL_MS,
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

test("agent run start records Codex runtime and silent approval settings", () => {
  const running = demoSessionReducer(createTestSession(), {
    type: "start_agent_run",
    runId: "run-1",
    kind: "initial_build",
    prompt: "生成 demo",
    now: "2"
  });
  const started = demoSessionReducer(running, {
    type: "mark_agent_run_started",
    runId: "run-1",
    codexThreadId: "thread-1",
    codexTurnId: "turn-1",
    runtime: {
      provider: "codex_app_server",
      version: "codex-cli 0.144.1",
      transport: "stdio",
      sandbox: "workspace-write",
      approvalPolicy: "on-request",
      approvalsReviewer: "auto_review",
      transportLogPath: "/tmp/demo/.voicecoder/agent_run_run-1_app_server.jsonl"
    },
    now: "3"
  });

  assert.equal(started.runs[0].codexThreadId, "thread-1");
  assert.equal(started.runs[0].codexTurnId, "turn-1");
  assert.deepEqual(started.runs[0].runtime, {
    provider: "codex_app_server",
    version: "codex-cli 0.144.1",
    transport: "stdio",
    sandbox: "workspace-write",
    approvalPolicy: "on-request",
    approvalsReviewer: "auto_review",
    transportLogPath: "/tmp/demo/.voicecoder/agent_run_run-1_app_server.jsonl"
  });
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

test("item lifecycle events upsert one authoritative agent message", () => {
  const running = startTestRun();
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "6",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "message-1",
        itemType: "agentMessage",
        lifecycle: "in_progress",
        startedAt: "2",
        item: { id: "message-1", type: "agentMessage", text: "", phase: "commentary" },
        createdAt: "2"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "message-1",
        itemType: "agentMessage",
        lifecycle: "in_progress",
        method: "item/agentMessage/delta",
        delta: "正在修改",
        createdAt: "3"
      },
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "message-1",
        itemType: "agentMessage",
        lifecycle: "completed",
        completedAt: "5",
        item: {
          id: "message-1",
          type: "agentMessage",
          text: "文件修改完成。",
          phase: "final_answer"
        },
        createdAt: "6"
      }
    ]
  });

  const item = updated.runs[0].itemsById["message-1"];
  assert.deepEqual(updated.runs[0].itemOrder, ["message-1"]);
  assert.equal(item.lifecycle, "completed");
  assert.equal(item.startedAt, "2");
  assert.equal(item.completedAt, "5");
  assert.equal(item.text, "文件修改完成。");
  assert.equal(item.phase, "final_answer");
  assert.equal(updated.runs[0].messagesByItemId["message-1"], item);
});

test("completed item remains authoritative when started arrives out of order", () => {
  const running = startTestRun();
  const completedFirst = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "4",
    events: [
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "completed",
        status: "completed",
        completedAt: "4",
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run check",
          status: "completed",
          aggregatedOutput: "ok"
        },
        createdAt: "4"
      },
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2",
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "stale command",
          status: "inProgress"
        },
        createdAt: "5"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        method: "item/commandExecution/outputDelta",
        delta: " stale output",
        createdAt: "6"
      }
    ]
  });

  const item = completedFirst.runs[0].itemsById["command-1"];
  assert.equal(item.lifecycle, "completed");
  assert.equal(item.status, "completed");
  assert.equal(item.startedAt, "2");
  assert.equal(item.data.command, "npm run check");
  assert.equal(item.output, "ok");
  assert.deepEqual(completedFirst.runs[0].itemOrder, ["command-1"]);
});

test("duplicate item lifecycle notifications do not duplicate item order", () => {
  const running = startTestRun();
  const startedEvent = {
    type: "item_started" as const,
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "file-1",
    itemType: "fileChange",
    lifecycle: "in_progress" as const,
    status: "inProgress",
    startedAt: "2",
    item: { id: "file-1", type: "fileChange", status: "inProgress", changes: [] },
    createdAt: "2"
  };
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    events: [startedEvent, startedEvent],
    now: "2"
  });

  assert.deepEqual(updated.runs[0].itemOrder, ["file-1"]);
  assert.equal(Object.keys(updated.runs[0].itemsById).length, 1);
});

test("structured plans and warning severity are retained on the run", () => {
  const running = startTestRun();
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "5",
    events: [
      {
        type: "plan_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        explanation: "按顺序执行",
        plan: [
          { step: "读取代码", status: "completed" },
          { step: "实现功能", status: "inProgress" }
        ],
        createdAt: "2"
      },
      {
        type: "warning",
        message: "上下文接近限制",
        threadId: "thread-1",
        createdAt: "3"
      },
      {
        type: "error",
        message: "连接中断，正在重试",
        retryable: true,
        terminal: false,
        threadId: "thread-1",
        turnId: "turn-1",
        createdAt: "4"
      },
      {
        type: "error",
        message: "重试次数耗尽",
        retryable: false,
        terminal: true,
        threadId: "thread-1",
        turnId: "turn-1",
        createdAt: "5"
      }
    ]
  });

  assert.deepEqual(updated.runs[0].currentPlan?.steps, [
    { step: "读取代码", status: "completed" },
    { step: "实现功能", status: "inProgress" }
  ]);
  assert.equal(updated.runs[0].warnings.length, 1);
  assert.equal(updated.runs[0].errors[0].retryable, true);
  assert.equal(updated.runs[0].errors[0].terminal, false);
  assert.equal(updated.runs[0].errors[1].terminal, true);
  assert.equal(updated.runs[0].error, "重试次数耗尽");
});

test("turn completion outcomes map to succeeded cancelled and failed runs", () => {
  const completed = demoSessionReducer(startTestRun(), {
    type: "complete_agent_run",
    runId: "run-1",
    status: "completed",
    now: "3"
  });
  const interrupted = demoSessionReducer(startTestRun(), {
    type: "complete_agent_run",
    runId: "run-1",
    status: "interrupted",
    now: "3"
  });
  const failed = demoSessionReducer(startTestRun(), {
    type: "complete_agent_run",
    runId: "run-1",
    status: "failed",
    error: "turn failed",
    now: "3"
  });

  assert.equal(completed.runs[0].status, "succeeded");
  assert.equal(interrupted.runs[0].status, "cancelled");
  assert.equal(failed.runs[0].status, "failed");
  assert.equal(failed.error, "turn failed");
});

test("agent event batching interval stays within the 50 to 100ms target", () => {
  assert.ok(AGENT_EVENT_BATCH_INTERVAL_MS >= 50);
  assert.ok(AGENT_EVENT_BATCH_INTERVAL_MS <= 100);
});

test("1200 message deltas update one domain item without growing the flat timeline", () => {
  const running = startTestRun();
  const deltas = Array.from({ length: 1_200 }, (_, index) => ({
    type: "item_delta" as const,
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "message-1",
    itemType: "agentMessage",
    lifecycle: "in_progress" as const,
    method: "item/agentMessage/delta",
    delta: "x",
    createdAt: `2026-07-13T00:00:${String(index % 60).padStart(2, "0")}Z`
  }));
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:01:00Z",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "message-1",
        itemType: "agentMessage",
        lifecycle: "in_progress",
        startedAt: "2026-07-13T00:00:00Z",
        item: { id: "message-1", type: "agentMessage", text: "", phase: "commentary" },
        createdAt: "2026-07-13T00:00:00Z"
      },
      ...deltas
    ]
  });

  assert.equal(updated.runs[0].events.length, 0);
  assert.equal(updated.runs[0].itemOrder.length, 1);
  assert.equal(updated.runs[0].itemsById["message-1"].text?.length, 1_200);
  assert.equal(Object.keys(updated.runs[0].messagesByItemId).length, 1);
});

test("legacy streaming messages coalesce into one retained timeline event", () => {
  const running = startTestRun();
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:03Z",
    events: [
      { type: "agent_message", text: "正在", createdAt: "2026-07-13T00:00:01Z" },
      { type: "agent_message", text: "兼容", createdAt: "2026-07-13T00:00:02Z" },
      { type: "agent_message", text: "旧协议", createdAt: "2026-07-13T00:00:03Z" }
    ]
  });

  assert.equal(updated.runs[0].events.length, 1);
  assert.deepEqual(updated.runs[0].events[0], {
    type: "agent_message",
    text: "正在兼容旧协议",
    createdAt: "2026-07-13T00:00:03Z"
  });
});

test("file patches update per-file stats and completed snapshot replaces interim changes", () => {
  const running = startTestRun();
  const withPatch = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "4",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "file-1",
        itemType: "fileChange",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2",
        item: { id: "file-1", type: "fileChange", status: "inProgress", changes: [] },
        createdAt: "2"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "file-1",
        itemType: "fileChange",
        lifecycle: "in_progress",
        method: "item/fileChange/patchUpdated",
        delta: [
          {
            path: "src/App.tsx",
            kind: { type: "update" },
            diff: "@@ -1 +1,2 @@\n-old\n+new\n+extra\n"
          },
          {
            path: "src/temporary.css",
            kind: { type: "add" },
            diff: "@@ -0,0 +1 @@\n+body {}\n"
          }
        ],
        createdAt: "3"
      },
      {
        type: "turn_diff_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        diff: "diff --git a/src/App.tsx b/src/App.tsx\n--- a/src/App.tsx\n+++ b/src/App.tsx\n@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
        createdAt: "4"
      }
    ]
  });

  assert.equal(withPatch.runs[0].itemsById["file-1"].fileChanges?.length, 2);
  assert.deepEqual(withPatch.runs[0].filesByPath["src/App.tsx"], {
    itemId: "file-1",
    path: "src/App.tsx",
    kind: "update",
    movePath: undefined,
    diff: "@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
    additions: 2,
    deletions: 1
  });
  assert.deepEqual(withPatch.runs[0].aggregateDiffStats, {
    additions: 2,
    deletions: 1,
    files: 1
  });
  assert.equal(withPatch.runs[0].aggregateDiffUpdatedAt, "4");

  const completed = demoSessionReducer(withPatch, {
    type: "append_agent_event",
    runId: "run-1",
    now: "5",
    event: {
      type: "item_completed",
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "file-1",
      itemType: "fileChange",
      lifecycle: "completed",
      status: "completed",
      completedAt: "5",
      item: {
        id: "file-1",
        type: "fileChange",
        status: "completed",
        changes: [{
          path: "src/App.tsx",
          kind: { type: "update", move_path: "src/Main.tsx" },
          diff: "@@ -1 +1 @@\n-old\n+new\n"
        }]
      },
      createdAt: "5"
    }
  });

  assert.deepEqual(Object.keys(completed.runs[0].filesByPath), ["src/App.tsx"]);
  assert.deepEqual(completed.runs[0].changedFiles, ["src/App.tsx"]);
  assert.equal(completed.runs[0].filesByPath["src/App.tsx"].movePath, "src/Main.tsx");
});

test("command execution streams a bounded output tail and preserves terminal metadata", () => {
  const running = startTestRun();
  const longOutput = "x".repeat(AGENT_COMMAND_OUTPUT_TAIL_LIMIT + 100);
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "5",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2026-07-13T00:00:00Z",
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run check",
          cwd: "/tmp/demo",
          status: "inProgress"
        },
        createdAt: "2"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        method: "item/commandExecution/outputDelta",
        delta: longOutput,
        createdAt: "3"
      },
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "completed",
        status: "failed",
        completedAt: "2026-07-13T00:00:02.5Z",
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run check",
          cwd: "/tmp/demo",
          status: "failed",
          exitCode: 1,
          durationMs: 2_500,
          aggregatedOutput: null
        },
        createdAt: "5"
      }
    ]
  });

  const item = updated.runs[0].itemsById["command-1"];
  assert.equal(item.command?.command, "npm run check");
  assert.equal(item.command?.cwd, "/tmp/demo");
  assert.equal(item.command?.status, "failed");
  assert.equal(item.command?.exitCode, 1);
  assert.equal(item.command?.durationMs, 2_500);
  assert.equal(item.command?.outputTail.length, AGENT_COMMAND_OUTPUT_TAIL_LIMIT);
  assert.equal(item.command?.outputTruncated, true);
  assert.equal(updated.runs[0].events.length, 0);
});

test("legacy file output delta stays a bounded compatibility detail", () => {
  const running = startTestRun();
  const updated = demoSessionReducer(running, {
    type: "append_agent_event",
    runId: "run-1",
    now: "3",
    event: {
      type: "item_delta",
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "file-legacy",
      itemType: "fileChange",
      lifecycle: "in_progress",
      method: "item/fileChange/outputDelta",
      delta: "Done!",
      createdAt: "3"
    }
  });

  assert.equal(updated.runs[0].itemsById["file-legacy"].output, "Done!");
  assert.deepEqual(updated.runs[0].itemsById["file-legacy"].fileChanges, []);
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

function startTestRun() {
  return demoSessionReducer(createTestSession(), {
    type: "start_agent_run",
    runId: "run-1",
    kind: "initial_build",
    prompt: "生成 demo",
    now: "2"
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
