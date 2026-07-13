import test from "node:test";
import assert from "node:assert/strict";
import type { AgentEvent, DemoSession, DevServerLifecycleEventEnvelope, RequirementUtterance } from "../types/app";
import { AGENT_COMMAND_OUTPUT_TAIL_LIMIT, getAgentLatestProgressAt } from "./agentProgress";
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

test("reasoning summary assembles indexed deltas and completed summary stays authoritative", () => {
  const rawStartedText = "raw-started-reasoning-must-not-persist";
  const rawDeltaText = "raw-delta-reasoning-must-not-persist";
  const rawCompletedText = "raw-completed-reasoning-must-not-persist";
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:08Z",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "reasoning-1",
        itemType: "reasoning",
        lifecycle: "in_progress",
        startedAt: "2026-07-13T00:00:01Z",
        item: {
          id: "reasoning-1",
          type: "reasoning",
          summary: [],
          content: [{ type: "reasoning_text", text: rawStartedText }]
        },
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "reasoning-1",
        itemType: "reasoning",
        lifecycle: "in_progress",
        method: "item/reasoning/summaryPartAdded",
        delta: { summaryIndex: 1 },
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "reasoning-1",
        itemType: "reasoning",
        lifecycle: "in_progress",
        method: "item/reasoning/summaryTextDelta",
        delta: { summaryIndex: 1, delta: "第二部分" },
        createdAt: "2026-07-13T00:00:03Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "reasoning-1",
        itemType: "reasoning",
        lifecycle: "in_progress",
        method: "item/reasoning/summaryPartAdded",
        delta: { summaryIndex: 0 },
        createdAt: "2026-07-13T00:00:04Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "reasoning-1",
        itemType: "reasoning",
        lifecycle: "in_progress",
        method: "item/reasoning/summaryTextDelta",
        delta: { summaryIndex: 0, delta: "第一部分" },
        createdAt: "2026-07-13T00:00:05Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "reasoning-1",
        itemType: "reasoning",
        lifecycle: "in_progress",
        method: "item/reasoning/textDelta",
        delta: { summaryIndex: 0, contentIndex: 0, delta: rawDeltaText },
        createdAt: "2026-07-13T00:00:06Z"
      },
      completedItem("reasoning-1", "reasoning", {
        id: "reasoning-1",
        type: "reasoning",
        summary: ["最终摘要一", "最终摘要二"],
        content: [{ type: "reasoning_text", text: rawCompletedText }]
      }, "2026-07-13T00:00:07Z")
    ]
  });

  const item = updated.runs[0].itemsById["reasoning-1"];
  assert.deepEqual(item.reasoningSummaryParts, ["最终摘要一", "最终摘要二"]);
  assert.equal(item.reasoningSummary, "最终摘要一\n\n最终摘要二");
  assert.equal(item.restrictedDebugAvailable, true);
  assert.deepEqual(item.presentation, {
    kind: "reasoning",
    summary: "最终摘要一\n\n最终摘要二",
    rawTextAvailable: true
  });
  const serialized = JSON.stringify(updated.runs[0]);
  assert.equal(serialized.includes(rawStartedText), false);
  assert.equal(serialized.includes(rawDeltaText), false);
  assert.equal(serialized.includes(rawCompletedText), false);
  assert.equal(serialized.includes("contentIndex"), false);
});

test("UI-safe lifecycle projections preserve restricted payload availability metadata", () => {
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:03Z",
    events: [
      completedItem("reasoning-projected", "reasoning", {
        id: "reasoning-projected",
        type: "reasoning",
        summary: ["已完成协议投影"],
        rawTextAvailable: true,
        contentCount: 2
      }, "2026-07-13T00:00:01Z"),
      completedItem("image-projected", "imageGeneration", {
        id: "image-projected",
        type: "imageGeneration",
        status: "completed",
        savedPath: "/tmp/demo/projected.png",
        resultAvailable: true,
        resultLength: 42_000
      }, "2026-07-13T00:00:02Z")
    ]
  });

  const reasoning = updated.runs[0].itemsById["reasoning-projected"];
  const image = updated.runs[0].itemsById["image-projected"];
  assert.equal(reasoning.restrictedDebugAvailable, true);
  assert.equal(reasoning.data.contentCount, 2);
  assert.equal(reasoning.presentation?.kind, "reasoning");
  assert.equal(image.data.resultAvailable, true);
  assert.equal(image.data.resultLength, 42_000);
  assert.equal(image.presentation?.kind, "image");
  assert.equal(JSON.stringify(updated.runs[0]).includes('"content"'), false);
  assert.equal(JSON.stringify(updated.runs[0]).includes('"result"'), false);
});

