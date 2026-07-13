import test from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { DemoProgressPanel } from "./DemoProgressPanel";
import { createDemoSession, demoSessionReducer } from "../utils/demoSession";

test("renders live file stats and completed command metadata from Agent items", () => {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-1",
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:02Z",
    events: [
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "file-1",
        itemType: "fileChange",
        lifecycle: "completed",
        status: "completed",
        completedAt: "2026-07-13T00:00:01Z",
        item: {
          id: "file-1",
          type: "fileChange",
          status: "completed",
          changes: [{
            path: "/tmp/demo/src/App.tsx",
            kind: { type: "update", move_path: "/tmp/demo/src/Main.tsx" },
            diff: "@@ -1 +1,2 @@\n-old\n+new\n+extra\n"
          }]
        },
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "completed",
        status: "completed",
        completedAt: "2026-07-13T00:00:02Z",
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run check",
          cwd: "/tmp/demo",
          status: "completed",
          exitCode: 0,
          durationMs: 1_200,
          aggregatedOutput: "tests passed"
        },
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "turn_diff_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        diff: "diff --git a/src/App.tsx b/src/Main.tsx\n--- a/src/App.tsx\n+++ b/src/Main.tsx\n@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-2",
        itemType: "commandExecution",
        lifecycle: "completed",
        status: "declined",
        completedAt: "2026-07-13T00:00:02Z",
        item: {
          id: "command-2",
          type: "commandExecution",
          command: "sudo dangerous-command",
          cwd: "/tmp/demo",
          status: "declined",
          exitCode: null,
          durationMs: 100,
          aggregatedOutput: null
        },
        createdAt: "2026-07-13T00:00:02Z"
      }
    ]
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assert.ok(/文件修改完成/.test(html));
  assert.ok(/src\/App\.tsx → src\/Main\.tsx/.test(html));
  assert.ok(/\+2/.test(html));
  assert.ok(/-1/.test(html));
  assert.ok(/命令执行完成/.test(html));
  assert.ok(/npm run check/.test(html));
  assert.ok(/cwd · \/tmp\/demo/.test(html));
  assert.ok(/tests passed/.test(html));
  assert.ok(/退出码 0/.test(html));
  assert.ok(/1\.2s/.test(html));
  assert.ok(/命令执行被拒绝/.test(html));
  assert.ok(/命令未获批准/.test(html));
});

test("renders file and command cards as soon as items start", () => {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-1",
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const started = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:01Z",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "file-1",
        itemType: "fileChange",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2026-07-13T00:00:00Z",
        item: { id: "file-1", type: "fileChange", status: "inProgress", changes: [] },
        createdAt: "2026-07-13T00:00:00Z"
      },
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: new Date().toISOString(),
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run build",
          cwd: "/tmp/demo",
          status: "inProgress"
        },
        createdAt: "2026-07-13T00:00:01Z"
      }
    ]
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={started} compact />);

  assert.ok(/正在修改文件/.test(html));
  assert.ok(/Codex 正在准备文件修改/.test(html));
  assert.ok(/正在执行命令/.test(html));
  assert.ok(/npm run build/.test(html));
  assert.ok(/命令正在执行，等待输出/.test(html));
});