test("debug-only protocol diagnostics stay out of the user timeline", () => {
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:02Z",
    events: [
      {
        type: "diagnostic",
        level: "debug",
        message: "收到尚未映射的 app-server notification",
        method: "turn/moderationMetadata",
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "diagnostic",
        level: "warning",
        message: "主动请求尚未接入 UI",
        method: "item/tool/requestUserInput",
        createdAt: "2026-07-13T00:00:02Z"
      }
    ]
  });

  assert.deepEqual(updated.runs[0].events.map((event) => event.type === "diagnostic" ? event.level : event.type), ["warning"]);
});

test("terminal interaction counts updates without retaining stdin", () => {
  const stdinSecret = "terminal-stdin-secret";
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:04Z",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-interactive",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2026-07-13T00:00:01Z",
        item: {
          id: "command-interactive",
          type: "commandExecution",
          command: "read TOKEN",
          stdin: stdinSecret,
          status: "inProgress"
        },
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-interactive",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        method: "item/commandExecution/terminalInteraction",
        delta: { stdin: stdinSecret },
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-interactive",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        method: "item/commandExecution/terminalInteraction",
        delta: { stdin: `${stdinSecret}-again` },
        createdAt: "2026-07-13T00:00:03Z"
      }
    ]
  });

  assert.equal(updated.runs[0].itemsById["command-interactive"].terminalInteractionCount, 2);
  const serialized = JSON.stringify(updated.runs[0]);
  assert.equal(serialized.includes(stdinSecret), false);
  assert.equal(serialized.includes('"stdin"'), false);
});

test("MCP tool calls retain latest progress and completed success or failure metadata", () => {
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:08Z",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "mcp-success",
        itemType: "mcpToolCall",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2026-07-13T00:00:01Z",
        item: {
          id: "mcp-success",
          type: "mcpToolCall",
          server: "docs",
          tool: "search",
          status: "inProgress",
          arguments: { query: "Codex" }
        },
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "mcp-success",
        itemType: "mcpToolCall",
        lifecycle: "in_progress",
        method: "item/mcpToolCall/progress",
        delta: { message: "正在检索" },
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "item_delta",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "mcp-success",
        itemType: "mcpToolCall",
        lifecycle: "in_progress",
        method: "item/mcpToolCall/progress",
        delta: { message: "正在整理结果" },
        createdAt: "2026-07-13T00:00:03Z"
      },
      completedItem("mcp-success", "mcpToolCall", {
        id: "mcp-success",
        type: "mcpToolCall",
        server: "docs",
        tool: "search",
        status: "completed",
        durationMs: 1_250,
        arguments: { query: "Codex" },
        result: { matches: 3 }
      }, "2026-07-13T00:00:04Z"),
      completedItem("mcp-failed", "mcpToolCall", {
        id: "mcp-failed",
        type: "mcpToolCall",
        server: "browser",
        tool: "open",
        status: "failed",
        durationMs: 800,
        arguments: { url: "https://example.test" },
        error: { message: "connection refused" }
      }, "2026-07-13T00:00:05Z")
    ]
  });

  const success = updated.runs[0].itemsById["mcp-success"];
  const failure = updated.runs[0].itemsById["mcp-failed"];
  assert.equal(success.progressMessage, "正在整理结果");
  const successPresentation = success.presentation;
  const failurePresentation = failure.presentation;
  if (successPresentation?.kind !== "toolCall" || failurePresentation?.kind !== "toolCall") {
    throw new Error("expected MCP tool call presentations");
  }
  assert.equal(successPresentation.status, "completed");
  assert.equal(successPresentation.durationMs, 1_250);
  assert.equal(successPresentation.progress, "正在整理结果");
  assert.ok(/\"matches\": 3/.test(successPresentation.result?.text ?? ""));
  assert.equal(failurePresentation.status, "failed");
  assert.equal(failurePresentation.durationMs, 800);
  assert.equal(failurePresentation.error, "connection refused");
});

test("remaining item types hydrate into bounded domain presentations", () => {
  const imagePayload = `data:image/png;base64,${"A".repeat(24_000)}`;
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:20Z",
    events: [
      completedItem("dynamic-1", "dynamicToolCall", {
        id: "dynamic-1",
        type: "dynamicToolCall",
        namespace: "workspace",
        tool: "inspect",
        status: "completed",
        durationMs: 50,
        success: true,
        arguments: { path: "src/App.tsx" },
        contentItems: [{ type: "inputText", text: "done" }]
      }),
      completedItem("collab-1", "collabAgentToolCall", {
        id: "collab-1",
        type: "collabAgentToolCall",
        tool: "spawnAgent",
        status: "completed",
        senderThreadId: "thread-1",
        receiverThreadIds: ["thread-child"],
        prompt: "检查协议实现",
        agentsStates: {
          "thread-child": { status: "completed", message: "审计完成" }
        }
      }),
      completedItem("subagent-1", "subAgentActivity", {
        id: "subagent-1",
        type: "subAgentActivity",
        kind: "interacted",
        agentThreadId: "thread-child",
        agentPath: "/root/protocol"
      }),
      completedItem("web-1", "webSearch", {
        id: "web-1",
        type: "webSearch",
        status: "completed",
        query: "Codex app-server",
        action: { type: "openPage", url: "https://developers.openai.com/codex" }
      }),
      completedItem("image-view-1", "imageView", {
        id: "image-view-1",
        type: "imageView",
        status: "completed",
        path: "/tmp/demo/reference.png"
      }),
      completedItem("image-generation-1", "imageGeneration", {
        id: "image-generation-1",
        type: "imageGeneration",
        status: "completed",
        revisedPrompt: "A compact UI",
        result: imagePayload,
        savedPath: "/tmp/demo/generated.png"
      }),
      completedItem("context-1", "contextCompaction", {
        id: "context-1",
        type: "contextCompaction",
        status: "completed"
      }),
      completedItem("sleep-1", "sleep", {
        id: "sleep-1",
        type: "sleep",
        status: "completed",
        durationMs: 2_000
      }),
      completedItem("review-enter-1", "enteredReviewMode", {
        id: "review-enter-1",
        type: "enteredReviewMode",
        status: "completed",
        review: "检查关键路径"
      }),
      completedItem("review-exit-1", "exitedReviewMode", {
        id: "review-exit-1",
        type: "exitedReviewMode",
        status: "completed",
        review: "检查完成"
      }),
      completedItem("unknown-1", "futureProtocolItem", {
        id: "unknown-1",
        type: "futureProtocolItem",
        status: "completed",
        label: "future event",
        nested: { enabled: true }
      })
    ]
  });

  assert.deepEqual(updated.runs[0].itemOrder, [
    "dynamic-1",
    "collab-1",
    "subagent-1",
    "web-1",
    "image-view-1",
    "image-generation-1",
    "context-1",
    "sleep-1",
    "review-enter-1",
    "review-exit-1",
    "unknown-1"
  ]);

  const presentations = Object.fromEntries(
    Object.entries(updated.runs[0].itemsById).map(([id, item]) => [id, item.presentation])
  );
  assert.deepEqual(presentations["dynamic-1"], {
    kind: "toolCall",
    toolKind: "dynamic",
    server: undefined,
    namespace: "workspace",
    tool: "inspect",
    status: "completed",
    durationMs: 50,
    progress: undefined,
    success: true,
    arguments: { text: "{\n  \"path\": \"src/App.tsx\"\n}", truncated: false },
    result: {
      text: "[\n  {\n    \"type\": \"inputText\",\n    \"text\": \"done\"\n  }\n]",
      truncated: false
    },
    error: undefined
  });
  assert.equal(presentations["collab-1"]?.kind, "collaboration");
  assert.equal(presentations["subagent-1"]?.kind, "collaboration");
  assert.deepEqual(presentations["web-1"], {
    kind: "webSearch",
    action: "openPage",
    query: "Codex app-server",
    url: "https://developers.openai.com/codex",
    pattern: undefined
  });
  assert.equal(presentations["image-view-1"]?.kind, "image");
  assert.deepEqual(presentations["image-generation-1"], {
    kind: "image",
    activityKind: "generation",
    status: "completed",
    path: undefined,
    savedPath: "/tmp/demo/generated.png",
    revisedPrompt: { text: "A compact UI", truncated: false },
    resultAvailable: true
  });
  assert.equal(presentations["context-1"]?.kind, "status");
  assert.equal(presentations["sleep-1"]?.kind, "status");
  assert.equal(presentations["review-enter-1"]?.kind, "status");
  assert.equal(presentations["review-exit-1"]?.kind, "status");
  assert.equal(presentations["unknown-1"]?.kind, "status");
  assert.equal(updated.runs[0].itemsById["image-generation-1"].data.resultAvailable, true);
  assert.equal(updated.runs[0].itemsById["image-generation-1"].data.resultLength, imagePayload.length);
  assert.equal(JSON.stringify(updated.runs[0]).includes(imagePayload), false);
});

test("structured item previews redact secrets and omit large base64 payloads", () => {
  const passwordSecret = "password-value-must-not-persist";
  const apiKeySecret = "api-key-value-must-not-persist";
  const embeddedCredential = "sk-abcdefghijklmnopqrstuvwxyz123456";
  const signedUrl = "https://example.test/file?X-Amz-Signature=signed-url-must-not-persist";
  const base64Payload = `data:image/png;base64,${"B".repeat(24_000)}`;
  const compactBase64Payload = "Q".repeat(64);
  const collabPromptSecret = "token=collab-prompt-secret-must-not-persist";
  const widePayload = Object.fromEntries(
    Array.from({ length: 500 }, (_, index) => [`field-${index}`, { value: `value-${index}` }])
  );
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:04Z",
    events: [
      completedItem("dynamic-secret", "dynamicToolCall", {
        id: "dynamic-secret",
        type: "dynamicToolCall",
        namespace: "private",
        tool: "upload",
        status: "completed",
        success: true,
        arguments: {
          password: passwordSecret,
          apiKey: apiKeySecret,
          image: base64Payload,
          imageData: compactBase64Payload,
          opaqueValue: embeddedCredential,
          downloadUrl: signedUrl
        },
        contentItems: { authorization: "Bearer hidden-token" }
      }),
      completedItem("unknown-secret", "futureSecretItem", {
        id: "unknown-secret",
        type: "futureSecretItem",
        status: "completed",
        credentials: { secret: "unknown-secret-value" },
        payload: base64Payload,
        widePayload
      }),
      completedItem("collab-secret", "collabAgentToolCall", {
        id: "collab-secret",
        type: "collabAgentToolCall",
        tool: "spawnAgent",
        status: "completed",
        receiverThreadIds: [],
        prompt: collabPromptSecret,
        agentsStates: {}
      })
    ]
  });

  const serialized = JSON.stringify(updated.runs[0]);
  assert.equal(serialized.includes(passwordSecret), false);
  assert.equal(serialized.includes(apiKeySecret), false);
  assert.equal(serialized.includes(embeddedCredential), false);
  assert.equal(serialized.includes(signedUrl), false);
  assert.equal(serialized.includes("hidden-token"), false);
  assert.equal(serialized.includes("unknown-secret-value"), false);
  assert.equal(serialized.includes(base64Payload), false);
  assert.equal(serialized.includes(compactBase64Payload), false);
  assert.equal(serialized.includes(collabPromptSecret), false);
  assert.equal(serialized.includes("field-499"), false);
  assert.ok(/\[REDACTED\]/.test(serialized));
  assert.ok(/large string omitted/.test(serialized));
  assert.ok(/encoded payload omitted/.test(serialized));
  assert.ok(/credential-like text redacted/.test(serialized));
  assert.ok(/additional fields omitted/.test(serialized));
  const collabPresentation = updated.runs[0].itemsById["collab-secret"].presentation;
  assert.equal(collabPresentation?.kind, "collaboration");
  if (collabPresentation?.kind === "collaboration") {
    assert.equal(collabPresentation.prompt?.truncated, true);
  }
});

test("hook runs upsert once and completed state resists duplicate or stale started events", () => {
  const started = {
    type: "hook_run_updated" as const,
    threadId: "thread-1",
    turnId: "turn-1",
    hookId: "hook-1",
    lifecycle: "in_progress" as const,
    run: {
      displayOrder: 2,
      eventName: "postToolUse",
      handlerType: "command",
      executionMode: "sync",
      status: "running",
      startedAt: 1_783_900_800_000,
      completedAt: null,
      entries: [{ kind: "context", text: "starting" }]
    },
    createdAt: "2026-07-13T00:00:01Z"
  };
  const completed = {
    type: "hook_run_updated" as const,
    threadId: "thread-1",
    turnId: "turn-1",
    hookId: "hook-1",
    lifecycle: "completed" as const,
    run: {
      displayOrder: 2,
      eventName: "postToolUse",
      handlerType: "command",
      executionMode: "sync",
      status: "completed",
      statusMessage: "hook finished",
      durationMs: 125,
      startedAt: 1_783_900_800_000,
      completedAt: 1_783_900_800_125,
      _uiProjectionTruncated: true,
      entries: [{ kind: "feedback", text: "completed output" }]
    },
    createdAt: "2026-07-13T00:00:02Z"
  };
  const staleStarted = {
    ...started,
    run: { ...started.run, status: "running", entries: [{ kind: "context", text: "stale" }] },
    createdAt: "2026-07-13T00:00:04Z"
  };
  const earlierDisplayHook = {
    ...completed,
    hookId: "hook-0",
    run: {
      ...completed.run,
      displayOrder: 1,
      startedAt: 1_783_900_799_000,
      completedAt: 1_783_900_799_050
    },
    createdAt: "2026-07-13T00:00:05Z"
  };
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:04Z",
    events: [started, started, completed, completed, staleStarted, earlierDisplayHook]
  });

  const hook = updated.runs[0].hooksById["hook-1"];
  assert.deepEqual(updated.runs[0].hookOrder, ["hook-0", "hook-1"]);
  assert.equal(hook.lifecycle, "completed");
  assert.equal(hook.status, "completed");
  assert.equal(hook.displayOrder, 2);
  assert.equal(hook.startedAt, "2026-07-13T00:00:00.000Z");
  assert.equal(hook.completedAt, "2026-07-13T00:00:00.125Z");
  assert.equal(hook.durationMs, 125);
  assert.equal(hook.restrictedDebugAvailable, true);
  assert.deepEqual(hook.entries, [{ kind: "feedback", text: "completed output" }]);

  const previousOrder = updated.runs[0].hookOrder;
  const refreshed = demoSessionReducer(updated, {
    type: "append_agent_event",
    runId: "run-1",
    now: "2026-07-13T00:00:06Z",
    event: { ...completed, createdAt: "2026-07-13T00:00:06Z" }
  });
  assert.deepEqual(updated.runs[0].hookOrder, ["hook-0", "hook-1"]);
  assert.deepEqual(refreshed.runs[0].hookOrder, ["hook-0", "hook-1"]);
  assert.ok(refreshed.runs[0].hookOrder !== previousOrder);
});

test("token usage is latest-wins without advancing real progress", () => {
  const withProgress = demoSessionReducer(startTestRun(), {
    type: "append_agent_event",
    runId: "run-1",
    now: "2026-07-13T00:00:10Z",
    event: {
      type: "agent_message",
      text: "正在修改文件",
      createdAt: "2026-07-13T00:00:10Z"
    }
  });
  const updated = demoSessionReducer(withProgress, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:30Z",
    events: [
      tokenUsageEvent(100, 20, "2026-07-13T00:00:20Z"),
      tokenUsageEvent(250, 40, "2026-07-13T00:00:30Z")
    ]
  });

  assert.equal(updated.runs[0].tokenUsage?.total.totalTokens, 250);
  assert.equal(updated.runs[0].tokenUsage?.last.totalTokens, 40);
  assert.equal(updated.runs[0].tokenUsage?.updatedAt, "2026-07-13T00:00:30Z");
  assert.equal(getAgentLatestProgressAt(updated.runs[0]), "2026-07-13T00:00:10Z");
});

test("model safety and verification snapshots accept explicit clearing updates", () => {
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:04Z",
    events: [
      {
        type: "model_safety_buffering_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        model: "gpt-5.4",
        useCases: ["coding"],
        reasons: ["safety evaluation"],
        showBufferingUi: true,
        fasterModel: "gpt-5.4-mini",
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "model_safety_buffering_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        model: "gpt-5.4",
        useCases: [],
        reasons: [],
        showBufferingUi: false,
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "model_verification_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        verifications: ["organization policy"],
        createdAt: "2026-07-13T00:00:03Z"
      },
      {
        type: "model_verification_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        verifications: [],
        createdAt: "2026-07-13T00:00:04Z"
      }
    ]
  });

  assert.deepEqual(updated.runs[0].modelSafetyBuffering, {
    threadId: "thread-1",
    turnId: "turn-1",
    model: "gpt-5.4",
    useCases: [],
    reasons: [],
    showBufferingUi: false,
    fasterModel: undefined,
    createdAt: "2026-07-13T00:00:02Z"
  });
  assert.deepEqual(updated.runs[0].modelVerification, {
    threadId: "thread-1",
    turnId: "turn-1",
    verifications: [],
    createdAt: "2026-07-13T00:00:04Z"
  });
});

test("runtime config and guardian warnings use distinct severity and de-duplicate", () => {
  const updated = demoSessionReducer(startTestRun(), {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:06Z",
    events: [
      {
        type: "warning",
        message: "runtime warning",
        threadId: "thread-1",
        turnId: "turn-1",
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "warning",
        message: "runtime warning",
        threadId: "thread-1",
        turnId: "turn-1",
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "config_warning",
        summary: "invalid config",
        details: "unknown key",
        path: "/tmp/demo/.codex/config.toml",
        range: {
          start: { line: 3, column: 1 },
          end: { line: 3, column: 12 }
        },
        createdAt: "2026-07-13T00:00:03Z"
      },
      {
        type: "config_warning",
        summary: "invalid config",
        details: "unknown key",
        path: "/tmp/demo/.codex/config.toml",
        range: {
          start: { line: 3, column: 1 },
          end: { line: 3, column: 12 }
        },
        createdAt: "2026-07-13T00:00:04Z"
      },
      {
        type: "guardian_warning",
        message: "policy requires attention",
        threadId: "thread-1",
        createdAt: "2026-07-13T00:00:05Z"
      },
      {
        type: "guardian_warning",
        message: "policy requires attention",
        threadId: "thread-1",
        createdAt: "2026-07-13T00:00:06Z"
      }
    ]
  });

  assert.equal(updated.runs[0].warnings.length, 3);
  assert.deepEqual(updated.runs[0].warnings.map((warning) => ({
    source: warning.source,
    severity: warning.severity,
    count: warning.count,
    updatedAt: warning.updatedAt
  })), [
    { source: "runtime", severity: "warning", count: 2, updatedAt: "2026-07-13T00:00:02Z" },
    { source: "config", severity: "warning", count: 2, updatedAt: "2026-07-13T00:00:04Z" },
    { source: "guardian", severity: "important", count: 2, updatedAt: "2026-07-13T00:00:06Z" }
  ]);
  assert.equal(updated.runs[0].warnings[1].details, "unknown key");
  assert.equal(updated.runs[0].warnings[1].path, "/tmp/demo/.codex/config.toml");
  assert.deepEqual(updated.runs[0].warnings[1].range, {
    start: { line: 3, column: 1 },
    end: { line: 3, column: 12 }
  });
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

function completedItem(
  itemId: string,
  itemType: string,
  item: Record<string, unknown>,
  createdAt = "2026-07-13T00:00:01Z"
): Extract<AgentEvent, { type: "item_completed" }> {
  return {
    type: "item_completed",
    threadId: "thread-1",
    turnId: "turn-1",
    itemId,
    itemType,
    lifecycle: "completed",
    status: typeof item.status === "string" ? item.status : "completed",
    completedAt: createdAt,
    item,
    createdAt
  };
}

function tokenUsageEvent(
  totalTokens: number,
  lastTokens: number,
  createdAt: string
): Extract<AgentEvent, { type: "token_usage_updated" }> {
  return {
    type: "token_usage_updated",
    threadId: "thread-1",
    turnId: "turn-1",
    tokenUsage: {
      total: {
        totalTokens,
        inputTokens: totalTokens - 10,
        cachedInputTokens: 10,
        outputTokens: 10,
        reasoningOutputTokens: 5
      },
      last: {
        totalTokens: lastTokens,
        inputTokens: lastTokens - 5,
        cachedInputTokens: 5,
        outputTokens: 5,
        reasoningOutputTokens: 2
      },
      modelContextWindow: 200_000
    },
    createdAt
  };
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
